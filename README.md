# TWANGA

![logo](assets/logo.png)

TWANGA is an open-source learning tool for fretted/strung instruments. Bring-your-own-tabs, arbitrary tunings as first-class, no subscription, no library lock-in. Built for people who play banjo, ukulele, mandolin, or anything else with strings and frets — not just guitar.

## About

Most instrument-trainer software is locked: subscription pricing, walled song libraries, single-instrument tunnel vision (almost always 6-string guitar + 4-string bass), and tunings hard-coded into the data model. TWANGA inverts the trade-off — you own the inputs:

- **Tabs** — record what you play, play back tabs, transpose across instruments, capo per-string. Native format is alphaTex (the open text format from the alphaTab project); MusicXML interop is a future open-standard target. Proprietary formats (Guitar Pro `.gp5`/`.gpx`) are explicit non-goals.
- **Instrument** — `Tuning` is just `Vec<TunedString>`, so drop-D, the banjo's high-pitched 5th-string drone, the ukulele's reentrant G, baritone configurations, and 7-string layouts all fall out of the same data model.
- **Audio device** — any CPAL-supported input. ASIO behind a feature flag on Windows for lower-latency USB instrument cables.

The author is an amateur dev and amateur musician learning Rust through this project. Set your expectations accordingly.

## Getting started

TWANGA ships **two first-class surfaces** — a GUI and a CLI, with the same feature set on each. Pick whichever fits.

### GUI

Open the deployed app at **[andrewiankidd.github.io/TWANGA/app](https://andrewiankidd.github.io/TWANGA/app/)** — no install. Same HTML + WASM bundle the (work-paused) Tauri desktop shell hosts, so anything that lands on web lands unchanged on desktop later.

![TWANGA GUI — main menu](assets/screencaps/gui-menu.png)

Five screens: Tuner, Recorder, Playback, Editor, Tunings. Mic capture uses Web Audio + AudioWorklet; pitch detection runs in the same Rust YIN implementation the CLI uses, compiled to WASM. User recordings persist to IndexedDB (with a Download button on every entry so you can back them up to real files), user-defined tunings live in `localStorage` mirroring the CLI's `tunings.toml`.

→ **Full GUI guide: [docs/GUI.md](docs/GUI.md)**

### CLI

Once you have [Rust](https://rustup.rs) installed, run `twanga` directly out of a clone:

```
$ cargo run -p twanga-cli
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.34s
     Running `target\debug\twanga.exe`

════════════════════════════════════════════════════════════════
████████ ██     ██  █████  ███    ██  ██████   █████
   ██    ██     ██ ██   ██ ████   ██ ██       ██   ██
   ██    ██  █  ██ ███████ ██ ██  ██ ██   ███ ███████
   ██    ██ ███ ██ ██   ██ ██  ██ ██ ██    ██ ██   ██
   ██     ███ ███  ██   ██ ██   ████  ██████  ██   ██
════════════════════════════════════════════════════════════════
  Tries Without Achieving Notable Greatness, Apparently
════════════════════════════════════════════════════════════════

TWANGA CLI

Usage: twanga [COMMAND]

Commands:
  tune     Live tuner — capture audio and show detected pitch vs the nearest target
  play     Play back a `.alphatex` recording
  record   Live tab recorder — capture played notes as horizontal ASCII tab notation
  devices  List available audio input devices
  convert  Convert a tab file from one format to another
  tunings  Manage user-defined tunings stored at the platform config dir
  help     Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

Each subcommand prompts for anything you didn't pass on the command line, so the discoverable path is `cargo run -p twanga-cli -- tune` (or `record`, or `play <path>`). Every flag has a non-interactive form for scripts.

→ **Full CLI guide: [docs/CLI.md](docs/CLI.md)**
→ **Per-flag reference: [crates/twanga-cli/README.md](crates/twanga-cli/README.md)**

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
| **[twanga-web](crates/twanga-web/)** | `wasm-bindgen` bridge that exposes `twanga-core` / `twanga-dsp` / `twanga-tabs` to the browser frontend. |

## Project docs

- [GUI guide](docs/GUI.md) — every screen, storage notes, deploy steps.
- [CLI guide](docs/CLI.md) — quickstart + subcommand tour with sample output.
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
