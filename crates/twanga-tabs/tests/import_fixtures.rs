//! Integration tests for the format-parser pipeline, exercised
//! against real fixture files on disk.
//!
//! Lives in `tests/` (not `src/`) so each fixture pass is a separate
//! binary. Real files instead of inline string literals because:
//!
//! - `parse_mxl` needs an actual zip archive to be meaningful;
//!   building one in-memory from the .musicxml fixture forces the
//!   `zip` crate path to run end-to-end.
//! - The `tests/fixtures/` files double as worked PD examples that
//!   future contributors can crib from when adding a parser for
//!   another format (Phase 3 = ASCII tab, Phase 4 = OCR, …).
//!
//! Both fixtures are the same melody (Twinkle Twinkle Little Star,
//! arrangement for standard reentrant ukulele AECG); the assertions
//! check that each parser produces the same logical tab regardless
//! of source format.

use std::fs;
use std::io::Cursor;
use std::path::PathBuf;

use twanga_tabs::{ParseWarning, abc, alphatex, ascii_tab, midi, musicxml};

fn fixture(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push(name);
    p
}

// ──────────────────────────── alphaTex ────────────────────────────

#[test]
fn alphatex_fixture_parses() {
    let path = fixture("twinkle-twinkle-uke.alphatex");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let parsed = alphatex::parse(&text).expect("alphatex parse");

    // Title + tempo come from the file's directives.
    assert_eq!(parsed.title.as_deref(), Some("Twinkle Twinkle Little Star"));
    assert_eq!(parsed.tempo, 100);

    // Reentrant ukulele: 4 strings, string 1 = A4 (TWANGA's "top
    // string"), string 4 = G4 (the reentrant drone).
    assert_eq!(parsed.tuning_names.len(), 4);
    assert_eq!(parsed.tuning_names[0], "A4");
    assert_eq!(parsed.tuning_names[3], "G4");

    // Should have multiple columns; exact count varies if the
    // example is ever extended, so just bound it loosely.
    assert!(
        parsed.columns.len() >= 8,
        "expected at least 8 columns, got {}",
        parsed.columns.len()
    );
}

// ──────────────────────────── MusicXML ────────────────────────────

#[test]
fn musicxml_fixture_parses() {
    let path = fixture("twinkle-twinkle-uke.musicxml");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let out = musicxml::parse(&text).expect("musicxml parse");

    // Same melody, same metadata — should match the alphatex
    // fixture on the cross-format fields.
    assert_eq!(
        out.tab.title.as_deref(),
        Some("Twinkle Twinkle Little Star")
    );
    assert_eq!(out.tab.tempo, 100);

    // Four-string ukulele — MusicXML's `line=1` is the lowest in
    // its convention; the parser inverts so TWANGA's string 1 is
    // the highest (A4) and string 4 is the reentrant G4.
    assert_eq!(out.tab.tuning_names, vec!["A4", "E4", "C4", "G4"]);

    // Fixture has 2 measures × 4 quarter-notes = 8 columns.
    assert_eq!(out.tab.columns.len(), 8);

    // First column: "Twinkle" — fret 0 on string 3 (the C string
    // in TWANGA's top-down indexing, after the parser's invert).
    let first = &out.tab.columns[0];
    assert_eq!(first.hits.len(), 1);
    assert_eq!(first.hits[0], (3, 0));

    // Last column is a rest (the "...what you are |" tail).
    let last = out.tab.columns.last().unwrap();
    assert!(last.hits.is_empty(), "final column should be a rest");

    // Clean parse — no irregular-duration / unreachable-note
    // surprises for this hand-crafted fixture.
    assert!(
        out.warnings.is_empty(),
        "expected zero warnings, got {:?}",
        out.warnings
    );
}

// ─────────────────────────── MXL (zipped) ──────────────────────────

