# TWANGA Backlog

Ideas and feature directions worth not forgetting. Not commitments — just things we might do someday. Loosely grouped by theme. Priority and feasibility vary wildly; some are afternoon projects, some are six-month subsystems.

See [ROADMAP.md](ROADMAP.md) for the bigger future milestones that have crossed the line into "we're going to do this." Shipped work lives in [CHANGELOG](../CHANGELOG.md).

## Renderer plugin system — future delivery mechanisms

The plugin contract is stable (`{ id, name, version, create(container, options) }` registered via the module-singleton `RendererRegistry`; same path built-ins use, no fast-lane). See [`frontend/web/render/`](../frontend/web/render/). What's *missing* is delivery beyond "compiled in":

- **Tauri desktop** — filesystem load from `$CONFIG/twanga/renderers/*.js`. User drops files in, registry picks them up on startup.
- **"Load from URL"** in the web build, with explicit user consent + content-hash check.
- **Decentralised plugin directory** — a list of plugin-manifest URLs the user adds (like git remotes or APT sources). Anyone can host a registry; we ship a default one. Aligns with the local-first / no-walled-garden ethos. Steam Workshop is explicitly *not* the model.

## Pattern trainer — content + UX

The Patterns screen ships with ten bundled drills across four groups (clawhammer banjo, bluegrass picking, ukulele strums, guitar). Each pattern is a small `.alphatex` file in `assets/patterns/` plus a manifest entry — no code changes needed to add more.

### Content expansion (further out)

- **More clawhammer variants.** Frailing strum (single down-brush + drone, easier than bum-diddy — Level 0 entry to the group). Hammer-on / pull-off variants of bum-diddy. Galax lick. Drop-thumb on string 4 every iteration as a Level 3 capstone.
- **More banjo rolls.** Forward-reverse roll (Level 2). Foggy Mountain roll (Level 3 — Earl Scruggs signature). Square roll.
- **More uke strums.** Calypso / reggae offbeat (accent on the offbeats — Level 2). Fingerstyle "Lava" arpeggio (Level 2 — opens up arpeggio practice without leaving the strums group).
- **Guitar group, deeper.** Standard `D-D-U-D-U` strum (Level 0 alongside boom-chick). 6/8 ballad strum.
- **Drop-D guitar group.** Drone-and-melody on the low D — highlights what drop-D actually buys you over standard. New group, tuning: `drop-d-guitar`.
- **Tenor banjo / tenor uke.** Lower-priority — the audience is smaller. Hold until requested.
- **Mandolin chop.** Defer until a `standard-mandolin` tuning ships (not in the built-in registry yet).

### UX

- **Difficulty progression within a group.** Right now the bum-diddy basic → drop-thumb tree is two entries. Add a Level 3 "advanced" once the content expansion lands.
- **Pattern descriptions in the row.** The manifest already supports `description` per group; per-pattern descriptions would let us explain "where this fits" without the user having to know the tradition.
- **Tempo presets per pattern.** Default tempos right now are baked into the alphaTex `\tempo` line. Surfacing 3-4 tempo presets per pattern ("slow / target / fast") in the GUI would lower the activation energy of practice without forcing the user to type a BPM.

## Tab editor — what's next

The editor screen is shipped (cell-level fret editing on the same Tab renderer Playback uses, column insert / delete / clear, save back in place or as a new copy). Future polish:

- **Per-bar annotations.** "Watch the slide here," "this is the hard part" — see the [Annotated tabs] item under "Learning aids" below; the editor is its natural home.
- **Hook into Recorder mid-take.** While the Recorder is paused, jump into the editor for the captured-so-far score, make corrections, return to the Recorder to keep going. Adjacent to (and possibly subsumes) the per-cell undo that already shipped — same data model on both sides.
- **CLI equivalent: probably never.** A terminal tab editor would re-implement what TuxGuitar already does well. The CLI's role stays "capture + play," the editor's role is "fix up post-capture." One of the few intentional asymmetries between the surfaces.

## Tauri desktop polish

The Tauri shell hosts the same `frontend/web/` bundle in a native window. Filesystem-backed library + tunings sync + plugin-shell wiring are all shipped (see [CHANGELOG](../CHANGELOG.md)). Remaining polish:

