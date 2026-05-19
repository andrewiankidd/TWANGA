// User-defined tunings storage layer.
//
// Same `PresetEntry` schema the CLI writes to
// `$CONFIG/twanga/tunings.toml`, persisted to `localStorage` under the
// `twanga-user-tunings-v1` key. A future Tauri command will bridge the
// two surfaces so a custom tuning defined in the GUI is visible to
// `twanga record` on the same machine.
//
// Schema: `Array<PresetEntry>` where `PresetEntry` is
// `{ slug, name, strings: [{ name, midi }] }`. Same shape the WASM
// `validate_preset_entry` enforces.

import { validate_preset_entry } from '../pkg/twanga_web.js';

const USER_TUNINGS_KEY = 'twanga-user-tunings-v1';

export function loadUserTunings() {
    try {
        const raw = localStorage.getItem(USER_TUNINGS_KEY);
        if (!raw) return [];
        const parsed = JSON.parse(raw);
        return Array.isArray(parsed) ? parsed : [];
    } catch (e) {
        console.warn('localStorage read failed for user tunings; ignoring', e);
        return [];
    }
}

export function saveUserTunings(entries) {
    try {
        localStorage.setItem(USER_TUNINGS_KEY, JSON.stringify(entries));
    } catch (e) {
        console.warn('localStorage write failed; user tuning will not persist', e);
    }
}

/// Look up a user tuning by slug. Returns `null` for the chromatic
/// sentinel and for any slug not in the user file (built-ins are handled
/// by `builtin_preset_entry` in the WASM module).
export function userTuningBySlug(slug) {
    if (typeof slug !== 'string' || !slug || slug === 'chromatic') return null;
    return loadUserTunings().find((e) => e.slug === slug) ?? null;
}

/// Append a new user tuning. Validates against the same Rust rules the
/// CLI's `twanga tunings add` flow runs; throws a string on failure.
/// Refuses duplicates on slug.
export function addUserTuningEntry(entry) {
    validate_preset_entry(entry); // throws a string on validation failure
    const entries = loadUserTunings();
    if (entries.some((e) => e.slug === entry.slug)) {
        throw `a user tuning with slug '${entry.slug}' already exists`;
    }
    entries.push(entry);
    saveUserTunings(entries);
}

export function deleteUserTuning(slug) {
    const entries = loadUserTunings().filter((e) => e.slug !== slug);
    saveUserTunings(entries);
}
