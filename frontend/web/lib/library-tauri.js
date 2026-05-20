// Tauri filesystem backend for the tab library — same interface as
// `./library.js` but reads from / writes to `$CONFIG/twanga/recordings/`
// via Tauri commands (defined in `crates/twanga-app/src/commands.rs`)
// instead of IndexedDB.
//
// The dispatcher in `library.js` picks this backend at runtime when
// `window.__TAURI__` is defined. In the browser build this file is
// imported but never called, so the JS shim cost is minimal.
//
// Recording ids on the Tauri side are filenames (e.g.
// `my-take-1779133041.alphatex`) — strings, distinct from the IDB
// backend's auto-increment integer ids. The dispatcher doesn't try to
// reconcile the two namespaces because a given runtime only ever talks
// to one backend.

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

/// `library.list()` over Tauri. Returns recordings sorted
/// newest-first (the Rust side sorts by mtime DESC). Bundled
/// examples are not included here — they're surfaced via the
/// existing fetch-based path in `library.js` (works identically
/// under Tauri because `frontendDist` serves them).
export async function list() {
    const invoke = ensureInvoke();
    const rows = await invoke('list_recordings');
    return (rows ?? []).map((r) => ({
        id: r.id,
        title: r.title || `Recording ${r.id}`,
        source: r.source ?? 'user',
        createdAt: r.created_at ?? null,
        lastBackedUpAt: r.last_backed_up_at ?? null,
    }));
}

/// `library.load(id)` over Tauri. The id is a filename — pass it
/// through verbatim. Path-traversal protection lives on the Rust side.
export async function load(id) {
    const invoke = ensureInvoke();
    const row = await invoke('load_recording', { id });
    return {
        id: row.id,
        title: row.title || `Recording ${row.id}`,
        source: row.source ?? 'user',
        alphatex: row.alphatex,
        createdAt: row.created_at ?? null,
        lastBackedUpAt: row.last_backed_up_at ?? null,
    };
}

/// `library.save({...})` over Tauri. Returns the new id (filename).
/// `source` is accepted for API parity but ignored — every Tauri-side
/// recording lives in the same `recordings/` dir; provenance is
/// implicit.
export async function save({ title, alphatex, source = 'user' }) {
    if (typeof alphatex !== 'string' || alphatex.length === 0) {
        throw new Error('save: alphatex content is required');
    }
    const invoke = ensureInvoke();
    return await invoke('save_recording', {
        title: title ?? '',
        alphatex,
        source,
    });
}

/// `library.update({id, title?, alphatex})` over Tauri. Writes the
/// new contents to the existing file. `title` is accepted but
/// currently unused on the Rust side — alphaTex's `\title` directive
/// IS the title, and the frontend already inlines it into the body
/// before calling update.
export async function update({ id, title, alphatex }) {
    if (typeof alphatex !== 'string' || alphatex.length === 0) {
        throw new Error('update: alphatex content is required');
    }
    const invoke = ensureInvoke();
    await invoke('update_recording', { id, title, alphatex });
}

/// `library.deleteTab(id)` over Tauri.
export async function deleteTab(id) {
    const invoke = ensureInvoke();
    await invoke('delete_recording', { id });
}

/// No-op on Tauri — the filesystem IS the backup. The browser
/// build's "Download" button + "Backed up <when>" tag exist because
/// IDB can be evicted; that doesn't apply to a real file in
/// `$CONFIG/twanga/recordings/`.
export async function markDownloaded(_id, _when) { /* no-op */ }

/// Always-persistent on the filesystem. The browser build needs to
/// ask the user for `navigator.storage.persist`; Tauri doesn't.
export async function requestPersistence() { return true; }

/// Cross-process change notifications. Filesystem-watch is the
/// eventual implementation (a Tauri event posted when a file under
/// `recordings/` changes); for first cut this is a no-op because
/// the desktop app is single-window, single-instance and the user
/// can refresh manually.
export function subscribe(_callback) { return () => {}; }
