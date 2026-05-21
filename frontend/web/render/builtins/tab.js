// Tab renderer — column-grid tab notation, mirrors the CLI's record/play view.
// One row per string (string 1 at top), one column per time slice. The playhead
// is a vertical bar overlaid at `playhead * cellWidth`.
//
// The plugin shape registered at the bottom of this file is the SAME shape a
// third-party plugin would export. Built-ins enjoy no privileges — they just
// happen to be registered at startup by the host. See ../registry.js.

// Sharps-only pitch-class table indexed by `midi % 12`. Mirrors
// twanga-core's PITCH_CLASS_NAMES so the JS-side live-note cell and
// the CLI's equivalent show the same letter+accidental for the same
// played MIDI. No octave — the open-string label in the row's first
// cell already establishes octave context.
const PITCH_CLASS_NAMES = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B'];
function pitchClassName(midi) {
    return PITCH_CLASS_NAMES[((midi % 12) + 12) % 12];
}

class TabRenderer {
    constructor(container, options = {}) {
        this.container = container;
        this.options = {
            cellWidth: 30,
            cellHeight: 30,
            labelWidth: 56,
            // Width of the live-note column that sits between the
            // string label and the tab body. Just wide enough for
            // "F#" — keeps the tab-body alignment tight while still
            // giving the user "what note is this fret" at a glance.
            noteColWidth: 32,
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
            // Pre-roll runway — N faded cells rendered BEFORE col 0 so a
            // negative playhead (count-in) has somewhere visible to sit.
            // The host (playback / recorder) calls setPreRoll(N) to keep
            // this in sync with the user's pre-roll setting.
            preRoll: 0,
            // Body-cell label mode — 'fret' (default) shows the fret
            // number ("0", "3", "12"); 'note' shows the absolute pitch
            // class ("C", "F#") that the fret produces given the string's
            // open MIDI + capo offset. Same uniform contract on every
            // renderer; host toggles via setCellLabel(mode).
            cellLabel: 'fret',
            ...options,
        };
        this.score = null;
        this.playhead = 0;
        // Per-string handles for the live-note cells. Populated on
        // _rebuild, mutated in-place by _refreshLiveNotes(); no grid
        // rebuild required when the playhead moves.
        this.noteCells = [];

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
        this._refreshLiveNotes();
    }

    /// Host-driven pre-roll size. Triggers a rebuild because the
    /// runway is implemented as extra leading grid cells (cheaper than
    /// trying to fake them with margins / transforms). The host calls
    /// this on load and whenever the user changes the pre-roll value;
    /// no-op if the value hasn't actually changed.
    setPreRoll(n) {
        const next = Math.max(0, Math.floor(n) || 0);
        if (next === this.options.preRoll) return;
        this.options.preRoll = next;
        if (this.score) {
            this._rebuild();
            this._positionPlayhead();
        }
    }

    /// Toggle body-cell labels between 'fret' (default) and 'note'
    /// (absolute pitch class). Same contract as the highway renderer.
    /// Unrecognised modes are coerced to 'fret' so the host can't
    /// drift the renderer into an undefined state. Triggers a rebuild
    /// because every body cell's text content depends on the mode —
    /// cheaper to rebuild than to track per-cell handles for a rare
    /// toggle.
    setCellLabel(mode) {
        const next = mode === 'note' ? 'note' : 'fret';
        if (next === this.options.cellLabel) return;
        this.options.cellLabel = next;
        if (this.score) {
            this._rebuild();
            this._positionPlayhead();
        }
    }