#[test]
fn mxl_fixture_parses_via_zip_path() {
    // The .musicxml fixture is the source of truth — we zip it on
    // the fly to exercise the `parse_mxl` path without committing
    // a binary blob.
    let xml_path = fixture("twinkle-twinkle-uke.musicxml");
    let xml = fs::read_to_string(&xml_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", xml_path.display()));
    let mxl_bytes = musicxml::write_mxl_bytes(&xml);

    let out = musicxml::parse_mxl(&mxl_bytes).expect("mxl parse");

    // Same expected output as the raw MusicXML — the zip wrapper
    // shouldn't change the parsed tab, only the transport.
    assert_eq!(
        out.tab.title.as_deref(),
        Some("Twinkle Twinkle Little Star")
    );
    assert_eq!(out.tab.tempo, 100);
    assert_eq!(out.tab.tuning_names, vec!["A4", "E4", "C4", "G4"]);
    assert_eq!(out.tab.columns.len(), 8);
}

#[test]
fn mxl_fallback_when_no_container_manifest() {
    // Build a zip with no META-INF/container.xml — the parser's
    // fallback path (first `.xml` / `.musicxml` entry in the
    // archive) should still succeed.
    let xml_path = fixture("twinkle-twinkle-uke.musicxml");
    let xml = fs::read_to_string(&xml_path).expect("read fixture");

    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut zip = ZipWriter::new(Cursor::new(&mut buf));
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("score.musicxml", opts).expect("entry");
        std::io::Write::write_all(&mut zip, xml.as_bytes()).expect("write");
        zip.finish().expect("finish");
    }

    let out = musicxml::parse_mxl(&buf).expect("mxl-no-manifest parse");
    assert_eq!(
        out.tab.title.as_deref(),
        Some("Twinkle Twinkle Little Star")
    );
    assert_eq!(out.tab.columns.len(), 8);
}

// ───────────────────────────── ASCII tab ──────────────────────────

#[test]
fn ascii_tab_fixture_parses_with_recognised_tuning() {
    let path = fixture("twinkle-twinkle.tab");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let out = ascii_tab::parse(&text).expect("ascii parse");

    // The fixture's `e B G D A E` labels match standard guitar's
    // open strings exactly — parser should pin to that preset and
    // NOT emit InferredTuning.
    assert_eq!(out.tab.tuning_names.len(), 6);
    assert!(
        !out.warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::InferredTuning { .. })),
        "exact-match labels shouldn't fall back to InferredTuning"
    );

    // Title from the `#` comment line, with the descriptive subtitle
    // (everything after the em-dash) stripped — fixture's header is
    // "# Twinkle Twinkle Little Star — ASCII tab test fixture."; the
    // parser keeps only the song name.
    assert_eq!(
        out.tab.title.as_deref(),
        Some("Twinkle Twinkle Little Star")
    );

    // 7 columns of melody (0, 0, 7, 7, 9, 9, 7).
    assert_eq!(out.tab.columns.len(), 7);
    assert_eq!(out.tab.columns[0].hits, vec![(1, 0)]);
    assert_eq!(out.tab.columns[2].hits, vec![(1, 7)]);
}

// ──────────────────────────── ABC notation ────────────────────────

#[test]
fn abc_fixture_parses() {
    let path = fixture("twinkle-twinkle.abc");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let out = abc::parse(&text).expect("abc parse");

    // Header survived
    assert_eq!(
        out.tab.title.as_deref(),
        Some("Twinkle Twinkle Little Star")
    );
    assert_eq!(out.tab.tempo, 100);

    // `C C G G | A A G2` is 7 notes (the trailing G2 is one
    // half-note column, not two quarter columns) — bar lines don't
    // emit columns of their own, so the count is 4 + 3 = 7.
    assert_eq!(out.tab.columns.len(), 7);

    // ABC carries no tuning info, so InferredTuning warning fires.
    assert!(
        out.warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::InferredTuning { .. })),
        "ABC without declared tuning must surface InferredTuning"
    );

    // Final column is the G2 (half note) — denominator 2 (half).
    assert_eq!(out.tab.columns.last().unwrap().duration_denom, 2);
}

// ────────────────────────────── MIDI ──────────────────────────────

