# TWANGA — CLI

`twanga` is the command-line surface. Same feature set as the GUI
([docs/GUI.md](GUI.md)) — every flag and screen has a CLI equivalent and
vice versa. Reach for the CLI when you want scripting, predictable
plumbing into pipes, or simply prefer terminals.

This page is the **getting-started + tour**. For the full per-subcommand
flag reference (every `--bpm`, `--capo`, `--resolution`, `--loop`, etc.)
see [crates/twanga-cli/README.md](../crates/twanga-cli/README.md). Each
subcommand also responds to `--help`.

## Install

Once you have [Rust](https://rustup.rs) installed, no separate install
step is needed — `cargo run -p twanga-cli -- <subcommand>` from a clone
of this repo just works. To put `twanga` on your `$PATH`:

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

## Quickstart

```bash
# Type-check + run the workspace tests
cargo check --workspace
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
cargo run -p twanga-cli -- tunings remove --slug my-tuning
```

## Subcommand tour

Each subcommand opens with the TWANGA banner + a randomly-picked splash;
the body is whatever flow you asked for. Outputs below are from running
the three main flows against the bundled Twinkle Twinkle uke example.

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

## Where things live

- **Recordings** — `recordings/<title>-<unix-secs>.alphatex` next to wherever
  you ran `twanga record`. The web Recorder writes to IndexedDB instead
  (see [docs/GUI.md](GUI.md)).
- **User tunings** — `$CONFIG/twanga/tunings.toml` (Windows:
  `%APPDATA%\twanga\`; macOS: `~/Library/Application Support/twanga/`;
  Linux: `~/.config/twanga/`). The exact path is whatever
  `twanga tunings path` prints. Same TOML schema as the built-in
  `presets.toml` baked into the binary.
- **Bundled tab examples** — [`assets/examples/`](../assets/examples/) at the
  repo root. The CLI reads them directly when you pass their path to
  `play`. (The GUI ships them in the deployed bundle so they're available
  in-browser without a clone.)

## Bundled examples

[`assets/examples/`](../assets/examples/) ships a small set of
public-domain demo `.alphatex` files. Arrangements are original to TWANGA
(MIT/Apache-2.0).

- [`twinkle-twinkle-uke.alphatex`](../assets/examples/twinkle-twinkle-uke.alphatex)
  — Twinkle Twinkle Little Star, standard uke (GCEA), 12 bars.
- [`cripple-creek-banjo.alphatex`](../assets/examples/cripple-creek-banjo.alphatex)
  — Cripple Creek (Appalachian trad), standard 5-string banjo (open G),
  clawhammer-style with drone, 16 bars.

## Beyond this page

- **Full flag reference** — [crates/twanga-cli/README.md](../crates/twanga-cli/README.md)
- **GUI counterpart** — [docs/GUI.md](GUI.md)
- **What works today** — [docs/PROJECT_STATUS.md](PROJECT_STATUS.md)
- **What's next** — [docs/ROADMAP.md](ROADMAP.md)
