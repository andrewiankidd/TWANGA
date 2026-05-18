# twanga-core

Domain types for the TWANGA workspace — `Frequency`, `MidiNote`, `TunedString`, `Tuning` — plus a small set of music-theory helpers.

The bottom of the dependency graph. Every other crate depends on it for the value types they pass around. IO-free, async-free, no algorithms — anything that needs the filesystem, an audio device, or a runtime belongs higher up.

Public helpers worth knowing:

- `Tuning::PRESETS` / `Tuning::from_preset(&str)` — single source of truth for the `standard-guitar` / `standard-banjo` / `standard-ukulele` preset slugs the CLI and (eventually) the GUI both consume.
- `Tuning::nearest_string(freq)` — closest open string by absolute cents. Used by the tuner's per-string display.
- `Tuning::match_to_fret(freq, max_fret)` — closest playable `(string, fret)` with smallest non-negative fret (so D5 on uke registers as A-string fret 5, not C-string fret 14). Used by the recorder.
- `MidiNote::nearest_to(freq)` — chromatic snap with signed cents off. Used by the chromatic tuner mode.
- `MidiNote::name()` / `MidiNote::to_frequency()` — round-trips between MIDI numbers and pitch names like `A4`, `C#3`.
- `SPLASHES` / `splashes()` — MOTD splash list (150 backronyms for TWANGA, Minecraft-style). Lives here so the CLI banner (`twanga_tui::motd`) and the future Tauri main menu (`twanga_app`) draw from a single shared list.

- **Check**: `cargo check -p twanga-core`
- **Test**: `cargo test -p twanga-core`
- **Depends on**: nothing (std only)
- **Used by**: every other workspace crate

See [the workspace README](../../README.md) for project context.
