# TWANGA Backlog

Ideas and feature directions discussed during early scoping. Not commitments — just things worth not forgetting. Loosely grouped by theme. Priority and feasibility vary wildly; some are afternoon projects, some are six-month subsystems.

See [ROADMAP.md](ROADMAP.md) for what's actually committed next; this file is everything we've thought about that we might do someday.

## Web build feature parity with CLI

The web frontend (`frontend/web/`) is shipped via GitHub Pages and wrapped by the Tauri 2 desktop shell. Same HTML/JS/WASM bundle, two delivery paths. Today the web tuner is a chromatic Hz readout — these items close the gap with `twanga-cli`:

- **Per-string tuner display.** Tuning picker (built-in presets + future user tunings) → per-string targets with cents-deviation indicator. Mirrors `format_string_row` from the CLI. Same `Tuning::nearest_string` math lives in WASM via `twanga-web` already.
- **Capo picker on the tuner screen.** Integer slider for uniform; "advanced" mode collapses to per-string offsets for partial capos / banjo 5th-string. Backend data model already ships.
- **Tab playback** — alphaTab.js loads in the webview, scrolls a cursor through bundled / user-uploaded `.alphatex` files. BPM slider, wait-mode toggle, transpose dropdown.
- **Tab recorder in browser** — equivalent of `twanga record`: mic → fret detection → alphaTex string the user can download or copy to clipboard. The native CLI writes to disk; the web build will write to a Blob → save-as.
- **Tab library.** Drop-zone for local `.alphatex` files (no upload; processed entirely in the browser). Persist the list across sessions.
- **Custom tunings in the browser.** `twanga tunings add` equivalent, persisting to `localStorage` instead of `$CONFIG/twanga/tunings.toml`. Same `PresetEntry` schema either way.
- **Persist "last session" state** — last opened tab + position + selected tuning + capo. One-click resume.

## Tauri desktop polish

Quality-of-life work for the Tauri shell after the basic webview-bundle path is in. Mobile via Tauri Mobile is its own roadmap item ([ROADMAP.md](ROADMAP.md)).

- **Wire `@tauri-apps/plugin-shell`.** External-link clicks in app.html already get intercepted via the `window.__TAURI__` runtime check, but the resulting `shell.open(url)` call is currently a no-op until the plugin is added to `tauri.conf.json`'s allowlist + the JS-side glue is imported. Without this the personal-site logo + "TWANGA Homepage" link don't actually open in the system browser when running under Tauri.
- **Native CPAL backend exposed as a Tauri command.** Web Audio's AudioWorklet runs at 50-150ms latency depending on backend; CPAL gives sub-20ms on desktop. Expose `start_capture` / `stop_capture` Tauri commands that stream samples from `twanga-audio` to the JS frontend over Tauri events. Only worth doing once wait-mode lands and the latency actually matters.
- **Filesystem access for `~/.config/twanga/tunings.toml`.** Browser uses `localStorage`; Tauri can read/write the same TOML the CLI uses. Means custom tunings sync across the CLI + desktop app without a separate user file.
- **Hand-crafted icon set.** `cargo tauri icon` auto-converted the workspace logo. Production builds want a hand-tuned `icon.icns` / `icon.ico` with proper rounded-rect / dark-mode variants per platform's conventions.
- **`cargo tauri build` smoke run + installer asset upload.** The release workflow currently ships only `twanga-cli`. Once a known-good `cargo tauri build` exists, add it to the release matrix so each tag also publishes desktop installers (`.msi`, `.dmg`, `.deb`, `.AppImage`).

## Practice mechanics — compounding over time

- **Session journaling.** Auto-log every session: tab, duration, BPM, accuracy %, sections looped. Append-only JSON. Power-source for everything below.
- **Calendar heatmap.** GitHub-contribution-style visualisation of practice days. Strong motivator.
- **"Last week vs today" comparison.** Play your Monday recording then today's recording of the same section. Reveals progress that feels invisible day-to-day.
- **Plateau detection.** If a section's accuracy hasn't moved in N attempts, gently suggest a different approach (slow down, isolate hand, switch song).
- **Spaced repetition.** Track which specific bars/chords you flubbed; surface them in tomorrow's warmup automatically. SM-2-style algorithm applied to phrases.
- **Streak counter.** Daily playing streak. Low-key motivational, opt-in.
- **Practice timer with soft chime.** "Just 15 minutes" reduces the activation energy that's usually the hardest part of practice.

## Physical practice — calluses, strength, motor skills

