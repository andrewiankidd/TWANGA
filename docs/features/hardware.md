# Hardware

How to get sound *into* TWANGA. Not a feature so much as a setup guide
— picking the right input path is the difference between "wait mode
catches every note" and "wait mode never matches because the buffer
arrived 200ms late."

The whole project is hardware-agnostic on purpose: anything that
presents itself to your OS as an audio input device works. What
follows is the practical picking-order, with the tradeoffs each
choice actually costs you.

## TL;DR

| Path | Latency | Signal quality | Cost | Best for |
|------|---------|----------------|------|----------|
| Built-in laptop mic | ~50–150 ms | poor | £0 | Trying TWANGA out for 10 minutes |
| External USB mic | ~30–100 ms | good for acoustic | £20–£100 | Acoustic instruments, casual practice |
| Instrument-to-USB cable | ~20–60 ms | clean, no room noise | £10–£40 | Electric / electro-acoustic with a 1/4" jack |
| USB audio interface | ~5–20 ms | excellent | £80–£200+ | Anyone doing this seriously |
| Pickup → DI box → interface | ~5–20 ms | excellent | £150–£300+ | Acoustic instruments with no pickup, who want low latency |

Latency is end-to-end (input → detection); the lower the number, the
more responsive [wait-mode playback](playback.md) feels and the less
chance of the cursor "running ahead" because detection arrived late.

## What the author uses

For grounding, my own setup splits by instrument:

- **Banjo** — [KNA BP-1](https://www.knapickups.com/en/folk-instruments/bp-1-kna):
  a passive, wooden-cased piezo sensor that clamps to the side
  of the bridge. Designed as a removable, non-invasive pickup,
  so I just cable-tie the output jack assembly to one of the pot
  brackets and leave it there → Realtone cable (the 1/4" to USB
  instrument cable originally shipped with Rocksmith) handles
  the A/D conversion at the USB end. Roughly Option 3 territory
  below.
- **Ukulele** — cheap adhesive piezo disc, stuck to the
  underside of the uke head with the jack dangling out of the
  back of the body → cheap USB DAC with a 3.5mm input. The DAC
  gives the OS a USB audio device; the piezo gives a usable
  pickup signal for under a fiver. Crude, but it beats trying
  to mic a uke's quiet acoustic body.

Both paths cost well under £50 total. Neither is the "best" option
in raw signal-quality terms (a real interface — Option 4 — wins on
preamp headroom and noise floor), but both clear the latency budget
for [wait mode](playback.md) comfortably and require zero room
treatment.

## Option 1 — Built-in laptop / phone microphone

The path of least resistance. Nothing to plug in.

**Pros**
- Zero setup. Open the app, allow mic access, play.
- Works for any acoustic instrument.

**Cons**
- **Variable latency.** Most laptop mics route through OS audio
  drivers that buffer aggressively. 50–150 ms round-trip is common;
  150 ms is enough that [wait mode](playback.md) will feel sluggish.
- **Room noise.** Picks up the fridge, fan, keyboard taps, your dog,
  passers-by. The pitch detector handles silence gating but a noisy
  room degrades detection accuracy.
- **No isolation.** If you've got a backing track playing on the same
  laptop's speakers, it'll feed back into the mic.
- **Single point of failure** — your laptop mic's frequency response
  is what it is; deep bass strings (low B on a 7-string, drop-tuned
  bass) can confuse it.

**When this is fine.** First 10 minutes of trying the app. The Tuner
needs ~300 ms of clean signal to lock; even a mediocre mic gives you
that on a strummed note.

## Option 2 — External USB microphone

Plug-in USB mic (cardioid pattern, USB-C or USB-A). Many models exist;
look for ones that explicitly advertise low driver latency.

**Pros**
- Cleaner pickup pattern (cardioid) cuts most room noise.
- Better frequency response than built-in mics.
- Usually has lower driver-side latency than the laptop's onboard
  audio chipset (USB audio class-compliant devices avoid some of the
  buffering overhead).
- Works for any instrument that produces sound.

**Cons**
- Still acoustic. Still picks up the room, just less of it.
- **Latency depends heavily on the OS audio stack.** macOS Core Audio
  is fast (~20–40 ms). Windows can be 30–100 ms unless you use
  WASAPI exclusive or an ASIO driver. Linux's PipeWire is generally
  good now (~25 ms); PulseAudio is worse.
- **You can't easily silence the room.** Friends in the same Zoom
  call, noisy neighbours, etc.

**When this is right.** Acoustic instruments (uke, mando, fiddle,
classical guitar, acoustic banjo) practiced in a reasonably quiet
space.

## Option 3 — Instrument-to-USB cable

A single cable that's 1/4" jack at one end (plugs into your
instrument) and USB at the other end (plugs into your computer).
Internally there's a small A/D converter sitting at the USB end.
Multiple brands exist; look for the generic "USB instrument cable"
category — there's no need for a branded one.

