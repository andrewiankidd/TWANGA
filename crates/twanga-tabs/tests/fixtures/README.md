# MusicXML parser test fixtures

Two kinds of fixture live here:

- **Hand-written** (top-level of `fixtures/`) — small, focused test
  inputs written specifically for TWANGA's parser. Public-domain
  melodies in MIT/Apache-2.0 arrangements, matching the rest of
  TWANGA's bundled examples (`assets/examples/`).
- **External** (under `external/`) — files imported from
  independent third-party MusicXML test corpora. The closed-loop
  problem — "the same person wrote the parser AND the test data"
  — is mitigated by using files that an independent author
  produced for a completely different parser project. If TWANGA's
  parser passes the same fixtures lilypond's `musicxml2ly` was
  tested against, we have meaningful confidence that the parser
  handles real-world MusicXML rather than just what we'd write
  ourselves.

## Hand-written fixtures

| File | Purpose |
|------|---------|
| `twinkle-twinkle-uke.alphatex` | alphaTex parser baseline. Public-domain melody (Mozart's variations on "Ah! vous dirai-je, maman" → centuries out of copyright). Same file as `assets/examples/twinkle-twinkle-uke.alphatex`. Arrangement under MIT/Apache-2.0. |
| `twinkle-twinkle-uke.musicxml` | Cross-format equivalence baseline — same melody as the alphaTex version above, encoded as MusicXML for reentrant ukulele tuning. Hand-written for TWANGA, MIT/Apache-2.0. |

## External fixtures

Files under `external/` come from [the lilypond project's MusicXML
regression test suite](https://github.com/lilypond/lilypond/tree/master/input/regression/musicxml),
originally created by Reinhold Kainhofer as an independent MusicXML
test corpus. Distributed under the MIT license — the full license
text lives in `external/LICENSE-lilypond-musicxml-testsuite`.

| File | What it tests |
|------|---------------|
| `external/01a-Pitches-Pitches.xml` | Basic pitch coverage — broad sweep across the chromatic range. Catches step / alter / octave parsing bugs in a single fixture. |
| `external/71e-TabStaves.xml` | Tablature notation specifically — multiple guitar parts with explicit `<staff-tuning>`, `<string>`, and `<fret>` elements. This is the TWANGA-relevant fixture: the parser's tab-mode path with externally-authored input. |
| `external/21d-Chords-SchubertStabatMater.xml` | Real public-domain music — Schubert's *Stabat Mater* (1816, centuries out of copyright). Exercises chord-member parsing (`<chord/>`) on a real piece rather than a synthetic test. |

## Adding new external fixtures

If you pull in another file from an external corpus:

1. Verify the licence is MIT-compatible (MIT / Apache-2.0 / CC0
   / public domain). Drop the licence text alongside the file
   in `external/` so the provenance trail is visible without
   leaving the repo.
2. Note the upstream URL in this README so future-you can
   refresh from source if needed.
3. Write an assertion in `tests/import_fixtures.rs` that pins
   *something specific* about the file (column count, tempo, a
   known string-fret hit) so a future parser regression breaks
   the test rather than silently producing the wrong output.
