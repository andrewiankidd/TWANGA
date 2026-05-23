# Calibrate

Set up + verify your digital audio chain so the proximity-score
[Playback](playback.md) modes (`tight` / `casual`) credit your on-time
plucks as Hit rather than scoring them as Late by however many milliseconds
it takes your hardware to get the sound from instrument to TWANGA — plus
your reaction time, which we want to absorb too.

The screen covers three things, all of which directly affect whether
scoring is reliable:

1. **Latency calibration** — three methods. **Pluck-along** is the
   recommended default and works for any input. **Speaker→mic
   round-trip** measures system delay only (no reaction time) and needs
   mic + speakers. **Manual entry** is always available as a fallback.
2. **Mic + pitch-detection check** — confirm the whole pipeline (mic
   permission → input driver → silence gate → YIN pitch detection) is
   wired before you trust it for scoring. GUI has an inline panel; CLI
   users can run `twanga tune` for the same diagnostic.
3. **Audio output check** — play one click and confirm you can hear it.
   Catches "speakers muted" / "wrong output device" failure modes before
   they masquerade as a failed measurement. Standalone button on the GUI.

## Why pluck-along is the default

Most users want scoring to credit them as Hit when they play "on the beat
as they perceive it." That's how every other rhythm trainer works. The
quantity to subtract is **system delay + your reaction time** — because
"on the beat as you perceive it" already includes your own reaction time
between hearing the click and triggering the pluck.

Pluck-along measures exactly that: TWANGA plays a metronome, you pluck a
single note on each beat, the median offset between scheduled-click-time
and detected-onset-time becomes the latency. Works for any input — mic,
line-in, USB instrument cable — because it captures *what you actually
play*.

The speaker→mic round-trip is an alternative for users who specifically
want hardware-only compensation (e.g. recording-studio use cases where
you want to record exactly when you played, not corrected for your
reaction time). It's narrower in applicability (needs mic + speakers
acoustically coupled) and measures less of what you usually care about.

## When to (re)calibrate latency

- **Once per input device.** Each mic / cable / interface combination
  produces a different latency; the stored value is per-device. The
  Playback screen warns when the saved value's device name doesn't match
  the live mic.
- **After changing audio output devices** if practical (system speakers vs
  headphones round-trip differently for the round-trip method; pluck-along
  is less sensitive but reaction time can shift between speakers and
  headphones). Recalibrate when you notice scoring drifting.
- **Skip entirely if you only use Wait mode or Free Play.** Latency
  calibration is consumed only by `tight` / `casual` proximity scoring.
  Wait mode is timing-agnostic; Free Play doesn't score at all.

## Methods

### Pluck-along (recommended)

Plays a metronome at 80 BPM: 4 pre-roll clicks (no recording, just lock
onto the tempo) then 8 measurement clicks. You pluck a single note on
each measurement click — any string, any fret. The onset detector fires
on each pluck; we pair each scheduled click with its nearest detected
onset (within ±half-a-beat) and take the median signed offset as the
latency.

Requires:
- A working input that produces a detectable energy spike per pluck.
  Mic, line-in, USB instrument cable — anything TWANGA can hear.
- A working output you can listen to (speakers, headphones).

Results include "matched N/8 beats" so you can tell how clean the
measurement was. Less than 50% matched and the procedure errors out
with "play louder / clearer" guidance — better than saving a value
derived from one or two plucks.

### Speaker→mic round-trip

Plays 5 short clicks through your default audio output, captures the
mic input throughout, finds the loudest peak per click, records the
median click-to-peak offset as the latency. Measures system delay only
— no reaction time included. Useful for users who specifically want
hardware compensation (recording-studio use, comparing to a calibrated
reference).

