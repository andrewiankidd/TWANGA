# Recorder

Capture played notes as an alphaTex tab. Live multi-string view scrolls as you
play; each detected pitch maps to the smallest non-negative fret on the active
tuning. Capo, BPM, resolution, metronome, pre-roll count-in, and pause/resume
all work identically on CLI and GUI.

## CLI — `twanga record`

```bash
twanga record
```

Each detected pitch is mapped to the smallest non-negative fret position on the
(capo'd) tuning, so a played D5 on uke registers as fret 5 on the A string, not
fret 14 on the C string. A status line below the scrolling block tracks elapsed
time, total columns, and (when non-zero) a count of pitches that didn't fit on
the active fretboard.

When a capo is set, logged frets are **capo-relative** (the musical
convention) and the file's `\subtitle` field carries the capo via a
`; capo=<spec>` suffix so the recording round-trips through playback without
the user having to remember and re-pass the same value. Any `--title` you
supply gets written to the alphaTex `\title` directive so playback (CLI or
GUI) shows it in the header.

| Flag | Description |
|------|-------------|
| `--tuning <slug>` | Built-in slug (see `twanga tunings list`) or user-defined. Omit to be prompted. |
| `--capo <spec>` | Uniform integer (`--capo 3`) or per-string list (`--capo "0,2,2,2,2,2"`). |
| `--bpm <N>` | Tempo (20–400). Prompted if omitted; default 120. |
| `--resolution <1/N>` | Note value per column: `1/4`, `1/8`, `1/16`, `1/32`. Default `1/8`. |
| `--block-width <N>` | Columns per scrolling block (4–200). Default 32. |
| `--no-metronome` | Disable the metronome click on each beat (default: on). |
| `--pre-roll <N>` | Audible count-in ticks before recording starts (0–16). Default 4. Always audible, even when `--no-metronome` is set. Aborts cleanly on Ctrl-C / `q + Enter`. |
| `--title <text>` | Human-readable title — written to `\title` AND used to derive the filename (`<slug>-<unix-secs>.alphatex`). Blank input falls back to `recording-<unix-secs>.alphatex`. |
| `--device "<name>"` | Substring-match against the audio input device list (see `twanga devices`). Omit to use the OS default. |
| `--silence-rms <RMS>` | Override the silence-gate threshold (linear-amplitude window-RMS, 0..1; default 0.005 ≈ -46 dB). Lower for quiet plucks at the cost of more cable-hum / room-noise false positives. |

Controls during a take (all `+ Enter`):

- `q` — stop (or Ctrl-C)
- `p` — pause / resume
- `u` (while paused) — undo last committed column. Pops it from the
  score and rewinds the sample counter so resume doesn't fire a
  phantom catch-up click. Match for the GUI's "Undo last column"
  button.
- `[` / `]` — drop / raise the silence threshold by ~6 dB. Prints the
  new value on its own line.

Example output:

```
Tuning:     Standard Ukulele (Reentrant GCEA) (4 strings)
Tempo:      120 BPM, 1/8 notes (250 ms/col)
Block:      32 cols (8000 ms wide)
Saving to:  recordings/recording-1779133041.alphatex

A4             | --------0000--------------------
E4             | ----------------11110000--------
C4             | 0000--------------------222200--
g4 (reentrant) | ----0000----00------------------
```

## GUI

Open the Recorder card from the main menu (or `#recorder`).

- **Tuning + capo** — same controller widget as the Tuner.
- **BPM / resolution / block width** — number steppers with the same
  defaults the CLI uses (`120` / `1/8` / `32`).
- **Metronome toggle / pre-roll input** — live-toggleable; pre-roll is the
  audible count-in (always plays, regardless of metronome state).
- **Pause / Resume / Stop** — pause is mid-take safe (the wall-clock
  tick driver subtracts paused time so resumed columns line up).
- **Undo last column** — visible only while paused. Pops the most
  recently committed column and rewinds the wall clock by one column so
  resume doesn't fire a phantom catch-up click. Click repeatedly to undo
  multiple. Disabled when nothing has been committed.
- **Renderer picker** — Tab (column-grid) or Highway (notes drop down
  lanes toward a "now" line) for the live in-progress view.
- **Input device picker** — dropdown above the meter, populated via
  `navigator.mediaDevices.enumerateDevices()`. Persists in
  `localStorage` under `twanga-recorder-device-v1`. Hot-plug supported.
  No mid-take swap; the choice takes effect on the next Start.
- **Mic-level meter + silence-threshold slider** — small RMS bar
  surfaces "no signal" diagnostics (helps distinguish missing pitch
  detection from a dead input; particularly useful on macOS where the
  AudioContext can start suspended). The slider thumb overlays the
  bar; drag to set the silence gate on the same dB axis. Persisted
  under `twanga-recorder-silence-rms-v1`.
- **Save** — writes the take as `.alphatex` into the in-browser library
  (IndexedDB). Title field on the save dialog populates the alphaTex
  `\title` field and the derived row title in the library.

State persists to `localStorage` (tuning + capo + BPM + resolution +
block width + metronome flag + pre-roll value + last-used renderer).

## Where things live

- **CLI recordings** — `recordings/<title>-<unix-secs>.alphatex` next
  to wherever you ran `twanga record`. Plain text alphaTex.
- **GUI recordings** — IndexedDB `twanga-tabs-v1` / `tabs` store. The
  Playback library shows a "Backed up <when>" tag per entry; Download
  exports a real file. The browser-storage warning banner reminds the
  user that IDB can be evicted.

## See also

- [Tab editor feature](editor.md) — post-capture cell-level edits to a
  recording.
- [Playback feature](playback.md) — play your recordings back, with
  wait mode for practice.
- [CLI overview](../CLI.md) · [GUI overview](../GUI.md).
