# Project status

**Functional end-to-end on the CLI.** All three core flows work today:

- **Tuner** — live multi-string display (per-target cents indicator) or chromatic mode (snap to nearest 12-TET note). Cable hum / silence are gated out.
- **Tab recorder** — capture what you play as alphaTex (open standard, alphaTab-compatible). Saved to `recordings/<timestamp>.alphatex` with per-block fret detection.
- **Tab playback** — load an alphaTex file, scroll a cursor through it at tempo, optional metronome click on each beat, optional "wait" practice mode that pauses until you play each note. `--loop` for full or section repeats. `--tuning <preset>` transposes the tab onto a different instrument (so a uke recording plays on banjo and vice versa).

The Tauri shell (desktop UI) is the next milestone. Same domain code (`twanga-core`, `twanga-dsp`, `twanga-tabs`, etc.) — the GUI is just a different presentation layer.
