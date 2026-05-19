# Changelog

All notable changes to TWANGA are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows
[SemVer](https://semver.org). Pre-release everything currently lives under
`[Unreleased]` — the first tag (`v0.1.0`) will roll this list forward into a
dated section.

## [Unreleased]

### Fixed

- Browser tuner crashed on entry with `tunerState.tuner.name is not a
  function`. Cause: the `#[wasm_bindgen] impl WebTuner` block was
  accidentally split when the custom-tuning constructors landed,
  silently dropping `name()` / `string_labels()` / `strings_info()` /
  `feed()` / `take_readings()` from the exported JS class. Cargo tests
  didn't catch it because `#[wasm_bindgen]` is a no-op on native
  targets. Moved the helper into the bindgen impl so all instance
  methods stay exported.

### Added

- Main-menu splash refreshes on every return to the menu and on a
  2-minute idle timer while the menu is visible (paused when the tab
  is backgrounded). No more page-reload-to-reroll.
- Per-string capo on the Tuner screen. A "Per-string" toggle next to
  the uniform stepper expands a panel of one stepper per string in
  the active tuning, mirroring `twanga tune --capo "0,2,2,2,2,2"` on
  the CLI. Useful for drop-D-style partial capos, banjo 5th-string
  spike, etc. Mode + per-string spec persist to localStorage; older
  saves migrate as uniform.
- Custom user-defined tunings in the browser Tunings screen. Define a
  tuning by name + per-string pitches (`A4` / `C#3` notation, live MIDI
  preview), saved to `localStorage` under `twanga-user-tunings-v1` —
  same `PresetEntry` shape the CLI writes to
  `$CONFIG/twanga/tunings.toml`. The Tuner picker reads from the merged
  built-in + user list; capo support works for user tunings too. New
  WASM bindings `validate_preset_entry`,
  `WebTuner.new_for_strings_custom`, and
  `WebTuner.new_for_strings_custom_with_capo` enforce the same slug /
  range / name rules the CLI's `twanga tunings add` flow does.
- `twanga-web` crate exposing a small slice of `twanga-core` / `twanga-dsp`
  through `wasm-bindgen`: `pick_splash`, `builtin_tuning_slugs`,
  `midi_from_name` / `midi_to_name`, and `detect_pitch` (real YIN at the
  same threshold the CLI tuner uses). 9 cargo tests cover the wrapper
  logic without needing a browser. (`27c13ca`)
- Browser frontend scaffold under `frontend/web/`: a landing page
  (`index.html`) styled after the tinnedspaghetti template — animated wave
  header, light/dark toggle, download buttons, footer with personal-site
  logo — and a separate `app.html` shell with a main-menu / hash-routed
  screens layout. Tuner screen runs the synthetic-440-Hz pitch test
  against the real WASM-compiled YIN. Tauri-safe external-link interception
  baked in via a `window.__TAURI__` runtime check. (`038fe22`)
- Functional end-to-end CLI: `twanga tune`, `record`, `play`, `tunings`,
  `devices`, `convert`. YIN pitch detection, alphaTex recorder, tab playback
  with metronome, wait-for-note practice mode, cross-instrument transpose,
  and `--loop` for full or section repeats. (`74b2e22`)
- TOML-backed tuning registry — built-in presets live in
  `crates/twanga-core/src/presets.toml` and share their schema with the
  user-config file at `$CONFIG/twanga/tunings.toml`. Ships `drop-d-guitar`,
  `tenor-banjo`, and `tenor-ukulele` alongside the original three built-ins.
  New `twanga tunings list | path | add` subcommand for managing user
  tunings interactively. (`79cab31`)
- Per-string `Capo` type that composes with any tuning — uniform
  (`--capo 3`) or partial (`--capo "0,2,2,2,2,2"` for drop-D-style setups,
  banjo 5th-string spike, etc.). Wired through `tune` / `record` / `play`.
  (`553113f`)
- Capo round-trips through alphaTex via a `; capo=<spec>` suffix in the
  `\subtitle` field, so a recording made with a capo replays correctly
  without the user re-typing the value. alphaTab-compatible — no custom
  directive. (`75fc775`)

### Changed

- MOTD banner now prints on every subcommand invocation (single call site at
  the top of `main`) rather than only on interactive subcommands. Writes to
  stderr so piped stdout stays clean. All value-bearing flags accept a
  consistent three-form pattern across the CLI: `--flag value` (direct),
  `--flag` (bare → prompt), or omitted (prompt or default). (`87b5855`)

### Other

- Release workflow now produces stable user-facing archive filenames
  (`twanga-windows.zip`, `twanga-linux.tar.gz`,
  `twanga-macos-{intel,apple-silicon}.tar.gz`) so
  `releases/latest/download/<file>` resolves the same URL across releases.
  Version + triple stay visible in the extracted directory name. (`36bca79`)
- Project docs reorganised: `Project status`, `Roadmap`, `Scope` lifted out
  of the README into `docs/`; CLI screenshots added to the main README to
  show what each subcommand actually looks like. (`21013c0`)
- `docs/BACKLOG.md` captures the long-tail feature dump (GUI surfaces,
  practice mechanics, audio pipeline, etc.) that previously only lived in
  chat. (`1c62976`)
- Tab-format pitch corrected across all docs: alphaTex is the supported
  format today, MusicXML is the future open-standard target. Proprietary
  formats (Guitar Pro `.gp5` / `.gpx`) added to the explicit non-goals.
  (`cf25ae6`)
- Dependabot configured for weekly cargo + github-actions updates; patch
  bumps auto-merge through a companion workflow once CI passes (branch
  protection on `main` gates the merge). (`03afa61`)