**Pros**
- **Direct signal from the pickup** — no room noise at all.
- **Cheap** — generic models are £10–£25.
- **Latency is decent** — typically 20–60 ms depending on the chipset
  + your OS audio stack.
- Plugs into any instrument with a 1/4" output jack (electric
  guitar/bass, electro-acoustic guitar/uke/banjo/mando with a built-in
  pickup, electric violin, etc.).

**Cons**
- **Only works for instruments with a 1/4" output.** Pure acoustic
  instruments don't have one; you'd need to add a clip-on transducer
  pickup (£15–£40) first.
- **Chipset quality varies a lot.** Cheap clones can introduce
  noticeable hum, latency spikes, or driver flakiness. Read reviews
  before buying.
- **Mono signal only.** Fine for TWANGA (the pitch detector is
  mono anyway) but worth flagging.

**When this is right.** You play an electric or electro-acoustic
instrument and you want the cleanest possible signal without spending
audio-interface money.

## Option 4 — Proper USB audio interface

A dedicated external box (Focusrite, PreSonus, Behringer, MOTU, etc.)
with combo XLR + 1/4" inputs, dedicated preamps, and ASIO/Core Audio
drivers. The category most "real" home recording happens through.

**Pros**
- **Lowest latency available** without buying boutique gear —
  5–15 ms is normal, 3 ms is achievable with ASIO + tight buffer
  sizes.
- **Two inputs minimum.** Plug in a mic AND an instrument cable
  simultaneously; the OS sees both.
- **Phantom power** for condenser mics if you want a really good
  acoustic capture.
- **High-quality preamps** — the same signal sounds noticeably
  cleaner than the same mic plugged into a USB hub.

**Cons**
- **More expensive.** Entry-level (Behringer UM2, Focusrite Solo) is
  £80–£100; pro-grade is £200+.
- **One more thing to plug in / power up.** Some require a separate
  power supply.
- **Windows users may want a dedicated ASIO driver** for best
  performance (the unit's bundled driver, not the generic Windows
  one).

**When this is right.** You're going to use TWANGA more than once a
week, OR you also record your own playing elsewhere, OR latency
matters enough that wait mode feeling tight is a deal-breaker.

## Acoustic instruments without electronics

If your instrument has no 1/4" output (most acoustic guitars, mandos,
banjos, ukes, violins), you've got two paths:

1. **Mic it.** Options 1, 2, or 4 (with an XLR mic) above.
2. **Fit a pickup.** Clip-on transducer pickups (acoustic guitar
   sound-hole pickups, banjo head transducers, fiddle bridge
   pickups) feed into a 1/4" jack you then plug into options 3 or 4.
   Removable, non-invasive, £15–£60 depending on the instrument.

The mic path is friendlier (no instrument modification, works for
fingerstyle nuance); the pickup path gives lower latency + zero room
noise. Both are valid.

## What about ASIO / latency tuning on Windows?

Windows ships WASAPI by default; ASIO is the lower-latency
alternative typically used with audio interfaces. TWANGA on Windows
currently ships **without** ASIO support in the default build
(redistribution licensing for Steinberg's SDK is unresolved — see
[ROADMAP](../ROADMAP.md) / [SCOPE](../SCOPE.md)). If you build from
source you can enable the `asio` feature flag:

