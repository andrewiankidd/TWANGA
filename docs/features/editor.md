# Tab editor

Post-capture cell-level editing for recordings (or any `.alphatex` you can
load into the library). The GUI surface is a rendered-tab editor; the CLI
surface is a set of scriptable subcommands. Both write through the same
`AlphaTexWriter` path the Recorder uses on save, so edited output is
bit-for-bit indistinguishable from a fresh recording with the same notes.

## CLI — `twanga edit <path> <action>`

Each invocation applies one operation and writes the result back to the
file in place. Pass `--out <path>` to write to a different file instead
(handy for branching an edit without touching the original).

| Action | Description |
|--------|-------------|
| `set <column> <string> <fret>` | Set a single cell. `string` is 1-based (string 1 = top of tab); `column` is 0-based; `fret` is any non-negative integer (no upper cap). |
| `clear <column> <string>` | Clear a single cell. |
| `clear-col <column>` | Clear every cell in the column (rest the entire beat). |
| `insert-col [--after <n>]` | Insert a blank column. `--after N` inserts at position N+1; omit to append at the end. New column inherits the file's first column's duration. |
| `delete-col <column>` | Delete the column at `column`. Columns after it shift down by one. |
| `title <text>` | Set the `\title` directive. Pass `""` to clear. |
| `bpm <n>` | Set the `\tempo` (20–400 BPM). |

Examples:

```bash
# Bump string 1 / col 0 to fret 7 in twinkle
twanga edit assets/examples/twinkle-twinkle-uke.alphatex set 0 1 7

# Branch an edit to a new file instead of overwriting
twanga edit my-take.alphatex --out my-take-edited.alphatex bpm 90

# Insert a rest column at the start
twanga edit foo.alphatex insert-col

# Insert a column after column 7
twanga edit foo.alphatex insert-col --after 7

# Chain edits with a shell script — one op per invocation, file is the state
twanga edit foo.alphatex set 0 1 3 && \
twanga edit foo.alphatex set 0 2 0 && \
twanga edit foo.alphatex title "Adjusted Take"
```

Each action returns 0 on success and a friendly error on out-of-range
inputs (column past the file's end, string outside the tuning's range,
etc.). The capo + subtitle round-trip correctly: the subtitle's human
name + `; capo=...` annotation are both preserved.

## GUI

Open the Editor card from the main menu (or `#editor`). The screen has the
same two-view shape as Playback: a **library list** first, then a
**per-tab edit view** once a tab is loaded.

### Library view

- **Tab rows** — bundled examples + user recordings, same as Playback.
  Bundled rows are tagged "read-only".
- Loading a bundled tab still works — the editor will route through
  "Save as new" since the bundled file can't be overwritten.

### Per-tab edit view

- **Title / BPM** — inline-editable inputs above the grid. The title
  flows into the alphaTex `\title` directive and into the library row's
  label.
- **Tuning + capo** — read-only in the editor (transposing belongs in
  Playback). The Editor preserves whatever the source file had.
- **Interactive grid** — the Tab renderer with click handlers:
  - **Left-click** a cell to bump the fret up (empty → 0 → 1 → …
    unbounded).
  - **Right-click** to bump down (0 → empty).
  - **Double-click** to type a number directly (covers "I want fret
    17 without 17 clicks").
  - **Click a column index header** to select that column.
  - **Live-note cell** sits between each row's string label and the
    fret body, showing the absolute pitch class for the fret in the
    currently-selected column on that string. Updates as you click
    different cells; useful for "is fret 7 on the A string really a
    D?" without doing the maths.
- **Column controls** — Insert column after (the selected column),
  Delete column, Clear column.
- **Dirty pill** — small "unsaved" badge when the in-memory grid
  differs from the last-saved snapshot.

### Save flow

- **Save** — overwrites the source IDB entry (user recordings) via
  `library.update({ id, title, alphatex })`.
- **Save as new** — always appends a fresh IDB row. Required path for
  bundled examples (read-only); also handy for branching an edit
  without losing the original.
- **Revert** — restore the in-memory snapshot from the last save
  point. Confirmation prompt before discarding dirty edits.

The serialised output goes through the same `serialize_recording` Rust
path the Recorder uses on save, so the Editor's output is bit-for-bit
indistinguishable from a fresh recording with the same notes.

## Where things live

- **Editor state** lives in memory only — there's no autosave. Closing
  the tab view without Save discards changes (Revert helps; cross-tab
  refresh doesn't pull a half-edited buffer).
- **Saved edits** land in the same IDB store as recordings
  (`twanga-tabs-v1` / `tabs`). The Playback library picks them up
  immediately via BroadcastChannel cross-tab sync.

## See also

- [Recorder feature](recorder.md) — the natural source of tabs to edit.
- [Playback feature](playback.md) — what you do with the result.
- [CLI overview](../CLI.md) · [GUI overview](../GUI.md).
