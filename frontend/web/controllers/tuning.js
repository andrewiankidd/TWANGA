// Shared tuning + capo controller used by the Tuner, Recorder, and
// Playback screens. Each instance owns its own state (mode, capo,
// per-string spec) and DOM wiring; consumers wire it once and read
// state via accessors when they need it.
//
// Vanilla JS factory pattern — no framework. Returns a controller
// object whose methods read state and a `select(slug)` method that
// changes state programmatically. State changes fire the `onChange`
// callback so consumers can rebuild whatever they own (e.g. the
// `WebTuner` in Tuner, the chromatic tuner + score model in Recorder).
//
// Built-in vs user tunings: the controller treats them uniformly —
// the merged slug list is `[chromatic?, ...builtins, ...users]`. The
// `getPresetEntry()` accessor returns a `PresetEntry`-shaped object
// that callers pass to `match_pitch_to_fret` / `serialize_recording` /
// `new_for_strings_custom*` without branching on origin.
//
// CSS classes used (must exist in the page stylesheet):
//
//   .tuning-option / .tuning-option.active
//   .capo-control / .capo-uniform / .capo-mode-toggle / .capo-mode-toggle.active
//   .capo-per-string / .capo-per-string .ps-header
//   .capo-string-row / .capo-string-row .ps-label / .capo-btn / .capo-value

import {
    builtin_tuning_slugs,
    preset_display_name,
    builtin_preset_entry,
    midi_from_name,
    WebTuner,
} from '../pkg/twanga_web.js';

import { loadUserTunings, userTuningBySlug } from '../lib/user-tunings.js';

const MAX_CAPO = 12;

/// Find a registry slug (built-in or user) whose open-string MIDI
/// list matches the given note names. Used by Playback to pre-load
/// the loaded file's tuning into its controller, so the picker
/// reflects what was recorded rather than silently transposing.
///
/// Matches by MIDI list (not by name string) so reentrant / drone
/// annotations on registry tunings don't prevent a match — the
/// alphaTex `\tuning` line carries canonical names without
/// annotations, but the registry's `PresetString.name` can include
/// `g4 (reentrant)` etc.
///
/// Returns the slug if exactly one match is found, the first match
/// otherwise (built-ins precede user tunings). Returns `null` for
/// unparseable names or when no registry tuning matches the
/// string count + pitch list.
export function matchRegistrySlugForTuningNames(names) {
    if (!Array.isArray(names) || names.length === 0) return null;
    const fileMidi = names.map((n) => midi_from_name(n));
    if (fileMidi.some((m) => m == null)) return null;

    for (const slug of builtin_tuning_slugs()) {
        const entry = builtin_preset_entry(slug);
        if (!entry || !Array.isArray(entry.strings)) continue;
        if (entry.strings.length !== fileMidi.length) continue;
        if (entry.strings.every((s, i) => s.midi === fileMidi[i])) return slug;
    }
    for (const u of loadUserTunings()) {
        if (!Array.isArray(u.strings) || u.strings.length !== fileMidi.length) continue;
        if (u.strings.every((s, i) => s.midi === fileMidi[i])) return u.slug;
    }
    return null;
}

/// Display name for a slug, falling back through the same lookup
/// chain the original ad-hoc copies used: chromatic → built-in →
/// user → slug. Exported so consumers can re-use it for non-picker
/// labels (e.g. the Playback header).
export function fullName(slug) {
    if (slug === 'chromatic') return 'Chromatic';
    const builtin = preset_display_name(slug);
    if (builtin) return builtin;
    const user = userTuningBySlug(slug);
    return user ? user.name : slug;
}

/// Picker buttons use the trimmed display name (first segment before
/// any parenthetical). The full name is shown via the button's title
/// attribute for tooltip / a11y.
export function shortName(slug) {
    const full = fullName(slug);
    const head = full.split('(')[0].trim();
    return head || full;
}

/// Build a no-capo WebTuner just to read its string labels. One
/// construct/destroy per tuning change is fine; we only call this when
/// the user picks a new slug.
function computeBaseLabels(slug) {
    if (slug === 'chromatic') return [];
    try {
        const user = userTuningBySlug(slug);
        const t = user
            ? WebTuner.new_for_strings_custom(user, 48_000)
            : WebTuner.new_for_strings(slug, 48_000);
        const labels = t.string_labels();
        t.free?.();
        return labels;
    } catch (e) {
        console.warn('computeBaseLabels failed for', slug, e);
        return [];
    }
}

