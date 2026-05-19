# Project status

**Functional end-to-end on the CLI.** The three core flows work today, with full tuning + capo support:

- **Tuner** — live multi-string display (per-target cents indicator) or chromatic mode (snap to nearest 12-TET note). Cable hum / silence are gated out.
- **Tab recorder** — capture what you play as alphaTex (open standard, alphaTab-compatible). Saved to `recordings/<timestamp>.alphatex` with per-block fret detection.
- **Tab playback** — load an alphaTex file, scroll a cursor through it at tempo, optional metronome click on each beat, optional "wait" practice mode that pauses until you play each note. `--loop` for full or section repeats. `--tuning <preset>` transposes the tab onto a different instrument (so a uke recording plays on banjo and vice versa).

**Tuning registry.** Built-in presets (`standard-guitar`, `standard-banjo`, `standard-ukulele`, `drop-d-guitar`, `tenor-banjo`, `tenor-ukulele`) ship from a TOML file compiled into the binary. The same schema covers user-defined tunings stored at `$CONFIG/twanga/tunings.toml`; the `twanga tunings add` subcommand walks the user through defining one interactively. Built-in slugs shadow user-defined ones to prevent silent overrides.

**Capo.** Per-string semitone offsets (`Capo::offsets: Vec<i32>`) that compose with any tuning, built-in or custom. `--capo 3` is a uniform capo; `--capo "0,2,2,2,2,2"` is a partial capo (drop-D style); `--capo "3,3,3,3,0"` keeps the banjo 5th-string drone open while capoing the body. Capo info round-trips through the alphaTex `\subtitle` field — `; capo=<spec>` — so a recording made with a capo replays without the user having to remember the value.

The Tauri shell (desktop UI) is the next milestone. Same domain code (`twanga-core`, `twanga-dsp`, `twanga-tabs`, etc.) — the GUI is just a different presentation layer.
