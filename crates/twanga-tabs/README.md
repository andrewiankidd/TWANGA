# twanga-tabs

Tab data: live capture (`TabRecorder`) and the alphaTex serialiser + parser (`alphatex::{AlphaTexWriter, parse, ParsedTab, TabColumn}`). MusicXML is a future open-standard interop target (placeholder module exists); proprietary binary formats (Guitar Pro `.gp5`/`.gpx`) are explicit non-goals — see [SCOPE.md](../../docs/SCOPE.md).

`TabRecorder` turns a stream of `(string_idx, fret, time)` events into a streaming series of `TabEvent`s — one per column tick — that the CLI renders to screen (multi-line refresh) and to disk (alphaTex). The writer is incremental: each completed column is appended to the file as it happens, so a Ctrl-C mid-recording still keeps everything that already made it into a full column. The alphaTex roundtrip is covered by tests: write some columns, parse them back, recover the original notes/chords/rests.

`alphatex::parse` handles the subset of alphaTex that the writer emits — `\subtitle`, `\tempo`, `\tuning`, the `.` body separator, `:N` durations (carries forward across bars), single notes (`fret.string`), chords (`(fret.string ...)`), rests (`r`), bar lines (`|`), and comments (`//`). Other header directives (e.g. `\title`, `\copyright`) are silently ignored, which is forward-compatible: files that work in alphaTab's web renderer still parse cleanly here.

`ParsedTab` exposes a few accessors that pull structured data out of the parsed file:

- `tuning()` — reconstructs the `Tuning` from the `\tuning` header.
- `transpose_to(target, max_fret)` — re-frets every note onto a different instrument's strings by absolute pitch. Used by `twanga play --tuning <preset>` so a uke tab can be played on banjo (and so on). Notes that can't be reached within `max_fret` on the target are dropped.
- `capo()` — extracts a `Capo` from the `\subtitle` field. alphaTex has no native `\capo` directive, so the writer co-opts the subtitle: a `; capo=<spec>` suffix is appended after the human-readable tuning name. alphaTab still renders the whole string as a subtitle; our loader splits it back via `twanga_core::split_capo_from_subtitle`.
- `subtitle_display()` — the subtitle with any `; capo=...` annotation stripped. Use this when you want to show the human-readable label without the machine tail.

`AlphaTexWriter::new(writer, tuning, &capo, bpm, denom)` emits the BASE tuning to `\tuning` and joins the capo into `\subtitle` via `twanga_core::join_capo_into_subtitle`. Files recorded before capo support — or with `Capo::none(n)` — produce a plain subtitle and are byte-identical with the pre-capo writer output, so the format change is backwards-compatible in both directions.

Bundled demo files live in [`assets/examples/`](../../assets/examples/) at the workspace root — see e.g. [`twinkle-twinkle-uke.alphatex`](../../assets/examples/twinkle-twinkle-uke.alphatex), an original public-domain arrangement that the `twanga play` command can load.

- **Check**: `cargo check -p twanga-tabs`
- **Test**: `cargo test -p twanga-tabs`
- **Depends on**: `twanga-core`
- **Used by**: `twanga-cli`, `twanga-web` (the browser Recorder's
  `serialize_recording` WASM binding runs the same `AlphaTexWriter`
  the CLI uses, so browser-saved recordings are byte-compatible)

See [the workspace README](../../README.md) for project context.
