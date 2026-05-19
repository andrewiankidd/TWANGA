# twanga-cli

The `twanga` command-line binary. Mostly UX glue — the analysis logic lives in `twanga-dsp` (`Tuner`), `twanga-tabs` (`TabRecorder`, alphaTex serialiser/parser), and `twanga-audio` (`InputStream`, `OutputStream`); this crate opens streams, feeds samples through the right pipeline, renders via `twanga-tui`, and merges the built-in tuning registry with `$CONFIG/twanga/tunings.toml` for the menus.

**Flag convention.** Every value-bearing flag takes one of three forms, consistently:

- `--flag value` — use the value directly. Same as the legacy "pass it in" style. Script-friendly.
- `--flag` (no value) — explicitly ask to be prompted (or, for `--bpm` on `play`, explicitly defer to the file's tempo).
- `(omitted)` — same as bare `--flag` when run in a terminal; falls back to a sensible default (or just doesn't apply) when stdin isn't a TTY.

Ctrl-C and `q + Enter` both produce a clean exit. `p + Enter` toggles pause/resume on `record` and `play`.

## Subcommands

### `tune` — live tuner

Capture audio, show detected pitch vs nearest target. Prompts for tuning if `--tuning` is omitted; the prompt includes "(no instrument — chromatic tuner)" to disable per-string targets and just show the nearest 12-TET note. When a tuning is selected, prompts again for a capo position (default 0).

| Flag | Description |
|------|-------------|
| `--tuning <slug>` | Any built-in slug (see `twanga tunings list`) or a user-defined one. Omit to be prompted. |
| `--capo <spec>` | Uniform integer (`--capo 3`) or per-string list (`--capo "0,2,2,2,2,2"`). Omit to be prompted (uniform only via prompt). |

### `record` — live tab recorder

Capture played notes as alphaTex. If you provide a `--title`, the filename slugifies to `<title>-<unix-secs>.alphatex`; otherwise it falls back to `recordings/recording-<unix-secs>.alphatex`. Each detected pitch is mapped to the smallest non-negative fret position on the (capo'd) tuning, so a played D5 on uke registers as fret 5 on the A string, not fret 14 on the C string. The display shows a refreshing multi-string view of the in-progress block; a status line below tracks elapsed time, total columns, and (when non-zero) a count of pitches that didn't fit on the active fretboard.

When a capo is set, logged frets are **capo-relative** (the musical convention) and the file's `\subtitle` field carries the capo via a `; capo=<spec>` suffix so the recording round-trips through playback without the user having to remember and re-pass the same value. Any `--title` you supply gets written to the alphaTex `\title` directive so playback (CLI or GUI) shows it in the header.

| Flag | Description |
|------|-------------|
| `--tuning <slug>` | As above. |
| `--capo <spec>` | As above. |
| `--bpm <N>` | Tempo (20–400). Prompted if omitted; default 120. |
| `--resolution <1/N>` | Note value per column: `1/4`, `1/8`, `1/16`, `1/32`. Prompted if omitted; default `1/8`. |
| `--block-width <N>` | Columns per scrolling block (4–200). Prompted if omitted; default 32. |
| `--no-metronome` | Disable the metronome click on each beat (default: on). |
| `--pre-roll <N>` | Audible count-in ticks before recording starts (0–16). Prompted if omitted; default 4. Always audible, even when `--no-metronome` is set. Aborts cleanly on Ctrl-C / `q + Enter`. |
| `--title <text>` | Human-readable title — written to `\title` in the alphaTex header AND used to derive the filename (`<slug>-<unix-secs>.alphatex`). Prompted if omitted; accept blank to fall back to the original `recording-<unix-secs>.alphatex` shape. |

### `play <path>` — play back a recording

Load an `.alphatex` file, scroll a cursor through it at the file's (or overridden) tempo. The audio loop is gated by either time (default) or by detected input (`--wait`).

| Flag | Description |
|------|-------------|
| `path` (positional) | Path to a `.alphatex` file. |
| `--tuning <slug>` | Re-tune the tab to a different instrument. Notes are transposed by absolute pitch — e.g. play `twinkle-twinkle-uke.alphatex` on a banjo with `--tuning standard-banjo`. Pitches the target can't reach (within fret 0–20) are silently dropped. |
| `--capo <spec>` | Capo applied to the tab's tuning for wait-mode pitch comparison. Precedence: `--capo` wins; otherwise falls back to whatever the file embedded in its `\subtitle` field. |
| `--bpm <N>` | Override the tempo from the file. |
| `--no-metronome` | Silence the click (default is on). |
| `--wait` | Practice mode — cursor pauses at each note until you play it (within ±50 cents on any expected string/fret). Rests still advance with time so the metronome stays musical. |
| `--loop` | Loop the entire file continuously. |
| `--loop <START:END>` | Loop a specific column range (0-indexed, end exclusive). E.g. `--loop 0:20` plays columns 0–19 on repeat; `--loop 20:30` loops columns 20–29. |
| `--pre-roll <N>` | Audible count-in ticks before playback starts (0–16). Prompted if omitted; default 4. Always audible, even when `--no-metronome` is set. |

Example with the bundled demo:

```bash
# Play the uke arrangement on uke
cargo run -p twanga-cli -- play assets/examples/twinkle-twinkle-uke.alphatex --bpm 60 --wait

# Transpose it to banjo, loop the first phrase
cargo run -p twanga-cli -- play assets/examples/twinkle-twinkle-uke.alphatex \
    --tuning standard-banjo --bpm 70 --loop 0:16

# Same arrangement, but with a capo on fret 3
cargo run -p twanga-cli -- play assets/examples/twinkle-twinkle-uke.alphatex --capo 3
```

### `tunings` — manage user-defined tunings

User-defined tunings live at the platform config dir (`$CONFIG/twanga/tunings.toml` via the `directories` crate) and share the same TOML schema as the built-in `presets.toml`. Built-in slugs shadow user-defined ones to prevent silent overrides.

| Action | Description |
|--------|-------------|
| `twanga tunings list` | Print built-in + user-defined tunings with origin tags. |
| `twanga tunings path` | Print the absolute path to the user file (whether it exists yet or not). |
| `twanga tunings add` | Interactive flow: number of strings → per-string open pitch → display name → auto-slug. Saves to the user file, rejects slugs that collide with built-ins. |

### `devices` — list audio input devices

No arguments. Useful for sanity-checking before `tune` / `record` / `play --wait`.

### `convert <input> <output>` — tab format conversion (stub)

Placeholder. Will eventually round-trip alphaTex ↔ MusicXML once the MusicXML parser lands in `twanga-tabs`. Proprietary formats (`.gp5`/`.gpx`) are explicit non-goals.

## Local development

- **Check**: `cargo check -p twanga-cli`
- **Test**: `cargo test -p twanga-cli`
- **Run**: `cargo run -p twanga-cli -- <subcommand>` (each subcommand also responds to `--help`)
- **Depends on**: `twanga-core`, `twanga-audio`, `twanga-dsp`, `twanga-synth`, `twanga-tabs`, `twanga-tui`, `clap`, `anyhow`, `toml`, `directories`

See [the workspace README](../../README.md) for project context.