Requires:
- Microphone (acoustic mic, USB mic, mic'd amp). Line-in / USB
  instrument cable won't work — there's no acoustic capture path.
- Speakers in the same room as the mic (or near enough that the click
  reaches the mic at meaningful amplitude).

### Manual entry

Type a known value (0–1000 ms). Typical starting points:

| Latency | Setup |
|---|---|
| 10 ms | ASIO / CoreAudio dedicated interface |
| 30 ms | Built-in audio (WASAPI / CoreAudio / PulseAudio) |
| 40 ms | Class-compliant USB mic |
| 150 ms | Bluetooth audio (any direction) |

Use when you've measured your interface externally, or when neither
measurement method is practical (e.g. you're setting up TWANGA before
plugging anything in).

## Wizard — pick a method based on your setup

Both surfaces present the same flow:

1. **Two setup dropdowns/prompts** — pick your input (microphone /
   direct cable / nothing) and your output (speakers / headphones /
   silent). These describe your hardware, not a method.
2. **Compatibility matrix** — TWANGA evaluates which methods are
   physically possible for your combination and recommends the best
   one. Methods that can't work are surfaced with the reason they're
   blocked rather than silently omitted, so you know what to change
   if you want a different method.
3. **Method confirmation** — accept the recommendation or pick another
   compatible method. Manual entry is always available as a fallback.

Compatibility matrix:

| Input | Output | Pluck-along | Round-trip | Manual |
|---|---|---|---|---|
| Mic | Speakers | ✓ (recommended) | ✓ | ✓ |
| Mic | Headphones | ✓ (recommended) | ✗ no acoustic loop | ✓ |
| Mic | Silent | ✗ no audible metronome | ✗ | ✓ (recommended) |
| Direct cable | Speakers | ✓ (recommended) | ✗ no acoustic capture | ✓ |
| Direct cable | Headphones | ✓ (recommended) | ✗ | ✓ |
| Direct cable | Silent | ✗ no audible metronome | ✗ | ✓ (recommended) |
| Nothing | * | ✗ no input | ✗ | ✓ (recommended) |

Pluck-along wins when it's possible because it (a) works across the
widest variety of setups and (b) measures system delay + your reaction
time, which is the correct value to subtract for scoring.

## GUI

Open the Calibrate card from the main menu (or `#calibrate`).

- **Context panel** — read-only display of the currently-selected mic
  and the active silence threshold.
- **Setup dropdowns** — Input + Output, defaulting to Microphone +
  Speakers. Changing either re-runs the matrix; the recommendation
  banner and the method radio's enabled state update live.
- **Recommendation banner** — one-line "Recommended: <method> — <why>"
  above the method radio.
- **Method radio** — three options; incompatible ones are greyed out
  with the blocking reason underneath ("Not available: needs
  microphone + speakers"). The recommended option is auto-checked;
  you can override to any other compatible method. Manual selection
  reveals a numeric input with the typical-values list.
- **Result panel** — appears after a successful calibration. Shows
  the measured ms, the method ("via pluck-along" / "via round-trip" /
  "via manual"), the device, and the timestamp. A stale-calibration
  warning surfaces when the saved value's device name doesn't match
  the live mic.
- **Input + pitch-detection check** — "Start input check" opens the input,
  shows live RMS-in-dB + the most recent detected pitch + sample rate.
  Verifies the end-to-end pipeline before you trust scoring.
- **Audio output check** — "Play test click" plays one click and asks
  yes/no whether you heard it. Catches output-side failure modes
  independent of running calibration.

Persisted to `localStorage` under `twanga-latency-calibration-v1`.

## CLI

`twanga calibrate` runs the interactive wizard. For scripts that need a
specific method without prompts, use one of the non-interactive flags
below.

```
$ twanga calibrate
Calibration
───────────
Two setup questions — TWANGA will recommend the right method.

Input — how does TWANGA hear you?
  [a] Microphone (captures sound acoustically)
  [b] Direct cable from instrument (USB instrument cable, line-in via audio interface)
  [c] Nothing connected yet
> a
Output — how do you hear TWANGA?
  [a] Speakers in the same room as the mic
  [b] Headphones (or speakers far from the mic)
  [c] No audible playback (visual cues only)
> a

Recommended for your setup: pluck-along
  Why: works for your setup and includes reaction time (best for scoring)

Pick a method
  [p] Pluck-along — pluck a note on each metronome click (recommended)
  [r] Speaker→mic round-trip — TWANGA plays + listens (system-only delay)
  [m] Manual entry — type a known value
> p

Pluck-along calibration
───────────────────────
4 pre-roll clicks at 80 BPM to lock onto the tempo, then 8 more.
Pluck a single note on each click — any string, any fret.

Pre-roll 4/4…
Beat 8/8 — pluck!
Measured 7/8 beats (via pluck-along)
Device:    USB PnP Sound Device
Latency:   45 ms (via pluck-along)
Saved to:  /home/user/twanga/latency.toml
```

`twanga calibrate --show` reads the stored value without re-measuring.

`twanga play` surfaces the calibration state on stderr before the
session starts:

```
Latency:    45 ms (calibrated for 'USB PnP Sound Device')
```

| Flag | Description |
|------|-------------|
| `twanga calibrate` | Run the interactive wizard (pick pluck-along / round-trip / manual). |
| `twanga calibrate --pluck-along` | Skip the wizard, run pluck-along directly. |
| `twanga calibrate --round-trip` | Skip the wizard, run the speaker→mic round-trip directly. |
| `twanga calibrate --manual <MS>` | Skip the wizard, save a hand-entered value (0–1000 ms). |
| `twanga calibrate --show` | Print the stored value without measuring. |

### Mic sanity diagnostic on the CLI

There's no dedicated `twanga calibrate --mic-check` mode — use `twanga
tune` instead, which already opens your input and shows live detected
pitch.

## Where things live

- **Native** — `$DATA_ROOT/latency.toml`. Schema:
  ```toml
  device_name = "USB PnP Sound Device"
  latency_ms = 45
  measured_at = "epoch-seconds:1779532834"
  method = "pluck-along"   # or "round-trip" / "manual"
  ```
- **Web** — `localStorage` key `twanga-latency-calibration-v1`. Same
  fields, JSON-encoded.

Both surfaces share the underlying peak-finder + median DSP via
`twanga-dsp::calibration`, and use the same metronome / pluck-along
procedure constants — what counts as "the user plucked at time X" is
identical on CLI and web.

## See also

- [Playback feature](playback.md) — the consumer of the measured
  latency, via `--policy tight|casual` (CLI) or the policy dropdown
  (GUI).
- [Tuner feature](tuner.md) — the dedicated mic + pitch-detection
  screen; the Calibrate screen's mic-check panel is a lightweight
  subset of it.
- [User guide → audio setup](user-guide.md) — broader context on getting
  your input + interface dialed in.
