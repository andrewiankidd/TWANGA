# Project status

**Functional end-to-end on the CLI *and* on the web build** (which is
also what the Tauri 2 desktop shell hosts — same `frontend/web/` bundle,
two delivery paths). Full CLI ↔ GUI feature parity is a project invariant
and currently holds across every shipped surface.

For the full shipped-feature inventory + when each landed, see
[CHANGELOG.md](../CHANGELOG.md). For what's coming, see
[ROADMAP.md](ROADMAP.md). This page is the high-level summary.

## What works today

| Feature | CLI | GUI | Page |
|---------|-----|-----|------|
| **Tuner** — live pitch detection, per-string + chromatic, capo-aware | ✅ | ✅ | [features/tuner.md](features/tuner.md) |
| **Recorder** — capture played notes as alphaTex, metronome, pre-roll, pause/resume, undo-column | ✅ (`u + Enter` undo) | ✅ (button) | [features/recorder.md](features/recorder.md) |
| **Playback** — scroll cursor + metronome + wait mode + loop + transpose (drop or octave-shift) + capo + resume bookmarks | ✅ | ✅ | [features/playback.md](features/playback.md) |
| **Patterns** — bundled rhythm + picking drills (10 across 4 groups), tree-of-difficulty | ✅ | ✅ | [features/patterns.md](features/patterns.md) |
| **Tab editor** — cell-level edits, column insert / delete / clear, title + BPM | ✅ (`twanga edit`, scriptable) | ✅ (interactive grid) | [features/editor.md](features/editor.md) |
| **Tunings** — built-in + user-defined registry, add/remove/list | ✅ | ✅ | [features/tunings.md](features/tunings.md) |
| **Docs** — per-feature pages embedded in the binary / bundled with the web app | ✅ (`twanga docs`) | ✅ (`#docs`) | [features/](features/) |

Backed by:

- **Pitch detection** — `twanga-dsp::Tuner` (YIN), shared between CLI
  (CPAL) and GUI (Web Audio + AudioWorklet → WASM). Identical Rust
  implementation in both.
- **Capo** — `twanga-core::Capo` per-string semitone offsets, composes
  with any tuning. Round-trips through the alphaTex `\subtitle` field
  so recordings replay with their capo without manual reentry.
- **alphaTex** — own parser + serializer in `twanga-tabs`. Used as the
  on-disk format on CLI and the in-IDB blob format in the GUI; same
  bytes either way.
- **Renderer plugin system** — `frontend/web/render/` with a stable
  `{ id, name, version, create() }` contract. Two built-ins ship
  (Tab + Highway); future third-party plugins register through the
  same path. The Editor uses an `interactive: true` variant of the
  Tab renderer, so the editor's grid IS the rendered tab.

## What's next

**The roadmap's "follows" tier** is the next big-rock work — see
[ROADMAP.md](ROADMAP.md) for the ordered list. Top of the list:

1. **Tab audio generation** (prerequisite for slow-down practice +
   backing tracks).
2. **Slow-down practice** via `rubato` / `signalsmith-stretch` — gated
   on (1).
3. **Pattern trainer accuracy verification** — rhythm-only verification
   on top of the existing pattern library.
4. **Section looper / adaptive difficulty / tab fade-out** —
   Master-Mode style; independent of audio generation.

**Tauri desktop shell** is now feature-complete on the filesystem
axis: recordings live at `$CONFIG/twanga/recordings/` (visible to
`twanga play` immediately), user tunings round-trip with
`$CONFIG/twanga/tunings.toml`, external links open in the OS browser.
Remaining Tauri polish (native CPAL backend for sub-20ms latency,
hand-crafted per-platform icons, installer build in the release
workflow) is on the [BACKLOG](BACKLOG.md#tauri-desktop-polish).
**Mobile (Tauri Mobile)** stays on the [ROADMAP](ROADMAP.md) v2
tier — Android / iOS path-resolution + audio-permission flows differ
enough to warrant a separate pass.

**Backlog** — see [BACKLOG.md](BACKLOG.md) for smaller adjustments,
QoL polish, content expansions, and longer-horizon directions
(practice mechanics, audio import / Demucs, sample bank, etc.).
