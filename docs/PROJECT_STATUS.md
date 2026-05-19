# Project status

**Functional end-to-end on the CLI** *and* on the web build (which is also the
Tauri 2 desktop shell — same `frontend/web/` bundle, two delivery paths).

## CLI

Three core flows, full tuning + capo support, recordings round-trip through
the alphaTex format:

- **Tuner** (`twanga tune`) — live multi-string display (per-target cents
  indicator) or chromatic mode (snap to nearest 12-TET note). Cable hum /
  silence are gated out.
- **Tab recorder** (`twanga record`) — capture what you play as alphaTex,
  saved to `recordings/<timestamp>.alphatex` with per-block fret detection.
- **Tab playback** (`twanga play`) — scroll a cursor through a tab at tempo,
  optional metronome click on each beat, optional "wait" practice mode that
  pauses until you play each note, `--loop` for full or section repeats,
  `--tuning <preset>` transposes the tab onto a different instrument.

## Web build + Tauri desktop shell

Three of the four CLI surfaces have GUI counterparts at 1:1 parity. The fourth
(Playback) is the next milestone.

- **Tuner** — built-in + user tuning picker, uniform capo stepper, per-string
  capo panel (for drop-D / banjo 5th-string / partial capos), live mic capture
  via Web Audio + AudioWorklet, YIN running in WASM. Settings persist to
  `localStorage` and round-trip cleanly when the user reloads. ✅ parity
- **Tunings** — merged built-in + user-defined list with inline "Define a new
  tuning" form (display name, per-string note names with live MIDI preview,
  auto-derived kebab-case slug, full validation via the same Rust rules
  `twanga tunings add` enforces). User tunings persist in `localStorage` under
  the same `PresetEntry` schema the CLI writes to `$CONFIG/twanga/tunings.toml`,
  ready for a future Tauri command to bidirectionally sync the two. ✅ parity
- **Recorder** — full `twanga record` parity: tuning picker, capo
  (uniform + per-string), BPM, resolution (1/4..1/32), block width, all
  persisted to `localStorage`. Live mic → chromatic `WebTuner` →
  `match_pitch_to_fret` against the active tuning + capo (same algorithm as the
  CLI's recorder, same `MAX_FRET=20` ceiling). Column-by-column commits at
  tempo, with the score fed to the active renderer plugin. Stop & Save →
  `.alphatex` written by the same Rust `AlphaTexWriter` the CLI uses, downloaded
  as `twanga-recording-<ISO-timestamp>.alphatex` — round-trips through
  `twanga play --capo`. ✅ parity
- **Playback** — not yet built. ⛔

## Renderer plugin system

The Recorder (and the future Playback screen) consume any registered renderer
through a uniform plugin contract. Two ship by default:

- **Tab** — column-grid view, one row per string, mirroring the CLI's record
  layout.
- **Highway** — Rocksmith-style notes-toward-you, one vertical lane per string.

Both built-ins register through the *same* `registry.register(plugin)` path
that future third-party plugins will use — no fast-lane for "core". The plugin
object is `{ id, name, version, create(container, options) }`; the renderer
instance implements `setScore` / `setPlayhead` / `destroy`. That's the entire
contract. The renderer fully owns its visual layout (canvas / DOM / SVG,
sizing, animation, colours); the host only hands over a container element and
the score data. Future delivery mechanisms (filesystem load on Tauri desktop,
"Load from URL", community plugin directory) all use the same registration call
at the end.

## Tuning registry

Built-in presets (`standard-guitar`, `standard-banjo`, `standard-ukulele`,
`drop-d-guitar`, `tenor-banjo`, `tenor-ukulele`) ship from a TOML file compiled
into the binary. The same schema covers user-defined tunings stored at
`$CONFIG/twanga/tunings.toml` (CLI) and in `localStorage` under the
`twanga-user-tunings-v1` key (browser). Built-in slugs shadow user-defined
ones to prevent silent overrides.

## Capo

Per-string semitone offsets (`Capo::offsets: Vec<i32>`) that compose with any
tuning, built-in or custom. `--capo 3` is a uniform capo; `--capo "0,2,2,2,2,2"`
is a partial capo (drop-D style); `--capo "3,3,3,3,0"` keeps the banjo
5th-string drone open while capoing the body. Capo info round-trips through
the alphaTex `\subtitle` field — `; capo=<spec>` — so a recording made with a
capo replays without the user having to remember the value. Both Tuner and
Recorder GUIs expose uniform + per-string capo controls; the per-string panel
is collapsible behind a "Per-string" toggle so the common-case uniform stepper
stays compact.

## What's next

Playback in the browser — load an alphaTex file (drop-zone or bundled
examples), scroll a playhead through it via the renderer host, layer
wait / loop / metronome controls on top. That closes the last whole-screen
parity gap.