/**
 * Factory. Returns a controller object — see the methods listed at the
 * bottom for the surface consumers use.
 *
 * @param {object} opts
 * @param {string} opts.pickerId           Container element ID for the tuning-button row.
 * @param {string} [opts.activeNameId]     Optional element ID that gets the full-name text.
 * @param {string} opts.capoControlId      Wrapper element for the capo controls.
 * @param {string} opts.capoUniformId      Wrapper element for the uniform-mode stepper.
 * @param {string} opts.capoDownId         Decrement button.
 * @param {string} opts.capoUpId           Increment button.
 * @param {string} opts.capoValueId        Element that gets the current value as text.
 * @param {string} opts.capoModeToggleId   Button toggling uniform <-> per-string.
 * @param {string} opts.capoPerStringId    Container for the per-string steppers (gets rebuilt on tuning change).
 * @param {string} opts.storageKey         localStorage key under which to persist state.
 * @param {boolean} [opts.includeChromatic=false]  Include the chromatic sentinel in the picker (Tuner only).
 * @param {function} [opts.onChange]       Fires after every state change (tuning select, capo change, mode toggle).
 */
export function makeTuningController(opts) {
    const {
        pickerId,
        activeNameId,
        capoControlId,
        capoUniformId,
        capoDownId,
        capoUpId,
        capoValueId,
        capoModeToggleId,
        capoPerStringId,
        storageKey,
        includeChromatic = false,
        onChange,
    } = opts;

    const $ = (id) => document.getElementById(id);

    const state = {
        mode: null,             // 'chromatic' or a tuning slug
        capoMode: 'uniform',    // 'uniform' | 'per-string'
        capo: 0,                // uniform integer
        capoSpec: [],           // per-string array, length matches active tuning's string count
        baseLabels: [],         // no-capo labels for the active tuning (for the per-string panel)
        presetEntry: null,      // PresetEntry JSON of the active tuning (null for chromatic)
        enabled: true,
    };

    function loadPersisted() {
        let saved = {};
        try {
            const raw = localStorage.getItem(storageKey);
            if (raw) saved = JSON.parse(raw) ?? {};
        } catch (e) {
            console.warn(`tuning-controller load failed for ${storageKey}`, e);
        }
        state.mode = typeof saved.mode === 'string' ? saved.mode : null;
        state.capoMode = saved.capoMode === 'per-string' ? 'per-string' : 'uniform';
        state.capo = Number.isInteger(saved.capo) && saved.capo >= 0 && saved.capo <= MAX_CAPO
            ? saved.capo : 0;
        state.capoSpec = Array.isArray(saved.capoSpec)
            ? saved.capoSpec.map((v) => (Number.isInteger(v) && v >= 0 && v <= MAX_CAPO ? v : 0))
            : [];
    }

    function savePersisted() {
        try {
            localStorage.setItem(storageKey, JSON.stringify({
                mode: state.mode,
                capoMode: state.capoMode,
                capo: state.capo,
                capoSpec: state.capoSpec,
            }));
        } catch (e) {
            console.warn(`tuning-controller save failed for ${storageKey}`, e);
        }
    }

    function allKnownSlugs() {
        const builtinSet = new Set(builtin_tuning_slugs());
        const users = loadUserTunings()
            .map((e) => e.slug)
            .filter((s) => !builtinSet.has(s));
        const out = [...builtin_tuning_slugs(), ...users];
        return includeChromatic ? ['chromatic', ...out] : out;
    }

    function renderPicker() {
        const picker = $(pickerId);
        picker.replaceChildren();
        for (const slug of allKnownSlugs()) {
            const btn = document.createElement('button');
            btn.className = 'tuning-option';
            btn.dataset.slug = slug;
            btn.textContent = shortName(slug);
            btn.title = fullName(slug);
            if (slug === state.mode) btn.classList.add('active');
            btn.addEventListener('click', () => select(slug));
            picker.appendChild(btn);
        }
    }

    function updateCapoDisplay() {
        $(capoValueId).textContent = String(state.capo);
        $(capoDownId).disabled = !state.enabled || state.capo <= 0;
        $(capoUpId).disabled = !state.enabled || state.capo >= MAX_CAPO;
        const isChromatic = state.mode === 'chromatic';
        $(capoControlId).hidden = !state.mode || isChromatic;
        $(capoModeToggleId).classList.toggle('active', state.capoMode === 'per-string');
        $(capoUniformId).hidden = state.capoMode === 'per-string';
        $(capoPerStringId).hidden =
            !state.mode || isChromatic || state.capoMode !== 'per-string';
        if (activeNameId) {
            const el = $(activeNameId);
            if (el) el.textContent = fullName(state.mode ?? '—');
        }
    }

    function renderPerStringCapo() {
        const panel = $(capoPerStringId);
        panel.replaceChildren();
        if (!state.mode || state.mode === 'chromatic' || state.capoMode !== 'per-string') {
            return;
        }

        const header = document.createElement('div');
        header.className = 'ps-header';
        header.textContent = 'Per-string capo (semitones)';
        panel.appendChild(header);

        state.baseLabels.forEach((label, idx) => {
            const row = document.createElement('div');
            row.className = 'capo-string-row';

            const lab = document.createElement('span');
            lab.className = 'ps-label';
            lab.textContent = `#${idx + 1} (${label})`;
            row.appendChild(lab);

            const down = document.createElement('button');
            down.className = 'capo-btn';
            down.textContent = '−';
            down.setAttribute('aria-label', `Decrease capo on string ${idx + 1}`);

            const value = document.createElement('span');
            value.className = 'capo-value';

            const up = document.createElement('button');
            up.className = 'capo-btn';
            up.textContent = '+';
            up.setAttribute('aria-label', `Increase capo on string ${idx + 1}`);

            const refreshRow = () => {
                const v = state.capoSpec[idx] ?? 0;
                value.textContent = String(v);
                down.disabled = !state.enabled || v <= 0;
                up.disabled = !state.enabled || v >= MAX_CAPO;
            };
            down.addEventListener('click', () => {
                if (!state.enabled) return;
                if ((state.capoSpec[idx] ?? 0) > 0) {
                    state.capoSpec[idx] -= 1;
                    refreshRow();
                    savePersisted();
                    onChange?.();
                }
            });
            up.addEventListener('click', () => {
                if (!state.enabled) return;
                if ((state.capoSpec[idx] ?? 0) < MAX_CAPO) {
                    state.capoSpec[idx] = (state.capoSpec[idx] ?? 0) + 1;
                    refreshRow();
                    savePersisted();
                    onChange?.();
                }
            });
            refreshRow();

            row.appendChild(down);
            row.appendChild(value);
            row.appendChild(up);
            panel.appendChild(row);
        });
    }

    function toggleCapoMode() {
        if (!state.enabled || state.mode === 'chromatic' || !state.mode) return;
        if (state.capoMode === 'uniform') {
            state.capoMode = 'per-string';
            // Seed the per-string array from the uniform value so the
            // user sees the same capo state in both views the first
            // time they switch.
            state.capoSpec = Array.from(
                { length: state.baseLabels.length },
                () => state.capo,
            );
        } else {
            state.capoMode = 'uniform';
        }
        updateCapoDisplay();
        renderPerStringCapo();
        savePersisted();
        onChange?.();
    }

    function refreshPresetEntry() {
        if (state.mode === 'chromatic' || !state.mode) {
            state.presetEntry = null;
            return;
        }
        const user = userTuningBySlug(state.mode);
        state.presetEntry = user ?? builtin_preset_entry(state.mode);
    }

    function select(slug) {
        if (!state.enabled) return;
        state.mode = slug;
        state.baseLabels = computeBaseLabels(slug);
        if (state.capoSpec.length !== state.baseLabels.length) {
            state.capoSpec = state.baseLabels.map(() => 0);
        }
        refreshPresetEntry();
        document.querySelectorAll(`#${pickerId} .tuning-option`).forEach((b) => {
            b.classList.toggle('active', b.dataset.slug === slug);
        });
        updateCapoDisplay();
        renderPerStringCapo();
        savePersisted();
        onChange?.();
    }

    function init() {
        loadPersisted();
        renderPicker();
        // Capo wiring (uniform stepper + mode toggle):
        $(capoDownId).addEventListener('click', () => {
            if (!state.enabled || state.capo <= 0) return;
            state.capo -= 1;
            updateCapoDisplay();
            savePersisted();
            onChange?.();
        });
        $(capoUpId).addEventListener('click', () => {
            if (!state.enabled || state.capo >= MAX_CAPO) return;
            state.capo += 1;
            updateCapoDisplay();
            savePersisted();
            onChange?.();
        });
        $(capoModeToggleId).addEventListener('click', toggleCapoMode);
    }

    init();

    return {
        /// Active tuning mode — slug or `'chromatic'` (the latter only
        /// possible if `includeChromatic: true` was passed at construct).
        /// `null` if the controller hasn't selected anything yet.
        getMode() { return state.mode; },
        /// PresetEntry JSON for the active tuning, or `null` for
        /// chromatic. Used directly by `match_pitch_to_fret`,
        /// `serialize_recording`, and `WebTuner.new_for_strings_custom*`.
        getPresetEntry() { return state.presetEntry; },
        /// Uniform capo value.
        getCapo() { return state.capo; },
        /// Current capo mode (`'uniform'` or `'per-string'`).
        getCapoMode() { return state.capoMode; },
        /// Per-string capo offsets array.
        getCapoSpec() { return state.capoSpec.slice(); },
        /// No-capo labels for the active tuning's strings (string 1 first).
        getBaseLabels() { return state.baseLabels.slice(); },
        /// True iff a non-zero capo is in effect on at least one string.
        capoIsActive() {
            return state.capoMode === 'per-string'
                ? state.capoSpec.some((v) => v > 0)
                : state.capo > 0;
        },
        /// CLI-syntax capo spec — `"3"` uniform, `"0,2,2,2,2,2"` per-string.
        /// Pass directly to WASM bindings that take a `capo_spec: &str`.
        capoSpecString() {
            return state.capoMode === 'per-string'
                ? state.capoSpec.join(',')
                : String(state.capo);
        },
        /// Change the active tuning programmatically. Fires `onChange`.
        select,
        /// Apply a CLI-syntax capo spec — `""` / `"0"` clears, `"3"`
        /// goes uniform, `"0,2,2,2,2,2"` goes per-string. Per-string
        /// specs whose length doesn't match the active tuning's
        /// string count are ignored (same length-validation `Capo::parse`
        /// does on the Rust side). Returns true if applied. Used by
        /// Playback to pre-load the file's embedded capo on load.
        setCapo(spec) {
            if (!state.enabled || !state.mode || state.mode === 'chromatic') return false;
            if (typeof spec !== 'string') return false;
            const trimmed = spec.trim();
            if (trimmed === '' || trimmed === '0') {
                state.capo = 0;
                state.capoMode = 'uniform';
                state.capoSpec = state.baseLabels.map(() => 0);
                updateCapoDisplay();
                renderPerStringCapo();
                savePersisted();
                onChange?.();
                return true;
            }
            if (trimmed.includes(',')) {
                const parts = trimmed.split(',').map((s) => Number.parseInt(s.trim(), 10));
                if (parts.length !== state.baseLabels.length) return false;
                if (parts.some((v) => !Number.isFinite(v) || v < 0 || v > MAX_CAPO)) return false;
                state.capoMode = 'per-string';
                state.capoSpec = parts;
            } else {
                const n = Number.parseInt(trimmed, 10);
                if (!Number.isFinite(n) || n < 0 || n > MAX_CAPO) return false;
                state.capoMode = 'uniform';
                state.capo = n;
            }
            updateCapoDisplay();
            renderPerStringCapo();
            savePersisted();
            onChange?.();
            return true;
        },
        /// Disable / re-enable all inputs (used by Recorder while
        /// recording so the user can't swap mid-take).
        setEnabled(enabled) {
            state.enabled = enabled;
            updateCapoDisplay();
            renderPerStringCapo();
            document.querySelectorAll(`#${pickerId} .tuning-option`).forEach((b) => {
                b.disabled = !enabled;
            });
        },
        /// Re-render the picker. Call after user tunings are added or
        /// removed elsewhere in the app.
        refresh() {
            renderPicker();
            // If the active slug was deleted in the meantime, fall back to
            // chromatic (if available) or the first built-in.
            const valid = new Set(allKnownSlugs());
            if (!valid.has(state.mode)) {
                const fallback = includeChromatic
                    ? 'chromatic'
                    : (builtin_tuning_slugs()[0] ?? null);
                if (fallback) select(fallback);
            }
        },
    };
}
