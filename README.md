# TWANGA

![logo](assets/logo.png)

TWANGA is an open-source learning tool for fretted/strung instruments. Bring-your-own-tabs, arbitrary tunings as first-class, no subscription, no library lock-in. Built for people who play banjo, ukulele, mandolin, or anything else with strings and frets — not just guitar.

## About

Most instrument-trainer software is locked: subscription pricing, walled song libraries, single-instrument tunnel vision (almost always 6-string guitar + 4-string bass), and tunings hard-coded into the data model. TWANGA inverts the trade-off — you own the inputs:

- **Tabs** — `.gp5`, `.gpx`, `.xml`, alphaTex. Imported from your own files, not hosted or aggregated.
- **Instrument** — `Tuning` is just `Vec<TunedString>`, so drop-D, the banjo's high-pitched 5th-string drone, the ukulele's reentrant G, baritone configurations, and 7-string layouts all fall out of the same data model.
- **Audio device** — any CPAL-supported input. ASIO behind a feature flag on Windows for lower-latency USB instrument cables.

The author is an amateur dev and amateur musician learning Rust through this project. Set your expectations accordingly.

## Project status

**Functional end-to-end on the CLI.** All three core flows work today:

- **Tuner** — live multi-string display (per-target cents indicator) or chromatic mode (snap to nearest 12-TET note). Cable hum / silence are gated out.
- **Tab recorder** — capture what you play as alphaTex (open standard, alphaTab-compatible). Saved to `recordings/<timestamp>.alphatex` with per-block fret detection.
- **Tab playback** — load an alphaTex file, scroll a cursor through it at tempo, optional metronome click on each beat, optional "wait" practice mode that pauses until you play each note. `--loop` for full or section repeats. `--tuning <preset>` transposes the tab onto a different instrument (so a uke recording plays on banjo and vice versa).

The Tauri shell (desktop UI) is the next milestone. Same domain code (`twanga-core`, `twanga-dsp`, `twanga-tabs`, etc.) — the GUI is just a different presentation layer.

## Workspace layout

Multi-crate Cargo workspace. Each crate has a narrow, deliberately-enforced responsibility:

