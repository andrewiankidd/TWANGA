# TWANGA

![logo](assets/logo.png)

TWANGA is an open-source learning tool for fretted/strung instruments. Bring-your-own-tabs, arbitrary tunings as first-class, no subscription, no library lock-in. Built for people who play banjo, ukulele, mandolin, or anything else with strings and frets — not just guitar.

## About

Most instrument-trainer software is locked: subscription pricing, walled song libraries, single-instrument tunnel vision (almost always 6-string guitar + 4-string bass), and tunings hard-coded into the data model. TWANGA inverts the trade-off — you own the inputs:

- **Tabs** — record what you play, play back tabs, transpose across instruments, capo per-string. Native format is alphaTex (the open text format from the alphaTab project); MusicXML interop is a future open-standard target. Proprietary formats (Guitar Pro `.gp5`/`.gpx`) are explicit non-goals.
- **Instrument** — `Tuning` is just `Vec<TunedString>`, so drop-D, the banjo's high-pitched 5th-string drone, the ukulele's reentrant G, baritone configurations, and 7-string layouts all fall out of the same data model.
- **Audio device** — any CPAL-supported input. ASIO behind a feature flag on Windows for lower-latency USB instrument cables.

The author is an amateur dev and amateur musician learning Rust through this project. Set your expectations accordingly.

## Workspace layout

Multi-crate Cargo workspace. Each crate has a narrow, deliberately-enforced responsibility:

| Crate | Role |
|-------|------|
| **[twanga-core](crates/twanga-core/)** | Domain types (`Frequency`, `MidiNote`, `TunedString`, `Tuning`) + the preset registry, nearest-string lookup, and fret-aware string matching. IO-free, async-free. |
| **[twanga-dsp](crates/twanga-dsp/)** | Pure pitch detection (`Yin`) + the streaming `Tuner` that wraps it. No allocations after first call. Pinned to `opt-level = 3` in dev so `cargo run` is usable without `--release`. |
| **[twanga-synth](crates/twanga-synth/)** | Deterministic audio synthesis (sines, harmonic stacks, envelopes, noise) used as a `dev-dependency` by `twanga-dsp` tests and at runtime for the metronome click. |
| **[twanga-audio](crates/twanga-audio/)** | Realtime audio capture (`InputStream`) and playback (`OutputStream`), wrapping CPAL. ASIO behind the `asio` cargo feature on Windows. |
| **[twanga-tabs](crates/twanga-tabs/)** | Tab data: live `TabRecorder`, alphaTex serialiser + parser. MusicXML is a future open-standard interop target; proprietary formats (`.gp5`/`.gpx`) are explicit non-goals. |
| **[twanga-tui](crates/twanga-tui/)** | Terminal UX primitives shared across TWANGA's CLIs — selection menus, refreshing displays, Ctrl-C handling, ANSI colours. |
| **[twanga-cli](crates/twanga-cli/)** | CLI binary `twanga` — `tune`, `record`, `play`, `tunings`, `devices`, `convert` subcommands. |
| **[twanga-bench](crates/twanga-bench/)** | Latency + pitch-detection accuracy benchmarks (placeholder). |
| **[twanga-app](crates/twanga-app/)** | Tauri 2 desktop shell — hosts the shared [frontend/web/](frontend/web/) bundle (HTML + WASM bindings) in a native window. Same UI as the web build, just wrapped in a Tauri webview. |

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

# Play with a capo on fret 3 (uniform). Pass `0,2,2,2,2,2` for a partial capo.
cargo run -p twanga-cli -- play assets/examples/twinkle-twinkle-uke.alphatex --capo 3

# Manage the tuning registry — list built-ins + user tunings, or define your own
cargo run -p twanga-cli -- tunings list
cargo run -p twanga-cli -- tunings add
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

## CLI screenshots

Each subcommand opens with a banner and a randomly-picked splash; the body is whatever flow you asked for. Notes below are from running the three flows against the bundled Twinkle Twinkle uke example.

### Tuner — `twanga tune`

```
$ cargo run -p twanga-cli -- tune

    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.13s
     Running `target\debug\twanga.exe tune`

════════════════════════════════════════════════════════════════
████████ ██     ██  █████  ███    ██  ██████   █████
   ██    ██     ██ ██   ██ ████   ██ ██       ██   ██
   ██    ██  █  ██ ███████ ██ ██  ██ ██   ███ ███████
   ██    ██ ███ ██ ██   ██ ██  ██ ██ ██    ██ ██   ██
   ██     ███ ███  ██   ██ ██   ████  ██████  ██   ██
════════════════════════════════════════════════════════════════
  Twang, Wince, Apologise, Notate, Groan, Again
════════════════════════════════════════════════════════════════

Choose a tuning:
   1) (chromatic — guesses the nearest note)
   2) standard-guitar
   3) standard-banjo
   4) standard-ukulele
> 4

Tuning: Standard Ukulele (Reentrant GCEA) (4 strings)
Device: Microphone (Default)
Audio:  48000 Hz, 1 channel(s)

─────────────────────────────────────────────────
  Controls: type 'q' + Enter to stop  (or Ctrl-C)
─────────────────────────────────────────────────

A4               | current:    442.10 Hz | target:    440.00 Hz | Tune Down! (+8.2 cents)
E4               | current:    326.40 Hz | target:    329.63 Hz | Tune Up! (-17.0 cents)
C4               | current:    261.10 Hz | target:    261.63 Hz | Tuned! (-3.5 cents)
g4 (reentrant)   | current:    392.10 Hz | target:    392.00 Hz | Tuned! (+0.4 cents)
```

