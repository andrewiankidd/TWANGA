# User guide

Cross-cutting reference for how TWANGA behaves under the hood —
where files live, how the audio path is wired, what (nothing) leaves
your machine, and the open-source projects we lean on. Per-feature
pages live alongside; this one's for the questions that aren't tied
to a single screen.

## Paths and portable mode

TWANGA persists user data — recordings, imported tabs, the user
tunings file, per-file playback bookmarks — to a single root
directory resolved at startup. There are two modes.

### Default: `~/twanga/`

Lower-case, visible (no leading dot), and identical on every
platform — same string whether you're on Windows, macOS, or Linux.
The Unix convention of dot-prefixed `~/.twanga/` was deliberately
rejected because Windows Explorer doesn't hide dot-dirs by default
but many third-party file managers do, and we'd rather the directory
be findable.

```text
~/twanga/
    tunings.toml        ← user-defined tunings (CLI + Tauri share this)
    play-resume.toml    ← per-file resume bookmarks
    recordings/
        my-take-1779133041.alphatex
    library/
        imported-song.alphatex
```

Per-OS examples (substitute your username):

- **Linux**: `/home/<user>/twanga/`
- **macOS**: `/Users/<user>/twanga/`
- **Windows**: `C:\Users\<user>\twanga\`

### Portable mode

Drop a file named `twanga.portable` next to the TWANGA binary and
the data root switches to `<binary-dir>/twanga-data/` — everything
stays self-contained, perfect for USB-stick installs or for testing
without touching the user's home dir.

```text
on-disk layout in portable mode:

<wherever you put it>/
    twanga.exe          (or twanga / twanga-app)
    twanga.portable     (sentinel — empty file, any contents OK)
    twanga-data/
        tunings.toml
        recordings/
        …
