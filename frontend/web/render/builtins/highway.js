// Highway renderer — falling-notes view, "notes coming toward you".
// One vertical lane per string, notes drop from the top toward a horizontal
// "now" line near the bottom. Notes past the now line scroll off below.
//
// Same plugin shape as `tab.js`. The registry doesn't care which is which —
// they're both registered via `registry.register(plugin)` with no fast-lane
// for built-ins. See ../registry.js.
//
// "Highway in a recorder context is weird because there are no future notes" —
// that's fine. The renderer just paints whatever the score says exists. During
// live capture, the score has no future columns, so the upper part of each
// lane is empty. Past notes still scroll off as expected. The renderer
// doesn't need to know whether it's being used by a recorder or a player.

// Live-scoring outcome → note-fill tint. Mirrors the
// `.live-outcome-*` colours on the floating outcome badge + the
// tab renderer's per-column tint so all three surfaces speak the
// same colour language. Duplicated from tab.js — same "hoist if a
// third renderer needs them" posture as parseCapoForLabels.
const OUTCOME_BG = {
    hit:    'rgba(76, 175, 80, 0.85)',
    late:   'rgba(255, 193, 7, 0.85)',
    missed: 'rgba(244, 67, 54, 0.85)',
    wrong:  'rgba(186, 85, 211, 0.85)',
};
// Note's base fill when no outcome has been recorded — kept here
// so the clear path can restore it without re-querying CSS.
const NOTE_BG_DEFAULT = 'var(--highlight)';

class HighwayRenderer {
    constructor(container, options = {}) {
        this.container = container;
        this.options = {
            laneWidth: 56,
            laneGap: 6,
            // 480px gives a 12-slot column (10 lookahead + 2 lookbehind)
            // a per-slot height of ~38px, comfortably bigger than the
            // 30px note circles. The previous 320px height meant
            // slot-height ≈ 25px which let notes overlap on top of each
            // other when several columns landed close together.
            height: 480,
            lookahead: 10,   // columns visible above the now line
            lookbehind: 2,   // columns visible below the now line
            // Pre-roll runway — host-supplied count-in slot count. The
            // highway uses it to widen the effective lookahead so col 0
            // is visible for the ENTIRE count-in (otherwise with
            // preRoll > lookahead the first note pops in mid-count).
            // Same contract as the tab renderer's `preRoll` option;
            // the host calls setPreRoll(n) to keep it in sync.
            preRoll: 0,
            // Note-circle label mode — 'fret' (default) shows the fret
            // number on each note circle; 'note' shows the absolute
            // pitch class ("C", "F#") given the string's open MIDI +
            // capo offset. Same contract as the tab renderer.
            cellLabel: 'fret',
            ...options,
        };
        this.score = null;
        this.playhead = 0;
        // Live-scoring outcome per column (col → outcome.kind).
        // Preserved across _rebuildLanes so re-rendering keeps the
        // outcome tints — only `clearColumnOutcomes` (called at
        // session start by the host) drops them.
        this.columnOutcomes = new Map();

        this.root = document.createElement('div');
        this.root.className = 'tw-rt-highway';
        Object.assign(this.root.style, {
            position: 'relative',
            display: 'flex',
            justifyContent: 'center',
            gap: `${this.options.laneGap}px`,
            background: 'var(--card)',
            border: '1px solid var(--border)',
            borderRadius: 'var(--radius, 6px)',
            padding: '0.8rem',
            height: `${this.options.height}px`,
            overflow: 'hidden',
            fontFamily: 'monospace',
        });
        container.appendChild(this.root);

        // "Now" line — fixed horizontal indicator near the bottom of the
        // highway, drawn over all lanes. Notes "land" on this line at their
        // moment in time.
        this.nowLine = document.createElement('div');
        Object.assign(this.nowLine.style, {
            position: 'absolute',
            left: '0',
            right: '0',
            bottom: this._nowLineBottomPx(),
            height: '2px',
            background: 'var(--highlight)',
            boxShadow: '0 0 12px var(--highlight)',
            pointerEvents: 'none',
        });
        this.root.appendChild(this.nowLine);
    }

    setScore(score) {
        this.score = score;
        this._rebuildLanes();
        this._repositionAll();
        this._refreshLiveNotes();
    }

    setPlayhead(columnIndex) {
        this.playhead = columnIndex;
        this._repositionAll();
        this._refreshLiveNotes();
    }