App doesn't always need to know what you're playing. Sometimes it just needs to be there while you put in the time.

- **Chord-hold timer.** Fret a chord cleanly, hold for 30 seconds without buzzing. Resets on buzz. Pure endurance.
- **Chord-swap drill.** Switch between two chord shapes at increasing tempo. Pure motor skill, no melody.
- **Finger independence exercises.** Press only one finger, verify only that string rings cleanly. Classical-guitar-style pedagogy.
- **Pinky strengtheners.** Drills that explicitly target the weakest finger.
- **Stretch builders.** Progressive span widening between fingers over days/weeks.
- **Endurance mode.** Pick a progression, play it for 20 minutes slowly. No scoring, no fail state. Just keep the metronome going. Gentle chime every 5 minutes.
- **Callous tracking.** Daily check-in: fingertip soreness 1-5, optional "where does it hurt?" tap-on-finger UI. Reveals body-awareness patterns over weeks.
- **Silent practice mode.** Show what to do; user fingers it on a muted/unplugged instrument. No verification. Builds muscle memory without sound.
- **"Practice without practising" mode.** Small unobtrusive overlay (chord name + timer). Changes chord every 30-60s. No verification. Supports the two-minutes-while-the-kettle-boils style of practice that's how most adults actually improve.

## Mode toggling (UX-level concept)

Explicit user-selectable practice intent that changes what the app verifies and emphasises:

- **Learning the song** — accuracy + rhythm matter
- **Physical practice** — clean fretting matters, notes don't
- **Just playing** — nothing verified; backing track + metronome only
- **Performance** — full song run-through, scored at the end, no mid-song interruptions

## Audience-specific (folk/amateur, non-guitar)

- **Strum/pick rhythm-only mode.** Detect *that* you're playing on the beat, not *what*. Lets uke beginners feel like they're playing along before precise verification is possible.
- **Visual tab rendering relative to capo.** Backend is shipped (the `Capo` type + `--capo` flag + alphaTex subtitle round-trip); a future GUI tab renderer should display fret numbers relative to the capo position rather than absolute frets, including a visual indicator of the capo bar across the fretboard.
- **Strumming pattern trainer.** "D D-U U-D-U" visualisation + rhythm-only verification.
- **Roll trainer for banjo.** Forward roll, backward roll, alternating thumb. Right-hand finger visualisation. Banjo learners need this more than they need songs.
- **Clawhammer rhythm trainer.** Specifically the "bum-di-tee" forearm motion — entirely a right-hand motor skill.
- **Backing track generation.** Synth pad chords played in time as a I-IV-V backing. Turns the app into a basic jam partner without needing pre-existing tracks.

## Continuous-pitch practice ("Audiosurf mode")

Pitch isn't always discrete. For slide, vibrato, bends, fretless, and intonation work, the actual quantity to practice is a continuous pitch contour over time.

- **Continuous pitch visualisation.** Time on X, pitch on Y. Reference line + user's live line overlaid.
- **Trace the legend.** Load an isolated stem of a famous player's solo. Their pitch contour is the ghost line. User plays to match. Phrasing and timing absorbed in a way notation can't convey.
- **Vibrato matching.** Reference vibrato at specific depth and rate. User's vibrato overlaid. Bring them into sync.
- **Bend accuracy.** Target line bends up by exactly a whole step over defined duration. User's bend traced live. Reveals chronic under-bending.
- **Slide accuracy with tolerance bands.** Fat reference line shows the acceptable zone. Narrows as user improves (Master Mode for continuous pitch).
- **Drone pitch lock.** Sustained reference note. Hold the same note steadily. Reveals how unsteady "steady" actually is.
- **Audiosurf-style "play along with any audio" mode.** Drop in MP3 → Demucs isolation → pitch contour → user plays along to match the line. Way easier than transcribe-and-verify. Forgiving by default.
- **Implementation note:** new crate (`twanga-trace`?) for contour comparison logic. DTW for time alignment + point-wise pitch distance for scoring.

## Learning aids for tab-illiterate / bad-memory users

(Author profile, but generally useful.)

