# Tuner

Live pitch detection vs your chosen tuning. Multi-string display with cents-off
indicators; chromatic mode (no instrument) snaps to the nearest 12-TET note.

The same Rust YIN implementation drives both surfaces — CPAL audio in the CLI,
Web Audio + AudioWorklet in the GUI, with the WASM bindings calling the
identical `twanga-dsp::Tuner`. Cable hum and silence are gated out the same way
on both.

## CLI — `twanga tune`

```bash
twanga tune
```

Prompts for a tuning if `--tuning` is omitted; the prompt includes
"(no instrument — chromatic tuner)" to disable per-string targets and just show
the nearest 12-TET note. When a tuning is selected, prompts again for a capo
position (default 0).

| Flag | Description |
|------|-------------|
| `--tuning <slug>` | Any built-in slug (see `twanga tunings list`) or a user-defined one. Omit to be prompted. |
| `--capo <spec>` | Uniform integer (`--capo 3`) or per-string list (`--capo "0,2,2,2,2,2"`). Omit to be prompted (uniform only via prompt). |

Example output:

```
Tuning: Standard Ukulele (Reentrant GCEA) (4 strings)
Device: Microphone (Default)
Audio:  48000 Hz, 1 channel(s)

A4               | current:    442.10 Hz | target:    440.00 Hz | Tune Down! (+8.2 cents)
E4               | current:    326.40 Hz | target:    329.63 Hz | Tune Up! (-17.0 cents)
C4               | current:    261.10 Hz | target:    261.63 Hz | Tuned! (-3.5 cents)
g4 (reentrant)   | current:    392.10 Hz | target:    392.00 Hz | Tuned! (+0.4 cents)
```

Stop with `q + Enter` or Ctrl-C.

## GUI

Open the Tuner card from the main menu (or hash route `#tuner`).

- **Tuning picker** — built-in + user-defined tunings merged into one
  registry. First option is "chromatic (no instrument)" for a generic tuner.
- **Capo control** — uniform stepper or per-string panel (for drop-D /
  banjo 5th-string drone / partial capos).
- **Per-string display** — every open string of the selected tuning
  rendered as its own row with live frequency, target, and a cents-off
  indicator. The mic-level meter (small RMS bar) sits above the rows so
  you can tell "no signal" apart from "signal but no target match".

State (selected tuning, capo, last-used renderer) persists to `localStorage`
under `twanga-tuner-tuning-v1`. On Tauri the same shape will round-trip to
`$CONFIG/twanga/tunings.toml` once the filesystem-sync command lands
([see the ROADMAP](../ROADMAP.md)).

## Where things live

- **Tunings** — built-in presets are baked into the binary (and the WASM
  bundle). User-defined live at `$CONFIG/twanga/tunings.toml` (CLI) /
  `localStorage` key `twanga-user-tunings-v1` (GUI).
- **Audio device** — CLI uses the system default mic; override coming
  via `--device` in a future release. GUI uses whatever the browser's
  `getUserMedia({ audio: true })` returns; OS-level input selection
  applies.

## See also

- [Tunings feature](tunings.md) — defining custom tunings.
- [Recorder feature](recorder.md) — reuses the same tuning + capo machinery.
- [CLI overview](../CLI.md) · [GUI overview](../GUI.md).