    /// Update interactive-mode options (selected column, click handlers)
    /// without rebuilding the whole renderer. The editor calls this when
    /// the selection changes so the highlight follows clicks. Triggers a
    /// rebuild because cell styling depends on `selectedColumn` and the
    /// listener set is wired per-cell at build time.
    setInteractiveOptions(opts) {
        Object.assign(this.options, opts);
        if (this.score) {
            this._rebuild();
            // _rebuild already calls _refreshLiveNotes, but call it
            // again explicitly so the dependency is obvious.
            this._refreshLiveNotes();
        }
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
        const { noteColWidth, preRoll } = this.options;
        // Grid layout: `[labelWidth] [noteColWidth] [runway × preRoll] [body × numCols]`.
        // Runway cells live in grid columns 3..3+preRoll-1; body cells
        // live in grid columns 3+preRoll..3+preRoll+numCols-1. Existing
        // body references (`${c + 3}`) shift to `${c + 3 + preRoll}`.
        this.grid.style.gridTemplateColumns =
            `${labelWidth}px ${noteColWidth}px repeat(${preRoll + numCols}, ${cellWidth}px)`;
        this.grid.style.gridTemplateRows =
            (headerRows ? `${headerRowHeight}px ` : '') +
            `repeat(${strings.length}, ${cellHeight}px)`;

        if (interactive) {
            // Top-left corner of the header row (empty cell over the
            // string labels). Placeholder so the grid lines up — now
            // spans both the label and live-note columns.
            const corner = document.createElement('div');
            Object.assign(corner.style, {
                gridColumn: '1 / span 2',
                gridRow: '1',
            });
            this.grid.appendChild(corner);

            for (let c = 0; c < numCols; c++) {
                const head = document.createElement('div');
                head.textContent = String(c + 1);
                const isSel = c === selectedColumn;
                Object.assign(head.style, {
                    gridColumn: `${c + 3 + preRoll}`,
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
        // corner (over the string-label + live-note columns, where the
        // column-index header doesn't apply). Only shown when there's
        // actually a capo set; mirrors the CLI's "Capo:" header line.
        if (hasCapo) {
            const badge = document.createElement('div');
            badge.textContent = capoBadgeText(capoSpec);
            Object.assign(badge.style, {
                gridColumn: '1 / span 2',
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

        // Reset the live-note-cell handle array; populated as we
        // build each string row below.
        this.noteCells = new Array(strings.length).fill(null);

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

            // Live-note cell. Shows the absolute pitch class for the
            // fret being played on this string at the current playhead
            // (or selectedColumn in interactive mode). Empty when this
            // string isn't playing at the active column. Populated
            // initially via _refreshLiveNotes at the end of _rebuild;
            // updated in-place on every setPlayhead call after that.
            const noteCell = document.createElement('div');
            noteCell.title = `Live note — ${baseName} + current fret`;
            Object.assign(noteCell.style, {
                gridColumn: '2',
                gridRow: `${s + 1 + stringRowOffset}`,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                color: 'var(--highlight)',
                fontWeight: '700',
                fontSize: '0.85rem',
                fontFamily: 'monospace',
                borderRight: '1px solid var(--border)',
            });
            this.grid.appendChild(noteCell);
            this.noteCells[s] = noteCell;

            // Runway cells — `preRoll` faded "·" cells before col 0, one
            // per pre-roll tick. Purely cosmetic: they give the playhead
            // bar somewhere visible to sit during the count-in instead
            // of being hidden until column 0 lands. Styled muted so the
            // user reads them as "not real tab content."
            for (let r = 0; r < preRoll; r++) {
                const runway = document.createElement('div');
                runway.textContent = '·';
                Object.assign(runway.style, {
                    gridColumn: `${r + 3}`,
                    gridRow: `${s + 1 + stringRowOffset}`,
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    color: 'var(--muted)',
                    opacity: '0.35',
                    fontSize: '0.95rem',
                    // Bar-line separator on the LAST runway cell so the
                    // boundary between count-in and real tab is obvious.
                    borderRight: r === preRoll - 1 ? '1px dashed var(--border)' : 'none',
                });
                this.grid.appendChild(runway);
            }

            for (let c = 0; c < numCols; c++) {
                const cell = document.createElement('div');
                const fret = columns[c]?.[s];
                cell.textContent = formatCellLabel(
                    fret, strings[s]?.midi, capoOffsets[s] ?? 0, this.options.cellLabel,
                );
                const onBar = columnsPerBar > 0 && c > 0 && c % columnsPerBar === 0;
                const isSelCol = interactive && c === selectedColumn;
                Object.assign(cell.style, {
                    gridColumn: `${c + 3 + preRoll}`,
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

        // Populate the live-note cells against the current playhead /
        // selectedColumn. Subsequent setPlayhead / setInteractiveOptions
        // calls update these in place without rebuilding.
        this._refreshLiveNotes();
    }

    /// Update the per-string live-note cells to reflect the active
    /// column (selectedColumn in interactive mode if valid; otherwise
    /// the playhead). Cheap — just text-content writes, no DOM
    /// reshape. Called from _rebuild, setPlayhead, and
    /// setInteractiveOptions.
    _refreshLiveNotes() {
        if (!this.score || this.noteCells.length === 0) return;
        const { columns, tuning, capoSpec } = this.score;
        const strings = tuning?.strings ?? [];
        const capoOffsets = parseCapoForLabels(capoSpec, strings.length);
        const { interactive, selectedColumn } = this.options;
        // Editor (interactive) tracks the user's selected-for-edit
        // column independently from the playhead. Read-only mode
        // (Playback / Recorder) follows the playhead. Either way, the
        // live-note cell shows the note for whichever column is
        // "active" for this surface. During pre-roll the playhead is
        // negative (the highway renderer shows notes scrolling in from
        // above); for the tab side we clamp to 0 so the cell shows
        // what's *about to* play — the imminent first note.
        const activeCol = interactive && selectedColumn >= 0 && selectedColumn < columns.length
            ? selectedColumn
            : Math.max(0, this.playhead);
        for (let s = 0; s < this.noteCells.length; s++) {
            const cell = this.noteCells[s];
            if (!cell) continue;
            const fret = columns[activeCol]?.[s];
            const baseMidi = strings[s]?.midi;
            if (fret == null || baseMidi == null) {
                cell.textContent = '';
                continue;
            }
            const off = capoOffsets[s] ?? 0;
            cell.textContent = pitchClassName(baseMidi + off + fret);
        }
    }

    _positionPlayhead() {
        if (!this.score || this.score.columns.length === 0) {
            this.playheadEl.style.display = 'none';
            return;
        }
        const { cellWidth, labelWidth, noteColWidth, preRoll } = this.options;
        const max = this.score.columns.length - 1;
        // Pre-roll: playhead can be negative down to `-preRoll`, where
        // the bar sits over the leftmost runway cell. When preRoll is
        // 0 and playhead < 0, hide the bar (no runway to sit on).
        if (this.playhead < 0 && preRoll === 0) {
            this.playheadEl.style.display = 'none';
            return;
        }
        const clamped = Math.max(-preRoll, Math.min(max, this.playhead));
        // Body cells start `preRoll` slots after the label+note columns;
        // a playhead of -preRoll lands on runway cell 0, a playhead of 0
        // lands on body cell 0.
        const playheadX = labelWidth + noteColWidth + (clamped + preRoll) * cellWidth;
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

/// Format a single body-cell label. 'fret' mode shows the fret
/// number (or '—' for an empty cell); 'note' mode shows the absolute
/// pitch class for the fret on this string (with capo applied).
/// Duplicated in highway.js — see the parseCapoForLabels comment
/// for the "hoist when more renderers need it" note. Empty cells
/// render the same dash in either mode so the user can still see
/// the rhythmic grid.
function formatCellLabel(fret, baseMidi, capoOffset, mode) {
    if (fret == null) return '—';
    if (mode === 'note' && baseMidi != null) {
        return pitchClassName(baseMidi + (capoOffset ?? 0) + fret);
    }
    return String(fret);
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