    /// Host-driven pre-roll size. Same contract as the tab renderer.
    /// On the highway it widens the effective lookahead (so col 0
    /// stays visible during the full count-in) and refreshes the now-
    /// line position (the line moves up the lane to make room for
    /// the wider upper zone). DOM-wise it re-runs the same recompute
    /// path as a resize — no rebuild, just reposition.
    setPreRoll(n) {
        const next = Math.max(0, Math.floor(n) || 0);
        if (next === this.options.preRoll) return;
        this.options.preRoll = next;
        // The now-line bottom depends on slotPx which depends on the
        // effective lookahead (which depends on preRoll). Update the
        // CSS value, then reposition every note.
        this.nowLine.style.bottom = this._nowLineBottomPx();
        this._repositionAll();
    }

    /// Toggle note-circle labels between 'fret' and 'note'. Same
    /// contract as the tab renderer. Unlike the tab, the highway can
    /// update in-place without a rebuild — each note circle is an
    /// already-tracked DOM node in `noteEls`. We re-derive the label
    /// per note from the score state captured at rebuild time.
    setCellLabel(mode) {
        const next = mode === 'note' ? 'note' : 'fret';
        if (next === this.options.cellLabel) return;
        this.options.cellLabel = next;
        this._refreshNoteCircleLabels();
    }

    _refreshNoteCircleLabels() {
        if (!this.score || !this.noteEls) return;
        const strings = this.score.tuning?.strings ?? [];
        const capoOffsets = parseCapoForLabels(this.score.capoSpec, strings.length);
        for (const { string: s, column: c, el } of this.noteEls) {
            const fret = this.score.columns[c]?.[s];
            el.textContent = formatCellLabel(
                fret, strings[s]?.midi, capoOffsets[s] ?? 0, this.options.cellLabel,
            );
        }
    }

    destroy() {
        this.root.remove();
    }

    /// Effective lookahead — never smaller than `preRoll`, so col 0
    /// remains in the visible "future" zone for the full count-in.
    /// Used by `_slotHeight` and `_nowLineBottomPx` so the lane
    /// geometry adapts when preRoll changes.
    _effectiveLookahead() {
        return Math.max(this.options.lookahead, this.options.preRoll);
    }

    /// Distance from the bottom of the lane to the now line, in px. This
    /// is where past notes start scrolling off. Equal to `lookbehind`
    /// columns worth of vertical space. Uses the effective lookahead so
    /// a wide pre-roll shrinks slot height (notes get closer together)
    /// rather than pushing col 0 off the top of the lane.
    _nowLineBottomPx() {
        const usable = this.options.height - 20; // padding above/below
        const totalSlots = this._effectiveLookahead() + this.options.lookbehind;
        const slotPx = usable / totalSlots;
        return `${this.options.lookbehind * slotPx + 8}px`;
    }

    _slotHeight() {
        const usable = this.options.height - 20;
        const totalSlots = this._effectiveLookahead() + this.options.lookbehind;
        return usable / totalSlots;
    }