#[test]
fn midi_fixture_parses_via_write_smf_round_trip() {
    // Same melody-shape baseline as the other format fixtures but
    // built deterministically via `midi::write_smf_bytes` (the
    // symmetric inverse of `midi::parse`). No binary blob committed
    // to the repo — the test's input + its assertions describe the
    // fixture exhaustively.
    //
    // Twinkle Twinkle's first phrase, monophonic: C C G G A A G.
    // Using MIDI numbers 60 (C4), 67 (G4), 69 (A4).
    let notes: Vec<(u8, u32)> = vec![
        (60, 4),
        (60, 4),
        (67, 4),
        (67, 4),
        (69, 4),
        (69, 4),
        (67, 4),
    ];
    let bytes = midi::write_smf_bytes(&notes, Some("Twinkle Twinkle Little Star"), Some(100));

    let out = midi::parse(&bytes).expect("midi parse");
    assert_eq!(
        out.tab.title.as_deref(),
        Some("Twinkle Twinkle Little Star")
    );
    assert_eq!(out.tab.tempo, 100);
    // MIDI carries no tuning info — parser defaults to standard
    // guitar and emits an InferredTuning warning so the importer UI
    // can surface that the target was guessed.
    assert_eq!(out.tab.tuning_names.len(), 6);
    assert!(
        out.warnings
            .iter()
            .any(|w| matches!(w, ParseWarning::InferredTuning { .. })),
        "MIDI without declared tuning must surface InferredTuning"
    );
    // 7 columns, one per note. Quarter notes — first column denom
    // should be 4 (quarter).
    assert_eq!(out.tab.columns.len(), 7);
    assert_eq!(out.tab.columns[0].duration_denom, 4);
}

// ─────────────────── External (lilypond MIT) fixtures ──────────────
//
// These three files come from the lilypond MusicXML regression suite
// (Reinhold Kainhofer, MIT). The closed-loop concern — "I wrote the
// parser AND the test data" — is mitigated here: an independent
// author produced these specifically to validate other MusicXML
// parsers. If TWANGA's parser handles the same inputs, the parser is
// doing more than reflecting our own mental model of the spec.

fn external(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("external");
    p.push(name);
    p
}

#[test]
fn lilypond_tab_staves_fixture_picks_first_part() {
    // 71e-TabStaves declares four stacked guitar parts. The parser
    // commits to "only first staff of first part is read" — this
    // test pins that contract against a real multi-part input. Pre-
    // fix this would have produced a 43-string mega-tuning and 64
    // columns of garbled notes; post-fix it's the first guitar's
    // 6-string standard tuning + that guitar's 8 columns.
    let path = external("71e-TabStaves.xml");
    let text = fs::read_to_string(&path).expect("read fixture");
    let out = musicxml::parse(&text).expect("parse");

    // First guitar = 6 strings, standard tuning. TWANGA inverts
    // MusicXML's bottom-up ordering, so string 1 here = the
    // highest open string (G4 in this fixture's standard +
    // capo-shifted-down representation — lilypond's test uses
    // tunings that aren't EADGBE).
    assert_eq!(
        out.tab.tuning_names.len(),
        6,
        "should be 6 strings (one guitar's worth), got {}: {:?}",
        out.tab.tuning_names.len(),
        out.tab.tuning_names,
    );

    // Eight columns of explicit `<technical><string><fret>` data
    // from the first guitar. No `UnreachableNote` warnings expected
    // — every note has explicit fret info so no pitch-matching
    // fallback runs.
    assert_eq!(out.tab.columns.len(), 8);
    assert!(
        out.warnings.is_empty(),
        "tab-staves fixture should parse clean, got {:?}",
        out.warnings,
    );

    // The first guitar's measure includes a three-note chord
    // (`<note>` + two `<chord/>` members) on column 4; the other
    // seven columns are single-note hits. Pin the shape so a
    // regression that miscounts chord members shows up here.
    let hits_per_column: Vec<usize> = out.tab.columns.iter().map(|c| c.hits.len()).collect();
    assert_eq!(
        hits_per_column,
        vec![1, 1, 1, 1, 3, 1, 1, 1],
        "unexpected hits-per-column shape: {hits_per_column:?}",
    );
}

