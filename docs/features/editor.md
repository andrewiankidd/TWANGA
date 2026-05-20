# Tab editor

Post-capture cell-level editing for recordings (or any `.alphatex` you can
load into the library). Same Tab renderer Playback uses, in
`interactive: true` mode — so the editor's grid IS the rendered tab; there's
no separate spreadsheet view.

**GUI-only** — the CLI's role stays "capture + play"; mid-recording terminal
editing is unwieldy and existing tab editors (TuxGuitar etc.) cover the
needs better. [See SCOPE.md](../SCOPE.md).

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