- **Native CPAL backend exposed as a Tauri command.** Web Audio's AudioWorklet runs at 50-150ms latency depending on backend; CPAL gives sub-20ms on desktop. Expose `start_capture` / `stop_capture` Tauri commands that stream samples from `twanga-audio` to the JS frontend over Tauri events. Only worth doing once the latency actually matters in practice (likely once chord/polyphonic verification lands).
- **Hand-crafted icon set.** `cargo tauri icon` auto-converted the workspace logo. Production builds want a hand-tuned `icon.icns` / `icon.ico` with proper rounded-rect / dark-mode variants per platform's conventions.
- **Filesystem-watch-driven cross-process sync.** Today the GUI bootstraps user tunings from `tunings.toml` on startup and write-throughs on every save, but doesn't notice if the CLI mutates the file while the GUI is running. A `notify`-crate watcher + a Tauri event would close that gap. Same idea for the recordings dir (Playback library refresh on external file add). Not high-priority — desktop users tend to be in one app at a time and a manual refresh works.
- **Browser-storage warning banner under Tauri.** Already CSS-hidden via the `body.is-tauri` class (because the filesystem doesn't evict). Verify it stays hidden as the GUI evolves.

## Web feature parity

The web build isn't a "tuner demo only" tier — full feature parity is the goal, accepting that browser latency and audio quality are inferior to native. **Limited and inferior is acceptable; absent is not.**

- **All features available in the browser.** Tuner, Recorder, Playback, Library, transposing — everything ships to web. Wait-mode tolerance is wider; that's the only honest compromise.
- **Calibration-driven tolerance** rather than device-class heuristics. See the Latency calibration wizard entry below — same approach works on every surface, just with looser default tolerances on web until calibration runs.
- **Quick mode vs precise mode labelling.** App detects which class via calibration, surfaces it in the UI honestly (see the Latency calibration wizard entry for the per-mode tolerance bands). Honest > pretending the experience is identical across hardware.

## Setup diagnostic mode

Walks users through verifying their instrument is actually playable before practice. Most beginners fail because the instrument is undermining them and they can't tell. Author profile: two years of struggle on a banjo with the bridge an inch out of position before noticing. This is the kind of thing TWANGA is uniquely placed to help with — a learning tool that includes the parts of "learning" other tools assume you already know.

- **Bridge position check.** Compare 12th-fret fretted note to the open string an octave up. Report direction + magnitude of any error in plain language: "Bridge needs to move 3mm toward the tail" (with "12th fret +18 cents on D" alongside as the technical version — see Cross-cutting principles → Two-tier output below).
- **Bridge intonation per string.** Loop the check across all strings. If different strings want the bridge in different positions, recommend a bridge angle.
- **Head tension check (banjo).** User taps the head with a hard object (pencil, plectrum); app analyses the tap response. Detect whether there's a sustained periodic component (resonant — healthy tension) or just a damped transient (loose — needs tightening). Optionally identify the resonant pitch.
- **Verify-the-measurement feedback loop.** If the input is a damped transient with no sustain, prompt the user to retry with a harder object — "we heard your tap, but it died too fast for us to read. Try tapping with a pencil so the head can ring freely." Generalises to any diagnostic where measurement technique matters.
- **Dead-string detection.** Compare decay envelopes across strings. Strings that decay much faster than peers are probably worn or damaged.
- **String age estimation.** Spectral brightness drops as strings age. Log over time, prompt replacement when it crosses a threshold.
- **Bridge geometry vs orientation.** Some bridges are deliberately wedge-shaped (vertical face toward tailpiece, slope toward the neck). Users sometimes install them backwards. A visual check or comparison test could catch this.
- **Action assessment.** Indirectly estimate string-to-fret gap from signal characteristics (attack sharpness, sustain length, harmonic content). Approximate but useful as a flag.
- **Style-aware setup guidance.** Different styles want different setups (bright bluegrass vs mellow clawhammer; resonator on vs off vs stuffed). App asks the user's intended style and recommends setup tweaks accordingly — the kind of advice that takes a beginner years to discover unaided.

## Twanga as a setup-teaching resource (onboarding flow)

The first-run experience teaches setup *while* diagnosing it. Each step in the setup diagnostic flow doubles as a vocabulary lesson — users learn what a 12th-fret harmonic is when the app needs to use one, not in a separate "Banjo 101" tutorial they'd skip. Reinforces the project's identity: TWANGA is a learning tool, not a tab player.

- **Just-in-time learning.** Each diagnostic step explains what's being checked and why before measuring. Beginners build vocabulary through repeated exposure to the technical terms in context.
- **Optional deeper dives.** "Why does bridge position matter?" → expandable explanation with diagrams. Doesn't gate progress; available for the curious.
- **No upfront tutorial wall.** Skip the "intro to banjo" course nobody finishes; teach in the moment when the user has a reason to learn.

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
- **Silent practice mode (muted / unplugged).** Show what to do; user fingers it on a muted/unplugged instrument. No verification. Builds muscle memory without sound.
- **Silent practice mode (acoustic-quiet, audio-loud) via pickup + headphones.** Distinct from the muted variant above. User plays acoustic instrument lightly or with a mute; pickup feeds TWANGA; app outputs synthesised or EQ'd version of their playing through headphones at apparent full volume. Full tonal feedback, near-zero acoustic noise — the "practice at night without waking anyone" mode that doesn't require buying an electric instrument and doesn't suffer mechanical-mute tonal compromises. Dependencies: the synth path from the tab-audio-generation roadmap milestone covers most of it; the new bit is routing live pickup input through synthesis/EQ to a low-latency output bus.
- **"Practice without practising" mode.** Small unobtrusive overlay (chord name + timer). Changes chord every 30-60s. No verification. Supports the two-minutes-while-the-kettle-boils style of practice that's how most adults actually improve.

## Mode toggling (UX-level concept)

Explicit user-selectable practice intent that changes what the app verifies and emphasises:

- **Learning the song** — accuracy + rhythm matter
- **Physical practice** — clean fretting matters, notes don't
- **Just playing** — nothing verified; backing track + metronome only
- **Performance** — full song run-through, scored at the end, no mid-song interruptions

## Audience-specific (folk/amateur, non-guitar)

- **Strum/pick rhythm-only mode.** Detect *that* you're playing on the beat, not *what*. Lets uke beginners feel like they're playing along before precise verification is possible.
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

### Articulation detection

Builds on the continuous-pitch visualisation above. Extracts and verifies specific articulation events (vibrato, slide, bend, hammer-on, pull-off) rather than just tracking the contour line.

- **Articulation data model.** Per-note metadata for vibrato (rate + depth), slides (start/end pitch + duration), bends (target interval + duration), hammer-ons, pull-offs. Generic annotation layer attached to notes rather than baked into the note model — keeps `TunedString` / `MidiNote` etc. clean.
- **Vibrato detection.** FFT of pitch-contour-over-time, look for a 4–7 Hz peak with appropriate depth. Cheap to compute with the existing `rustfft` dependency. Big payoff — nobody else does this for amateur learners.
- **Vibrato trainer.** Reference contour + user's contour overlaid. User tries to match rate and depth. Probably the most visually compelling feature once continuous-pitch visualisation lands.
- **Banjo / uke constraint.** Short sustain limits which articulations are detectable. Vibrato detection only works on sustained notes (end-of-phrase holds), not rapid passages. Real constraint, not a bug — document it.

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
- **Latency calibration wizard.** First-run flow measures end-to-end output-to-input latency (cable + buffer + reaction) and offsets `wait` mode's pitch-comparison timing. Cheap USB cables + Bluetooth headphones can push round-trip to 100ms+; without calibration the cursor sits waiting because detection arrives late. **Implementation note:** acoustic loopback (click out speakers → captured by mic) is the easy version but dies the moment the user puts headphones on, which is most of them. The fallback that works regardless is tap-along calibration: play 8 clicks, ask the user to tap their instrument on each beat, take the median offset between expected-beat and detected-impulse. Captures output + reaction + input as one number, which is what `wait` mode actually needs. **Same approach replaces any device-class heuristic on web AND desktop** — browsers can't reliably identify input devices (privacy restrictions), so measuring latency + jitter directly via the calibration is the only honest path. Surface the result in the UI alongside its tolerance band: "🎤 built-in mic — casual mode, ±50ms tolerance" vs "🎸 audio interface — precise mode, ±15ms tolerance". Same code path either surface, just different defaults if calibration hasn't run yet.
- **Smart tuner input mode.** Detect noisy mic vs clean direct-in, adjust filter strategy.

## Tab ingestion (the import pipeline)

Multiple sources funnel into one internal arrangement format with per-note confidence scores. Proprietary formats (Guitar Pro `.gp5`/`.gpx`) are explicit non-goals — see [SCOPE.md](SCOPE.md). Phases 1–3 have shipped: alphaTex (Phase 1, in `twanga-tabs::alphatex`); MusicXML / MXL (Phase 2, in `twanga-tabs::musicxml`); MIDI, ABC notation, and ASCII tab round out the open-format coverage at Phase 3 (`twanga-tabs::midi`, `::abc`, `::ascii_tab`). All are exposed by both `twanga import` on the CLI and the Importer screen in the GUI. The rest:

- **Phase 4:** OCR for image tabs — Tesseract via Rust bindings, feed images of ASCII tabs or printed tablature.
- **Phase 5 (stretch):** Audiveris-backed staff-to-tab — sheet music PDFs → MusicXML → fingering algorithm → tab on target instrument.
- **Phase 6 (probably never):** audio-to-tab — Klang.io-style polyphonic transcription. Open-source quality not there yet.
- **Bulk import workflow.** Drop a folder, app processes everything in background, library shows status badges (green/yellow/red) by confidence. Play the high-confidence stuff immediately; review the rest as you reach it.
- **Source badges in library.** Icon per arrangement showing where it came from (alphaTex, MusicXML, ASCII, OCR, audio).
- **Confidence rendering.** Low-confidence notes shown with subtle visual cue (dotted underline, paler). Naturally caught while playing.
- **Diff and merge.** Multiple imports of the same song → offer to merge, keep user edits across versions.
- **Importer tuning picker.** When a source has no declared tuning (every MIDI, every ABC, ASCII tabs with non-standard labels) the parser surfaces an `InferredTuning` warning naming the fallback choice. Today that's informational only — the user commits the import and re-tunes at playback. Surface a tuning picker on the preview card before commit so the guess is one click to correct; pre-select the parser's match.
- **Fuzz the heuristic parsers.** `cargo fuzz` against `ascii_tab::parse` and `abc::parse` — both are content-shape heuristics on untrusted text. Cheap to set up, almost guaranteed to find panics in the first run. Backlog'd because the import is already gated by user action (drop a file you trust) and a panic surfaces as a clean error toast — but worth doing before any code path that auto-imports unattended.
- **Real-world MIDI / ABC / ASCII tab fixture corpus.** The Phase 3 parsers (`twanga-tabs::midi`, `::abc`, `::ascii_tab`) are tested against fixtures built by their own writers (or hand-authored). External fixtures from independent third-party sources — same posture as the lilypond MusicXML regression suite under `crates/twanga-tabs/tests/fixtures/external/` — would catch real-world interop bugs the self-written fixtures miss. PD sources: Sessions ABC archive for ABC; MuseScore's MIDI-export corpus for MIDI; Ultimate Guitar's CC-licensed tab sets for ASCII.
- **Articulation data model.** TWANGA's `TabColumn` carries an `articulation: Option<u8>` field today, populated by the ASCII tab parser and round-tripped through alphaTex (h/p/s only in v1). The article (b'h' hammer-on, b'p' pull-off, b's' slide) is preserved as data but the playback / renderer don't consume it yet. Wiring it through requires extending the renderer (visual cue), playback (different envelope on hammered/pulled notes), and the wait-mode detector (recognise a hammer-on by attack profile rather than fresh pluck). Distinct from the broader articulation entry above — that one introduces a richer per-note metadata layer; this is the minimum work to USE the data we already preserve.

## Self-recorded sample bank ("your own soundfont")

Tuner-driven passive sample collection. The chore (tuning) becomes the data source. **The "Playback engine uses bank when available" item below is the bank-aware half of the [Tab audio generation](ROADMAP.md#follows-next-big-rocks) roadmap milestone**; the synth-fallback half is unlocked first by the same milestone. The rest of this section is bank-management UX that compounds value once the playback hook exists.

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
- **MusicXML export.** Interop with MuseScore, Guitar Pro, etc. Bidirectional via internal TwangaTab model. (MusicXML *import* is on the [ROADMAP](ROADMAP.md); export would land alongside.)

## Mic-input parity gaps

Minor asymmetries between the CLI and GUI around live mic configuration. The static-flag versions exist on both sides; the live versions only on one.

- **Runtime device-switch on CLI.** GUI has a device picker on Tuner / Recorder / Playback wait-mode that hot-swaps the active input. CLI has `--device "<name>"` at startup; switching mid-session would need to drop and re-open the `InputStream`. Doable but low-value — CLI users typically know their device before they start.

## Architecture / infrastructure

- **CI mobile targets.** Linux/Windows/macOS already ship via the CI + release workflows. GitHub Pages deploy for the web build also ships (`pages.yml`). Mobile (iOS / Android via Tauri Mobile) is the next platform tier to add — see [ROADMAP](ROADMAP.md).
- **ASIO-enabled Windows build variant.** Two Windows binaries — without ASIO (works for everyone) and with (lower latency, needs ASIO driver). Deferred on the redistribution-license question (Steinberg SDK).
- **`twanga-trace` crate.** Continuous pitch contour comparison (DTW + pitch distance). For the Audiosurf-mode work.
- **`twanga-import` crate?** Once OCR / Demucs / transcription pipelines arrive, isolating them in a single crate keeps the optional heavy dependencies out of the core build.
- **Shared playback engine in Rust (bound to WASM).** Wait-mode + tick-loop bookkeeping is currently duplicated CLI ↔ web. The CLI has its own loop in Rust; the web has its own in JS. Drift has already bitten once (wait-mode column-skip was a web-only bug because the CLI uses a different state machine). Worth lifting into `twanga-tabs` or a new `twanga-playback` crate once we have a second drift incident.
- **Single internal arrangement format (TwangaTab).** Near-superset of MusicXML with instrument-agnostic extensions (tuning per string, fingering hints, folk-specific technique tags). All importers funnel into this; all renderers/players consume it.
- **Three serialisation formats:** TwangaTab (internal) ↔ alphaTex (human-friendly paste/edit) ↔ MusicXML (interop). One model, three faces.

## Cross-cutting principles

App-wide design rules that surface across multiple features. Not features themselves — design constraints that should shape any feature that interacts with the user. Strong candidates to move to a dedicated `docs/DESIGN.md` later; parked here for now to keep everything in one place.

### Two-tier diagnostic output

Plain language is the primary surface; technical detail is the secondary surface. Both always present.

- **Plain language as the primary surface** — "Move your bridge 3mm toward the tailpiece," not "intonation +18 cents." Beginners can act on it without prior knowledge.
- **Technical detail as the secondary surface** — subtitle, hover text, debug line. Smaller font, lower contrast, maybe monospace. Visually distinct but **present by default**, not hidden behind a toggle.
- **Both surfaces always present.** Never plain-language-only (patronising, opaque about *why* the suggestion exists); never technical-only (impenetrable to beginners).
- **Applies app-wide:**
  - Tuner: "C# — tune down slightly" + "264.7 Hz, +12 cents"
  - Playback feedback: "Played slightly late" + "+45ms vs expected onset"
  - Setup check: "Bridge needs to move 3mm toward the tail" + "12th fret +18 cents (D string)"
  - Session summary: "Solid 15 minutes" + "Avg accuracy 78%, 23 retries on bar 4, tempo 92 BPM"
- **Why it matters:** teaches vocabulary over time through repeated exposure, lets users catch algorithm errors, supports cross-tool workflows (YouTube tutorials use technical terms), future-proofs the app as diagnostic algorithms evolve, signals honesty.

### Qualitative diagnostics where measurement is unreliable

Sometimes pattern-matching by ear beats tuner-chasing — especially on transient sounds. Worth capturing as a design rule because the natural instinct is to instrument everything with numbers.

- **Use sound, not numbers, when transients confuse pitch detection.** Drum-head taps, percussive onsets, and other brief signals fool pitch trackers (octave jumps, fundamental detection on harmonics, ambient noise pickup). Better to provide reference audio examples (loose vs proper tension, dead vs fresh string) and ask the user to A/B against their own.
- **The app could ship reference recordings** of common setup states. User taps their head, listens, taps a reference clip, compares by ear. Lower-tech than a perfect signal-processing solution but more reliable for measurement-resistant signals.

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
