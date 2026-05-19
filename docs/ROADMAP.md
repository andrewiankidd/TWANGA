# Roadmap

| Milestone | Status |
|-----------|--------|
| Workspace scaffold | done |
| Domain model (`Tuning`, `MidiNote`, `Frequency`) | done |
| CLI tuner (YIN + CPAL + multi-string UI + chromatic mode) | done |
| CLI tab recorder → alphaTex | done |
| CLI tab playback (cursor view + metronome + wait mode + loop) | done |
| Tuning registry (built-in TOML + user-defined `tunings.toml`) | done |
| Capo (per-string offsets, alphaTex subtitle round-trip) | done |
| WASM bridge crate (`twanga-web`) + cargo tests for the bindings | done |
| Web frontend scaffold (landing + app) deployed to GitHub Pages | done |
| Web tuner: mic capture via Web Audio + AudioWorklet → YIN via WASM | done |
| Tauri 2 desktop shell hosting the shared `frontend/web/` bundle | done |
| **Per-string tuner display in the web build** — tuning picker + cents indicator (replaces the bare chromatic Hz readout) | next |
| Tab rendering via [alphaTab](https://github.com/CoderLine/alphaTab) (works in both the web build + the Tauri webview) | after the per-string tuner |
| Web tab recorder — mic → alphaTex download (browser equivalent of `twanga record`) | follows |
| Chord trainer with polyphonic *verification* (not transcription) | follows |
| Slow-down practice (time-stretch via `rubato` or signalsmith-stretch) | follows |
| Section looper / adaptive difficulty / tab fade-out | follows |
| Right-hand pattern trainer (banjo rolls, uke strums) | follows |
| MusicXML import in `twanga-tabs` (open-standard sheet-music interop) | follows |
| Mobile (Tauri Mobile) | v2 |
