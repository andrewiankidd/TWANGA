# Changelog

All notable changes to TWANGA are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows
[SemVer](https://semver.org). Pre-release everything currently lives under
`[Unreleased]` — the first tag (`v0.1.0`) will roll this list forward into a
dated section.

## [Unreleased]

### Added

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
  CLI mirror of the GUI's Patterns screen + Playback library list.
  See squashed commit message.

- **Six new bundled patterns** (uke baseline + waltz, banjo reverse +
  alternating-thumb rolls, guitar boom-chick + Travis). See the
  squashed commit message.

- **Tests for the docs system.** 7 Rust tests + 25 Node tests
  covering the markdown renderer + docs slug sync point. See
  squashed commit message.

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
  (Rocksmith-style notes-toward-you) plugins register through the *same*
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