```

The portable distribution artefacts (Windows portable ZIP, macOS
`.app.tar.gz`, Linux AppImage, CLI portable tarball) ship the
sentinel pre-included. The installer / DMG / MSI variants omit it
so a "normal" install uses home mode automatically.

**Linux AppImage** is a special case: the binary at runtime lives
inside a squashfs mount, not at the AppImage's user-facing path.
TWANGA reads the `$APPIMAGE` env var (set by the AppImage runtime)
and checks for the sentinel next to the `.AppImage` file the user
actually downloaded, not next to the mount-extracted binary. So
"drop `twanga.portable` next to the `.AppImage` you put in
`~/Apps/`" works as expected.

### Mobile and web

Neither uses this scheme.

- **iOS / Android** (Tauri Mobile): OS-private sandbox dirs via the
  Tauri filesystem APIs. Apps can't write to the user's home dir on
  these platforms.
- **Web** (browsers): IndexedDB for recordings + imported tabs,
  `localStorage` for tunings and settings. The browser-storage
  warning banner on the Recorder / Playback / Editor screens
  reminds users that this storage can be evicted under pressure.

## Audio architecture

The same `twanga-dsp::Tuner` runs on every surface — the only thing
that changes is who delivers the samples.

### Pitch detection

[YIN](https://hal.archives-ouvertes.fr/hal-02158340/document)
(de Cheveigné & Kawahara, 2002). Deterministic, no model files,
no training, no AI at runtime. Implemented in pure Rust in
[`twanga-dsp`](../../crates/twanga-dsp/); same code runs natively
and compiled to WASM in the browser.

YIN gives you `(frequency, confidence)` per window. TWANGA picks
the lowest non-negative fret on the active tuning that matches the
detected frequency within ±50 cents, accounting for the capo.

### Backends

- **Native** (CLI + Tauri desktop): [CPAL](https://github.com/RustAudio/cpal)
  → WASAPI on Windows, CoreAudio on macOS, ALSA / PipeWire on Linux.
  Sample rate is whatever the device reports (usually 48 000 Hz);
  no resampling is done.
- **Browser**: Web Audio API + AudioWorklet → WASM. The worklet
  delivers fixed-size buffers to a Rust ring buffer; the same
  YIN code consumes them.

### Silence gate

Below a configurable RMS threshold (default `0.005` ≈ -46 dB) the
detector doesn't fire, which avoids cable hum / room noise getting
mistaken for plucks. The threshold is runtime-tunable on both
surfaces — CLI keys `[` / `]`, GUI slider on the mic meter.

### Wait mode latency budget

`twanga play --wait` (CLI) and the GUI's Wait toggle both pause the
cursor at each note until the mic detects a matching pitch. Round-
trip latency budget is ≤100 ms — anything beyond that and the
practice loop feels broken. Hits this budget on a built-in laptop
mic + Realtone cable on a wired interface; struggles on Bluetooth
audio (HFP mic mode adds 100-300 ms) — see [Hardware](hardware.md).

### ASIO

Not shipped. The Steinberg ASIO SDK isn't redistributable under
TWANGA's open-source license. WASAPI handles low-latency on
Windows fine for the everyday case; users who already have ASIO
working can route through an ASIO-aware interface (it'll surface
to TWANGA as a WASAPI device).

## Data and privacy

Local-first. The whole stack runs on your machine.

- **No accounts.** No login, no email gate, no "click to verify."
- **No telemetry.** TWANGA doesn't phone home — not for crash
  reports, not for usage analytics, not for "anonymous improvement
  data." The binary makes zero network requests in normal use.
- **No subscriptions, no DRM, no walled garden.** See
  [SCOPE.md](../SCOPE.md) for the explicit list.
- **Your tabs are your files.** alphaTex is plain text. Open them
  in any text editor; back them up however you like; sync them via
  Dropbox / Syncthing / iCloud / a USB stick — TWANGA doesn't care
  and doesn't see.
- **No tab hosting.** TWANGA never had and never will have a
  tab-sharing service. Tabs are a legal grey zone; the project
  ships empty and stays empty. Community sharing happens off-
  platform.

### Where data lives

- Native (CLI / Tauri): `~/twanga/` (home mode) or
  `<binary-dir>/twanga-data/` (portable mode) — see above.
- Browser: IndexedDB (recordings, imported tabs) +
  `localStorage` (tunings, per-screen settings). All per-origin,
  per-user, never leaves the browser.

### Bundled examples

The `assets/examples/` directory ships with the binary and the web
bundle. Every entry is a verified-public-domain melody (traditional,
classical, or copyright-expired) with the arrangement under
MIT or Apache-2.0. Listed in `assets/examples/manifest.json` with
copyright attribution in each `.alphatex` file's `\copyright` line.

## Credits and acknowledgements

### Prior art and direct influences

- **[alphaTab](https://github.com/CoderLine/alphaTab)** (Daniel
  Kuschny et al.) — origin of the alphaTex format TWANGA uses as
  its native persistence layer. We don't ship alphaTab itself
  (it's a JS-side renderer); we write a Rust parser + serialiser
  against the documented grammar.
- **[YIN paper](https://hal.archives-ouvertes.fr/hal-02158340/document)**
  (Alain de Cheveigné, Hideki Kawahara, 2002) — the pitch
  detection algorithm at the core of `twanga-dsp`.
- **[TuxGuitar](https://sourceforge.net/projects/tuxguitar/)** —
  established open-source tab editor. TWANGA is complementary, not
  a replacement: TuxGuitar's notation editor is mature; TWANGA
  focuses on practice-loop UX and live capture.
- **[slopsmith](https://github.com/byrongamatos/slopsmith)** —
  adjacent project (custom-content management for Clone Hero /
  YARG). Different niche, similar "local-first, no cloud" ethos.

### Major runtime dependencies

In rough order of importance to the project:

| Crate / library | Role |
|---|---|
| [CPAL](https://github.com/RustAudio/cpal) | Cross-platform audio I/O on native targets |
| [rustfft](https://github.com/ejmahler/RustFFT) | FFT primitive used inside YIN |
| [Tauri](https://tauri.app/) | Desktop + mobile shell (`twanga-app`) |
| [wasm-bindgen](https://github.com/rustwasm/wasm-bindgen) | Rust ↔ JS bindings for the browser build |
| [clap](https://github.com/clap-rs/clap) | CLI argument parsing |
| [crossterm](https://github.com/crossterm-rs/crossterm) | Terminal control (cursor, colour, raw mode) |
| [ringbuf](https://github.com/agerasev/ringbuf) | Lock-free SPSC ring buffer between audio callback and DSP loop |
| [serde](https://serde.rs/) + [toml](https://github.com/toml-rs/toml) | Config serialisation (`tunings.toml`, `play-resume.toml`) |
| [directories](https://github.com/dirs-dev/directories-rs) | Home-directory resolution under [`twanga-paths`](../../crates/twanga-paths/) |

Full transitive list is whatever `cargo tree` prints; the table
above only calls out the libraries that shape the architecture.

### Public-domain content

Every bundled example tab is traditional or classical with the
arrangement released under MIT or Apache-2.0. The manifest at
`assets/examples/manifest.json` is the authoritative list; each
`.alphatex` file carries a `\copyright` line documenting source +
licence.

### License

TWANGA itself is dual-licensed MIT or Apache-2.0 at your option.
See [`LICENSE-MIT`](../../LICENSE-MIT) and
[`LICENSE-APACHE`](../../LICENSE-APACHE) at the repo root.
