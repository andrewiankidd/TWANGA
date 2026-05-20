// Tauri-side bridge for the user tunings file at
// `$CONFIG/twanga/tunings.toml` (the same file `twanga tunings add`
// writes). Strategy: on app startup, parse the TOML into the same
// `Array<PresetEntry>` shape `user-tunings.js` keeps in `localStorage`,
// and mirror that into the existing `twanga-user-tunings-v1` key.
// `saveUserTunings()` in `user-tunings.js` calls `mirrorToDisk()` on
// every write to keep the file in sync.
//
// Why a write-through cache rather than reading on every access:
// `loadUserTunings` is called by sync code paths all over the
// frontend (Tuner picker, Recorder picker, tuning-controller…), and
// Tauri commands are async. The cleanest fit is "localStorage IS the
// truth at runtime; the TOML file IS the truth across restarts; we
// reconcile both at boot." If the CLI edits the file while the GUI
// is open, the GUI sees stale state until next startup — accepted
// trade-off, matches the browser build's "no live sync" assumption.

const USER_TUNINGS_KEY = 'twanga-user-tunings-v1';

function invokeFn() {
    if (typeof window === 'undefined') return null;
    const t = window.__TAURI__;
    if (!t) return null;
    return t.core?.invoke ?? t.invoke ?? null;
}

/// Read `tunings.toml`, parse, and write the resulting
/// `Array<PresetEntry>` into `localStorage[twanga-user-tunings-v1]`.
/// No-op (returns false) outside Tauri. Returns true on success.
/// Errors are caught and logged — a malformed tunings.toml shouldn't
/// crash the app, just shows the user with an empty set until they
/// fix the file (or `twanga tunings remove`).
export async function bootstrapFromDisk() {
    const invoke = invokeFn();
    if (!invoke) return false;
    try {
        const text = await invoke('read_tunings_toml');
        const entries = parseTuningsToml(text ?? '');
        try {
            localStorage.setItem(USER_TUNINGS_KEY, JSON.stringify(entries));
        } catch (e) {
            console.warn('tauri bootstrap: localStorage write failed', e);
        }
        return true;
    } catch (e) {
        console.warn('tauri bootstrap: read_tunings_toml failed', e);
        return false;
    }
}

/// Serialise `entries` back to the CLI's TOML schema and write via
/// Tauri. Fire-and-forget — failures are logged but don't propagate
/// to the caller. `user-tunings.js#saveUserTunings` already wrote to
/// `localStorage` synchronously by the time this runs, so the
/// in-memory state is correct even if the disk write fails.
export async function mirrorToDisk(entries) {
    const invoke = invokeFn();
    if (!invoke) return;
    try {
        const text = serialiseTuningsToml(entries);
        await invoke('write_tunings_toml', { contents: text });
    } catch (e) {
        console.warn('tauri: write_tunings_toml failed', e);
    }
}

/// Minimal TOML serialiser scoped to the tunings file schema (which
/// is fixed and small). Avoids pulling a full TOML library into the
/// frontend. Schema:
///
///   [[tunings]]
///   slug = "..."
///   name = "..."
///   strings = [
///       { name = "...", midi = N },
///       …
///   ]
///
/// The CLI's serde-toml roundtrips this shape exactly.
function serialiseTuningsToml(entries) {
    const out = [];
    for (const entry of entries ?? []) {
        if (!entry || typeof entry !== 'object') continue;
        out.push('[[tunings]]');
        out.push(`slug = ${tomlString(entry.slug ?? '')}`);
        out.push(`name = ${tomlString(entry.name ?? '')}`);
        const strs = Array.isArray(entry.strings) ? entry.strings : [];
        if (strs.length === 0) {
            out.push('strings = []');
        } else {
            out.push('strings = [');
            for (const s of strs) {
                const midi = Number.isFinite(s?.midi) ? s.midi : 0;
                out.push(`    { name = ${tomlString(s?.name ?? '')}, midi = ${midi} },`);
            }
            out.push(']');
        }
        out.push('');
    }
    return out.join('\n');
}

function tomlString(s) {
    // TOML basic-string rules: backslash + double-quote escape; embedded
    // newlines forbidden. None of our tuning names contain those in
    // practice, but escape defensively.
    const escaped = String(s)
        .replace(/\\/g, '\\\\')
        .replace(/"/g, '\\"')
        .replace(/\n/g, '\\n');
    return `"${escaped}"`;
}

/// Minimal TOML parser scoped to the tunings file schema. Tolerant
/// of whitespace + comments; rejects anything outside the expected
/// shape (returns an empty list rather than throwing — corrupt
/// disk state should degrade to "no tunings", not "app dead").
///
/// We don't pull a real TOML parser because the schema is tiny and
/// the JS-side dependency budget is one of our levers.
function parseTuningsToml(text) {
    const lines = text.split(/\r?\n/);
    const entries = [];
    let current = null;
    let inStringsArray = false;
    for (let raw of lines) {
        const line = stripComment(raw).trim();
        if (line === '') continue;
        if (line === '[[tunings]]') {
            if (current) entries.push(current);
            current = { slug: '', name: '', strings: [] };
            inStringsArray = false;
            continue;
        }
        if (!current) continue; // skip stray lines outside a table
        if (inStringsArray) {
            if (line === ']') {
                inStringsArray = false;
                continue;
            }
            // Per-string inline table: { name = "...", midi = N }
            const m = line.match(/^\{\s*name\s*=\s*"((?:[^"\\]|\\.)*)"\s*,\s*midi\s*=\s*(-?\d+)\s*\}[,\s]*$/);
            if (m) {
                current.strings.push({
                    name: unescapeTomlString(m[1]),
                    midi: Number.parseInt(m[2], 10),
                });
            }
            continue;
        }
        const kv = line.match(/^(\w+)\s*=\s*(.+?)\s*$/);
        if (!kv) continue;
        const [, key, value] = kv;
        if (key === 'slug' || key === 'name') {
            const str = value.match(/^"((?:[^"\\]|\\.)*)"$/);
            if (str) current[key] = unescapeTomlString(str[1]);
        } else if (key === 'strings') {
            if (value === '[]') {
                current.strings = [];
            } else if (value === '[') {
                inStringsArray = true;
            }
            // Other shapes (single-line array) aren't generated by our
            // serialiser, but tolerated by being ignored.
        }
    }
    if (current) entries.push(current);
    return entries.filter((e) => e.slug && e.name);
}

function stripComment(line) {
    // TOML `#` starts a comment unless inside a string. Our strings
    // can't legitimately contain `#` followed by content we'd want
    // to keep, so a naive split is fine for this constrained schema.
    const hashIdx = line.indexOf('#');
    if (hashIdx === -1) return line;
    // Don't strip if the `#` is inside a quoted string.
    const openQuotes = (line.slice(0, hashIdx).match(/"/g) ?? []).length;
    if (openQuotes % 2 === 1) return line;
    return line.slice(0, hashIdx);
}

function unescapeTomlString(s) {
    return s
        .replace(/\\n/g, '\n')
        .replace(/\\"/g, '"')
        .replace(/\\\\/g, '\\');
}
