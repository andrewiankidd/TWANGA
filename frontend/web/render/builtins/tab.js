// Tab renderer — column-grid tab notation, mirrors the CLI's record/play view.
// One row per string (string 1 at top), one column per time slice. The playhead
// is a vertical bar overlaid at `playhead * cellWidth`.
//
// The plugin shape registered at the bottom of this file is the SAME shape a
// third-party plugin would export. Built-ins enjoy no privileges — they just
// happen to be registered at startup by the host. See ../registry.js.

class TabRenderer {
    constructor(container, options = {}) {
        this.container = container;
        this.options = {
            cellWidth: 30,
            cellHeight: 30,
            labelWidth: 56,
            ...options,
        };
        this.score = null;
        this.playhead = 0;

        this.root = document.createElement('div');
        this.root.className = 'tw-rt-tab';
        Object.assign(this.root.style, {
            display: 'flex',
            flexDirection: 'column',
            gap: '0',
            background: 'var(--card)',
            border: '1px solid var(--border)',
            borderRadius: 'var(--radius, 6px)',
            padding: '0.6rem',
            fontFamily: 'monospace',
            overflowX: 'auto',
            position: 'relative',
        });
        container.appendChild(this.root);

        // The grid + playhead live inside a relatively-positioned wrapper so
        // the absolute-positioned playhead can sit on top of the cells.
        this.wrap = document.createElement('div');
        Object.assign(this.wrap.style, {
            position: 'relative',
            display: 'inline-block', // hugs cell width, lets parent scroll
        });
        this.root.appendChild(this.wrap);

        this.grid = document.createElement('div');
        Object.assign(this.grid.style, {
            display: 'grid',
            // No gap — cells touch so the playhead position math stays
            // simple (`labelWidth + col * cellWidth`). Bar-line separators
            // come from individual cells' `borderLeft`.
        });
        this.wrap.appendChild(this.grid);

        this.playheadEl = document.createElement('div');
        Object.assign(this.playheadEl.style, {
            position: 'absolute',
            top: '0',
            bottom: '0',
            width: `${this.options.cellWidth}px`,
            background: 'var(--highlight)',
            opacity: '0.18',
            pointerEvents: 'none',
            borderRadius: '4px',
            transition: 'left 0.12s ease-out',
            display: 'none',
        });
        this.wrap.appendChild(this.playheadEl);
    }

    setScore(score) {
        this.score = score;
        this._rebuild();
        this._positionPlayhead();
    }

    setPlayhead(columnIndex) {
        this.playhead = columnIndex;
        this._positionPlayhead();
    }

    destroy() {
        this.root.remove();
    }

    _rebuild() {
        this.grid.replaceChildren();
        if (!this.score) {
            this.playheadEl.style.display = 'none';
            return;
        }
        const { tuning, columns, columnsPerBar } = this.score;
        const strings = tuning?.strings ?? [];
        const numCols = columns.length;
        const { cellWidth, cellHeight, labelWidth } = this.options;

        // Grid layout: first column for labels, then one per tab column.
        this.grid.style.gridTemplateColumns =
            `${labelWidth}px repeat(${numCols}, ${cellWidth}px)`;
        this.grid.style.gridTemplateRows = `repeat(${strings.length}, ${cellHeight}px)`;

        for (let s = 0; s < strings.length; s++) {
            // String label on the left of the row.
            const label = document.createElement('div');
            label.textContent = strings[s].name;
            Object.assign(label.style, {
                gridColumn: '1',
                gridRow: `${s + 1}`,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'flex-end',
                paddingRight: '0.5rem',
                color: 'var(--primary)',
                fontWeight: '700',
                fontSize: '0.85rem',
            });
            this.grid.appendChild(label);

            for (let c = 0; c < numCols; c++) {
                const cell = document.createElement('div');
                const fret = columns[c]?.[s];
                cell.textContent = fret == null ? '—' : String(fret);
                const onBar = columnsPerBar > 0 && c > 0 && c % columnsPerBar === 0;
                Object.assign(cell.style, {
                    gridColumn: `${c + 2}`,
                    gridRow: `${s + 1}`,
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    color: fret == null ? 'var(--muted)' : 'var(--text)',
                    fontSize: fret == null ? '0.8rem' : '0.95rem',
                    fontWeight: fret == null ? '400' : '700',
                    borderLeft: onBar ? '1px solid var(--border)' : 'none',
                });
                this.grid.appendChild(cell);
            }
        }
    }

    _positionPlayhead() {
        if (!this.score || this.score.columns.length === 0) {
            this.playheadEl.style.display = 'none';
            return;
        }
        const { cellWidth, labelWidth } = this.options;
        const max = this.score.columns.length - 1;
        const clamped = Math.max(0, Math.min(max, this.playhead));
        const playheadX = labelWidth + clamped * cellWidth;
        this.playheadEl.style.display = 'block';
        this.playheadEl.style.left = `${playheadX}px`;

        // Auto-scroll horizontally so the playhead stays in view during
        // long tabs. `this.root` is the scrolling container (has
        // `overflowX: auto`). Centre the playhead when it leaves the
        // visible band — easier on the eyes than chasing it to an edge.
        const visibleStart = this.root.scrollLeft;
        const visibleEnd = visibleStart + this.root.clientWidth;
        const margin = 60;
        if (playheadX < visibleStart + margin || playheadX > visibleEnd - margin) {
            this.root.scrollTo({
                left: Math.max(0, playheadX - this.root.clientWidth / 2),
                behavior: 'smooth',
            });
        }
    }
}

export default {
    id: 'twanga.tab',
    name: 'Tab',
    description: 'Column-grid tab notation. One row per string, one column per time slice — same layout as the CLI.',
    version: '1.0.0',
    author: 'TWANGA',
    create(container, options) {
        return new TabRenderer(container, options);
    },
};
