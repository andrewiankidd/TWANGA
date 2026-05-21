// Renderer registry — every renderer (built-in or future third-party) registers
// here through the same `register(plugin)` call with the same plugin shape. The
// registry has no concept of "ours" vs "external"; it just stores plugins in
// registration order and looks them up by id.
//
// A plugin is a plain object:
//
//     {
//       id:          'twanga.tab',         // globally unique, slug-like
//       name:        'Tab',                // user-facing label (dropdowns, etc)
//       description: 'Column-grid…',       // optional, longer-form copy
//       version:     '1.0.0',              // semver
//       author:      'TWANGA',             // optional, surfaced in marketplace UI
//       create(container, options)         // returns a Renderer instance
//     }
//
// The renderer instance it returns must implement:
//
//     setScore(scoreModel)        — replace the score being displayed
//     setPlayhead(columnIndex)    — move the playhead cursor (integer column).
//                                   columnIndex may be NEGATIVE during the
//                                   pre-roll count-in; the renderer should
//                                   draw a "runway" leading into col 0.
//     destroy()                   — tear down DOM / event listeners
//
// And SHOULD implement (host gracefully no-ops if missing):
//
//     setPreRoll(n)               — declare how many runway slots precede
//                                   col 0. Each renderer visualises this in
//                                   its own metaphor (tab: faded leading
//                                   cells; highway: widened upper zone so
//                                   col 0 is visible during the full count).
//     setCellLabel(mode)          — body-cell label mode: 'fret' shows the
//                                   fret number (default); 'note' shows the
//                                   absolute pitch class for that fret on
//                                   that string (open MIDI + capo offset).
//                                   Unrecognised modes coerce to 'fret'.
//
// That's the entire contract. The renderer owns *everything* about its visual
// layout (canvas vs DOM vs SVG, colours, sizing, animation). The host only
// hands over a container element and the score data.

export class RendererRegistry {
    constructor() {
        this._plugins = [];
    }

    /// Register a renderer plugin. Throws on malformed input so an
    /// incompatible plugin fails loudly instead of being silently ignored.
    /// Built-ins go through this same path — there is no "trusted" fast
    /// lane for them. Future third-party plugins will pass through the
    /// exact same gate.
    register(plugin) {
        if (!plugin || typeof plugin !== 'object') {
            throw new Error('renderer plugin must be an object');
        }
        for (const field of ['id', 'name', 'version']) {
            if (typeof plugin[field] !== 'string' || plugin[field].length === 0) {
                throw new Error(`renderer plugin missing or invalid '${field}'`);
            }
        }
        if (typeof plugin.create !== 'function') {
            throw new Error(
                `renderer plugin '${plugin.id}': 'create' must be a function`,
            );
        }
        if (this._plugins.some((p) => p.id === plugin.id)) {
            throw new Error(
                `renderer plugin '${plugin.id}' is already registered`,
            );
        }
        this._plugins.push(plugin);
    }

    /// Lookup by id. Returns `undefined` if not registered.
    get(id) {
        return this._plugins.find((p) => p.id === id);
    }

    /// All registered plugins in registration order. Returns a shallow
    /// copy so callers can't accidentally mutate registry state.
    list() {
        return this._plugins.slice();
    }
}

/// Module-singleton registry. Importers across the app see the same instance
/// because ES modules are cached per URL. The Recorder/Playback screens, the
/// dropdown widget, and any future "plugin manager" UI all read from this.
export const registry = new RendererRegistry();
