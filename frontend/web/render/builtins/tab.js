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
            // Interactive mode — when true, cells become clickable (with a
            // pointer cursor + hover halo) and an extra header row on top
            // gets per-column click targets. Editor uses this to drive
            // cell-by-cell edits without forking the renderer. Playback +
            // Record leave it false and get a read-only view.
            interactive: false,
            onCellClick: null,        // (stringIdx, colIdx, event) — left-click
            onCellContext: null,      // (stringIdx, colIdx, event) — right-click
            onCellDblClick: null,     // (stringIdx, colIdx, event) — double-click
            onColHeaderClick: null,   // (colIdx, event)
            selectedColumn: -1,       // column index to highlight (-1 = none)
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

    /// Update interactive-mode options (selected column, click handlers)
    /// without rebuilding the whole renderer. The editor calls this when
    /// the selection changes so the highlight follows clicks. Triggers a
    /// rebuild because cell styling depends on `selectedColumn` and the
    /// listener set is wired per-cell at build time.
    setInteractiveOptions(opts) {
        Object.assign(this.options, opts);
        if (this.score) this._rebuild();
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
        const { tuning, columns, columnsPerBar, capoSpec } = this.score;
        const strings = tuning?.strings ?? [];
        const numCols = columns.length;
        const { cellWidth, cellHeight, labelWidth, interactive, selectedColumn } = this.options;

        // Parse the score's capo spec into per-string offsets so we can
        // annotate the string labels. Empty / "0" spec → no annotation
        // (cheap path). Mirrors twanga-core's Capo::parse: "3" = uniform
        // 3 on every string, "0,2,2,2,2,2" = per-string, partial. The
        // CLI shows the same info in its score header.
        const capoOffsets = parseCapoForLabels(capoSpec, strings.length);
        const hasCapo = capoOffsets.some((n) => n > 0);

        // Grid layout: first column for labels, then one per tab column.
        // In interactive mode we reserve grid row 1 for the column-index
        // header (clickable to select a column for insert/delete ops);
        // string rows shift down by 1 in that case. The playhead math
        // doesn't care about row counts — it spans full height — so no
        // changes are needed there.
        const headerRows = interactive ? 1 : 0;
        const headerRowHeight = Math.round(cellHeight * 0.6);
        this.grid.style.gridTemplateColumns =
            `${labelWidth}px repeat(${numCols}, ${cellWidth}px)`;
        this.grid.style.gridTemplateRows =
            (headerRows ? `${headerRowHeight}px ` : '') +
            `repeat(${strings.length}, ${cellHeight}px)`;

        if (interactive) {
            // Top-left corner of the header row (empty cell over the
            // string labels). Placeholder so the grid lines up.
            const corner = document.createElement('div');
            Object.assign(corner.style, {
                gridColumn: '1',
                gridRow: '1',
            });
            this.grid.appendChild(corner);

            for (let c = 0; c < numCols; c++) {
                const head = document.createElement('div');
                head.textContent = String(c + 1);
                const isSel = c === selectedColumn;
                Object.assign(head.style, {
                    gridColumn: `${c + 2}`,
                    gridRow: '1',
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    fontSize: '0.7rem',
                    color: isSel ? 'var(--highlight)' : 'var(--muted)',
                    borderBottom: isSel
                        ? '1px solid var(--highlight)'
                        : '1px solid var(--border)',
                    cursor: 'pointer',
                    userSelect: 'none',
                });
                head.addEventListener('click', (e) => {
                    this.options.onColHeaderClick?.(c, e);
                });
                this.grid.appendChild(head);
            }
        }

        // Optional "capo: N" / "capo: [...]" badge in the top-left
        // corner (over the string-label column, where the column-index
        // header doesn't apply). Only shown when there's actually a
        // capo set; mirrors the CLI's "Capo:" header line.
        if (hasCapo) {
            const badge = document.createElement('div');
            badge.textContent = capoBadgeText(capoSpec);
            Object.assign(badge.style, {
                gridColumn: '1',
                gridRow: headerRows ? '1' : '1 / 1',
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'flex-end',
                paddingRight: '0.5rem',
                color: 'var(--highlight)',
                fontWeight: '700',
                fontSize: '0.65rem',
                letterSpacing: '0.5px',
                textTransform: 'uppercase',
            });
            // In interactive mode the header row above the string rows is
            // already a row in the grid; reuse row 1 (replaces the empty
            // corner placeholder set above). In non-interactive mode there
            // IS no header row, so we insert a leading row purely for the
            // badge by tweaking the template-rows below the fact:
            if (!headerRows) {
                const newRows = `${Math.round(cellHeight * 0.7)}px repeat(${strings.length}, ${cellHeight}px)`;
                this.grid.style.gridTemplateRows = newRows;
            }
            this.grid.appendChild(badge);
        }
        // String rows shift down by 1 if we added the (read-only) capo
        // badge row. Interactive mode's existing header row already
        // accounted for the offset.
        const stringRowOffset = headerRows + (hasCapo && !headerRows ? 1 : 0);

        for (let s = 0; s < strings.length; s++) {
            // String label on the left of the row, with optional "+N"
            // capo annotation when that string is fretted up by the
            // capo. Per-string offsets handle drop-D-style partial
            // capos correctly.
            const label = document.createElement('div');
            const off = capoOffsets[s] ?? 0;
            const baseName = strings[s].name;
            label.textContent = off > 0 ? `${baseName} +${off}` : baseName;
            label.title = off > 0
                ? `String ${s + 1} (${baseName}), capo +${off} semitones`
                : `String ${s + 1} (${baseName})`;
            Object.assign(label.style, {
                gridColumn: '1',
                gridRow: `${s + 1 + stringRowOffset}`,
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
                const isSelCol = interactive && c === selectedColumn;
                Object.assign(cell.style, {
                    gridColumn: `${c + 2}`,
                    gridRow: `${s + 1 + stringRowOffset}`,
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    color: fret == null ? 'var(--muted)' : 'var(--text)',
                    fontSize: fret == null ? '0.8rem' : '0.95rem',
                    fontWeight: fret == null ? '400' : '700',
                    borderLeft: onBar ? '1px solid var(--border)' : 'none',
                    background: isSelCol ? 'rgba(212, 154, 74, 0.08)' : 'transparent',
                    cursor: interactive ? 'pointer' : 'default',
                    userSelect: interactive ? 'none' : 'auto',
                });
                if (interactive) {
                    cell.addEventListener('click', (e) => {
                        this.options.onCellClick?.(s, c, e);
                    });
                    cell.addEventListener('contextmenu', (e) => {
                        e.preventDefault();
                        this.options.onCellContext?.(s, c, e);
                    });
                    cell.addEventListener('dblclick', (e) => {
                        this.options.onCellDblClick?.(s, c, e);
                    });
                }
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

/// Parse a Capo spec string ("" / "3" / "0,2,2,2,2,2") into a
/// per-string offset array. Mirrors twanga-core's Capo::parse but
/// in JS so the renderer is self-contained (no extra WASM call per
/// rebuild). Returns an array of length `stringCount`, padded with
/// zeros if the spec is shorter.
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

/// Short text label for the top-left capo badge. "capo 3" for a
/// uniform capo, "capo [0,2,2,2,2,2]" for a partial. Same human
/// shape as the CLI's "Capo:" header line, minus the "(uniform)"
/// / "(partial)" tag (the per-string `+N` annotations already make
/// that obvious in the GUI).
function capoBadgeText(spec) {
    if (!spec) return '';
    if (spec.includes(',')) return `capo [${spec}]`;
    return `capo ${spec}`;
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
