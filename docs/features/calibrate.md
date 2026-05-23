# Calibrate

Set up + verify your digital audio chain so the proximity-score
[Playback](playback.md) modes (`tight` / `casual`) credit your on-time
plucks as Hit rather than scoring them as Late by however many milliseconds
it takes your hardware to get the sound from instrument to TWANGA.

The screen covers three things, all of which directly affect whether
scoring is reliable:

1. **Latency calibration** — measure or set your input pipeline latency.
   A 2-question wizard picks the right method based on your setup
   (acoustic round-trip when speakers and mic can loop together, manual
   entry when they can't).
2. **Mic + pitch-detection check** — confirm the whole pipeline (mic
   permission → input driver → silence gate → YIN pitch detection) is
   wired before you trust it for scoring. GUI has an inline panel; CLI
   users can run `twanga tune` for the same diagnostic.
3. **Audio output check** — play one click and confirm you can hear it.
   Catches "speakers muted" / "wrong output device" failure modes before
   they masquerade as a failed round-trip (where "no clicks detected"
   gives no clue which side of the loop is broken). Baked into the CLI
   wizard's round-trip path; available as a standalone button on the GUI.

## When to (re)calibrate latency

- **Once per input device.** Each mic / cable / interface combination
  produces a different round-trip; the stored value is per-device. The
  Playback screen warns when the saved value's device name doesn't match
  the live mic.
- **After changing audio output devices** if practical (system speakers vs
  headphones round-trip differently). TWANGA doesn't currently let you
  pick an output device, so this only matters if your OS default changes
  between sessions — recalibrate when you notice scoring drifting.
- **Skip entirely if you only use Wait mode or Free Play.** Latency
  calibration is consumed only by `tight` / `casual` proximity scoring.
  Wait mode is timing-agnostic; Free Play doesn't score at all.

## Calibration wizard

Two setup questions decide which measurement method runs:

```
Q1. Where does TWANGA listen for your playing?
    a) Microphone — captures sound acoustically (USB mic, condenser,
       dynamic mic in front of an amp)
    b) Direct cable from instrument — USB instrument cable, or
       instrument cable into an audio interface (no acoustic capture)
    c) Nothing connected yet (set manually for now)

Q2. How do you hear TWANGA's audio (metronome, count-in)?
    Only asked when Q1 = Microphone. Direct-cable and no-input setups
    skip Q2 since round-trip can't work for them regardless.

    a) Speakers in the same room as the mic
    b) Headphones (or speakers far from the mic)
    c) No audible playback (visual cues only)
```

Method dispatch:

| Q1 | Q2 | Method | Why |
|---|---|---|---|
| Mic | Speakers | **Round-trip** (play clicks, capture via mic, median offset) | TWANGA can physically capture its own click through the air |
| Mic | Headphones / silent | **Manual entry** | No speakers → mic path available |
| Direct cable | — | **Manual entry** | No acoustic capture, so the round-trip loop is broken regardless of how you hear playback |
| Nothing | — | **Manual entry** | Nothing to measure |

In the manual flow, both surfaces show typical values to pick from:

| Latency | Setup |
|---|---|
| 10 ms | ASIO / CoreAudio dedicated interface |
| 30 ms | Built-in audio (WASAPI / CoreAudio / PulseAudio) |
| 40 ms | Class-compliant USB mic |
| 150 ms | Bluetooth audio (any direction) |

The persisted record carries a `method` field (`round-trip` or `manual`)
so the result display says "via round-trip" / "via manual" — you can see
at a glance how trustworthy the number is.

## GUI

Open the Calibrate card from the main menu (or `#calibrate`).

- **Context panel** — read-only display of the currently-selected mic
  and the active silence threshold. Both come from the Playback screen's
  shared mic controls; changing them here would be misleading since
  calibration is per-device.
- **Wizard** — two radio groups for Q1 + Q2. The form re-renders as you
  pick: round-trip combinations show a single "Calibrate" button; manual
  combinations reveal a numeric input + typical-values list and the
  button switches to "Save manually."
- **Result panel** — appears after a successful calibration (or on
  screen activation if there's already a stored value). Shows the
  measured ms, the method, the device, and the timestamp. A stale-
  calibration warning surfaces when the saved value's device name
  doesn't match the live mic.
- **Mic + pitch-detection check** — "Start mic check" opens the mic,
  shows live RMS-in-dB + the most recent detected pitch + sample rate.
  If the level bar moves and a note name appears, your end-to-end
  pipeline is working. "Stop" releases the mic; navigating away from
  the screen auto-stops it (no orphan mic-in-use indicator).
- **Audio output check** — "Play test click" plays one click out the
  default output and reveals a yes/no row. "Yes" confirms output is
  reaching the speakers (round-trip should work); "No" points you at
  picking Headphones in the wizard and using manual entry.

The Playback screen surfaces a one-line **Latency: X ms (calibrated for
'<device>')** indicator under the mic component when `tight` or `casual`
is selected, so you can see at a glance whether scoring will be
calibrated before pressing Play.

Saved to `localStorage` under `twanga-latency-calibration-v1`.

## CLI

`twanga calibrate` runs the interactive wizard. For scripts that need a
specific method without prompts, use one of the non-interactive flags
below.

```
$ twanga calibrate
Calibration wizard
──────────────────
Two questions to pick the right measurement for your setup.

Q1. Where does TWANGA listen for your playing?
  [a] Microphone (acoustic mic, USB mic, mic'd amp)
  [b] Line-in (electric instrument via cable + audio interface)
  [c] Nothing connected yet (set manually for now)
> a
Q2. How do you hear TWANGA's audio (metronome, count-in)?
  [a] Speakers in the same room as the mic
  [b] Headphones (or speakers far from the mic)
  [c] No audible playback (visual cues only)
> a

Playing one test click — confirm you can hear it.
Did you hear the click? [y/n]: y

Setup detected: acoustic round-trip available. Running measurement.
Calibrating output→input round-trip latency over 5 clicks.
Have your mic positioned where you'd play. Stay quiet during the measurement.

Device:    USB PnP Sound Device
Latency:   42 ms (via round-trip)
Saved to:  /home/user/twanga/latency.toml
```

If you answer **n** to the output prompt the wizard reroutes to manual
entry rather than failing inside the measurement. The headphones / silent
branches also route to manual, with a tip pointing at `twanga tune` if
you want a separate mic-check diagnostic.

`twanga calibrate --show` reads the stored value without re-measuring:

```
$ twanga calibrate --show
Device:       USB PnP Sound Device
Latency:      42 ms (via round-trip)
Measured at:  epoch-seconds:1779532834
Path:         /home/user/twanga/latency.toml
```

`twanga play` surfaces the calibration state on stderr before the
session starts:

```
Latency:    42 ms (calibrated for 'USB PnP Sound Device')
```

Or, when uncalibrated:

```
Latency:    uncalibrated. Run `twanga calibrate` for tighter scoring.
```

Or, when the active mic doesn't match the saved calibration:

```
Latency:    uncalibrated for 'Built-in Microphone' (saved value is for 'USB PnP Sound Device'). Scoring may be off; run `twanga calibrate` to refresh.
```

| Flag | Description |
|------|-------------|
| `twanga calibrate` | Run the interactive wizard (Q1/Q2 → round-trip or manual). |
| `twanga calibrate --round-trip` | Skip the wizard + output check, run the round-trip measurement directly. For scripts. |
| `twanga calibrate --manual <MS>` | Skip the wizard, save a hand-entered value (0–1000 ms). For scripts and headphones / line-in users. |
| `twanga calibrate --show` | Print the stored value without measuring. |

### Mic sanity diagnostic on the CLI

There's no dedicated `twanga calibrate --mic-check` mode — use `twanga
tune` instead, which already opens your mic and shows live detected
pitch. The wizard suggests this when it routes you to the manual branch.

## Where things live

- **Native** — `$DATA_ROOT/latency.toml`. Schema:
  ```toml
  device_name = "USB PnP Sound Device"
  latency_ms = 42
  measured_at = "epoch-seconds:1779532834"
  method = "round-trip"   # or "manual"
  ```
- **Web** — `localStorage` key `twanga-latency-calibration-v1`. Same
  fields, JSON-encoded.

Both surfaces share the underlying peak-finder + median DSP via
`twanga-dsp::calibration` — what counts as "a click landed at time X" is
identical on CLI and web.

## See also

- [Playback feature](playback.md) — the consumer of the measured
  latency, via `--policy tight|casual` (CLI) or the policy dropdown
  (GUI).
- [Tuner feature](tuner.md) — the dedicated mic + pitch-detection
  screen; the Calibrate screen's mic-check panel is a lightweight
  subset of it.
- [User guide → audio setup](user-guide.md) — broader context on getting
  your mic + interface dialed in.
