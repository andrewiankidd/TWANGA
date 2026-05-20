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
| Web tuner per-string display + tuning picker (full parity with `twanga tune`) | done |
| Web Tunings screen: built-in + user-defined merged list, inline editor, capo round-trip | done |
| Pluggable renderer system (registry + Tab + Highway built-in plugins, same path for future third-party renderers) | done |
| Web tab recorder — mic → fret detection → alphaTex download (full parity with `twanga record`) | done |
| Web tab playback — library list (bundled examples + IDB-stored user recordings + drop-zone import), playhead-driven renderer, wait / loop / metronome / pre-roll / BPM override / transpose / pause | done |
| Recorder + Playback QoL pass across CLI + GUI — title prompt on record, duration display, "couldn't fit on fretboard" indicator, metronome on record, pre-roll / count-in, pause / resume | done |
| Shared `makeTuningController` widget consumed by Tuner + Recorder + Playback (closes the duplication backlog item) | done |
| Custom CLI subcommand `twanga tunings remove` for parity with the GUI's delete button | done |
| Tab editor (GUI-first) — post-capture cell-level editing of recordings with alphaTex round-trip | next |
| Bidirectional sync of `$CONFIG/twanga/tunings.toml` ↔ `localStorage` via a Tauri command | deferred (Tauri work paused while we prove things on web) |
| Tauri filesystem backend for the browser tab library — `library-tauri.js` reads `$CONFIG/twanga/recordings/` once the matching Tauri commands land in `twanga-app` | deferred (Tauri work paused while we prove things on web) |
| Chord trainer with polyphonic *verification* (not transcription) | follows |
| Slow-down practice (time-stretch via `rubato` or signalsmith-stretch) | follows |
| Section looper / adaptive difficulty / tab fade-out | follows |
| Right-hand pattern trainer (banjo rolls, uke strums) | follows |
| MusicXML import in `twanga-tabs` (open-standard sheet-music interop) | follows |
| Mobile (Tauri Mobile) | v2 |
