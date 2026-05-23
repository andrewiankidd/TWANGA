# Changelog

All notable changes to TWANGA are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows
[SemVer](https://semver.org). Pre-release everything currently lives under
`[Unreleased]` — the first tag (`v0.1.0`) will roll this list forward into a
dated section.

## [Unreleased]

### Added

- **`twanga play --from-file <wav>` + 23-scenario integration suite.**
  Playback now swaps the live mic for a paced WAV-file replay when
  `--from-file` is set, making the whole pipeline (sample read →
  tuner → onset detector → wait-mode match / proximity scoring →
  summary print) deterministically testable. A new in-house mono
  PCM WAV reader/writer (`twanga-cli/src/wav.rs`, ~80 LOC, zero new
  deps, supports PCM int-16 + IEEE float-32) backs a
  `WavSampleSource` that paces sample delivery to wall-clock so the
  loop behaves the same as it would against a live stream. The bin
  picked up a small lib target alongside it so
  `tests/play_from_file.rs` can pull the WAV synthesizer in
  without duplicating it. The 23 scenarios cover: perfect takes
  (wait + tight + casual); consistent +70 ms lateness under both
  tight and casual (documenting that tight's effective window is
  unreachable once YIN's ~170 ms detection latency stacks); 2-second
  late = Missed; wrong pitch on time; silent input; chord input
  (pinned as "completes without crashing" — chord scoring is a
  known limitation); fast-passage long-tail regression (Ship 1
  fix); 60 / 200 / 300 BPM tempo edges; ring-out and back-to-back
  plucks; jittery + accelerating + decelerating pacing; mixed
  right/wrong pitches; dropped middle note; extra in-between
  strums; 8-note mixed-timings long passage; and a free-play
  sanity check. A module-level mutex serialises the scenarios so
  default `cargo test` runs reliably without `--test-threads=1`.
- **Tab importer — MIDI / ABC notation / ASCII tab.** Three new
  open-format parsers extend the importer beyond alphaTex /
  MusicXML / MXL. CLI (`twanga import` / `twanga convert`), WASM
  bridge, and GUI Importer all gain `.mid` / `.midi` / `.abc` /
  `.tab` support in parity; ambiguous `.txt` now content-sniffs to
  decide alphaTex vs ASCII tab. MIDI uses the first note-bearing
  track (others surface as `ParseWarning::SkippedTrack`); MIDI / ABC
  / non-recognised ASCII surface a `ParseWarning::InferredTuning`
  since those formats carry no string/fret data. ASCII tab labels
  are matched against the built-in tuning registry exactly first
  (so `D B G D g` pins to banjo cleanly), then fall back to
  nearest-by-string-count.
- **Articulation preserved through ASCII tab → alphaTex.**
  `TabColumn` now carries `articulation: Option<u8>` and the
  alphaTex writer + parser round-trip the `h` (hammer-on), `p`
  (pull-off), `s` (slide) prefixes. The data survives even though
  TWANGA's playback / renderer don't yet consume it — backlog
  entry tracks wiring it through the rest of the stack.
- **`.txt` content sniffing.** `.txt` no longer assumes alphaTex;
  the resolver now peeks the first 8 KiB and routes the file to
  alphaTex or ASCII tab based on which format's shape dominates
  (backslash directives vs string-label-pipe-content lines).
  `--from` still overrides unconditionally.
- **Tab importer — MusicXML / MXL / alphaTex, CLI + GUI parity.**
  TWANGA now ingests external tab files into the user library
  (`<data-root>/library/`, distinct from `recordings/`) via a
  shared conversion pipeline. Lands as the format-agnostic "tab
  ingestion Phase 2" on the roadmap.
  - **`twanga-paths::DataRoot::library_dir()`** — `<root>/library/`,
    parallel to `recordings_dir()`. Both resolve through the
    portable / home modes; both have CLI scanning helpers
    (`bundled::scan_recordings` + `bundled::scan_library`) and
    Tauri shim coverage (`list_library_tabs` / `load_library_tab`
    / `save_library_tab` / `update_library_tab` /
    `delete_library_tab`).
  - **`twanga-tabs::musicxml`** — new MusicXML 3.1 partwise
    parser via `quick-xml` (streaming pull-parser, no DOM allocation).
    Covers `<work-title>` / `<creator>` / `<sound tempo>` /
    `<staff-tuning>` / `<staff-details><capo>` / `<note>` (incl.
    `<chord/>` chord members, `<rest>`, `<technical><string>` +
    `<fret>` explicit placements, and pitch-only notes inferred
    against the staff tuning). Surfaces non-fatal observations
    (irregular durations, unreachable notes, missing string
    tuning) via a `ParseWarning` enum so the importer UI can
    show a preflight summary. `.mxl` (zipped MusicXML, the
    MuseScore default export) supported via `musicxml::parse_mxl`
    — container manifest read first, fallback to "first
    `.xml`/`.musicxml` entry in the archive".
  - **`twanga import <path>`** — accepts `.alphatex` /
    `.musicxml` / `.xml` / `.mxl`, format detection by extension
    (override via `--from`). Title override via `--title`.
    Writes to `<data-root>/library/<slug>-<unix-secs>.alphatex`,
    same filename convention as `twanga record`.
  - **`twanga convert <input> --out <output>`** — sibling
    stateless transform (no library involvement). Useful for
    scripting bulk MusicXML→alphaTex conversion before importing.
  - **GUI Importer screen** — dedicated `#importer` route from the
    main menu. Drop-zone accepts the same extensions as the CLI;
    parses via WASM (`parse_musicxml` / `parse_mxl` /
    `parse_alphatex`); preview card shows title / source / tempo /
    tuning / column count + any parse warnings before the user
    commits. Title is editable in the preview before "Add to
    library". The previous Playback drop-zone was removed —
    `#importer` is the single entry point now.
  - **Format-agnostic `ParsedTab`** — promoted from `alphatex` to
    the `twanga_tabs` crate root via `pub use` so both parsers
    return the canonical type without weird namespacing. Existing
    `alphatex::ParsedTab` paths still work (re-export).
  - **CLI picker** — `twanga play` with no path now also lists
    imported tabs alongside bundled examples / patterns /
    recordings, with an `[imported]` prefix.
  - **New `docs/features/importer.md`** with the standard
    GUI / CLI tabs. CLI's `twanga docs importer` + the in-app
    docs viewer both surface it.

- **Pre-roll runway on every renderer (uniform contract, no view-
  specific hacks).** Both built-in renderers — Tab and Highway — now
  implement an optional `setPreRoll(n)` method declaring the count-in
  slot count. Each visualises it in its own metaphor, but the host
  treats them identically: one `setPreRoll` call, one `setPlayhead`
  call, no branching on view.
  - **Renderer contract** — `setPreRoll(n)` is documented in
    `frontend/web/render/registry.js` as a SHOULD-implement method
    alongside `setScore` / `setPlayhead` / `destroy`. `setPlayhead`
    now explicitly accepts NEGATIVE column indices during pre-roll,
    so a renderer that doesn't implement `setPreRoll` still has a
    well-defined runway model (the host gracefully no-ops the call).
  - **Tab renderer** — grid template extended from
    `[label] [note] [body…]` to `[label] [note] [runway × N] [body…]`.
    The runway is `N` faded `·` cells per string with a dashed
    bar-line separator on the LAST runway cell so the user can see
    where the count-in ends and real tab begins. Playhead bar slides
    through these cells during the count instead of being hidden.
  - **Highway renderer** — runway is implicit in lane geometry rather
    than DOM. A new `_effectiveLookahead = max(lookahead, preRoll)`
    helper widens the upper "future" zone when the pre-roll is bigger
    than the default 10-slot lookahead, shrinking slot height so col 0
    stays visible for the FULL count-in. (Previously: with `preRoll=16`
    and `lookahead=10`, the first 6 count-in ticks showed nothing.)
  - **Note-landing math fix** — a `delta=0` note now lands with its
    BOTTOM edge on the line top (falling-notes-strike geometry)
    instead of its centre straddling the line. The renderer comment
    already promised "notes land on this line at their moment in
    time" — the geometry now matches.
  - **View-switch state caching** — `wireRendererHost` in `app.html`
    caches the host's current score, playhead, and pre-roll, and
    replays all three on every mount. Without this, switching from
    Tab to Highway mid-stopped-state would jump the new renderer back
    to col 0 with no runway, even though the host was at e.g.
    playhead=-4 with preRoll=4. Plugin constructors now receive
    `preRoll` as an option too, so the first rebuild is correct
    in one pass.
  - **Host plumbing** — playback's `playbackPushScoreToRenderer`
    calls `setPreRoll(playbackState.preRoll)` once on load; the
    pre-roll `−`/`+` change handler calls it again on every nudge.
    `playbackStop` parks the playhead at `-preRoll` so the runway
    is visible in stopped state.

- **Reusable tuning-picker disclosure component.** The tuning +
  capo controls that appear on the Tuner / Recorder / Playback
  screens were hand-duplicated HTML in three places, each ~30 lines
  of nested markup with separate IDs and per-screen wiring. The
  `makeTuningController` factory now mounts its OWN markup from a
  `mountId` + `prefix` pair, deriving all internal element IDs from
  the prefix. Each screen's HTML reduces to one line:
  `<div id="tuner-tuning-mount"></div>`. The collapsed `<details>`
  summary shows just `Standard Ukulele · capo 3`; opening reveals
  the full dropdown + capo stepper + per-string toggle. Legacy
  explicit-ID mode is kept for any consumer that hand-writes the
  markup. Factory selected over per-screen duplication after the
  user pointed out "if it's the same UI in every project shouldn't
  it be reusable" — same pattern the mic-meter / device-picker /
  silence-threshold factories already follow.

- **Live-note column / pill on every renderer (CLI + GUI parity).**
  Every per-string row in the Tab renderer (and per-lane on the
  Highway renderer) now carries a small cell showing the absolute
  pitch class (`C`, `F#`, …) for the fret being played at the
  current playhead column on that string. The open-string label
  already establishes octave context, so the cell shows letter +
  accidental only — 2-char max, fixed width, doesn't shift the
  surrounding layout.
  - **Rust core**: new `MidiNote::pitch_class_name() -> &'static str`
    sharing a `PITCH_CLASS_NAMES` table with the existing `name()`.
  - **CLI**: row shape changes from `<label> | <body>` to
    `<label> | <note> | <body>` on `twanga play` and `twanga record`.
    Recorder shows the note for the last-committed fret per string;
    Playback shows the note for the fret at the playhead column.
  - **GUI Tab renderer**: new `noteColWidth: 32` option; grid
    template extended to `[label] [note] [body…]`. In read-only
    mode (Playback / Recorder) the cell follows the playhead; in
    interactive mode (Editor) it follows the user's selected-for-
    edit column.
  - **GUI Highway renderer**: per-lane live-note pill pinned just
    above the static string label, updated on every `setPlayhead`
    call to show the pitch class crossing the now-line on each
    lane. Empty when no note is at the current column.

  Use case: tells the user "fret 7 on the A string is an E" without
  any mental arithmetic. Particularly useful on banjo and uke where
  the same physical fret-position carries different absolute pitches
  per tuning. Same data shape on both surfaces; same `pitch_class_name`
  math on Rust and JS sides.

- **Runtime-tunable silence gate (CLI + GUI parity).** The YIN
  pitch detector's silence threshold (window-RMS below which no
  detection runs) was a hardcoded `Tuner::SILENCE_RMS = 0.005`;
  now it's a runtime field with full surfaces on both sides:
  - **Rust** — `Tuner` got `silence_rms()` getter and
    `set_silence_rms(rms)` setter. `DEFAULT_SILENCE_RMS = 0.005`
    stays as the out-of-box value; the `SILENCE_RMS` const sticks
    around as a backwards-compat alias.
  - **WASM** — `WebTuner.silence_rms()` and
    `WebTuner.set_silence_rms(rms)` exposed via `#[wasm_bindgen]`.
  - **CLI** — `--silence-rms <RMS>` flag on `tune`, `record`, and
    `play`. Runtime `[` + Enter / `]` + Enter keys step the gate
    by ~6 dB (×0.5 / ×2 in linear amplitude) on all three
    subcommands and on `wait_for_pitch`'s inner sub-loop. Each
    step prints `[silence: 0.00500 RMS (-46.0 dB)]` so the user
    sees what they've set.
  - **GUI** — new shared `makeSilenceThreshold` factory at
    `frontend/web/controllers/silence-threshold.js` matching the
    `makeMicMeter` / `makeDevicePicker` shape. A vertical-line
    thumb (`<input type="range">` overlay) sits on the
    mic-meter bar so the fill (live signal) and the thumb
    (threshold) share the same dB axis. Wired into all three
    mic-using screens (Tuner / Recorder / Playback wait mode);
    persists per-screen in `localStorage` under
    `twanga-{tuner,recorder,playback}-silence-rms-v1`. Pushed
    into the live `WebTuner` via `set_silence_rms` on every drag
    and re-applied to any freshly-constructed tuner instance.

- **Input-device picker on the three mic-using GUI screens.** Tuner,
  Recorder, and Playback (wait mode) now have a dropdown above the
  mic meter, populated via `navigator.mediaDevices.enumerateDevices()`
  and filtered to `kind === 'audioinput'`. Selected `deviceId` flows
  through `micSession.start({ deviceId })` into
  `getUserMedia({ audio: { deviceId: { exact: ... } } })`. New shared
  factory at `frontend/web/controllers/device-picker.js`, same shape
  as the mic-meter and silence-threshold controllers. Browsers gate
  device labels behind an existing mic-permission grant, so the
  first list shows generic names until permission is granted; Tauri's
  webview returns labels immediately. Hot-plug supported via the
  `devicechange` event. Stale stored device id (unplugged since the
  last session) falls back to default and clears its storage entry.
  CLI parity is the existing `--device "<name>"` flag.

- **Portable desktop variants alongside installers.** Each desktop
  platform now ships two flavours next to each other on the
  Releases page:
  - Windows: `twanga-desktop-windows-setup.msi` and
    `twanga-desktop-windows-setup.exe` (NSIS) for installers; a new
    `twanga-desktop-windows-portable.zip` (raw `twanga-app.exe` +
    README + licences) for no-install use.
  - macOS: `twanga-desktop-macos-setup.dmg` plus a new
    `twanga-desktop-macos-portable.app.tar.gz` (the `.app` bundle
    Tauri builds before wrapping into the DMG — drag-and-drop
    installable). Requires `--bundles app,dmg` so Tauri doesn't
    delete the intermediate `.app` after DMG packaging.
  - Linux: `twanga-desktop-linux-setup.deb` for the package path;
    the existing AppImage is now named `…-portable.AppImage` to
    match the convention. AppImage IS the Linux portable form;
    no raw-ELF tarball is shipped (Tauri needs libwebkit2gtk-4.1
    at runtime, so a bare ELF wouldn't be portable).

- **Platform-grouped download picker on the landing page.**
  `frontend/web/index.html` now renders a single button per platform
  (Windows / macOS / Linux); clicking opens a small dropdown of
  the available choices (CLI Tool / Desktop Portable / Desktop
  Installer + Windows's NSIS variant). Built on native
  `<details>` / `<summary>` — no framework, no JS toggle code,
  just CSS. Outside-click + open-another closes the previous. Also
  fixes the URL pattern to deep-link into the `latest-main` rolling
  pre-release rather than `/releases/latest/` (which 404s while we
  don't have a versioned tag).

- **Mic-level meter on the Tuner screen.** Tuner is the primary
  mic-consumer in the app but was the only screen WITHOUT the
  shared mic-meter that Recorder + Playback (wait mode) have.
  Wired as the third consumer of `makeMicMeter({...})` — same RMS
  bar, same dB readout, same "no signal" hint after 2 s of no
  audio chunks. "No reading" is now distinguishable from "no audio
  reaching the mic" (permission denied, OS-level mute, suspended
  `AudioContext`, dead cable).

- **macOS ad-hoc codesigning + Gatekeeper instructions inside the
  artefacts.** `bundle.macOS.signingIdentity = "-"` in
  `tauri.conf.json` makes Tauri ad-hoc sign the `.app` during the
  bundle step — without this, Gatekeeper shows "is damaged and
  can't be opened" and even the right-click → Open workaround
  fails. A new `crates/twanga-app/dist/MACOS-README.txt` ships
  inside both the DMG (injected via `hdiutil convert` to UDRW →
  mount → drop file → unmount → re-compress to UDZO) and the
  portable `.app.tar.gz`, so users who download via a deep link
  still see the workaround (Privacy & Security → "Open Anyway",
  or `xattr -d com.apple.quarantine /Applications/TWANGA.app`).
  Full Apple Developer ID notarisation is still future work.

- **Author's personal-setup section in the Hardware doc.**
  Concrete examples between the TL;DR table and Option 1:
  - Banjo — [KNA BP-1](https://www.knapickups.com/en/folk-instruments/bp-1-kna)
    passive wooden-cased piezo sensor clamped to the bridge
    (output jack assembly cable-tied to a pot bracket) → Realtone
    cable.
  - Ukulele — cheap adhesive piezo disc stuck to the underside
    of the body, jack dangling out the back → cheap USB DAC with
    a 3.5 mm input.
  Both clear the wait-mode latency budget; neither costs much.

- **Hardware doc "Common gotchas" section.** Four real footguns
  that aren't TWANGA bugs but feel like them — sample-rate
  mismatch making wait-mode laggy, USB hub power-budget issues,
  ambiguous-default-device when multiple inputs are present
  (with the `twanga tune --device "<name>"` substring match
  fix), and a hard "don't use Bluetooth headsets" note (HFP mic
  mode adds 100–300 ms beyond wait-mode's tolerance).

- **New [`docs/DISTRIBUTION.md`](docs/DISTRIBUTION.md).** Human-
  facing summary of what TWANGA ships per platform, what's signed
  vs unsigned, what warnings users will see and why, and what the
  release matrix actually does. Tables for per-platform artefact
  names, signing status, and the trigger → workflow mapping; a
  step-by-step "cutting a versioned release" recipe; and a
  future-work list (Apple Developer ID, Android release keystore,
  iOS signing pipeline, Windows EV cert — all deferred until
  there's demand justifying the recurring cost). Linked from
  README's Project docs section.

- **New Hardware setup guide** at
  [`docs/features/hardware.md`](docs/features/hardware.md). Cross-
  cutting reference page (not really a feature, but lives in the
  same per-page directory so the in-app docs viewer picks it up
  automatically) describing every realistic way to get sound into
  TWANGA: built-in mic, external USB mic, instrument-to-USB cable,
  proper audio interface, acoustic-pickup paths. Includes a
  latency / cost / quality comparison table up front, browser-vs-
  CLI latency notes, and pointers to `twanga devices` for
  verification. Wired through every slug-tracking spot: Rust
  `DOCS_PAGES` (with new `include_str!`), JS `DOCS_FEATURES`, the
  features index README, and both `CLI.md` / `GUI.md` hub tables.
  The Rust sync-point test (`slug_set_matches_expected_features`)
  was updated so the new slug is the canonical expected set.
  Renders correctly in both the in-app docs viewer (`#docs/hardware`
  card on the index) and via `twanga docs hardware`.

- **Release workflow builds all platforms.** `.github/workflows/release.yml`
  produces a complete cross-platform release on every push to main
  (refreshing the rolling `latest-main` pre-release) and on every
  `v*` tag (a draft versioned release). Final matrix shape:
  - **`build-platform`** — one job per desktop OS (Windows / Linux /
    macOS) that produces BOTH the CLI archive AND the Tauri desktop
    bundles in the same runner. Combined because `cargo tauri build`
    already compiles the entire workspace, so adding
    `cargo build -p twanga-cli` to the same job is nearly free
    thanks to incremental compilation on shared `target/`.
    Replaces the earlier `build-cli` + `build-desktop` split.
  - **macOS is a universal build** (`--target universal-apple-darwin`
    for Tauri; `lipo`-merged aarch64 + x86_64 CLI). Single DMG +
    single `.app.tar.gz` + single CLI tarball that all run natively
    on both Apple Silicon and Intel — the `macos-13` runner entry
    is gone (GitHub's Intel macOS capacity has multi-hour queues
    and the universal build sidesteps it entirely).
  - **`build-android`** — Tauri Mobile via `cargo tauri android
    build --apk --debug`. Debug-signed (Android's auto-generated
    debug keystore) so the APK sideloads cleanly; a real release
    keystore is a future step.
  - **`build-ios`** — best-effort Tauri Mobile simulator build via
    `cargo tauri ios build --target aarch64-sim`. macOS runner.
    Will produce a `.app.tar.gz` if the simulator build compiles;
    proper `.ipa` distribution needs Apple Developer signing,
    pending. Job is `continue-on-error: true` so iOS failures
    don't block the release going out, but honest red/green now —
    the previous `|| true` paper-over was removed and
    `if-no-files-found: error` makes empty artefacts surface
    properly.
  - **Aggregators wait for mobile too.** `release-rolling` and
    `release-tag` use `needs: [build-platform, build-android,
    build-ios]` + `if: always() && needs.build-platform.result ==
    'success'`, so Android + iOS finish uploading before the
    release is cut (avoids a race where mobile artefacts missed
    the release) while still allowing iOS to fail without
    blocking.

  Artefact names follow `twanga-{cli,desktop,mobile}-{platform}.{ext}`
  with `-setup` / `-portable` suffixes — so a glance at the
  Releases page tells you which file is what. See "Portable desktop
  variants alongside installers" above for the full layout.

  Matrix entries carry `name:` fields with emoji labels (🪟 / 🐧 /
  🍎 / 🍏 / 🤖) so the workflow output is scannable; step names
  follow the same pattern (📦 install, 🦀 rust, 🏗️ build, 🗜️ pack,
  ⬆️ upload).

- **Tauri desktop shell now reads + writes the same filesystem the
  CLI does.** The "Deferred" items on the ROADMAP have shipped:
  - **Recordings library** — `frontend/web/lib/library-tauri.js`
    is no longer a stub. New Tauri commands
    (`list_recordings` / `load_recording` / `save_recording` /
    `update_recording` / `delete_recording`) in
    `crates/twanga-app/src/commands.rs` read + write
    `.alphatex` files under `$CONFIG/twanga/recordings/` — the
    same directory `twanga record` writes to. Tabs made on the
    CLI appear in the GUI Playback library; tabs made in the
    GUI are playable via `twanga play`. Path-traversal
    protection on the Rust side: recording ids must be bare
    filenames.
  - **User tunings sync** — new commands
    `read_tunings_toml` / `write_tunings_toml` reading +
    writing `$CONFIG/twanga/tunings.toml` (the same file
    `twanga tunings add` writes). A new
    `frontend/web/lib/user-tunings-tauri.js` bootstrap module
    populates the existing `localStorage` cache from the TOML
    file at startup; `saveUserTunings()` write-throughs back to
    disk on every change. A custom tuning defined in the GUI
    is immediately visible to `twanga tunings list`, and a
    tuning added via the CLI shows up on next GUI startup.
  - **Dispatcher** — `library.js` now picks the IDB or Tauri
    backend at runtime based on `window.__TAURI__`. Bundled
    examples + patterns continue to load via `fetch()` on both
    paths (Tauri serves `frontendDist` through the same
    relative URLs the browser uses).
  - **`tauri-plugin-shell` wired** in `crates/twanga-app/src/lib.rs`
    + a default capability in `capabilities/default.json`. The
    external-link interceptor that already existed in
    `app.html` now actually opens URLs in the OS browser
    instead of warning about a missing plugin.

  Net effect: the desktop shell graduates from "webview that
  hosts the same bundle" to "native app that shares filesystem
  state with the CLI." Mobile (Tauri Mobile) is still on the
  v2 roadmap tier — the filesystem path will need an
  app-specific dir, but the JS dispatcher already handles the
  Tauri runtime check correctly.

- **Closes the last three CLI ↔ GUI parity gaps.** Three GUI polish
  items that had shipped without CLI counterparts now have full
  parity:
  - **Recorder: `u + Enter` while paused** undoes the last
    committed column. Mirror of the GUI's "Undo last column"
    button. Backed by a new `TabRecorder::undo_last_column()`
    method in `twanga-tabs` (two cargo tests covering happy path
    + empty-recorder no-op).
  - **Playback: per-file resume bookmarks.** On any user-
    initiated stop, `twanga play` saves the file path + current
    column + title + timestamp to
    `$CONFIG/twanga/play-resume.toml` (mirror of the GUI's
    `localStorage` resume map). Next time you load the same
    file, a `Y/n` prompt offers to resume at the saved column.
    New `--resume` / `--no-resume` flags skip the prompt in
    scripts. Stale bookmarks (saved column past the file's
    current length) are cleared silently. New `play_resume`
    module with 5 cargo tests.
  - **`twanga edit <path> <action>`** — non-interactive
    counterpart to the GUI Editor screen. Actions: `set`,
    `clear`, `clear-col`, `insert-col [--after N]`,
    `delete-col`, `title`, `bpm`. Each invocation applies one
    mutation and writes back in place (or to `--out <path>`).
    Round-trips through the same `AlphaTexWriter` path the
    Recorder uses on save, including capo + subtitle
    preservation. Scriptable: chain multiple edits in a shell.

  The "GUI-only by design" note for the Editor is gone — it now
  has a scriptable CLI counterpart. Tab editor is no longer in
  the "intentional asymmetry" bucket.

- **`twanga patterns` subcommand + bare-`twanga play` picker.**
  Closes the last CLI / GUI parity gap on the file libraries —
  the GUI has Playback's library list + the Patterns screen; the
  CLI now mirrors both.
  - **`twanga play` with no path** opens an interactive picker
    that scans bundled examples, bundled patterns, and
    `./recordings/` and lets you pick from a merged menu —
    same library the GUI's Playback screen shows. Each row is
    prefixed `[example]` / `[pattern · <group> · <pips>]` /
    `[recording]` so the source is clear.
  - **`twanga patterns`** (bare) — interactive picker over the
    bundled patterns tree, sorted by difficulty within each
    group. Plays the chosen pattern with `--loop full`
    defaulted.
  - **`twanga patterns list`** — catalog dump grouped by
    tradition with difficulty pips (★★☆☆). Scriptable.
  - **`twanga patterns play <id>`** — non-interactive play by
    manifest id. Flags: `--bpm`, `--no-metronome`, `--wait`,
    `--no-loop` (default is to loop).
  - **`twanga patterns path`** — print the manifest path.
  - **Backed by a new `bundled` module** in twanga-cli with
    serde_json-based manifest loaders + a `./recordings/`
    scanner. 6 cargo tests cover the missing-manifest
    fall-through, JSON parsing for both manifests, difficulty
    sort + pip rendering, and the recordings directory scan.
  - Embedded docs (`twanga docs patterns`, `docs/features/patterns.md`,
    [`crates/twanga-cli/README.md`](crates/twanga-cli/README.md))
    updated to describe the new commands.

- **Six new bundled patterns** — closes the obvious gaps across the
  existing default tunings and opens a new `guitar` group entirely.
  No code changes; each is a small `.alphatex` file + a manifest
  entry. Library count goes from 4 → 10:
  - **Ukulele strums** — adds `Baseline (D-D-U-U-D-U)` (the canonical
    pattern, now Level 1) and `Waltz strum (3/4)` (introduces a
    non-4/4 time signature without leaving the group). Existing
    Island strum bumped to Level 2 — it's syncopated, genuinely
    harder than the baseline.
  - **Bluegrass picking** — adds `Reverse roll` (natural pair to the
    forward roll, Level 1) and `Alternating-thumb roll` (Level 2 —
    thumb on every other 8th, alternating drone and low-D bass).
  - **Guitar (standard tuning)** — new group. `Boom-chick` (Level 1,
    bass-strum-bass-strum over open G shape, the country/folk
    rhythmic backbone) and `Travis picking` (Level 2, alternating
    thumb bass with fingers filling between, same chord shape so
    muscle memory transfers between the two).

- **Tests for the docs system.** Two new test surfaces:
  - **Rust** — `docs_tests` module in `crates/twanga-cli/src/main.rs`
    (7 tests): every embedded page starts with an H1; slugs are
    unique; `docs_page_text()` returns the body for known slugs
    case-insensitively and errors with a helpful message for
    unknown ones; `docs_listing_text()` includes every slug; and
    a sync-point assertion that pins the slug set (Rust `DOCS_PAGES`
    ↔ JS `DOCS_FEATURES` ↔ `pages.yml` bundle copy) so adding a
    feature surfaces as a deliberate cross-file edit. Required a
    small refactor: `run_docs` now delegates to two pure helpers
    (`docs_listing_text` / `docs_page_text`) so tests don't have
    to capture stdout.
  - **JS** — `frontend/web/lib/markdown.test.js`, 25 tests covering
    every renderer feature (headings, paragraphs, emphasis, code
    spans + fences, lists, tables, links, blockquotes, hr, XSS
    escaping, `javascript:` URL defence). Plus a smoke test that
    runs every shipped `docs/features/*.md` through the renderer
    and asserts non-empty H1 output. Uses Node 18+'s built-in
    `node:test` + `node:assert/strict` — no installed deps. Wired
    into `ci.yml` as a new step in the existing `test` matrix:
    `node --test "frontend/web/**/*.test.js"`.

- **Per-feature docs (Tuner / Recorder / Playback / Patterns / Editor
  / Tunings), embedded on all three surfaces.** New
  [`docs/features/*.md`](docs/features/) is the single source of
  truth; each page covers CLI + GUI side by side so the
  feature-parity invariant is enforced by the docs structure too.

  - **GUI**: new `#docs` / `#docs/<feature>` routes in the SPA, with
    a hand-rolled markdown renderer at
    [`frontend/web/lib/markdown.js`](frontend/web/lib/markdown.js).
    CI copies `docs/features/*.md` into `frontend/web/assets/docs/`
    on every deploy, so the same bundle serves the web build (GH
    Pages) and the Tauri shell (file://) — no separate docs site,
    docs version always pinned to the app version. Main menu has a
    new **Docs** card.
  - **CLI**: new `twanga docs [<feature>]` subcommand. Markdown
    bodies are `include_str!`'d at build time. No-arg lists the
    available pages; `twanga docs playback` prints the raw
    markdown to stdout. Pipe through `glow` / `mdcat` / `bat -l md`
    for fancy rendering.
  - **Landing page**: the previously-broken `docs.html` /
    `changelog.html` nav links now deeplink to `app.html#docs`
    and the repo's `CHANGELOG.md`.
  - **Existing hubs slimmed**: [`docs/CLI.md`](docs/CLI.md) and
    [`docs/GUI.md`](docs/GUI.md) become short overviews that point
    into the per-feature pages; full content moved out so there's
    no duplication.

- **Pattern trainer screen — first cut.** New `#patterns` screen
  with a curated, tree-organised browser of bundled rhythm /
  picking / strumming drills. Four patterns ship out of the
  box, grouped by tradition:
  - **Clawhammer (banjo)** — Bum-diddy (basic), Bum-diddy with
    drop-thumb
  - **Bluegrass picking (banjo)** — Forward roll
  - **Ukulele strums** — Island strum (D D-U U-D-U)

  Patterns are short `.alphatex` files at `assets/patterns/`
  with `\title`, `\subtitle`, and `\tempo` set sensibly for
  practice; the GUI loads them via the same playback engine
  user recordings + bundled examples use, with **`loop=full`
  preset** so the practice loop starts on the first Play click.
  Each row shows a 3-star difficulty badge and the target
  tuning. CLI parity for free: `twanga play
  assets/patterns/bum-diddy-simple.alphatex --tuning
  standard-banjo --loop`.

  Implementation reuses the existing Playback engine
  end-to-end — the Patterns screen is just a curated browser
  that hands off to `playbackLoad('pattern:<slug>')`. New
  `library.load()` id prefix `pattern:` joins `bundled:` and
  the integer IDB key; `library.patternsManifest()` exposes
  the grouped manifest for screen rendering. CI bundles
  `assets/patterns/` into the deployed site alongside
  `assets/examples/`.

- **Octave-shift transpose mode** (CLI + GUI, full parity). When
  re-tuning a tab onto a different instrument, notes that don't
  fit on the target's fretboard can now be retried at
  progressively wider ±12-semitone offsets before being dropped.
  Standard cross-instrument convention (TuxGuitar / MuseScore
  behaviour). Fixes the banjo→ukulele case where bass drones like
  A3/B3/G3 used to disappear; with octave-shift they now play as
  A4/B4/G4 on the uke body.

  - CLI: new `--transpose-mode <drop|octave-shift>` on
    `twanga play`. Defaults to `drop` (the historical behaviour);
    the header line surfaces the active mode (`Transposed: ...
    [octave-shift]`).
  - GUI: dropdown on the Playback screen below the tuning picker.
    Persists to `localStorage` with the rest of the playback
    settings. Re-runs the transpose immediately so the "Skipped:"
    count and the renderer reflect the rescued notes.
  - Backend: new `TransposeMode` enum on
    `twanga_tabs::alphatex` with a `transpose_to_with_mode` entry
    point; the existing `transpose_to_with_report` keeps its
    signature and delegates with `Drop`. WASM bindings updated
    on both `transpose_to` and `transpose_to_dropped_notes` —
    new optional string param accepting `"drop"` or
    `"octave-shift"`; unknown values fall back to `"drop"`. 4
    new cargo tests cover the up-shift, down-shift, in-range,
    and unrescuable cases.

- **Visual capo indicator on the renderers (GUI + CLI).** Per-
  string capo offset is now surfaced wherever the user looks:

  - **GUI Tab renderer**: when `score.capoSpec` is non-empty,
    each string label suffixes a `+N` annotation (so a drop-D
    capo on a guitar reads `D4 +2 / A3 / D3 / G3 +2 / B3 +2 /
    E4 +2`), and a small `capo N` / `capo [...]` badge appears
    in the corner above the string labels. Works in both
    read-only mode (Recorder / Playback) and `interactive: true`
    mode (Editor).
  - **GUI Highway renderer**: same `+N` suffix on the lane
    labels so the playing surface tells the same story as the
    Tab view.
  - **CLI `twanga play`**: row labels in the scrolling tab body
    now read `D4 +2 | ...` for affected strings. The score
    header line still prints `Capo: N (uniform)` / `Capo: [...]
    (partial)` as before, so the two work together.

- **Last-session resume on Playback.** Saved per-tab to a new
  `localStorage` map (`twanga-playback-resume-v1`); the most-recent
  bookmark surfaces as a banner above the library list on screen
  entry. Click **Resume** to load that tab and start playback at
  the saved column (the engine back-shifts `playStartTime` so the
  tick math reads as already-N-columns-in without firing phantom
  metronome clicks). **Dismiss** clears that bookmark. Bookmarks
  save on Stop and on Back-to-library; "finished" stops don't save
  (replaying from the end is useless). A library `delete` event
  in another tab also clears the matching bookmark.
- **Undo last column on Recorder.** While paused, an extra
  **Undo last column** button pops the most-recently committed
  column from the score and rewinds the wall-clock perception by
  one column (`totalPausedMs += msPerColumn`) so Resume doesn't
  immediately re-commit it with a phantom click. Click repeatedly
  to undo multiple. Disabled when nothing has been committed.

### Changed

- **Transpose-mode dropdown is hidden when no notes drop.** The
  "When a note doesn't fit" dropdown in the Playback per-tab view
  now shows only when the current configuration actually drops at
  least one note. When the tuning naturally fits everything, both
  the dropdown and the "Skipped:" message hide together — the
  drop-vs-octave-shift choice is meaningless if no notes are out
  of range (both modes produce identical output). The persisted
  value still applies invisibly the next time it matters.
  Companion change: the "Skipped:" message itself moved to sit
  directly under the transpose-mode dropdown so the consequence
  of the choice is visible next to the choice itself.

- **Highway lanes centre horizontally** in the renderer container,
  so a tab with three or four strings doesn't sit pinned to the
  left edge. Single `justifyContent: 'center'` on the flex root
  in `frontend/web/render/builtins/highway.js`.

- **Per-feature docs are bundled into desktop + mobile builds, not
  just the Pages site.** `pages.yml` had been copying
  `docs/features/*.md` into `frontend/web/assets/docs/` for a while
  so the in-app docs viewer can fetch them in the browser; the
  Tauri desktop / Android / iOS jobs in `release.yml` never did the
  same copy, so any doc the user added showed up on Pages but was
  missing from every native build. Hardware was the first case to
  expose this; tuner / recorder / playback / patterns / editor /
  tunings had been bundled-only-on-the-web too.

- **ROADMAP and BACKLOG are future-only.** Everything previously
  marked `done` lives in the CHANGELOG; the ROADMAP now lists only
  Deferred + Follows + v2 items, and the BACKLOG has shed all
  shipped sections (mic meter, last-session resume, undo column,
  cell-level edit, insert/delete columns, `twanga tunings remove`,
  the "parity owed" subsection, and so on). Added a new
  **architecture/infrastructure** entry flagging the duplicated
  playback engine (CLI Rust loop vs web JS loop) — the wait-mode
  column-skip bug only existed on the web side because of that
  drift.

- **Dedicated GUI + CLI docs.** Top-level README now leads with a
  Getting Started section featuring both first-class surfaces side
  by side: a screenshot of the GUI main menu and a code block of
  `cargo run -p twanga-cli` showing the splash banner + commands.
  Detail moved to [`docs/GUI.md`](docs/GUI.md) (per-screen tour +
  storage notes + local-dev steps) and [`docs/CLI.md`](docs/CLI.md)
  (subcommand tour with sample output + bundled-example references).
  Existing per-flag reference at
  [`crates/twanga-cli/README.md`](crates/twanga-cli/README.md) is
  unchanged.

### Fixed

- **External links with `target="_blank"` no longer open twice in
  Tauri.** The webview already routes `_blank` anchors through the
  shell plugin to the OS browser natively; our JS interceptor was
  also catching them and calling `plugin:shell|open`, so the URL
  opened twice (e.g. the footer's andrewkidd.co.uk link). Fixed by
  skipping anchors with `target="_blank"` in the interceptor's
  early-return chain — those are Tauri's job. Anchors without
  `target="_blank"` still go through our invoke so the homepage
  link (and similar) opens once.

- **Android APK now has proper launcher icons + microphone
  permissions.** `cargo tauri android init` regenerates
  `gen/android/` on every CI run from `tauri.conf.json`'s icon
  array. Our `icon.png` is 512×512 (below Tauri's recommended
  1024×1024), so the auto-generated `mipmap-*/ic_launcher.png`
  came out a bit blurry. New CI step right after `init` rsyncs
  the hand-tuned `crates/twanga-app/icons/android/` set over the
  auto-generated one. The same step also injects
  `<uses-permission android:name="android.permission.RECORD_AUDIO" />`
  and `<uses-permission android:name="android.permission.MODIFY_AUDIO_SETTINGS" />`
  into the generated `AndroidManifest.xml` — Tauri 2's schema has
  no field for app-level Android permissions (verified against the
  v2 reference), and without both permissions the WebView's
  `getUserMedia()` either fails silently or trips
  `NotReadableError` after grant. See `tauri-apps/tauri#10846` for
  the underlying constraint.

- **macOS `.app` now declares microphone entitlements + usage
  description.** The released bundle was throwing
  `undefined is not an object (evaluating
  'navigator.mediaDevices.getUserMedia')` on launch, blocking the
  tuner before it could even prompt for consent. The cause is the
  same family of problem as the Android entry above, but the fix
  is cleaner: WKWebView strips the `mediaDevices` object entirely
  (not just denies `getUserMedia()`) when the bundle lacks the
  `com.apple.security.device.audio-input` entitlement, and macOS
  refuses to surface a consent prompt at all without an
  `NSMicrophoneUsageDescription` Info.plist string. Two new files
  at [`crates/twanga-app/Entitlements.plist`](crates/twanga-app/Entitlements.plist)
  and [`crates/twanga-app/Info.plist`](crates/twanga-app/Info.plist)
  declare them; `tauri.conf.json` now points at the entitlements
  file via `bundle.macOS.entitlements`, and Tauri 2 auto-merges
  the Info.plist keys at bundle time (no `release.yml` hack
  needed — unlike Android, the macOS schema has a first-class
  config surface for both). Tauri's ad-hoc `codesign` step picks
  up the entitlements during `cargo tauri build` so the signed
  `.app` inside the DMG actually carries them; the CI fallback
  `codesign` line was updated to pass `--entitlements` too so a
  re-sign (if it ever fires) keeps parity. First-launch UX on
  macOS now matches every other native audio app: one consent
  prompt, then tuning works.

- **External links in the Tauri webview now actually open in the OS
  browser.** The interceptor in `frontend/web/app.html` was looking
  up `window.__TAURI__.shell?.open` — which doesn't exist in Tauri 2
  unless the `@tauri-apps/plugin-shell` JS package is bundled (we
  don't use a bundler). The lookup silently fell through to a
  warning; meanwhile `e.preventDefault()` fired but a race with the
  webview's default anchor handling let the navigation happen anyway,
  stranding the user inside the embedded site with no back button.
  Switched to the IPC invoke path (`invoke('plugin:shell|open', ...)`)
  that the rest of the codebase already uses successfully, moved
  the listener to capture phase, and added `stopPropagation` to
  close the race.

- **macOS app no longer trips Gatekeeper's "is damaged" path.**
  Without ANY signature, the `.app` was getting flagged as
  unreadable and even the right-click → Open workaround failed
  on macOS Sequoia+. Ad-hoc signing during the bundle step (via
  `signingIdentity = "-"` in `tauri.conf.json`) means the .app
  inside the DMG now carries a signature — Gatekeeper still warns
  the developer is unidentified, but the Privacy & Security →
  "Open Anyway" and `xattr -d com.apple.quarantine` workarounds
  both work. See the new MACOS-README.txt that ships in the
  artefacts under "Added".

- **Android APK now actually installs.**
  `cargo tauri android build --apk` defaults to release mode,
  which produces an UNSIGNED APK that Android refuses to install
  ("package appears to be invalid"). Added `--debug` so the build
  uses Android's auto-generated debug keystore — sideloadable for
  dev distribution. Real release-keystore signing is future work.

- **iOS build no longer fails on the target name.** Tauri's iOS
  CLI takes shorthand (`aarch64-sim` / `aarch64` / `x86_64`), not
  the full rustc triple (`aarch64-apple-ios-sim`). Previous runs
  errored out immediately with `invalid value for --target`.

- **iOS + Android jobs no longer fake-pass on empty output.**
  Removed the `|| true` after `cargo tauri ios build` (so a real
  compile failure surfaces as a failed step, not a green
  checkmark on an empty artifact). Switched the artifact-upload
  step to `if-no-files-found: error` so a missed `.app` / `.apk`
  fails loudly. iOS staging now finds `.app` *directories* under
  `gen/apple/build`, tars each one, and explicitly fails with an
  `::error::` annotation if neither a `.app` nor `.ipa` was
  produced.

- **macOS portable `.app.tar.gz` now exists.** Tauri's bundler
  deletes the intermediate `TWANGA.app` after wrapping it into
  the DMG when only `dmg` is requested as a bundle target
  (visible in logs as `Cleaning .../bundle/macos/TWANGA.app`).
  Asking for `app,dmg` instead keeps the `.app` around for the
  portable tarball.

- **Mobile artefacts now actually make the rolling release.**
  `release-rolling` previously only `needs: [build-platform]`, so
  it could fire before `build-android` finished uploading — the
  17 MB APK existed as a workflow artifact but never got into
  the release. Added `build-android` + `build-ios` to `needs:`
  with `if: always() && needs.build-platform.result == 'success'`
  so the aggregator waits for mobile to settle, and a failing iOS
  build still doesn't block the desktop + CLI release.

- **README homepage link** now points at
  `https://andrewiankidd.github.io/TWANGA/` (the marketing landing
  page, 200) rather than `/TWANGA/app/` (404 — there's no `app/`
  directory; the deployed site serves `app.html` directly).

- **Tauri Windows MSI bundling** now succeeds — added `.ico` (plus
  `.icns` and sized PNGs) to `tauri.conf.json`'s
  `bundle.icon` array. The previous single-`.png` entry made the
  Windows bundler error out with "Couldn't find a .ico icon".

- **Wait-mode no longer skips columns after a pause.** Web Playback's
  wait mode used to let the wall clock keep ticking while waiting for
  the user to play the expected note; the next `playbackTick` saw a
  large elapsed-time delta and jumped the playhead multiple columns
  forward (effectively "where the user should be by now"). Fixed by
  recording `waitStartedAt` on wait entry and rolling
  `now - waitStartedAt` into `totalPausedMs` when the mic detects the
  match — same shape as the explicit Pause/Resume bookkeeping. Pause
  pressed *during* wait closes out the current wait segment first to
  avoid double-counting; resume starts a fresh wait segment if still
  waiting. The CLI was unaffected — wait state lives in a different
  loop on that side. (Worth flagging that the playback engine is
  duplicated CLI ↔ web; a future "Rust playback engine bound to WASM"
  pass would let them share again.)

### Changed

- **Playback + Editor library rows now show the tuning / instrument
  subtitle** beneath each row's title. Same shape as the loaded-tab
  header (`<subtitle> · <bpm> BPM`, falling back to the matched
  registry tuning's display name, then the raw note names). Resolved
  async per row so the list paints instantly and the subtitle line
  fills in once parsing finishes; cached per-id (cleared on cross-tab
  save/delete) so re-renders don't reparse. Shared
  `attachRowSubtitle` used by both library views.

- **Loop control on Playback is a dropdown.** Replaced the freetext
  "off / full / 0:20" input with a dropdown of `off` / `full` /
  `range…`; selecting `range…` reveals two number inputs for the
  start + end columns. The underlying spec text the engine parses
  is unchanged ("off" / "full" / "START:END"), so the CLI flag
  syntax is still 1:1.

### Fixed

- **Bare `twanga` now prints the MOTD banner.** Running the CLI
  with no subcommand previously hit clap's "missing subcommand"
  error and exited with code 2 *before* `print_banner()` got a
  chance to fire. `command` is now an `Option<Command>`; the
  `None` case prints the banner first, then clap's standard
  long-help, then exits 0.

### Added

- **Tab editor — first cut.** New `#editor` screen lets you open
  any recording from the library and edit it directly on the
  same Tab renderer Playback uses, just with `interactive: true`:
  **left-click** a cell to bump the fret up (empty → 0 → 1 → 2 …
  unbounded), **right-click** to bump it back down (0 → empty),
  **double-click** to type a number directly. Click a column
  index to select it for the Insert / Delete / Clear column
  buttons. Title + BPM are editable inline; tuning + capo are
  preserved from the source file (transposing belongs in
  Playback, not here). User recordings save back in place via the
  new `library.update({id, title, alphatex})`; bundled examples
  are read-only and always route through "Save as new".
  Round-trips through the same `serialize_recording` path the
  Recorder uses on save, so the Editor's output is bit-for-bit
  indistinguishable from a fresh recording with the same notes.
  Required a small extension to the Tab renderer
  (`interactive` / `onCellClick` / `onCellContext` /
  `onCellDblClick` / `onColHeaderClick` / `selectedColumn`); the
  same Tab plugin shape now serves Recorder + Playback (read-only)
  AND Editor (interactive), with the plugin file unaware of who's
  consuming it.
- **Mic-level meter on Playback (wait mode)** — same diagnostic
  surface the Recorder already had: a small `RMS → dB` bar that
  appears while the mic is live, and a "no signal" hint after 2 s
  of no audio chunks (catches suspended `AudioContext` / OS-level
  mute). Extracted into a shared `controllers/mic-meter.js` factory
  (`makeMicMeter({...})`) so the Recorder + Playback share one
  implementation — same pattern as `makeTuningController`.
- **`twanga tunings remove`** — closes the last reverse-parity gap
  with the GUI (which has had a per-row Delete button on user
  tunings since the Tunings screen shipped). Usage:
  - `twanga tunings remove` → interactive menu of user tunings,
    confirmation prompt before delete.
  - `twanga tunings remove --slug open-d-guitar` → skips the
    menu but still asks for confirmation.
  - `twanga tunings remove --slug open-d-guitar --force` →
    skips both prompts (scripts).

  Built-in tunings are compiled into the binary and rejected
  upfront — same posture as the GUI, which hides Delete on
  built-in rows. New `tunings::remove_user_tuning_at(path, slug)`
  helper backs the subcommand with 3 cargo tests covering the
  happy path, the built-in-rejection case, and the unknown-slug
  case.
- **Cross-tab library sync** via `BroadcastChannel`. When the
  user records in one browser tab and has another open on the
  Playback library, the second tab refreshes its list
  automatically (save / delete / markDownloaded all publish).
  Silent no-op on older browsers without `BroadcastChannel`
  support; the single-tab experience is unaffected.
  `library.subscribe(callback)` is the new public API.
- **GUI Playback at full CLI parity.** New `#playback` screen with
  a combined library list + per-tab playback view:
  - **Library** — bundled examples (shipped via
    `assets/examples/manifest.json` — same files the CLI reads
    from disk) merged with user recordings from IndexedDB
    (`twanga-tabs-v1`). Drop-zone import accepts local
    `.alphatex` files. Per-row Load / Download / Delete (bundled
    examples skip the latter two). User rows show a
    `Backed up <when>` / `Never backed up` pill so the user can
    see at a glance which recordings have been exported.
  - **Per-tab view** — header (title / subtitle / file tempo +
    tuning), tuning picker (third consumer of the shared
    `makeTuningController` factory — see below) with full capo
    support, BPM override (stepper + reset-to-file-tempo button),
    loop range input (`off` / `full` / `START:END` — same syntax
    as `twanga play --loop`), pre-roll count-in, metronome
    toggle, wait-mode toggle, renderer dropdown (Tab / Highway),
    Play / Pause / Stop transport with a Spacebar shortcut,
    "Skipped:" preamble listing notes that couldn't be placed on
    the transposed tuning. Tab renderer now smooth-scrolls
    horizontally as the playhead crosses out of the visible band.
- **Tab library module** (`frontend/web/lib/library.js`) backing
  the Recorder save flow + Playback library. IndexedDB store
  keyed by auto-increment integer; bundled examples merged in
  lazily from the manifest. Stub `library-tauri.js` defines the
  same shape for the future filesystem backend.
- **Browser-storage warning layer** on the Recorder + Playback
  screens (web only — Tauri's `is-tauri` body class hides the
  warning). Persistent amber banner explains that browser
  storage can be cleared; post-save toast offers a one-click
  Download for safekeeping; per-entry "Backed up: <when>" tag
  on library rows. `navigator.storage.persist()` requested
  opportunistically on first save.
- **WASM `WebParsedTab`** wrapping `twanga_tabs::alphatex::ParsedTab`
  with accessors (`title`, `subtitle_display`, `tempo`,
  `tuning_names`, `columns_count`, `column_at`, `capo_spec`) plus
  `transpose_to` / `transpose_to_dropped_notes` for the playback
  transpose flow. New `parse_alphatex(text)` free function.
  3 new cargo tests covering the parse surface.

### Changed

- **Playback auto-loads the file's tuning + capo on load.** Was:
  the controller stayed on its last selection so loading a uke
  tab while the controller was on standard-guitar silently
  transposed (the "Skipped:" preamble surfaced drops but gave
  no signal about the cause). Now: each `playbackLoad` matches
  the file's `\tuning` line against the merged registry by
  MIDI list (so reentrant / drone-suffixed registry names
  don't block matches) and, on hit, also applies the file's
  `\subtitle ; capo=<spec>` annotation. Falls back to the
  controller's restored state when no registry match exists.
  New `matchRegistrySlugForTuningNames(names)` and controller
  method `setCapo(spec)` in `controllers/tuning.js`.
- **Shared tuning + capo controller**
  (`frontend/web/controllers/tuning.js`) consumed by Tuner,
  Recorder, and Playback. Roughly ~400 lines of duplicated picker
  + capo + per-string state-machine code collapsed into one
  factory function. Vanilla ES module, no framework — preserves
  the project's "no React" stance. Closes the BACKLOG item
  explicitly tagged "once Playback adds a third consumer."
- **User-tunings storage** extracted to its own ES module
  (`frontend/web/lib/user-tunings.js`). Same
  `twanga-user-tunings-v1` localStorage key + same `PresetEntry`
  schema; now imported by the controller, the Tunings screen, and
  the (eventual) library metadata.
- **Recorder save flow** now persists to the library instead of
  going straight to a download. Post-save toast offers Download
  as a one-click action and records the export time via
  `library.markDownloaded(id)` so the Library list can surface a
  `Backed up <when>` tag.
- **Recorder localStorage shape:** tuning + capo state moved into
  the controller's separate `twanga-recorder-tuning-v1` key;
  recorder-only settings (BPM, resolution, block width,
  metronome, pre-roll) stay under `twanga-recorder-v1`. Users
  see a one-time tuning + capo reset on upgrade — recoverable in
  one click.

### Added (earlier in this batch)

- Pause / resume on `twanga record`, `twanga play`, and the browser
  Recorder. CLI: type `p` + Enter (or `pause`) to toggle; pause
  freezes the column-tick driver / playhead, resume continues at
  the same column. The header "Controls:" line on record + play
  now documents both `q` (stop) and `p` (pause). GUI: a new Pause
  button next to Start, plus a Spacebar shortcut while the
  Recorder screen is active (scoped so it doesn't hijack typing in
  text inputs). The wall-clock-based tick driver subtracts
  cumulative paused-time so resuming after a long pause doesn't
  fast-forward through missed beats. Wait-mode pause is the
  prerequisite for the eventual "undo last column" GUI affordance.
- Pre-roll / count-in on `twanga record` and `twanga play` (CLI),
  and on the browser Recorder (GUI). New `--pre-roll <N>` flag with
  the standard three-form pattern; default 4 ticks (one bar at
  4/4), range 0–16, 0 disables. Always audible — fires even when
  `--no-metronome` silences the main run, because the whole point
  is to be heard. Aborts cleanly on Ctrl-C / `q + Enter` (CLI) or a
  Stop click during the count (GUI), so the user isn't stuck
  through 16 beats if they change their mind. Setting persists in
  the GUI's `localStorage` under the existing recorder key. New
  shared `run_pre_roll` helper in `twanga-cli` is reused by both
  `record` and `play`; saved the duplication while the surfaces
  still grow.
- Metronome click on `twanga record`. `--no-metronome` disables it,
  matching the existing `play` flag exactly (default on). Click
  fires on every beat boundary derived from the current resolution
  (1/8 → every other column, 1/16 → every fourth). The recorder's
  startup header gains a `Metronome:` line for parity with
  `play`. Browser Recorder gets a live "Metronome" checkbox in the
  view-controls row — flipping it mid-recording takes effect on the
  next beat tick. Selection persists in `localStorage`. The browser
  click is a short 1000 Hz pulse with the same ~50 ms exponential
  decay the CLI's `metronome_click` produces, played through the
  live `AudioContext` already open for the mic.
- "Couldn't fit on fretboard" indicator on every existing flow.
  Detected pitches that no `(string, fret)` combination within
  `MAX_FRET=20` can reach used to be silently dropped on `twanga
  record` and on `twanga play --tuning <other>` transposes. Now:
  - `twanga record` shows an aggregate counter in the status line
    while recording: `M:SS | N cols | X dropped (out of fretboard
    range)`. Per-event logging would be too noisy.
  - `twanga play --tuning <other>` pre-scans the transposed tab and
    prints a "Skipped:" preamble before the cursor starts, listing
    up to 8 unique note names that couldn't be placed. User can
    bail (Ctrl-C / `q + Enter`) if it's worse than expected.
  - Browser Recorder mirrors the CLI's record behaviour: a `✗ N
    dropped` suffix on the live status hint while recording (with a
    tooltip explaining what "dropped" means) and on the post-stop
    summary. New `ParsedTab::transpose_to_with_report(target,
    max_fret) -> (ParsedTab, Vec<DroppedNote>)` in `twanga-tabs`
    backs the CLI play side; the original `transpose_to` is now a
    thin wrapper around it. 2 new cargo tests (round-trip empty
    + reports unreachable pitches).
- Live duration / progress display on every record + play surface.
  - `twanga record`: a status line below the scrolling tab block
    showing `M:SS | N cols` (elapsed wall-clock + total committed
    columns). Refreshes every column tick.
  - `twanga play`: the existing `col N/M (bar X, beat Y)` progress
    line now also shows `M:SS / M:SS` (elapsed in the current loop
    iteration / total length of the loop range). For non-loop
    playback this reads as full-tab elapsed / total; for section
    loops it resets at the top of each iteration.
  - GUI Recorder: hint line below the renderer updates each tick to
    `recording — M:SS / N cols (BPM, resolution)`. Stop message
    also carries the final elapsed time.
  - GUI Playback gets parity automatically when that screen lands.
- Recording titles on both `twanga record` and the browser Recorder.
  CLI: new `--title` flag with the three-form pattern (`--title value`
  / `--title` bare → prompt / omitted → prompt). Filename derives from
  the title (`<slug>-<unix-secs>.alphatex`) when one is provided;
  blank input keeps the pre-feature `recording-<unix-secs>.alphatex`
  shape. GUI Recorder: Save now opens a title prompt before download,
  with the same blank-is-no-title fallback. The title flows into the
  alphaTex `\title` header via the canonical `AlphaTexWriter`, so
  files round-trip through `twanga play` and surface in its header
  (new "Title:" line) as well as in the parsed `ParsedTab.title`
  field for any future tooling.

### Changed

- Tunings list rows now stack the display name on top and the slug on
  a second line underneath, rather than running both inline. The
  per-row "BUILTIN" / "USER" origin badge has been removed — built-in
  vs user is signalled implicitly by the presence/absence of a Delete
  button on user-origin rows.

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

- Browser Recorder at full CLI parity. New screen with the same controls
  `twanga record` exposes — tuning picker (built-in + user), uniform +
  per-string capo, BPM (20-400), resolution (1/4..1/32), block width
  (4-200) — all persisted to `localStorage`. Live mic capture pipes
  through a chromatic `WebTuner`, runs `match_pitch_to_fret` against
  the active tuning + capo for every detection (same algorithm
  `twanga record` uses), accumulates hits into time columns at the
  configured tempo, and feeds the score into the active renderer
  (Tab or Highway). Stop and Save → a `.alphatex` file serialised
  by the **same** `AlphaTexWriter` the CLI uses (new
  `serialize_recording` WASM binding), so browser-saved recordings
  round-trip through `twanga play --capo`. Files download with an
  ISO-stamped `twanga-recording-<timestamp>.alphatex` name.
- Pluggable renderer system in [frontend/web/render/](frontend/web/render/).
  A `RendererRegistry` holds plugin objects (`{ id, name, version, create }`)
  that return instances implementing `setScore` / `setPlayhead` / `destroy`.
  Built-in `twanga.tab` (column-grid, CLI-style) and `twanga.highway`
  (falling-notes view) plugins register through the *same*
  path future third-party renderers will use — no special-cased "core" lane.
  Recorder screen now hosts a "View" dropdown that swaps renderers live,
  with the selection persisting in `localStorage`. The renderer fully owns
  its visual layout (canvas / DOM / SVG, sizing, colours); the host hands
  over a container element + score data and steps out of the way.
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
