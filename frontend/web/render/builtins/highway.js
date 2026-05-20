// Highway renderer — Rocksmith-style "notes coming toward you" view.
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

class HighwayRenderer {
    constructor(container, options = {}) {
        this.container = container;
        this.options = {
            laneWidth: 56,
            laneGap: 6,
            height: 320,
            lookahead: 10,   // columns visible above the now line
            lookbehind: 2,   // columns visible below the now line
            ...options,
        };
        this.score = null;
        this.playhead = 0;

        this.root = document.createElement('div');
        this.root.className = 'tw-rt-highway';
        Object.assign(this.root.style, {
            position: 'relative',
            display: 'flex',
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
    }

    setPlayhead(columnIndex) {
        this.playhead = columnIndex;
        this._repositionAll();
    }

    destroy() {
        this.root.remove();
    }

    /// Distance from the bottom of the lane to the now line, in px. This
    /// is where past notes start scrolling off. Equal to `lookbehind`
    /// columns worth of vertical space.
    _nowLineBottomPx() {
        const usable = this.options.height - 20; // padding above/below
        const totalSlots = this.options.lookahead + this.options.lookbehind;
        const slotPx = usable / totalSlots;
        return `${this.options.lookbehind * slotPx + 8}px`;
    }

    _slotHeight() {
        const usable = this.options.height - 20;
        const totalSlots = this.options.lookahead + this.options.lookbehind;
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

            this.root.appendChild(lane);
            this.lanes.push(lane);

            // Create a note element for every column where this string is
            // played. We position them via `top` in `_repositionAll`; here
            // we just create the DOM nodes once.
            for (let c = 0; c < this.score.columns.length; c++) {
                const fret = this.score.columns[c]?.[s];
                if (fret == null) continue;
                const note = document.createElement('div');
                note.textContent = String(fret);
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
    }

    _repositionAll() {
        if (!this.score) return;
        const slotPx = this._slotHeight();
        const { lookahead, lookbehind, height } = this.options;
        // The now line sits at this y-coordinate from the top:
        const nowY = height - 10 - lookbehind * slotPx;

        for (const { column, el } of this.noteEls) {
            const delta = column - this.playhead;
            // delta > 0 → note is in the future, above the now line.
            // delta = 0 → note is at the now line.
            // delta < 0 → note is in the past, below the now line.
            const y = nowY - delta * slotPx;
            const visible = delta <= lookahead && delta >= -lookbehind;
            el.style.top = `${y}px`;
            el.style.opacity = visible ? '1' : '0';
        }
    }
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
    description: 'Rocksmith-style highway. Notes drop down lanes toward a "now" line at the bottom.',
    version: '1.0.0',
    author: 'TWANGA',
    create(container, options) {
        return new HighwayRenderer(container, options);
    },
};