```bash
cargo run -p twanga-cli --features twanga-audio/asio -- tune
```

For most users, WASAPI exclusive mode with a 256-sample buffer in
your interface's driver panel is plenty (~6 ms at 44.1 kHz). Only
chase ASIO if you actually feel the difference.

## Browser-specific latency notes (GUI)

The web GUI uses Web Audio + AudioWorklet for capture. Latencies are
*higher* than the CLI on the same hardware because the browser adds
its own audio buffering on top of the OS layer:

- **Chrome / Edge** — typically 25–50 ms additional buffering.
- **Firefox** — typically 30–60 ms.
- **Safari** — typically 20–40 ms on macOS (Core Audio is fast).

This is on top of whatever your input hardware adds. If [wait
mode](playback.md) feels sluggish in the GUI but tight in the CLI,
the browser overhead is the difference — switch to CLI or wait for
the Tauri desktop shell's native CPAL backend to land (see
[ROADMAP](../ROADMAP.md)).

## Common gotchas

A handful of footguns that aren't bugs in TWANGA but feel like ones:

- **Sample-rate mismatch.** TWANGA reads at whatever rate the OS
  exposes the device at (typically 44.1 or 48 kHz). If the input
  device advertises one rate and the OS resamples to another behind
  the scenes, pitch detection still works but wait-mode can feel
  laggy because the buffer cadence stutters. If wait-mode feels
  wrong on one device but tight on another, check Sound settings:
  same rate on input + output of the device avoids the resampler
  hop.
- **USB hub power budget.** Audio interfaces (and some USB mics)
  draw enough current that a passive hub starves them — symptoms
  are dropouts, the device disappearing mid-session, or refusing
  to enumerate after sleep. Plug audio gear into a powered hub or
  directly into a host port.
- **Multiple input devices, ambiguous default.** When more than
  one input is present (built-in mic + USB mic + interface),
  TWANGA uses the OS default. `twanga devices` shows the list;
  pick a specific one with `twanga tune --device "<name>"`
  (substring match) if the wrong one is winning. On Windows,
  the "default device" in Sound settings is what matters — not
  the "default communications device".
- **Bluetooth audio.** Don't. Bluetooth headsets add 100–300 ms
  of additional latency in HFP mode (the only mode that opens a
  mic), more than wait-mode can tolerate. Use any wired path
  instead.
- **Silence gate eating quiet plucks.** The 8192-sample (~170 ms)
  RMS window has to clear 0.005 linear amplitude (≈ -46 dB) for
  detection to fire. Plucked-string transients peak high then
  decay fast, so the average across the window can fall below the
  gate even when the meter shows a tall spike on attack. If quiet
  plucks aren't registering, drop the threshold: GUI has a slider
  on the mic meter (drag the vertical-line thumb left); CLI has
  `--silence-rms 0.002` (-54 dB) at startup or `[` + Enter at
  runtime to step down by ~6 dB per press.

## Verifying your setup

```bash
# List the audio input devices the OS exposes to TWANGA.
twanga devices
```

You should see your chosen input in the list. If it's missing, the
OS isn't exposing it — check your system sound settings before
debugging TWANGA.

For latency calibration there's a planned `wait-mode latency wizard`
([ROADMAP](../ROADMAP.md)) that'll measure your end-to-end offset
and shift wait-mode pitch comparison by that amount. Until that
lands, "feels tight" is the metric — if wait mode never matches even
when you're clearly playing the right note, your latency budget is
blown.

## See also

- [Tuner](tuner.md) · [Recorder](recorder.md) · [Playback](playback.md)
  — the three features that actually consume audio input.
- [CLI overview](../CLI.md) · [GUI overview](../GUI.md).
- [ROADMAP](../ROADMAP.md) — latency calibration wizard, native CPAL
  backend in Tauri.
- [SCOPE](../SCOPE.md) — why ASIO redistribution is unresolved.