### Recorder — `twanga record`

```
$ cargo run -p twanga-cli -- record

    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.13s
     Running `target\debug\twanga.exe record`

════════════════════════════════════════════════════════════════
████████ ██     ██  █████  ███    ██  ██████   █████
   ██    ██     ██ ██   ██ ████   ██ ██       ██   ██
   ██    ██  █  ██ ███████ ██ ██  ██ ██   ███ ███████
   ██    ██ ███ ██ ██   ██ ██  ██ ██ ██    ██ ██   ██
   ██     ███ ███  ██   ██ ██   ████  ██████  ██   ██
════════════════════════════════════════════════════════════════
  Trustworthy, Without Ads, No Garbage Attached
════════════════════════════════════════════════════════════════

Choose a tuning to record against:
   1) standard-guitar
   2) standard-banjo
   3) standard-ukulele
> 3
Tempo (BPM) [120]:
Resolution:
   1) 1/4
 * 2) 1/8
   3) 1/16
   4) 1/32
>
Block width (columns per scrolling block) [32]:

Tuning:     Standard Ukulele (Reentrant GCEA) (4 strings)
Device:     Microphone (Default)
Audio:      48000 Hz
Tempo:      120 BPM, 1/8 notes (250 ms/col)
Block:      32 cols (8000 ms wide)
Saving to:  recordings/recording-1779133041.alphatex

─────────────────────────────────────────────────
  Controls: type 'q' + Enter to stop  (or Ctrl-C)
─────────────────────────────────────────────────

A4             | --------0000--------------------
E4             | ----------------11110000--------
C4             | 0000--------------------222200--
g4 (reentrant) | ----0000----00------------------

A4             | --------------------------------
E4             | ----11110000--------11110000----
C4             | ------------22--------------22--
g4 (reentrant) | 0000----------------0000--------

A4             | --------0000--------------------
E4             | ----------------11110000--------
C4             | 0000--------------------222200--
g4 (reentrant) | ----0000----00------------------
```

### Playback — `twanga play ... --bpm 60 --wait`

```
$ cargo run -p twanga-cli -- play assets/examples/twinkle-twinkle-uke.alphatex --bpm 60 --wait

    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.13s
     Running `target\debug\twanga.exe play assets/examples/twinkle-twinkle-uke.alphatex --bpm 60 --wait`

════════════════════════════════════════════════════════════════
████████ ██     ██  █████  ███    ██  ██████   █████
   ██    ██     ██ ██   ██ ████   ██ ██       ██   ██
   ██    ██  █  ██ ███████ ██ ██  ██ ██   ███ ███████
   ██    ██ ███ ██ ██   ██ ██  ██ ██ ██    ██ ██   ██
   ██     ███ ███  ██   ██ ██   ████  ██████  ██   ██
════════════════════════════════════════════════════════════════
  Tunes Whatever's Approximately Notated, Generously Allowing
════════════════════════════════════════════════════════════════

Choose a tuning for playback:
 * 1) (as recorded in file: A4 E4 C4 G4)
   2) standard-guitar
   3) standard-banjo
   4) standard-ukulele
> 4
Playback:   assets/examples/twinkle-twinkle-uke.alphatex
Subtitle:   for standard ukulele (GCEA, reentrant)
Transposed: standard-ukulele (A4 E4 C4 G4)
Tempo:      60 BPM, 1/4 notes (1000 ms/col)
Metronome:  on
Wait mode:  on
Loop:       off

─────────────────────────────────────────────────
  Controls: type 'q' + Enter to stop  (or Ctrl-C)
─────────────────────────────────────────────────

A4 | ----00------[-]-----------
E4 | --------1100[-]-----1100--
C4 | 00----------[2]20-------2-
G4 | --00--0-----[-]---00------
     col 13/48  (bar 4, beat 1)
```

## Project docs

- [Project status](docs/PROJECT_STATUS.md) — current capability and the next milestone.
- [Roadmap](docs/ROADMAP.md) — milestone table.
- [Scope](docs/SCOPE.md) — what TWANGA deliberately isn't.

## Acknowledgements

- **[slopsmith](https://github.com/byrongamatos/slopsmith)** — found while scoping TWANGA; I haven't used it. Different audience and a different problem, but excellent prior art on plugin architecture, A-B loop UX, and JUCE-based desktop wrappers around web UIs. Worth studying carefully before TWANGA ever ships a plugin system.
- **[alphaTab](https://github.com/CoderLine/alphaTab)** — the open-source tab project whose `alphaTex` text format the recorder writes to. We render tabs ourselves via a small pluggable renderer system in `frontend/web/render/` (column-grid and Rocksmith-style highway built in, custom renderers welcome) so user-extensible visual paradigms aren't locked behind alphaTab's engraving choices, but alphaTab remains the natural choice for engraved-staff tab rendering if and when we add it as a renderer plugin.
- **[CPAL](https://github.com/RustAudio/cpal)** and the wider [RustAudio](https://github.com/RustAudio) ecosystem — the realtime cross-platform audio stack that made Rust a viable choice over Godot, Flutter, or JUCE.
- **[TuxGuitar](https://sourceforge.net/projects/tuxguitar/)** — long-standing open-source tab editor; reference for tab-editing UX even if not embeddable (GPL).
- **de Cheveigné & Kawahara, _YIN, a fundamental frequency estimator for speech and music_ (2002)** — the pitch-detection algorithm TWANGA leans on for short-sustain plucked-string detection on banjo and ukulele.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option (per Rust ecosystem convention).
