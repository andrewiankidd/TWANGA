// Tab library — browser-side storage for `.alphatex` recordings + a
// loader for the bundled examples that ship with the app.
//
// Two sources merged into one logical library:
// - **User tabs** persist in IndexedDB under `twanga-tabs-v1` /
//   `tabs` object store. Auto-incrementing integer IDs.
// - **Bundled examples** load lazily from `assets/examples/manifest.json`
//   and are returned alongside user tabs in `list()`. Read-only;
//   IDs are prefixed `bundled:` so they can't collide with integers.
//
// Interface:
//   list()           -> [{ id, title, source, createdAt, lastBackedUpAt }]
//   load(id)         -> { id, title, source, alphatex, ... }
//   save({title, alphatex, source}) -> id
//   update({id, title, alphatex})   -> in-place overwrite (no bundled)
//   delete(id)
//   markDownloaded(id, when?)  -> sets lastBackedUpAt
//
// Important: browser IDB can be evicted by the user (Clear browsing
// data) or under storage pressure. The web-warning layer (Phase 4)
// uses `markDownloaded` to track which entries the user has exported
// as real files so we can surface "Backed up <when>" hints. On the
// Tauri side, storage is the filesystem and warnings are not needed.

const DB_NAME = 'twanga-tabs-v1';
const STORE_NAME = 'tabs';
const DB_VERSION = 1;

let _dbPromise = null;

// Cross-tab change notifications via BroadcastChannel. When the user
// records in one tab and has another tab open on the Playback library
// screen, the second tab can refresh its list without a manual reload.
// BC posts only to OTHER tabs (not the originating one), which is fine —
// the originating tab updates its own UI synchronously.
//
// Older browsers without BroadcastChannel: silent no-op. The single-tab
// experience is unaffected; cross-tab refresh just won't happen.
const _channel = typeof BroadcastChannel !== 'undefined'
    ? new BroadcastChannel('twanga-tabs-changed')
    : null;

function _notify(event) {
    if (!_channel) return;
    try { _channel.postMessage(event); } catch (e) { /* swallow */ }
}

/// Subscribe to cross-tab library changes. The callback receives an
/// event object `{ type: 'save'|'delete'|'markDownloaded', id }`. The
/// returned function unsubscribes; idempotent + safe to call after
/// the channel was never created (no-op when unsupported).
export function subscribe(callback) {
    if (!_channel) return () => {};
    const handler = (e) => {
        try { callback(e.data); }
        catch (err) { console.warn('library.subscribe handler threw', err); }
    };
    _channel.addEventListener('message', handler);
    return () => _channel.removeEventListener('message', handler);
}

function openDb() {
    if (_dbPromise) return _dbPromise;
    _dbPromise = new Promise((resolve, reject) => {
        const req = indexedDB.open(DB_NAME, DB_VERSION);
        req.onupgradeneeded = () => {
            const db = req.result;
            if (!db.objectStoreNames.contains(STORE_NAME)) {
                db.createObjectStore(STORE_NAME, { keyPath: 'id', autoIncrement: true });
            }
        };
        req.onsuccess = () => resolve(req.result);
        req.onerror = () => reject(req.error);
    });
    return _dbPromise;
}

function tx(mode, fn) {
    return openDb().then((db) => new Promise((resolve, reject) => {
        const t = db.transaction(STORE_NAME, mode);
        const store = t.objectStore(STORE_NAME);
        let result;
        try {
            result = fn(store);
        } catch (e) {
            reject(e);
            return;
        }
        t.oncomplete = () => resolve(result);
        t.onerror = () => reject(t.error);
        t.onabort = () => reject(t.error);
    }));
}

function reqAsPromise(req) {
    return new Promise((resolve, reject) => {
        req.onsuccess = () => resolve(req.result);
        req.onerror = () => reject(req.error);
    });
}

// ---- Bundled examples ------------------------------------------------

let _bundledPromise = null;

function loadBundledManifest() {
    if (_bundledPromise) return _bundledPromise;
    _bundledPromise = fetch('assets/examples/manifest.json', { cache: 'force-cache' })
        .then((r) => {
            if (!r.ok) throw new Error(`bundled manifest fetch failed: ${r.status}`);
            return r.json();
        })
        .then((json) => Array.isArray(json?.examples) ? json.examples : [])
        .catch((e) => {
            console.warn('failed to load bundled examples manifest', e);
            return [];
        });
    return _bundledPromise;
}

async function loadBundledAlphatex(path) {
    const r = await fetch(`assets/examples/${path}`, { cache: 'force-cache' });
    if (!r.ok) throw new Error(`bundled example fetch failed: ${r.status}`);
    return await r.text();
}

// ---- Public API ------------------------------------------------------

/// All tabs in the library — bundled examples first, then user
/// recordings (newest first). Returns metadata only (no alphatex
/// content — call `load(id)` for that).
export async function list() {
    const [bundled, users] = await Promise.all([
        loadBundledManifest(),
        tx('readonly', (store) => reqAsPromise(store.getAll())),
    ]);
    const bundledRows = bundled.map((b) => ({
        id: `bundled:${b.id}`,
        title: b.title,
        source: 'bundled',
        createdAt: null,
        lastBackedUpAt: null,
    }));
    const userRows = (users ?? [])
        .map((u) => ({
            id: u.id,
            title: u.title || `Recording ${u.id}`,
            source: u.source ?? 'user',
            createdAt: u.createdAt ?? null,
            lastBackedUpAt: u.lastBackedUpAt ?? null,
        }))
        // Most recent first — `createdAt` is a millisecond timestamp from
        // Date.now() at save time, so direct numeric comparison works.
        .sort((a, b) => (b.createdAt ?? 0) - (a.createdAt ?? 0));
    return [...bundledRows, ...userRows];
}

