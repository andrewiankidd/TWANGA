// Tauri filesystem backend for the tab library — same interface as
// `./library.js` but reads from / writes to `$CONFIG/twanga/recordings/`
// via Tauri commands instead of IndexedDB. STUB for now; the desktop
// app's actual implementation needs Tauri commands like
// `list_recordings` / `load_recording` / `save_recording` to be
// registered in the Tauri host (`crates/twanga-app/src/lib.rs`).
//
// The shape stays identical to `library.js` so a single import line in
// the host code can swap backends at runtime based on `window.__TAURI__`
// — once that machinery exists, the import paths don't churn.
//
// Until the commands land, every method here throws so the dispatcher
// (next commit batch) loudly falls back to the IDB backend. Throwing
// is preferable to silent no-ops; we want the error surface to be
// obvious during development.

const NOT_READY = 'Tauri library backend not yet implemented';

export async function list() { throw new Error(NOT_READY); }
export async function load(_id) { throw new Error(NOT_READY); }
export async function save(_entry) { throw new Error(NOT_READY); }
export async function deleteTab(_id) { throw new Error(NOT_READY); }
export async function markDownloaded(_id, _when) { /* no-op: filesystem persistence is automatic */ }
export async function requestPersistence() { return true; /* filesystem is always persistent */ }

/// Filesystem-watch-based subscribe is the eventual implementation —
/// the Tauri command would post `file changed in $CONFIG/twanga/recordings/`
/// events back to JS. For now: no-op (single-tab usage on desktop is the
/// common case and the user can just hit refresh).
export function subscribe(_callback) { return () => {}; }
