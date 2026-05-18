# twanga-tabs

Tab data: live capture (`TabRecorder`), alphaTex serialiser and parser (`alphatex::{AlphaTexWriter, parse, ParsedTab, TabColumn}`), plus stubs for the `gp5` / `musicxml` formats Twanga will eventually import from.

`TabRecorder` turns a stream of `(string_idx, fret, time)` events into a streaming series of `TabEvent`s — one per column tick — that the CLI renders to screen (multi-line refresh) and to disk (alphaTex). The writer is incremental: each completed column is appended to the file as it happens, so a Ctrl-C mid-recording still keeps everything that already made it into a full column. The alphaTex roundtrip is covered by tests: write some columns, parse them back, recover the original notes/chords/rests.

`alphatex::parse` handles the subset of alphaTex that the writer emits — `\tempo`, `\tuning`, the `.` body separator, `:N` durations (carries forward across bars), single notes (`fret.string`), chords (`(fret.string ...)`), rests (`r`), bar lines (`|`), and comments (`//`). Other directives (e.g. `\title`, `\subtitle`) are silently ignored, which is forward-compatible: files that work in alphaTab's web renderer still parse cleanly here.

`ParsedTab::tuning()` reconstructs the `Tuning` from the `\tuning` header. `ParsedTab::transpose_to(target, max_fret)` re-frets every note onto a different instrument's strings by absolute pitch — used by `twanga play --tuning <preset>` so a uke tab can be played on banjo (and so on). Notes that can't be reached within `max_fret` on the target are dropped.

Bundled demo files live in [`assets/examples/`](../../assets/examples/) at the workspace root — see e.g. [`twinkle-twinkle-uke.alphatex`](../../assets/examples/twinkle-twinkle-uke.alphatex), an original public-domain arrangement that the `twanga play` command can load.

- **Check**: `cargo check -p twanga-tabs`
- **Test**: `cargo test -p twanga-tabs`
- **Depends on**: `twanga-core`
- **Used by**: `twanga-cli`

See [the workspace README](../../README.md) for project context.
