# twanga-cli

The `twanga` command-line binary. Mostly UX glue — the analysis logic lives in `twanga-dsp` (`Tuner`, onset detection, calibration peak-finder), `twanga-tabs` (`TabRecorder`, alphaTex / MusicXML / MIDI / ABC / ASCII-tab parsers, proximity-score scoring engine), and `twanga-audio` (`InputStream`, `OutputStream`); this crate opens streams, feeds samples through the right pipeline, renders via `twanga-tui`, and merges the built-in tuning registry with `$CONFIG/twanga/tunings.toml` for the menus.

**Flag convention.** Every value-bearing flag takes one of three forms, consistently:

- `--flag value` — use the value directly. Same as the legacy "pass it in" style. Script-friendly.
- `--flag` (no value) — explicitly ask to be prompted (or, for `--bpm` on `play`, explicitly defer to the file's tempo).
- `(omitted)` — same as bare `--flag` when run in a terminal; falls back to a sensible default (or just doesn't apply) when stdin isn't a TTY.

Ctrl-C and `q + Enter` both produce a clean exit. `p + Enter` toggles pause/resume on `record` and `play`. On `record`, `u + Enter` while paused undoes the last committed column (parity with the GUI Recorder's Undo button).

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

Controls during a take: `q + Enter` (or Ctrl-C) stops, `p + Enter` toggles pause/resume, **`u + Enter` while paused** undoes the most recently committed column. Matches the GUI Recorder's "Undo last column" button.

### `play [path]` — play back a recording

Load an `.alphatex` file, scroll a cursor through it at the file's (or overridden) tempo. The audio loop is gated by either time (default) or by detected input (`--wait`).

Omit `path` to open an interactive picker that scans bundled examples (`assets/examples/`), bundled patterns (`assets/patterns/`), and the user's `./recordings/` directory — same library the GUI's Playback screen shows. The picker prefixes each entry with `[example]` / `[pattern · <group> · <pips>]` / `[recording]` so you can see at a glance what you're picking.

| Flag | Description |
|------|-------------|
| `path` (positional, optional) | Path to a `.alphatex` file. Omit to open the picker. |
| `--tuning <slug>` | Re-tune the tab to a different instrument. Notes are transposed by absolute pitch — e.g. play `twinkle-twinkle-uke.alphatex` on a banjo with `--tuning standard-banjo`. Pitches the target can't reach (within fret 0–20) are dropped (or shifted; see `--transpose-mode`). |
| `--transpose-mode <drop\|octave-shift>` | What to do with notes that don't fit on the target tuning during `--tuning` transposition. `drop` (default) silently omits them and reports a "Skipped:" pre-flight summary. `octave-shift` retries each unreachable note at ±12-semitone offsets before giving up — the standard TuxGuitar / MuseScore convention. Particularly relevant for banjo→ukulele where bass drones would otherwise vanish. |
| `--capo <spec>` | Capo applied to the tab's tuning for wait-mode pitch comparison. Precedence: `--capo` wins; otherwise falls back to whatever the file embedded in its `\subtitle` field. |
| `--bpm <N>` | Override the tempo from the file. |
| `--no-metronome` | Silence the click (default is on). |
| `--wait` | Shorthand for `--policy wait`. Cursor pauses at each note until you play it (within ±50 cents on any expected string/fret). Rests still advance with time so the metronome stays musical. |
| `--policy <wait\|tight\|casual\|free>` | Playback behaviour. `wait` pauses on each note; `tight` / `casual` run at tempo and score each column by proximity to expected onsets (±50 ms / ±150 ms hit windows, ColumnOutcome = Hit / Late / Missed / WrongPitch, summary printed at session end); `free` just scrolls with no scoring. Score modes consume the `twanga calibrate` value if one is stored — uncalibrated systems will see on-time plucks score Late under tight. |
| `--from-file <path>` | Replay a mono PCM WAV in place of the live mic — wall-clock-paced so the playback loop treats it like a live stream. Used by the integration test harness against deterministic synth fixtures; handy for scoring an externally-captured recording (phone voice memo, DAW export) against a tab without re-performing it. |
| `--device "<name>"` | Substring-match against the audio input device list (see `twanga devices`). Defaults to the OS default input. |
| `--silence-rms <RMS>` | Override the silence-gate threshold (linear-amplitude window RMS, 0..1). Default 0.005 (≈ -46 dB). Auto-calibrated per session start unless this flag is set. Runtime `[` / `]` keys adjust by ±6 dB. |
| `--loop` | Loop the entire file continuously. |
| `--loop <START:END>` | Loop a specific column range (0-indexed, end exclusive). E.g. `--loop 0:20` plays columns 0–19 on repeat; `--loop 20:30` loops columns 20–29. |
| `--pre-roll <N>` | Audible count-in ticks before playback starts (0–16). Prompted if omitted; default 4. Always audible, even when `--no-metronome` is set. |
| `--resume` | Auto-accept any saved bookmark for this file (jump to the saved column without the interactive prompt). Mirrors the GUI's Resume banner button. |
| `--no-resume` | Auto-decline any saved bookmark. Useful in scripts. |

Bookmarks land in `$CONFIG/twanga/play-resume.toml` on every user-initiated stop (Ctrl-C / `q + Enter`). Naturally-finished plays don't save. Stale bookmarks pointing past a file's edited length are cleared silently on the next load.

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

### `patterns` — bundled rhythm + picking drills

Browse and play the curated patterns at `assets/patterns/` — same library the GUI's Patterns screen renders, grouped by tradition (clawhammer banjo, bluegrass picking, ukulele strums, guitar) with per-pattern difficulty pips. Read-only — patterns ship with the binary; there are no add / remove subcommands.

| Action | Description |
|--------|-------------|
| `twanga patterns` | Interactive picker. Lists every bundled pattern grouped + sorted by difficulty; the chosen pattern plays through `run_playback` with `--loop full` defaulted. |
| `twanga patterns list` | Print the catalog as a tree: group title + description, then each pattern's title, manifest id, tuning slug, and difficulty pips. Scriptable. |
| `twanga patterns play <id>` | Non-interactive equivalent — play a specific pattern by its manifest id (run `patterns list` to see them). Looping is on by default; pass `--no-loop` to play through once. Flags: `--bpm`, `--no-metronome`, `--wait`, `--no-loop`. |
| `twanga patterns path` | Print the absolute path to the patterns manifest (whether it exists at the current cwd or not). |

### `tunings` — manage user-defined tunings

User-defined tunings live at the platform config dir (`$CONFIG/twanga/tunings.toml` via the `directories` crate) and share the same TOML schema as the built-in `presets.toml`. Built-in slugs shadow user-defined ones to prevent silent overrides.

| Action | Description |
|--------|-------------|
| `twanga tunings list` | Print built-in + user-defined tunings with origin tags. |
| `twanga tunings path` | Print the absolute path to the user file (whether it exists yet or not). |
| `twanga tunings add` | Interactive flow: number of strings → per-string open pitch → display name → auto-slug. Saves to the user file, rejects slugs that collide with built-ins. |
| `twanga tunings remove [--slug <slug>] [--force]` | Delete a user-defined tuning from the user file. Pass `--slug` to skip the menu; pass `--force` to skip the confirmation prompt (useful in scripts). Built-in tunings are compiled into the binary and can't be removed. |

### `edit <path> <action>` — non-interactive tab editor

Scriptable counterpart to the GUI Editor screen. Each invocation performs one mutation and writes the file back in place (or to `--out <path>`). Chain in a shell script for batch edits.

| Action | Description |
|--------|-------------|
| `set <column> <string> <fret>` | Set a single cell. `string` is 1-based (string 1 = top of tab); `column` is 0-based; `fret` is any non-negative integer. |
| `clear <column> <string>` | Clear a single cell. |
| `clear-col <column>` | Clear every cell in a column (rest the entire beat). |
| `insert-col [--after <n>]` | Insert a blank column. `--after N` inserts at position N+1; omit to append at the end. |
| `delete-col <column>` | Delete the column at `column`. |
| `title <text>` | Set the `\title` directive. Pass `""` to clear. |
| `bpm <n>` | Set the `\tempo` (20–400 BPM). |

| Flag | Description |
|------|-------------|
| `path` (positional) | Path to the `.alphatex` file to mutate. |
| `--out <path>` | Write the result to a different file instead of overwriting `path`. |

Example:

```bash
# Bump string 1 / col 0 to fret 7 in twinkle
twanga edit assets/examples/twinkle-twinkle-uke.alphatex set 0 1 7

# Insert a rest column after column 7
twanga edit my-take.alphatex insert-col --after 7

# Branch an edit to a new file
twanga edit my-take.alphatex --out my-take-edited.alphatex bpm 90
```

Subtitle (human tuning name + capo annotation) round-trips correctly. Output goes through the same `AlphaTexWriter` the Recorder uses on save, so an edited file is bit-for-bit indistinguishable from a fresh recording with the same notes.

### `devices` — list audio input devices

No arguments. Useful for sanity-checking before `tune` / `record` / `play --wait` / `play --policy tight|casual`.

### `import <path>` — add a tab to the user library

One-shot "add this file to my library" verb. Saves the converted alphaTex to `<data-root>/library/`. Accepts `.alphatex`, `.musicxml` / `.xml`, `.mxl` (zipped MusicXML), `.mid` / `.midi`, `.abc`, and `.tab` (`.txt` content-sniffs to alphaTex vs ASCII tab). `--from <fmt>` overrides format detection; `--title` overrides the source's embedded title. Mirrors the GUI Importer screen.

### `convert <input> --out <output>` — stateless tab conversion

Sibling of `import` for file-in / file-out conversion without library involvement. Same format detection + `--from` flag. Output is always alphaTex today; MusicXML export is on the backlog. Proprietary formats (`.gp5`/`.gpx`) are explicit non-goals.

### `calibrate` — input-latency calibration

Measure your audio chain's input pipeline latency so the proximity-score modes (`play --policy tight|casual`) credit on-time plucks as Hit rather than skewing them Late. Bare invocation runs an interactive wizard (two setup questions → compatibility matrix → method confirmation). Flags skip the wizard for scripts.

| Flag | Description |
|------|-------------|
| `(none)` | Run the interactive wizard. |
| `--pluck-along` | Recommended for any input. TWANGA plays a metronome (4 pre-roll + 8 measurement clicks at 80 BPM); pluck a single note on each measurement click; median signed offset becomes the latency. Captures hardware delay + reaction time, which is the right value for scoring. |
| `--round-trip` | Speaker→mic round-trip. TWANGA plays 5 clicks via the default output, captures via the mic, takes the median click-to-peak offset. Measures system delay only (no reaction time). Requires mic + speakers in the same room. |
| `--manual <MS>` | Skip measurement, save a hand-entered value (0–1000 ms). Use when you know your interface's spec, or when neither measurement method is practical. |
| `--show` | Print the stored value without measuring. |

Saved to `$DATA_ROOT/latency.toml` keyed by input-device name (changing devices invalidates the value). `twanga play` reads it back on startup and prints a status line showing what's applied.

### `docs [feature]` — embedded per-feature documentation

Bare `twanga docs` lists the available pages (`tuner`, `recorder`, `playback`, `patterns`, `editor`, `importer`, `tunings`, `calibrate`, `hardware`, `user-guide`). Pass a slug to print that page's markdown to stdout — pipe through `glow`, `mdcat`, or `bat -l md` for rendering.

## Local development

- **Check**: `cargo check -p twanga-cli`
- **Test**: `cargo test -p twanga-cli`
- **Run**: `cargo run -p twanga-cli -- <subcommand>` (each subcommand also responds to `--help`)
- **Depends on**: `twanga-core`, `twanga-audio`, `twanga-dsp`, `twanga-synth`, `twanga-tabs`, `twanga-tui`, `clap`, `anyhow`, `toml`, `directories`

See [the workspace README](../../README.md) for project context.
