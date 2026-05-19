# CLAUDE.md

Working notes for Claude — and any future contributor — on **how** TWANGA is
built and the principles to honour when changing it. The README + `docs/`
cover *what* TWANGA is and where it's going; this file captures the
constraints those decisions need to fit inside.

Read this once when you join a session. Re-read it any time a change feels
like it conflicts with a principle below — if it does, the change probably
needs a different shape, or this file needs an explicit amendment.

## Mission, in one breath

An open-source learning tool for fretted/strung instruments — banjo,
ukulele, mandolin, guitar, anything with strings + frets. Local-first, no
subscription, bring-your-own-tabs. Built so an amateur dev + amateur
musician (the author) actually *uses* it.

## Build principles

### 1. First-class CLI and GUI with feature parity

Every flag, prompt, persisted setting, and runtime affordance in
[`twanga-cli`](crates/twanga-cli/) should have a GUI equivalent in
[`frontend/web/`](frontend/web/) (which is also the Tauri webview bundle).
Drift in either direction is treated as a bug:

- A CLI feature without a GUI counterpart → file a backlog entry for the
  web/desktop port, link the two.
- A GUI feature without a CLI counterpart → same, in reverse.

The current capo work is the canonical example: per-string `Capo` shipped
in `twanga-core` first → CLI flag → web Tuner control, in that order, with
identical semantics at each layer.

### 2. Cross-platform is a constraint, not a checklist

Win / macOS / Linux + iOS / Android + web are all in scope. Mobile and web
are *parallel* targets shaped into the architecture from the start, not a
v2 graft-on:

- `frontend/web/` is the same bundle whether GitHub Pages serves it or
  Tauri's webview wraps it — **one UI codebase, multiple delivery shells**.
- Audio I/O is Web Audio + AudioWorklet in the browser, CPAL on native.
  The DSP that runs on both is in `twanga-dsp` and stays identical.
- Persistence is `localStorage` in the browser, `$CONFIG/twanga/*.toml` on
  native. Same schema; the storage backend is the only difference.

A change that lands on one platform but breaks another isn't done.

### 3. Separation of concerns in purpose-built crates

The workspace is structured so each crate has a narrow responsibility:

- **`twanga-core`** — domain types only. IO-free, async-free, no algorithms.
  Bottom of the dependency graph. Adding a runtime dep here is a smell.
- **`twanga-dsp`** — pitch detection + streaming `Tuner`. No allocations in
  hot path; tests pin "no allocs after first call."
- **`twanga-synth`** — deterministic signal generation. Math-anchored
  against `sin()` arithmetic at `fs/4` and `fs/6` so the synth itself
  can't drift.
- **`twanga-audio`** — CPAL wrapping for native audio I/O. Browser audio
  goes through the Web Audio + WASM path, not this crate.
- **`twanga-tabs`** — alphaTex + (eventually) MusicXML. Tab-format parsing
  + serialising lives here, separate from everything else.
- **`twanga-tui`** — terminal UX primitives shared between CLI surfaces.
- **`twanga-cli`** — CLI binary; mostly UX glue over the above.
- **`twanga-app`** — Tauri 2 desktop shell; hosts `frontend/web/`.
- **`twanga-web`** — `wasm-bindgen` shim. Mirrors the native API surface
  where it makes sense (`WebTuner` ↔ `Tuner`, etc.), so browser code uses
  the same data model.

When in doubt: a new feature probably introduces a new module *inside* one
of these crates, not a new crate. New crates appear when the existing
boundary genuinely doesn't fit (e.g. `twanga-web` exists because WASM
bindings need their own cdylib).

### 4. No proprietary formats, no runtime AI, no walled garden

See [docs/SCOPE.md](docs/SCOPE.md) for the explicit list. The shorthand:

- **No Guitar Pro `.gp5` / `.gpx`.** alphaTex now, MusicXML future. Same
  legal posture as ASIO redistribution — out of scope even if a community
  PR offers one.
- **No runtime AI** in the shipped binary. Pitch detection is
  deterministic DSP. AI is welcome during *development* (this Claude
  session); it does not ship.
- **No cloud accounts, no telemetry, no subscription, no DRM.** Tabs are
  user-owned files. Sync happens via the user's chosen filesystem
  mechanism (Dropbox, Syncthing) — never a TWANGA server.

### 5. Tests pin behaviour, not implementation

The recent capo + tuning-registry work added tests for: round-tripping
through TOML, capo composition preserving reentrant labels at zero offset,
fret-aware string matching picking the lowest fret on ambiguous matches,
etc. The pattern is: **a test should still pass after a refactor that
preserves the user-visible contract**, and fail when the contract changes.

Tests are valuable specifically when they pin invariants you'd otherwise
have to re-discover from the code on a future change. They're a tax when
they pin implementation details that have no observable effect.

### 6. TDD when it pays off

`twanga-core`'s `Capo`, the tuning registry's TOML round-trip, the alphaTex
parser/writer, and the `MidiNote::from_name` parser were all written
test-first. TDD pays the most for pure functions with concrete I/O
contracts. It pays less for UX glue and graphics code, where the test cost
matches or exceeds the verification value. Use judgement.

## Where things live

- [`README.md`](README.md) — public-facing pitch + quick-start.
- [`docs/PROJECT_STATUS.md`](docs/PROJECT_STATUS.md) — what works today.
- [`docs/ROADMAP.md`](docs/ROADMAP.md) — committed next + what follows.
- [`docs/BACKLOG.md`](docs/BACKLOG.md) — everything we've considered,
  loosely prioritised. Long; don't expect to read top-to-bottom.
- [`docs/SCOPE.md`](docs/SCOPE.md) — what TWANGA explicitly is NOT.
- [`CHANGELOG.md`](CHANGELOG.md) — Keep-a-Changelog-style log; new commits
  add to `[Unreleased]` until a tag rolls it forward.
- [`.toolbox/.claude/skills/`](.toolbox/.claude/skills/) — Claude skills
  this repo uses (commit, changelog, cleanup, review-docs, security, …).

## Active conventions

- **Conventional commits.** `feat:` / `fix:` / `ci:` / `docs:` / `style:`
  / `refactor:` / `chore:` / `test:`. See recent `git log --oneline`.
- **Branch protection on `main`.** Required CI checks gate merges (set via
  `gh api ... /branches/main/protection`, not in source). Direct admin
  pushes still allowed for the solo-dev workflow.
- **Auto-merge for patch dependabot bumps.** Manual review for minor /
  major. See [`.github/workflows/dependabot-automerge.yml`](.github/workflows/dependabot-automerge.yml).
- **`frontend/web/pkg/` is gitignored.** WASM artifacts are regenerated by
  CI + the local build commands documented in
  [`crates/twanga-app/README.md`](crates/twanga-app/README.md).
- **Cross-platform persistence.** Native uses `$CONFIG/twanga/*.toml`
  (via the `directories` crate). Browser uses `localStorage`. Both
  schemas are identical (`PresetEntry`, `Capo` spec strings, etc.) so a
  future Tauri shell can sync the two.