| Crate | Role |
|-------|------|
| **[twanga-core](crates/twanga-core/)** | Domain types (`Frequency`, `MidiNote`, `TunedString`, `Tuning`) + the preset registry, nearest-string lookup, and fret-aware string matching. IO-free, async-free. |
| **[twanga-dsp](crates/twanga-dsp/)** | Pure pitch detection (`Yin`) + the streaming `Tuner` that wraps it. No allocations after first call. Pinned to `opt-level = 3` in dev so `cargo run` is usable without `--release`. |
| **[twanga-synth](crates/twanga-synth/)** | Deterministic audio synthesis (sines, harmonic stacks, envelopes, noise) used as a `dev-dependency` by `twanga-dsp` tests and at runtime for the metronome click. |
| **[twanga-audio](crates/twanga-audio/)** | Realtime audio capture (`InputStream`) and playback (`OutputStream`), wrapping CPAL. ASIO behind the `asio` cargo feature on Windows. |
| **[twanga-tabs](crates/twanga-tabs/)** | Tab data: live `TabRecorder`, alphaTex serialiser + parser, stubs for `gp5` and `musicxml`. |
| **[twanga-tui](crates/twanga-tui/)** | Terminal UX primitives shared across TWANGA's CLIs — selection menus, refreshing displays, Ctrl-C handling, ANSI colours. |
| **[twanga-cli](crates/twanga-cli/)** | CLI binary `twanga` — `tune`, `record`, `play`, `devices`, `convert` subcommands. |
| **[twanga-bench](crates/twanga-bench/)** | Latency + pitch-detection accuracy benchmarks (placeholder). |
| **[twanga-app](crates/twanga-app/)** | Tauri shell (placeholder). Frontend in [frontend/](frontend/) (framework TBD), tab rendering via [alphaTab](https://github.com/CoderLine/alphaTab) in the webview. |

## Quick start

Once you have [Rust](https://rustup.rs) installed:

```bash
# Type-check the workspace
cargo check --workspace

# Run all tests
cargo test --workspace

# List the audio input devices CPAL can see
cargo run -p twanga-cli -- devices

# Live tuner (prompts to pick a tuning — or chromatic if no instrument)
cargo run -p twanga-cli -- tune

# Record what you play to recordings/<timestamp>.alphatex
cargo run -p twanga-cli -- record

# Play back the bundled Twinkle Twinkle (uke) demo with metronome
cargo run -p twanga-cli -- play assets/examples/twinkle-twinkle-uke.alphatex

# Practice mode — cursor waits until you play each note
cargo run -p twanga-cli -- play assets/examples/twinkle-twinkle-uke.alphatex --bpm 60 --wait

# Transpose a uke tab onto banjo so you can play it on a different instrument
cargo run -p twanga-cli -- play assets/examples/twinkle-twinkle-uke.alphatex --tuning standard-banjo
```

See [crates/twanga-cli/README.md](crates/twanga-cli/README.md) for a full subcommand + flag reference. Each subcommand also responds to `--help`.

On Windows, for lower-latency capture from USB instrument cables, enable the `asio` feature:

```bash
cargo run -p twanga-cli --features twanga-audio/asio -- tune
```

## Bundled examples

[assets/examples/](assets/examples/) ships a small set of public-domain demo `.alphatex` files for the `play` subcommand. Arrangements are original to TWANGA (MIT/Apache-2.0).

- [`twinkle-twinkle-uke.alphatex`](assets/examples/twinkle-twinkle-uke.alphatex) — Twinkle Twinkle Little Star, standard uke (GCEA), 12 bars.
- [`cripple-creek-banjo.alphatex`](assets/examples/cripple-creek-banjo.alphatex) — Cripple Creek (Appalachian trad), standard 5-string banjo (open G), clawhammer-style with drone, 16 bars.

## Roadmap

| Milestone | Status |
|-----------|--------|
| Workspace scaffold | done |
| Domain model (`Tuning`, `MidiNote`, `Frequency`) | done |
| Tuner (YIN + CPAL + multi-string UI + chromatic mode) | done |
| Tab recorder → alphaTex | done |
| Tab playback (cursor view + metronome + wait mode + loop) | done |
| Tauri shell init + first window | next |
| Tab rendering via alphaTab in the Tauri webview | after Tauri shell |
| Chord trainer with polyphonic *verification* (not transcription) | follows |
| Slow-down practice (time-stretch via `rubato` or signalsmith-stretch) | follows |
| Section looper / adaptive difficulty / tab fade-out | follows |
| Right-hand pattern trainer (banjo rolls, uke strums) | follows |
| GP5 / MusicXML import in `twanga-tabs` | follows |
| Mobile (Tauri Mobile) | v2 |

## Scope (what TWANGA isn't)

- **Not a tab library.** Tabs are a legal grey zone. The app ships empty. Users bring their own `.gp5` / `.gpx` / `.xml` files. Community sharing happens off-platform, like emulator ROMs.
- **Not a custom-content player for proprietary game formats.** That's [slopsmith](https://github.com/byrongamatos/slopsmith)'s niche.
- **No free polyphonic transcription in v1.** Polyphonic transcription remains an open problem in the open-source world. v1 covers monophonic transcription (record-to-tab) and polyphonic *verification* (classify against a known chord set). Free polyphonic transcription is an explicit stretch goal, not v1.
- **No runtime AI.** Pitch detection is deterministic DSP. AI is used during *development* (Claude Code), not in the shipped binary.
- **Mobile is v2.** Desktop tuner first; Android Oboe / AAudio quirks deferred.

## Acknowledgements

- **[slopsmith](https://github.com/byrongamatos/slopsmith)** — found while scoping TWANGA; I haven't used it. Different audience and a different problem, but excellent prior art on plugin architecture, A-B loop UX, and JUCE-based desktop wrappers around web UIs. Worth studying carefully before TWANGA ever ships a plugin system.
- **[alphaTab](https://github.com/CoderLine/alphaTab)** — the open-source tab renderer TWANGA drops into its Tauri webview, and the source of the alphaTex format the recorder writes to.
- **[CPAL](https://github.com/RustAudio/cpal)** and the wider [RustAudio](https://github.com/RustAudio) ecosystem — the realtime cross-platform audio stack that made Rust a viable choice over Godot, Flutter, or JUCE.
- **[TuxGuitar](https://sourceforge.net/projects/tuxguitar/)** — long-standing open-source tab editor; reference for tab-editing UX even if not embeddable (GPL).
- **de Cheveigné & Kawahara, _YIN, a fundamental frequency estimator for speech and music_ (2002)** — the pitch-detection algorithm TWANGA leans on for short-sustain plucked-string detection on banjo and ukulele.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option (per Rust ecosystem convention).