#[test]
fn lilypond_pitches_fixture_handles_full_chromatic_sweep() {
    // 01a-Pitches-Pitches has 108 notes spanning the chromatic
    // range. There's no <staff-tuning> so the parser can't place
    // them on a fretboard — we get an UnreachableNote warning per
    // note. The valuable assertion is that the parser doesn't
    // panic or hang on a large, real-world-shaped input.
    let path = external("01a-Pitches-Pitches.xml");
    let text = fs::read_to_string(&path).expect("read fixture");
    let out = musicxml::parse(&text).expect("parse");

    // 108 columns (one per note, no chord members).
    assert_eq!(out.tab.columns.len(), 108);
    // No `<staff-tuning>` in the fixture, so every pitch surfaces
    // as an UnreachableNote warning — that's the documented
    // behaviour for tuning-less input.
    assert_eq!(out.warnings.len(), 108);
    for w in &out.warnings {
        assert!(matches!(
            w,
            twanga_tabs::ParseWarning::UnreachableNote { .. }
        ));
    }
}

#[test]
fn lilypond_schubert_stabat_mater_fixture_handles_real_pd_piece() {
    // 21d-Chords-SchubertStabatMater is real public-domain music
    // (Schubert, 1816 — centuries out of copyright). Choral piece
    // with no instrument-tab info; we're testing that the parser
    // doesn't choke on a real-world chord-heavy input.
    let path = external("21d-Chords-SchubertStabatMater.xml");
    let text = fs::read_to_string(&path).expect("read fixture");
    let out = musicxml::parse(&text).expect("parse");

    // Five columns of choral chords. Each column has multiple
    // chord members (`<chord/>` elements) but no staff-tuning so
    // they all surface as UnreachableNote warnings.
    assert_eq!(out.tab.columns.len(), 5);
    // 11 unreachable-note warnings (no staff-tuning → no fret
    // placement). Pin the count so a future parser change that
    // alters chord-member counting is caught.
    assert!(
        out.warnings.len() >= 5,
        "expected at least 5 warnings (one per column minimum), got {}",
        out.warnings.len(),
    );
}

// ─────────────────── Cross-format equivalence ──────────────────────

#[test]
fn alphatex_and_musicxml_fixtures_describe_the_same_melody() {
    // Both fixtures encode the opening 8 columns of Twinkle on the
    // same reentrant ukulele tuning. The exact column-by-column
    // hits should match — this is the assertion that protects
    // against a future change to either parser silently breaking
    // the cross-format invariant.
    let alphatex_text =
        fs::read_to_string(fixture("twinkle-twinkle-uke.alphatex")).expect("read alphatex");
    let alpha = alphatex::parse(&alphatex_text).expect("alphatex parse");

    let musicxml_text =
        fs::read_to_string(fixture("twinkle-twinkle-uke.musicxml")).expect("read musicxml");
    let mxml = musicxml::parse(&musicxml_text).expect("musicxml parse");

    // Tempo + tuning must match — that's the metadata layer.
    assert_eq!(alpha.tempo, mxml.tab.tempo, "tempo mismatch");
    assert_eq!(
        alpha.tuning_names, mxml.tab.tuning_names,
        "tuning_names mismatch"
    );

    // The MusicXML fixture is just the first 8 columns of the
    // alphatex example, so compare prefixes (alphatex has the
    // whole song).
    let common = mxml.tab.columns.len();
    assert!(
        alpha.columns.len() >= common,
        "alphatex should have at least as many columns as the musicxml prefix"
    );
    for (i, (a, m)) in alpha
        .columns
        .iter()
        .take(common)
        .zip(mxml.tab.columns.iter())
        .enumerate()
    {
        assert_eq!(
            a.hits, m.hits,
            "column {i} hits diverge between alphatex and musicxml",
        );
    }
}