- **Tab fade-as-mastered (Master Mode).** Once accuracy on a section passes threshold, tab notation fades. After more reps, fades further. Eventually playing without visual cues. Already have the data; need the UI.
- **Audio reference per note/chord.** Hover/tap a chord in a tab to hear it. Tap-to-hear is critical when you can't read tabs.
- **Hand-position diagrams.** Tap a chord, see fretboard mini-diagram with finger dots. Standard but often missing.
- **Reverse mode: "what chord am I playing?"** Hold a shape, app tells you the chord name. Useful when transposing or remembering shapes you forgot the name of.
- **Annotated tabs.** Personal notes per bar — "watch the slide here," "this is the hard part." Notes persist with the tab.
- **Fingering hints.** For a fret position, suggest a finger (1/2/3/4). Defaults override-able per user.

## Audio pipeline / AI-assisted prep

Runtime stays deterministic. AI is import-time only, like OCR.

- **Demucs source separation.** Isolate stems from MP3 for play-along. Run as ONNX inference one-time at import. Cache stems to disk.
- **"Mute this part, play it yourself" mode.** Inverse of solo-the-banjo — silence the banjo in the mix, user plays along as the missing part.
- **Monophonic transcription on isolated stems.** Isolated banjo line → pitch contour → tab notation. Output flagged as low-confidence, user-editable.
- **Polyphonic transcription via Basic Pitch.** Bolt-on for chord-heavy passages. Mark output as draft. Starting point for tab editor, not end product.
- **Latency calibration wizard.** First-run flow measures end-to-end output-to-input latency (cable + buffer + reaction) and offsets `wait` mode's pitch-comparison timing. Cheap USB cables + Bluetooth headphones can push round-trip to 100ms+; without calibration the cursor sits waiting because detection arrives late. **Implementation note:** acoustic loopback (click out speakers → captured by mic) is the easy version but dies the moment the user puts headphones on, which is most of them. The fallback that works regardless is tap-along calibration: play 8 clicks, ask the user to tap their instrument on each beat, take the median offset between expected-beat and detected-impulse. Captures output + reaction + input as one number, which is what `wait` mode actually needs. Deferred to post-GUI — a first-run setup wizard has a nicer home in the GUI shell.
- **Smart tuner input mode.** Detect noisy mic vs clean direct-in, adjust filter strategy.

## Tab ingestion (the import pipeline)

Multiple sources funnel into one internal arrangement format with per-note confidence scores. Proprietary formats (Guitar Pro `.gp5`/`.gpx`) are explicit non-goals — see [SCOPE.md](SCOPE.md).

- **Phase 1 (shipped):** alphaTex parser + writer (`twanga-tabs::alphatex`).
- **Phase 2:** MusicXML import — open W3C schema; Guitar Pro / MuseScore / Sibelius all export to it, so this is the realistic interop path for the existing Guitar Pro library most users have.
- **Phase 3:** ASCII tab parser — paste from Ultimate Guitar etc., or text file. Lossier than MusicXML (have to guess timing) but covers the "I have a text file" workflow.
- **Phase 4:** OCR for image tabs — Tesseract via Rust bindings, feed images of ASCII tabs or printed tablature.
- **Phase 5 (stretch):** Audiveris-backed staff-to-tab — sheet music PDFs → MusicXML → fingering algorithm → tab on target instrument.
- **Phase 6 (probably never):** audio-to-tab — Klang.io-style polyphonic transcription. Open-source quality not there yet.
- **Bulk import workflow.** Drop a folder, app processes everything in background, library shows status badges (green/yellow/red) by confidence. Play the high-confidence stuff immediately; review the rest as you reach it.
- **Source badges in library.** Icon per arrangement showing where it came from (alphaTex, MusicXML, ASCII, OCR, audio).
- **Confidence rendering.** Low-confidence notes shown with subtle visual cue (dotted underline, paler). Naturally caught while playing.
- **Diff and merge.** Multiple imports of the same song → offer to merge, keep user edits across versions.

## Self-recorded sample bank ("your own soundfont")

Tuner-driven passive sample collection. The chore (tuning) becomes the data source.

- **Capture when tuner is happy.** Pitch stability >0.95, <5 cents off, >300ms stable → grab the last 2s of audio, tag with pitch/string/timestamp, write to bank.
- **Hidden capture mode behind debug flag (do this first!).** Just dump WAVs to disk while you tune. Builds a corpus before the UI exists. Future-you will thank present-you.
- **Bank UI.** Manage samples — playback, retag, delete, label by articulation (clean pluck, muted, hammered, slid into).
- **Round-robin playback.** Multiple captures of the "same" note rotated during playback. Defeats machine-gun effect, sounds alive.
- **Playback engine uses bank when available.** Tab calls for G3 on string 3 → look up matching sample → fallback chain (exact match → same pitch other string → pitch-shifted nearest → synth placeholder + "want to capture this?" prompt).
- **Self-comparison over time.** Visualise decay envelope / brightness / pitch stability of the same note across weeks/months. Reveals string aging, technique improvement.
- **Comparison with reference instruments.** Overlay your G3 with a reference G3 from a different instrument or recording.
- **Bank sharing.** Export bank for others to use. Cottage industry of community-recorded folk-instrument soundfonts.
- **Per-tuning banks.** DADGAD captures stored separately. User builds a bank per tuning they use.

