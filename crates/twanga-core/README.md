# twanga-core

Domain types for the TWANGA workspace — `Frequency`, `MidiNote`, `TunedString`, `Tuning`, `Capo` — plus a small set of music-theory helpers and the built-in tuning registry.

The bottom of the dependency graph. Every other crate depends on it for the value types they pass around. IO-free, async-free, no algorithms — anything that needs the filesystem, an audio device, or a runtime belongs higher up. (The `presets.toml` file is included at compile time via `include_str!` and parsed once into a `LazyLock`, so the crate is still "no runtime IO" in the way that matters.)

Public surface worth knowing:

**Tuning registry.** Built-in presets live in [`src/presets.toml`](src/presets.toml) and are also the schema the CLI uses for user-defined tunings at `$CONFIG/twanga/tunings.toml` — same `PresetEntry` / `PresetString` / `PresetFile` types, so promoting a user creation to a built-in is just copying the TOML block.

- `Tuning::builtin_presets()` / `Tuning::builtin_slugs()` — enumerate everything shipped (`standard-guitar`, `standard-banjo`, `standard-ukulele`, `drop-d-guitar`, `tenor-banjo`, `tenor-ukulele`).
- `Tuning::from_preset(slug)` — build a `Tuning` from a built-in slug (returns `None` for unknowns; user-defined tunings live one layer up in `twanga-cli`).
- `Tuning::standard_guitar()` / `standard_banjo()` / `standard_ukulele()` — convenience constructors, used by `twanga-dsp` and `twanga-tabs` tests.
- `PresetEntry::to_tuning()` / `PresetEntry::from_tuning(slug, &tuning)` — round-trip between the on-disk schema and the runtime type.

**Pitch + fret math.**

- `Tuning::nearest_string(freq)` — closest open string by absolute cents. Used by the tuner's per-string display.
- `Tuning::match_to_fret(freq, max_fret)` — closest playable `(string, fret)` with smallest non-negative fret (so D5 on uke registers as A-string fret 5, not C-string fret 14). Used by the recorder.
- `MidiNote::nearest_to(freq)` — chromatic snap with signed cents off. Used by the chromatic tuner mode.
- `MidiNote::name()` / `MidiNote::from_name(&str)` / `MidiNote::to_frequency()` — round-trips between MIDI numbers and pitch names like `A4`, `C#3`.

**Capo.** Per-string semitone offsets that compose with any `Tuning`:

- `Capo::none(n)` / `Capo::uniform(n, semitones)` — common constructors.
- `Capo::apply(&tuning)` — produces the effective tuning. Validates string count + MIDI range.
- `Capo::parse(spec, n)` / `Capo::serialize()` — accepts `"3"` (uniform) or `"0,2,2,2,2,2"` (per-string) and round-trips back.
- `Capo::is_none()` / `Capo::is_uniform()` — discriminate for display ("Capo: 3" vs "Capo: [0,2,2,2,2,2]").
- `join_capo_into_subtitle(name, &capo)` / `split_capo_from_subtitle(s)` + `CAPO_SUBTITLE_TOKEN` — the convention `twanga-tabs` uses to persist a capo through the alphaTex `\subtitle` field (since alphaTex has no native `\capo` directive).

**MOTD splashes.** `SPLASHES` / `splashes()` — backronyms for the CLI banner (`twanga_tui::motd`), shared with the future Tauri main menu (`twanga_app`). One source of truth so the two surfaces can't drift.

- **Check**: `cargo check -p twanga-core`
- **Test**: `cargo test -p twanga-core`
- **Depends on**: `serde`, `toml` (compile-time preset parsing only — no runtime IO)
- **Used by**: every other workspace crate

See [the workspace README](../../README.md) for project context.