    _rebuildLanes() {
        // Clear non-now-line children — keep the now line so its DOM node
        // doesn't churn on every score swap.
        for (const child of [...this.root.children]) {
            if (child !== this.nowLine) child.remove();
        }
        this.lanes = [];
        this.noteEls = []; // [{ string, column, el }]
        // Per-lane handle for the live-note pill — pinned just above
        // the static string label. Populated on rebuild, mutated in
        // place by _refreshLiveNotes() when the playhead advances.
        this.liveNoteEls = [];

        if (!this.score) return;
        const strings = this.score.tuning?.strings ?? [];
        // Per-string capo offsets so we can suffix the lane labels
        // (same convention as the Tab renderer).
        const capoOffsets = parseCapoForLabels(this.score.capoSpec, strings.length);

        for (let s = 0; s < strings.length; s++) {
            const lane = document.createElement('div');
            Object.assign(lane.style, {
                position: 'relative',
                width: `${this.options.laneWidth}px`,
                height: '100%',
                background: 'linear-gradient(180deg, transparent, rgba(255,255,255,0.02))',
                border: '1px solid var(--border)',
                borderRadius: '4px',
                overflow: 'hidden',
            });

            // String label pinned to the bottom of the lane, centred.
            // Suffix with "+N" when the capo raises this string.
            const off = capoOffsets[s] ?? 0;
            const label = document.createElement('div');
            label.textContent = off > 0
                ? `${strings[s].name} +${off}`
                : strings[s].name;
            Object.assign(label.style, {
                position: 'absolute',
                left: '0',
                right: '0',
                bottom: '2px',
                textAlign: 'center',
                fontSize: '0.7rem',
                color: 'var(--muted)',
                pointerEvents: 'none',
            });
            lane.appendChild(label);

            // Live-note pill — sits just above the static string
            // label, shows the absolute pitch class (e.g. "C", "F#")
            // of whatever fret is at the now-line on this lane.
            // Updated in-place by _refreshLiveNotes; empty when no
            // note is at the playhead column.
            const liveNote = document.createElement('div');
            Object.assign(liveNote.style, {
                position: 'absolute',
                left: '0',
                right: '0',
                bottom: '18px',
                textAlign: 'center',
                fontSize: '0.8rem',
                fontWeight: '700',
                fontFamily: 'monospace',
                color: 'var(--highlight)',
                pointerEvents: 'none',
            });
            lane.appendChild(liveNote);
            this.liveNoteEls[s] = liveNote;

            this.root.appendChild(lane);
            this.lanes.push(lane);

            // Create a note element for every column where this string is
            // played. We position them via `top` in `_repositionAll`; here
            // we just create the DOM nodes once.
            for (let c = 0; c < this.score.columns.length; c++) {
                const fret = this.score.columns[c]?.[s];
                if (fret == null) continue;
                const note = document.createElement('div');
                note.textContent = formatCellLabel(
                    fret, strings[s]?.midi, capoOffsets[s] ?? 0, this.options.cellLabel,
                );
                Object.assign(note.style, {
                    position: 'absolute',
                    left: '50%',
                    transform: 'translate(-50%, -50%)',
                    width: '30px',
                    height: '30px',
                    borderRadius: '50%',
                    background: 'var(--highlight)',
                    color: 'var(--background)',
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    fontWeight: '700',
                    fontSize: '0.85rem',
                    transition: 'top 0.12s linear, opacity 0.2s',
                    pointerEvents: 'none',
                });
                lane.appendChild(note);
                this.noteEls.push({ string: s, column: c, el: note });
            }
        }

        // Re-apply any preserved live-scoring outcomes — _rebuildLanes
        // wiped the DOM, but `columnOutcomes` is durable across
        // rebuilds (only `clearColumnOutcomes` drops them).
        for (const [col] of this.columnOutcomes) {
            this._applyColumnOutcome(col);
        }
    }

    /// Tint all note circles in column `col` with the outcome
    /// colour. `outcome` is `{ kind, offsetMs?, detectedHz? }` —
    /// only `kind` (one of `'hit' | 'late' | 'missed' | 'wrong'`)
    /// drives the tint. Unknown kinds no-op.
    setColumnOutcome(col, outcome) {
        if (!outcome || typeof outcome.kind !== 'string') return;
        this.columnOutcomes.set(col, outcome.kind);
        this._applyColumnOutcome(col);
    }

    /// Drop all stored outcome tints + revert note fills to the
    /// default highlight colour. Host calls this at session start
    /// so a previous take's verdicts don't bleed into the new run.
    clearColumnOutcomes() {
        const previouslyTinted = [...this.columnOutcomes.keys()];
        this.columnOutcomes.clear();
        for (const col of previouslyTinted) this._applyColumnOutcome(col);
    }

    /// Apply the stored outcome (if any) to the note circles in
    /// `col`, or revert them to NOTE_BG_DEFAULT if none. Centralised
    /// so setColumnOutcome / clearColumnOutcomes / _rebuildLanes all
    /// go through the same styling path.
    _applyColumnOutcome(col) {
        if (!this.noteEls) return;
        const kind = this.columnOutcomes.get(col);
        const bg = OUTCOME_BG[kind] ?? NOTE_BG_DEFAULT;
        for (const { column, el } of this.noteEls) {
            if (column === col) el.style.background = bg;
        }
    }

