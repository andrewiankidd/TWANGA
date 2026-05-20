# Roadmap

Forward-looking only. Everything previously listed as `done` lives in the
[CHANGELOG](../CHANGELOG.md); this file is what's still ahead. Smaller
adjustments, QoL polish, bugs, and "maybe" items live in
[BACKLOG.md](BACKLOG.md) instead.

## Deferred (Tauri — paused while we prove things on web)

The Tauri 2 desktop shell already hosts the web bundle in a native
window; what's deferred is the filesystem integration that would
replace the browser's IDB + localStorage backends. We're holding off on
that until the web build's feature surface stabilises so the desktop
shell inherits a finished product rather than chasing a moving target.

| Item |
|------|
| Bidirectional sync of `$CONFIG/twanga/tunings.toml` ↔ `localStorage` via a Tauri command |
| Tauri filesystem backend for the browser tab library — `library-tauri.js` reads `$CONFIG/twanga/recordings/` once the matching Tauri commands land in `twanga-app` |

## Follows (next big rocks)

Each of these is genuinely new — not a polish pass on existing
surfaces. Ordered by current best-guess ROI / scope, not by commitment.

| Milestone | Notes |
|-----------|-------|
| **Tab audio generation** (prerequisite for slow-down + several practice mechanics) | Today Playback only scrolls a cursor + metronome — the user provides the audio. To make slow-down practice, "play this section while I listen first," or any kind of backing track meaningful, the engine needs to actually *produce sound* for the tab's notes. Two complementary paths: (a) a basic synth path in `twanga-synth` (sine + harmonic stack at the target pitch for the column's duration — already partly there for the metronome click), (b) a sample-bank path that uses captures from the tuner / recorder ([Self-recorded sample bank](BACKLOG.md#self-recorded-sample-bank-your-own-soundfont) in the backlog) when available, with the synth as fallback. The bank work is the part that's interesting long-term; the synth path is the cheap version that unblocks everything below. |
| Slow-down practice (time-stretch via `rubato` or signalsmith-stretch) | Depends on tab audio generation above — until there's audio to slow down, the existing BPM override already covers the user-plays-themselves case. Once audio exists, `rubato`/`signalsmith-stretch` gives you time-stretch without pitch shift. |
| Chord trainer with polyphonic *verification* (not transcription) | Biggest pedagogical win; hardest. Verification (does this match the expected chord?) stays tractable where transcription (what chord is this?) doesn't. |
| Section looper / adaptive difficulty / tab fade-out | Builds on the existing loop + a new accuracy-tracking subsystem. Master-Mode-style. Independent of audio generation. |
| MusicXML import in `twanga-tabs` | Open-standard interop. Unlocks the "I have a Guitar Pro library" workflow (most engravers export to MusicXML). |
| Pattern trainer — accuracy verification | A first cut of the pattern trainer (bundled rhythm + picking drills, GUI screen with grouped tree-of-difficulty browser, loop-by-default) has shipped. The remaining ROI here is **verification** — currently the user plays along to the loop and the app doesn't check whether the rhythm landed right. Rhythm-only verification (the existing wait-mode pitch match, plus an onset-timing tolerance) is the next bite. Independent of audio generation. |

## v2 (longer horizon)

| Milestone |
|-----------|
| Mobile (Tauri Mobile) |
