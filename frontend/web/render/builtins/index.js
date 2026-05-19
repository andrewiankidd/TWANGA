// Registers the built-in renderers with the module-singleton registry.
//
// IMPORTANT: this file uses the exact same `registry.register(plugin)` call
// path that a third-party plugin would use. Built-ins have no special access,
// no hidden fields, no fast lane. Adding a renderer in the future — whether
// shipped in `frontend/web/render/builtins/` or loaded from
// `$CONFIG/twanga/renderers/` on Tauri desktop, or fetched from a community
// directory URL — is the same `registry.register(plugin)` call, and the same
// plugin shape (`{ id, name, version, create, ... }`).

import { registry } from '../registry.js';
import tabPlugin from './tab.js';
import highwayPlugin from './highway.js';

let installed = false;

/// Idempotent — safe to call from multiple entry points or after hot reload.
/// Re-registering the same plugin id is a registry error, so the guard avoids
/// surprising failures if the host accidentally calls this twice.
export function installBuiltinRenderers() {
    if (installed) return;
    registry.register(tabPlugin);
    registry.register(highwayPlugin);
    installed = true;
}
