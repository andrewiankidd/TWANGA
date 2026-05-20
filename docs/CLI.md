# TWANGA — CLI overview

`twanga` is the command-line surface. Same feature set as the GUI
([docs/GUI.md](GUI.md)) — every flag has a GUI counterpart and vice
versa. Reach for the CLI when you want scripting, predictable plumbing
into pipes, or simply prefer terminals.

## Install

Once you have [Rust](https://rustup.rs) installed, run `twanga`
directly out of a clone:

```bash
cargo run -p twanga-cli -- <subcommand>
```

To put `twanga` on your `$PATH`:

```bash
cargo install --path crates/twanga-cli
```

On Windows, for lower-latency capture from USB instrument cables, build
with the `asio` feature:

```bash
cargo run -p twanga-cli --features twanga-audio/asio -- tune
```

(ASIO requires Steinberg's SDK to be installed — see the
[twanga-audio README](../crates/twanga-audio/) for the current state.)

## Feature pages

Each feature page documents both its CLI subcommand and its GUI
counterpart in one place.

| Feature | Subcommand | Page |
|---------|------------|------|
| Tuner | `twanga tune` | [features/tuner.md](features/tuner.md) |
| Recorder | `twanga record` | [features/recorder.md](features/recorder.md) |
| Playback | `twanga play [path]` | [features/playback.md](features/playback.md) |
| Patterns | `twanga patterns <list\|play\|path>` | [features/patterns.md](features/patterns.md) |
| Tab editor | (GUI only — see scope) | [features/editor.md](features/editor.md) |
| Tunings | `twanga tunings <list\|path\|add\|remove>` | [features/tunings.md](features/tunings.md) |
| Docs | `twanga docs [feature]` | The per-feature pages above, printed to stdout for `glow` / `mdcat` / `bat -l md`. |

Bare `twanga` with no subcommand prints the splash banner + the standard
clap long-help so you can discover what's available. Bare `twanga play`
opens an interactive picker over bundled examples + patterns + your
`./recordings/`, mirroring the GUI's Playback library.

## Other subcommands

- **`twanga devices`** — list audio input devices CPAL can see.
  Useful for sanity-checking before `tune` / `record` / `play --wait`.
  No arguments.
- **`twanga convert <input> <output>`** — *stub*. Will eventually
  round-trip alphaTex ↔ MusicXML once the MusicXML parser lands.
  Proprietary formats (`.gp5`/`.gpx`) are explicit non-goals.

## Bundled examples

[`assets/examples/`](../assets/examples/) ships a small set of
public-domain demo `.alphatex` files. Arrangements are original to
TWANGA (MIT/Apache-2.0).

- [`twinkle-twinkle-uke.alphatex`](../assets/examples/twinkle-twinkle-uke.alphatex)
  — Twinkle Twinkle Little Star, standard uke (GCEA), 12 bars.
- [`cripple-creek-banjo.alphatex`](../assets/examples/cripple-creek-banjo.alphatex)
  — Cripple Creek (Appalachian trad), standard 5-string banjo (open
  G), clawhammer-style with drone, 16 bars.

The [Patterns feature](features/patterns.md) also ships several short
rhythm drills under [`assets/patterns/`](../assets/patterns/).

## Conventions

- **Flag forms.** Every value-bearing flag takes one of three forms,
  consistently: `--flag value` (use the value), `--flag` with no value
  (explicitly prompt), or omitted (prompt if TTY, otherwise default).
- **Stop / pause.** `q + Enter` or Ctrl-C exits cleanly. `p + Enter`
  toggles pause/resume on `record` and `play`.
- **Where data lives.** Recordings → `recordings/<file>.alphatex` next
  to wherever you ran the command. User tunings →
  `$CONFIG/twanga/tunings.toml` (see `twanga tunings path`).

## Beyond this page

- **Full per-flag reference** —
  [crates/twanga-cli/README.md](../crates/twanga-cli/README.md).
- **GUI counterpart** — [docs/GUI.md](GUI.md).
- **What works today** — [docs/PROJECT_STATUS.md](PROJECT_STATUS.md).
- **What's next** — [docs/ROADMAP.md](ROADMAP.md).
