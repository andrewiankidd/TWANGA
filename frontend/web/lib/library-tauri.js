// Tauri filesystem backend for the tab library — same interface as
// `./library.js` but reads from / writes to two filesystem dirs:
// `<data-root>/recordings/` (live captures) and `<data-root>/library/`
// (imported tabs) via Tauri commands defined in
// `crates/twanga-app/src/commands.rs`.
//
// The dispatcher in `library.js` picks this backend at runtime when
// `window.__TAURI__` is defined. In the browser build this file is
// imported but never called, so the JS shim cost is minimal.
//
// Id namespacing — Tauri-side ids encode their source dir so the
// shim can route subsequent load / update / delete to the right
// command without an extra round-trip:
//
//   `recording:<filename>` → `<data-root>/recordings/<filename>`
//   `library:<filename>`   → `<data-root>/library/<filename>`
//
// Bare filenames (no prefix) are treated as recordings for
// historical compatibility — but no current code path emits one.
//
// `list()` merges both dirs into one list sorted newest-first so
// consumers see a single library that matches the browser-IDB
// experience.

const PREFIX_RECORDING = 'recording:';
const PREFIX_LIBRARY = 'library:';

/// Resolve Tauri's `invoke` function. Tauri 2 exposes it at
/// `window.__TAURI__.core.invoke` when `withGlobalTauri` is enabled
/// (it is in our `tauri.conf.json`). Returns `null` outside Tauri.
function invokeFn() {
    if (typeof window === 'undefined') return null;
    const t = window.__TAURI__;
    if (!t) return null;
    return t.core?.invoke ?? t.invoke ?? null;
}

function ensureInvoke() {
    const fn = invokeFn();
    if (!fn) {
        throw new Error(
            'library-tauri.js called outside Tauri (window.__TAURI__ not defined)'
        );
    }
    return fn;
}

/// Split a prefixed id into `{ backend, filename }`. `backend` is one
/// of `'recording'` / `'library'`. Bare filenames (no `:` separator
/// recognised here) fall back to `'recording'` so any older bookmarks
/// still load — the only consumer that mints ids today is this shim
/// itself, so this fallback is purely defensive.
function decodeId(id) {
    if (typeof id !== 'string' || id.length === 0) {
        throw new Error('library-tauri: id must be a non-empty string');
    }
    if (id.startsWith(PREFIX_LIBRARY)) {
        return { backend: 'library', filename: id.slice(PREFIX_LIBRARY.length) };
    }
    if (id.startsWith(PREFIX_RECORDING)) {
        return { backend: 'recording', filename: id.slice(PREFIX_RECORDING.length) };
    }
    return { backend: 'recording', filename: id };
}

/// Inverse: combine a backend tag + a bare filename back into the
/// prefixed id exposed to the rest of the frontend.
function encodeId(backend, filename) {
    const prefix = backend === 'library' ? PREFIX_LIBRARY : PREFIX_RECORDING;
    return `${prefix}${filename}`;
}

/// `library.list()` over Tauri. Pulls both the recordings dir AND
/// the imports library, merges them, sorts newest-first. Bundled
/// examples are NOT included — they go through the existing
/// fetch-based path in `library.js` (works identically under Tauri
/// because `frontendDist` serves them).
export async function list() {
    const invoke = ensureInvoke();
    const [recRows, libRows] = await Promise.all([
        invoke('list_recordings'),
        invoke('list_library_tabs'),
    ]);
    const recordings = (recRows ?? []).map((r) => ({
        id: encodeId('recording', r.id),
        title: r.title || `Recording ${r.id}`,
        source: r.source ?? 'user',
        createdAt: r.created_at ?? null,
        lastBackedUpAt: r.last_backed_up_at ?? null,
    }));
    const imports = (libRows ?? []).map((r) => ({
        id: encodeId('library', r.id),
        title: r.title || `Imported ${r.id}`,
        source: r.source ?? 'imported',
        createdAt: r.created_at ?? null,
        lastBackedUpAt: r.last_backed_up_at ?? null,
    }));
    // Same ordering as the browser-IDB backend: createdAt DESC,
    // unknown timestamps (None) sort to the bottom.
    return [...recordings, ...imports].sort(
        (a, b) => (b.createdAt ?? 0) - (a.createdAt ?? 0),
    );
}

/// `library.load(id)` over Tauri. The id carries the source prefix;
/// we route to the right Tauri command based on it. Path-traversal
/// protection runs on the Rust side.
export async function load(id) {
    const invoke = ensureInvoke();
    const { backend, filename } = decodeId(id);
    const command = backend === 'library' ? 'load_library_tab' : 'load_recording';
    const row = await invoke(command, { id: filename });
    return {
        id: encodeId(backend, row.id),
        title: row.title || `Tab ${row.id}`,
        source: row.source ?? (backend === 'library' ? 'imported' : 'user'),
        alphatex: row.alphatex,
        createdAt: row.created_at ?? null,
        lastBackedUpAt: row.last_backed_up_at ?? null,
    };
}

/// `library.save({...})` over Tauri. Returns the new prefixed id.
/// `source: 'imported'` routes to the library dir; anything else
/// (including the default `'user'`) routes to the recordings dir.
export async function save({ title, alphatex, source = 'user' }) {
    if (typeof alphatex !== 'string' || alphatex.length === 0) {
        throw new Error('save: alphatex content is required');
    }
    const invoke = ensureInvoke();
    const isImport = source === 'imported';
    const command = isImport ? 'save_library_tab' : 'save_recording';
    const args = isImport
        ? { title: title ?? '', alphatex }
        : { title: title ?? '', alphatex, source };
    const filename = await invoke(command, args);
    return encodeId(isImport ? 'library' : 'recording', filename);
}

/// `library.update({id, title?, alphatex})` over Tauri. Writes the
/// new contents to the existing file at whichever dir the id points
/// to. `title` is accepted but currently unused on the Rust side —
/// alphaTex's `\title` IS the title, and the frontend inlines it
/// into the body before calling update.
export async function update({ id, title, alphatex }) {
    if (typeof alphatex !== 'string' || alphatex.length === 0) {
        throw new Error('update: alphatex content is required');
    }
    const invoke = ensureInvoke();
    const { backend, filename } = decodeId(id);
    const command = backend === 'library' ? 'update_library_tab' : 'update_recording';
    await invoke(command, { id: filename, title, alphatex });
}

/// `library.deleteTab(id)` over Tauri. Routes by id prefix.
export async function deleteTab(id) {
    const invoke = ensureInvoke();
    const { backend, filename } = decodeId(id);
    const command = backend === 'library' ? 'delete_library_tab' : 'delete_recording';
    await invoke(command, { id: filename });
}

/// No-op on Tauri — the filesystem IS the backup. The browser
/// build's "Download" button + "Backed up <when>" tag exist because
/// IDB can be evicted; that doesn't apply to a real file in
/// `<data-root>/recordings/` or `<data-root>/library/`.
export async function markDownloaded(_id, _when) { /* no-op */ }

/// Always-persistent on the filesystem. The browser build needs to
/// ask the user for `navigator.storage.persist`; Tauri doesn't.
export async function requestPersistence() { return true; }

/// Cross-process change notifications. Filesystem-watch is the
/// eventual implementation (a Tauri event posted when a file under
/// `recordings/` or `library/` changes); for first cut this is a
/// no-op because the desktop app is single-window, single-instance
/// and the user can refresh manually.
export function subscribe(_callback) { return () => {}; }