## Social / collaboration (light-touch only)

- **Practice room.** Two users, different locations, same tab + tempo, app metronome syncs over WebRTC. Each hears the other roughly in time.
- **Recording yourself for review.** Capture a section, save locally, optionally share. Supports the "weekly recording to my teacher" workflow.
- **Splash text PRs.** Community-contributed `splashes.txt` entries. Low-stakes contribution funnel. CONTRIBUTING.md sets rules (spells TWANGA, no slurs, no punching down, under 80 chars).

## Honest feedback / scoring

- **Granular accuracy breakdown.** Not "85%" — "85% right note on time; 8% slightly late, 4% wrong note, 3% missed entirely." Per-section, exportable.
- **Replay mistakes.** End of section, offer to play back *just* the moments user got wrong. Hearing the mistake teaches more than the number.
- **No XP / achievements / badges.** Cargo-culted from games. Real progress is the heatmap + audio comparison. Resist.

## Hardware / connectivity

- **MIDI keyboard input.** Treat as another input device. Pianists and synth players welcome.
- **Foot pedal support.** Bluetooth page-turner pedals. Page turn, pause, restart loop. Stupid simple (arrow keys), transformative for practice flow.
- **Multiple simultaneous inputs.** Two players one session, or one player with two instruments mic'd.
- **Specific guidance on USB audio interfaces.** Short README section demystifying the Realtone cable vs cheap clones vs proper interfaces. Honest, useful, links to nothing specific because the open-source-cable scene isn't mature enough to recommend.

## Utility / export

- **Print/export to PDF.** Clean printable tab. Banjo/uke demographic still uses paper music stands.
- **Folder-based sync.** Point app data at a Dropbox/iCloud/Syncthing folder for multi-machine sync. No accounts, no server.
- **MusicXML export.** Interop with MuseScore, Guitar Pro, etc. Bidirectional via internal TwangaTab model.

## Architecture / infrastructure

- **CI mobile targets.** Linux/Windows/macOS already ship via the CI + release workflows. GitHub Pages deploy for the web build also ships (`pages.yml`). Mobile (iOS / Android via Tauri Mobile) is the next platform tier to add.
- **ASIO-enabled Windows build variant.** Two Windows binaries — without ASIO (works for everyone) and with (lower latency, needs ASIO driver). Deferred on the redistribution-license question (Steinberg SDK).
- **`twanga-trace` crate.** Continuous pitch contour comparison (DTW + pitch distance). For the Audiosurf-mode work.
- **`twanga-import` crate?** Once OCR / Demucs / transcription pipelines arrive, isolating them in a single crate keeps the optional heavy dependencies out of the core build.
- **Single internal arrangement format (TwangaTab).** Near-superset of MusicXML with instrument-agnostic extensions (tuning per string, fingering hints, folk-specific technique tags). All importers funnel into this; all renderers/players consume it.
- **Three serialisation formats:** TwangaTab (internal) ↔ alphaTex (human-friendly paste/edit) ↔ MusicXML (interop). One model, three faces.

## Explicit non-goals (do not build)

Restating so they don't sneak back in (see also [SCOPE.md](SCOPE.md)):

- Hosted song library / store — bring-your-own-tabs is the whole differentiation
- Cloud accounts / sync server — local data + optional folder sync only
- XP / achievements / badges
- Public leaderboards / profiles / follows
- Subscription tier
- AI tutor chat (contradicts no-runtime-AI positioning)
- Bundled non-public-domain content
- Anti-cheat or DRM of any kind
- Closed-source forks for app-store distribution
- Proprietary tab formats (Guitar Pro `.gp5`/`.gpx`) — same legal posture as ASIO; users with Guitar Pro libraries get the MusicXML export path

## Distribution (future, not now)

- Consider a Steam build later (Aseprite model — same MIT/Apache source, paid Steam version with bundled content + autoupdates + Workshop integration).
- Steam Workshop for community tab sharing + sample bank sharing.
- Don't decide until v1 GUI ships and you've used it daily for three months.