    _repositionAll() {
        if (!this.score) return;
        const slotPx = this._slotHeight();
        const { lookbehind, height } = this.options;
        const lookahead = this._effectiveLookahead();
        // Note radius — used to shift note centres up so a delta=0 note
        // "lands" on the line (bottom-of-note touches line-top) instead
        // of straddling the line by half its body. Notes are 30px circles
        // positioned with translate(-50%, -50%), so `top: y` puts the
        // note CENTRE at y. To make the bottom edge of the note sit at
        // the line top, the centre needs to be one radius above it.
        const noteRadius = 15;
        // Line top y from the top of the lane:
        const lineTopY = height - 10 - lookbehind * slotPx;
        // At delta=0 we want note-bottom == line-top, so note-centre is
        // `noteRadius` above the line. At higher deltas notes are
        // further above; at negative deltas they fall past the line.
        const nowY = lineTopY - noteRadius;

        for (const { column, el } of this.noteEls) {
            const delta = column - this.playhead;
            // delta > 0 → note is in the future, above the now line.
            // delta = 0 → note has just landed on the now line.
            // delta < 0 → note is in the past, below the now line.
            const y = nowY - delta * slotPx;
            const visible = delta <= lookahead && delta >= -lookbehind;
            el.style.top = `${y}px`;
            el.style.opacity = visible ? '1' : '0';
        }
    }

    /// Per-lane live-note pill — populated with the absolute pitch
    /// class (e.g. "C#", "B") of the fret at the now-line for each
    /// lane. Empty when no note is at the current playhead column on
    /// that lane. Mirrors the tab renderer's per-row live-note cell.
    _refreshLiveNotes() {
        if (!this.score || this.liveNoteEls.length === 0) return;
        const { columns, tuning, capoSpec } = this.score;
        const strings = tuning?.strings ?? [];
        const capoOffsets = parseCapoForLabels(capoSpec, strings.length);
        // During pre-roll the playhead is negative (notes scrolling in
        // from above); clamp to 0 for the live-note lookup so the user
        // can see what's about to play before it lands at the now-line.
        const lookupCol = Math.max(0, this.playhead);
        const col = columns[lookupCol];
        for (let s = 0; s < this.liveNoteEls.length; s++) {
            const cell = this.liveNoteEls[s];
            if (!cell) continue;
            const fret = col?.[s];
            const baseMidi = strings[s]?.midi;
            if (fret == null || baseMidi == null) {
                cell.textContent = '';
                continue;
            }
            const off = capoOffsets[s] ?? 0;
            cell.textContent = pitchClassName(baseMidi + off + fret);
        }
    }
}

// Sharps-only pitch-class table indexed by `midi % 12`. Duplicate of
// the helper in tab.js — see the parseCapoForLabels comment below
// for the "hoist into a shared util when more renderers need it"
// note. Both must stay in sync with twanga-core's PITCH_CLASS_NAMES.
const PITCH_CLASS_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
function pitchClassName(midi) {
    return PITCH_CLASS_NAMES[((midi % 12) + 12) % 12];
}

/// Format a single note-circle label. 'fret' shows the fret number,
/// 'note' shows the absolute pitch class for that fret on this
/// string (with capo applied). Duplicated in `tab.js` — see the
/// parseCapoForLabels comment for the "hoist when more is needed"
/// note. Returns `''` for empty cells (the highway only creates
/// note elements for non-null frets so this branch is defensive).
function formatCellLabel(fret, baseMidi, capoOffset, mode) {
    if (fret == null) return '';
    if (mode === 'note' && baseMidi != null) {
        return pitchClassName(baseMidi + (capoOffset ?? 0) + fret);
    }
    return String(fret);
}

/// Parse a Capo spec string into per-string offsets. Duplicates the
/// helper in `tab.js` because the two built-in renderers don't share
/// a module today; if more renderers ever need it, hoist into a tiny
/// `render/capo.js` shared util. Same behaviour as `Capo::parse` in
/// twanga-core: empty / "0" → zeros, "3" → uniform 3, "0,2,2,2,2,2" →
/// per-string.
function parseCapoForLabels(spec, stringCount) {
    if (!spec || stringCount === 0) return new Array(stringCount).fill(0);
    if (spec.includes(',')) {
        const parts = spec.split(',').map((s) => {
            const n = Number.parseInt(s.trim(), 10);
            return Number.isFinite(n) ? n : 0;
        });
        const out = new Array(stringCount).fill(0);
        for (let i = 0; i < Math.min(parts.length, stringCount); i++) {
            out[i] = parts[i];
        }
        return out;
    }
    const n = Number.parseInt(spec, 10);
    if (!Number.isFinite(n) || n <= 0) return new Array(stringCount).fill(0);
    return new Array(stringCount).fill(n);
}

export default {
    id: 'twanga.highway',
    name: 'Highway',
    description: 'Falling-notes highway. Notes drop down lanes toward a "now" line at the bottom.',
    version: '1.0.0',
    author: 'TWANGA',
    create(container, options) {
        return new HighwayRenderer(container, options);
    },
};
