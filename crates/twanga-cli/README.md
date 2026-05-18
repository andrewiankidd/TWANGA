# twanga-cli

The `twanga` command-line binary. Mostly UX glue — the analysis logic lives in `twanga-dsp` (`Tuner`), `twanga-tabs` (`TabRecorder`, alphaTex serialiser/parser), and `twanga-audio` (`InputStream`, `OutputStream`); this crate opens streams, feeds samples through the right pipeline, and renders via `twanga-tui`.

Any argument left unset triggers an interactive prompt with the default pre-filled (press enter to accept). Non-TTY contexts (pipes, scripts, CI) need every argument passed via flag — the prompts error cleanly when stdin/stderr isn't a terminal. Ctrl-C and `q + Enter` both produce a clean exit.

## Subcommands

### `tune` — live tuner

Capture audio, show detected pitch vs nearest target. Prompts for tuning if `--tuning` is omitted; the prompt includes "(no instrument — chromatic tuner)" to disable per-string targets and just show the nearest 12-TET note.

| Flag | Description |
|------|-------------|
| `--tuning <preset>` | `standard-guitar` / `standard-banjo` / `standard-ukulele`. Omit to be prompted. |

### `record` — live tab recorder

Capture played notes as alphaTex, saved to `recordings/recording-<unix-secs>.alphatex`. Each detected pitch is mapped to the smallest non-negative fret position on the given tuning (so a played D5 on uke registers as fret 5 on the A string, not fret 14 on the C string). The display shows a refreshing multi-string view of the in-progress block.

| Flag | Description |
|------|-------------|
| `--tuning <preset>` | As above. |
| `--bpm <N>` | Tempo (20–400). Prompted if omitted; default 120. |
| `--resolution <1/N>` | Note value per column: `1/4`, `1/8`, `1/16`, `1/32`. Prompted if omitted; default `1/8`. |
| `--block-width <N>` | Columns per scrolling block (4–200). Prompted if omitted; default 32. |

### `play <path>` — play back a recording

Load an `.alphatex` file, scroll a cursor through it at the file's (or overridden) tempo. The audio loop is gated by either time (default) or by detected input (`--wait`).

| Flag | Description |
|------|-------------|
| `path` (positional) | Path to a `.alphatex` file. |
| `--tuning <preset>` | Re-tune the tab to a different instrument. Notes are transposed by absolute pitch — e.g. play `twinkle-twinkle-uke.alphatex` on a banjo with `--tuning standard-banjo`. Pitches the target instrument can't reach (within fret 0–20) are silently dropped. |
| `--bpm <N>` | Override the tempo from the file. |
| `--no-metronome` | Silence the click (default is on). |
| `--wait` | Practice mode — cursor pauses at each note until you play it (within ±50 cents on any expected string/fret). Rests still advance with time so the metronome stays musical. |
| `--loop` | Loop the entire file continuously. |
| `--loop <START:END>` | Loop a specific column range (0-indexed, end exclusive). E.g. `--loop 0:20` plays columns 0–19 on repeat; `--loop 20:30` loops columns 20–29. |

Example with the bundled demo:

```bash
# Play the uke arrangement on uke
cargo run -p twanga-cli -- play assets/examples/twinkle-twinkle-uke.alphatex --bpm 60 --wait

# Transpose it to banjo, loop the first phrase
cargo run -p twanga-cli -- play assets/examples/twinkle-twinkle-uke.alphatex \
    --tuning standard-banjo --bpm 70 --loop 0:16
```

### `devices` — list audio input devices

No arguments. Useful for sanity-checking before `tune` / `record` / `play --wait`.

### `convert <input> <output>` — tab format conversion (stub)

Placeholder. Will eventually round-trip GP5/MusicXML/alphaTex once those parsers land in `twanga-tabs`.

## Local development

- **Check**: `cargo check -p twanga-cli`
- **Test**: `cargo test -p twanga-cli`
- **Run**: `cargo run -p twanga-cli -- <subcommand>` (each subcommand also responds to `--help`)
- **Depends on**: `twanga-core`, `twanga-audio`, `twanga-dsp`, `twanga-synth`, `twanga-tabs`, `twanga-tui`, `clap`, `anyhow`

See [the workspace README](../../README.md) for project context.
