# Patterns

A bundled library of short rhythm + picking drills — clawhammer figures,
bluegrass rolls, uke strums. Each pattern is a small `.alphatex` file
designed to loop, so you practice the groove rather than working through a
tune.

GUI-first feature with a small CLI counterpart (the patterns are just
`.alphatex` files, so `twanga play` reads them directly).

## GUI

Open the Patterns card from the main menu (or `#patterns`).

The screen renders a tree grouped by tradition. Each group has a short
description and a list of patterns; each pattern row shows the title, the
target tuning, and a difficulty marker.

Clicking a pattern loads it into Playback in loop-by-default mode, so you
land in a practice loop without extra configuration. Lower the BPM until
you can play it cleanly, then bring it back up.

### Bundled groups

- **Clawhammer (banjo)** — old-time rhythmic patterns built around the
  bum-diddy figure (melody note + brush + drone-string pluck). Includes
  a basic and a drop-thumb variant.
- **Bluegrass picking (banjo)** — three-finger picking patterns. Loop
  under tempo to lock in the muscle memory before applying them to a
  melody.
- **Ukulele strums** — strumming patterns over a held C-major shape
  (all open strings) so you can focus on the rhythm.

Adding more patterns is a content-only operation: drop a new
`assets/patterns/<name>.alphatex` and add a manifest entry. See the
[Pattern trainer backlog](../BACKLOG.md) for the queued expansions.

## CLI

There's no dedicated `twanga patterns` subcommand — the patterns are
ordinary `.alphatex` files. Play them directly:

```bash
twanga play assets/patterns/bum-diddy-simple.alphatex --loop
twanga play assets/patterns/forward-roll-banjo.alphatex --loop --bpm 80
twanga play assets/patterns/uke-island-strum.alphatex --loop --wait
```

The full set is in [`assets/patterns/`](../../assets/patterns/) with a
manifest at `manifest.json`. Same loop / wait / pre-roll / metronome
flags as any other `play` invocation.

## Where things live

- **Pattern files** — `assets/patterns/<id>.alphatex`, shipped with the
  binary AND the deployed web bundle.
- **Manifest** — `assets/patterns/manifest.json`, consumed by the
  Patterns screen's `patternsManifest()` library call.
- **In-app routing** — pattern ids surface via the `pattern:<id>`
  prefix in `library.load(id)` so the Playback engine doesn't need a
  separate code path. Patterns DO NOT appear in `library.list()` —
  they're a curated browsing experience, not a flat library.

## See also

- [Playback feature](playback.md) — the engine the Patterns screen
  hands tabs off to.
- [Pattern trainer roadmap](../ROADMAP.md#follows-next-big-rocks) —
  upcoming rhythm-only verification (the next bite).
- [Pattern content backlog](../BACKLOG.md) — drills queued for
  addition.