/// Load full content for a single tab. Returns
/// `{ id, title, source, alphatex, createdAt, lastBackedUpAt }`.
/// Throws if the id doesn't exist.
export async function load(id) {
    if (typeof id === 'string' && id.startsWith('bundled:')) {
        const slug = id.slice('bundled:'.length);
        const manifest = await loadBundledManifest();
        const entry = manifest.find((e) => e.id === slug);
        if (!entry) throw new Error(`unknown bundled example: ${id}`);
        const alphatex = await loadBundledAlphatex(entry.path);
        return {
            id,
            title: entry.title,
            source: 'bundled',
            alphatex,
            createdAt: null,
            lastBackedUpAt: null,
        };
    }
    const row = await tx('readonly', (store) => reqAsPromise(store.get(id)));
    if (!row) throw new Error(`tab id ${id} not found`);
    return {
        id: row.id,
        title: row.title || `Recording ${row.id}`,
        source: row.source ?? 'user',
        alphatex: row.alphatex,
        createdAt: row.createdAt ?? null,
        lastBackedUpAt: row.lastBackedUpAt ?? null,
    };
}

/// Save a new tab. `source` defaults to `'user'` for recordings made
/// in this browser; `'imported'` is reserved for files dropped in via
/// the Library upload picker. Bundled examples are not savable — pass
/// them through directly. Returns the new auto-assigned id.
export async function save({ title, alphatex, source = 'user' }) {
    if (typeof alphatex !== 'string' || alphatex.length === 0) {
        throw new Error('save: alphatex content is required');
    }
    const row = {
        title: typeof title === 'string' ? title.trim() : '',
        alphatex,
        source,
        createdAt: Date.now(),
        lastBackedUpAt: null,
    };
    const id = await tx('readwrite', (store) => reqAsPromise(store.add(row)));
    _notify({ type: 'save', id });
    return id;
}

/// Overwrite an existing user tab. Bundled examples are read-only —
/// throws if `id` is a `bundled:*` string. The Editor screen calls this
/// when the user saves edits to a recording in place; "save as new"
/// goes through `save({...})` instead.
export async function update({ id, title, alphatex }) {
    if (typeof id === 'string' && id.startsWith('bundled:')) {
        throw new Error('bundled examples are read-only; use save() for a new copy');
    }
    if (typeof alphatex !== 'string' || alphatex.length === 0) {
        throw new Error('update: alphatex content is required');
    }
    await tx('readwrite', (store) => new Promise((resolve, reject) => {
        const getReq = store.get(id);
        getReq.onsuccess = () => {
            const row = getReq.result;
            if (!row) {
                reject(new Error(`tab id ${id} not found`));
                return;
            }
            if (typeof title === 'string') row.title = title.trim();
            row.alphatex = alphatex;
            // Editing invalidates any prior backed-up state — the file
            // on disk no longer matches what's in IDB.
            row.lastBackedUpAt = null;
            const putReq = store.put(row);
            putReq.onsuccess = () => resolve();
            putReq.onerror = () => reject(putReq.error);
        };
        getReq.onerror = () => reject(getReq.error);
    }));
    _notify({ type: 'save', id });
}

/// Remove a user tab by id. Bundled examples are protected — calling
/// `delete('bundled:...')` is a no-op (the manifest is read-only).
export async function deleteTab(id) {
    if (typeof id === 'string' && id.startsWith('bundled:')) {
        return; // bundled examples are read-only
    }
    await tx('readwrite', (store) => reqAsPromise(store.delete(id)));
    _notify({ type: 'delete', id });
}

/// Mark a user tab as exported to a real file at `when` (defaults to
/// now). The web-warning layer reads this to surface "Backed up
/// <when>" hints. Bundled examples ignore the call.
export async function markDownloaded(id, when = Date.now()) {
    if (typeof id === 'string' && id.startsWith('bundled:')) return;
    await tx('readwrite', (store) => new Promise((resolve, reject) => {
        const getReq = store.get(id);
        getReq.onsuccess = () => {
            const row = getReq.result;
            if (!row) {
                resolve();
                return;
            }
            row.lastBackedUpAt = when;
            const putReq = store.put(row);
            putReq.onsuccess = () => resolve();
            putReq.onerror = () => reject(putReq.error);
        };
        getReq.onerror = () => reject(getReq.error);
    }));
    _notify({ type: 'markDownloaded', id });
}

/// Ask the browser to mark our storage as persistent so it's less
/// likely to get evicted under storage pressure. Returns the granted
/// state (true if persistent, false if denied or unavailable). Best
/// called once on first save; safe to call repeatedly. Does nothing
/// useful on Tauri (filesystem is already persistent).
export async function requestPersistence() {
    if (!navigator.storage || typeof navigator.storage.persist !== 'function') {
        return false;
    }
    try {
        return await navigator.storage.persist();
    } catch (e) {
        console.warn('storage.persist failed', e);
        return false;
    }
}
