# Patterns

A bundled library of short rhythm + picking drills — clawhammer figures,
bluegrass rolls, uke strums, guitar fingerpicking. Each pattern is a small
`.alphatex` file designed to loop, so you practice the groove rather than
working through a tune.

Full parity between CLI and GUI: the GUI has the Patterns screen; the
CLI has the `twanga patterns` subcommand. Both consume the same manifest
and play through the same engine.

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
- **Bluegrass picking (banjo)** — three-finger picking patterns: forward
  roll, reverse roll, alternating-thumb roll. Loop under tempo to lock
  in the muscle memory before applying them to a melody.
- **Ukulele strums** — strumming patterns over a held C-major shape
  (all open strings) so you can focus on the rhythm. Baseline DDUUDU,
  Island, Waltz (3/4).
- **Guitar (standard tuning)** — open-G chord drills. Boom-chick (the
  country/folk backbone) and Travis picking (alternating-bass fingerstyle).

Adding more patterns is a content-only operation: drop a new
`assets/patterns/<name>.alphatex` and add a manifest entry. See the
[Pattern trainer backlog](../BACKLOG.md) for the queued expansions.

## CLI

`twanga patterns` mirrors the GUI Patterns screen. Four actions:

```bash
twanga patterns                    # interactive picker (groups + difficulty pips)
twanga patterns list               # catalog dump — scriptable
twanga patterns play <id>          # non-interactive play; loops by default
twanga patterns path               # print the manifest path
```

Examples:

```bash
twanga patterns play forward-roll-banjo
twanga patterns play uke-island-strum --bpm 70 --wait
twanga patterns play boom-chick-guitar --no-loop
```

`twanga patterns play` looping is on by default (the whole point of a
pattern). Pass `--no-loop` to play through once.

Patterns are also surfaced through the bare-`twanga play` picker
alongside bundled examples and your local `./recordings/` directory, if
you want them all in one menu instead of a dedicated pattern flow.

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
