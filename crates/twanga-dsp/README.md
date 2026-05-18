# twanga-dsp

Pure pitch detection (`Yin`) and the streaming `Tuner` that wraps it.

`Yin` is the canonical de Cheveigné & Kawahara (2002) algorithm with parabolic interpolation for sub-sample precision. `Tuner` adds:

- a `TunerMode::Chromatic` / `TunerMode::Strings(Tuning)` switch — chromatic snaps to the nearest 12-TET note (used by the tuner's "no instrument" mode and the tab recorder, which does its own fret-aware string match downstream); strings mode snaps to the nearest open string in the tuning (used by the per-string tuner display);
- a sliding analysis-window buffer that emits `TunerReading`s when full;
- a silence gate (window RMS below 0.005 → skip) and an out-of-range gate (Strings mode only; rejects matches more than 700 cents from any string) so cable hum and mains EMI don't pollute readings.

Both `Yin::detect()` and `Tuner::feed()` deliberately allocate their scratch buffers *once* and reuse them across calls — `yin_reuses_scratch_buffers_across_calls` pins the no-allocs-in-hot-path invariant.

A second independent algorithm (Hann-windowed FFT peak detection with log-magnitude parabolic interpolation) is used in the test suite as a cross-check: YIN and FFT must agree within 10 cents across the MIDI 48–81 sweep, so neither implementation can silently drift against the synth.

- **Check**: `cargo check -p twanga-dsp`
- **Test**: `cargo test -p twanga-dsp`
- **Depends on**: `twanga-core`
- **Used by**: `twanga-cli`, `twanga-bench`, `twanga-app`

See [the workspace README](../../README.md) for project context.
