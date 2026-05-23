# Calibrate

Measure your audio chain's output→input round-trip latency so the
proximity-score [Playback](playback.md) modes (`tight` / `casual`) credit
your on-time plucks as Hit rather than scoring them as Late by however many
milliseconds it takes your speakers + air + mic + driver to round-trip the
audio.

The procedure plays five short clicks through your default audio output,
captures the mic input throughout, finds the loudest peak per click, and
records the median click-to-peak offset as your round-trip latency. The
measurement is keyed by input-device name — switching mics invalidates the
stored value and prompts a recalibration rather than scoring against a
stale number from a different device.

## When to calibrate

- **Once per input device.** Each mic / cable / interface combination
  produces a different round-trip; the stored value is per-device.
- **After changing audio output devices** if practical (system speakers vs
  headphones round-trip differently). TWANGA doesn't currently let you
  pick an output device, so this only matters if your OS default changes
  between sessions — recalibrate when you notice scoring drifting.
- **Skip entirely if you only use Wait mode or Free Play.** Calibration
  is consumed only by `tight` / `casual` proximity scoring. Wait mode
  is timing-agnostic; Free Play doesn't score at all.

## GUI

Open the Calibrate card from the main menu (or `#calibrate`).

- **Context panel** — read-only display of the currently-selected mic
  and the active silence threshold. Both come from the Playback screen's
  shared mic controls; changing them here would be misleading since
  calibration is per-device.
- **Calibrate / Recalibrate button** — runs the 5-click procedure.
  Status updates display click-by-click; on success, the result panel
  appears with the measured value, the device it was measured against,
  and the timestamp. Stay quiet during the measurement — incidental
  noise can throw off the peak detection.
- **Stale-calibration warning** — if the saved value's device name
  doesn't match the live mic (you switched USB cables, plugged in a
  different interface), the status line warns and prompts to
  recalibrate.

The Playback screen surfaces a one-line **Latency: X ms (calibrated for
'<device>')** indicator under the mic component when `tight` or `casual`
is selected, so you can see at a glance whether scoring will be
calibrated before pressing Play.

Saved to `localStorage` under `twanga-latency-calibration-v1`.

## CLI

`twanga calibrate` — run the measurement.
`twanga calibrate --show` — read back the stored value without re-measuring.

```
$ twanga calibrate

Calibrating output→input round-trip latency over 5 clicks.
Have your mic positioned where you'd play. Stay quiet during the measurement.

Device:    USB PnP Sound Device
Latency:   42 ms
Saved to:  /home/user/twanga/latency.toml
```

```
$ twanga calibrate --show
Device:       USB PnP Sound Device
Latency:      42 ms
Measured at:  epoch-seconds:1779532834
Path:         /home/user/twanga/latency.toml
```

The first time you run `twanga play <tab> --policy tight` (or `casual`)
after calibrating, the playback header surfaces the calibration state on
stderr so you know what scoring assumptions apply:

```
Latency:    42 ms (calibrated for 'USB PnP Sound Device')
```

Or, if you haven't calibrated yet:

```
Latency:    uncalibrated. Run `twanga calibrate` for tighter scoring.
```

Or, if your active mic doesn't match the saved calibration's device:

```
Latency:    uncalibrated for 'Built-in Microphone' (saved value is for 'USB PnP Sound Device'). Scoring may be off; run `twanga calibrate` to refresh.
```

| Flag | Description |
|------|-------------|
| `twanga calibrate` | Run the full 5-click measurement and persist. |
| `twanga calibrate --show` | Print the stored value without measuring. |

## Where things live

- **Native** — `$DATA_ROOT/latency.toml`. Schema:
  ```toml
  device_name = "USB PnP Sound Device"
  latency_ms = 42
  measured_at = "epoch-seconds:1779532834"
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
- [User guide → audio setup](user-guide.md) — broader context on getting
  your mic + interface dialed in.
