//! End-to-end CLI integration tests for `twanga convert` and
//! `twanga import`. Invokes the actual built `twanga` binary via
//! `CARGO_BIN_EXE_twanga` so the test surface is the real
//! user-facing one — clap parsing, file IO, the conversion
//! pipeline, exit codes — not just the underlying library calls.
//!
//! `import` is exercised only at unit level (it writes to the
//! real `<data-root>/library/` and we don't want test runs to
//! pollute the user's home dir); `convert` is the right candidate
//! for end-to-end because it takes an explicit `--out` we control.

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn binary() -> PathBuf {
    // Cargo provides the absolute path to the built binary for
    // the package under test via this env var, set per integration
    // test invocation.
    PathBuf::from(env!("CARGO_BIN_EXE_twanga"))
}

fn fixture(name: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push(name);
    p
}

/// Per-test scratch directory under `std::env::temp_dir()`. Each
/// test gets a unique nanosecond-suffixed path so concurrent runs
/// don't collide; the directory is removed at the end of the test
/// via the returned guard.
fn scratch(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("twanga-cli-test-{label}-{nanos}"));
    fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

#[test]
fn convert_alphatex_round_trip_writes_valid_alphatex() {
    let dir = scratch("convert-alphatex");
    let out = dir.join("out.alphatex");

    let status = Command::new(binary())
        .arg("convert")
        .arg(fixture("twinkle-twinkle-uke.alphatex"))
        .arg("--out")
        .arg(&out)
        .status()
        .expect("invoke twanga");
    assert!(status.success(), "twanga convert exit code: {status}");

    assert!(
        out.exists(),
        "convert output didn't write to {}",
        out.display()
    );

    // Output should be parseable as alphaTex — round trip is the
    // contract.
    let written = fs::read_to_string(&out).expect("read output");
    let parsed = twanga_tabs::alphatex::parse(&written).expect("parse output");
    assert_eq!(parsed.title.as_deref(), Some("Twinkle Twinkle Little Star"));
    assert_eq!(parsed.tempo, 100);
    assert_eq!(parsed.tuning_names.len(), 4);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn convert_musicxml_to_alphatex_writes_valid_alphatex() {
    let dir = scratch("convert-musicxml");
    let out = dir.join("out.alphatex");

    let status = Command::new(binary())
        .arg("convert")
        .arg(fixture("twinkle-twinkle-uke.musicxml"))
        .arg("--out")
        .arg(&out)
        .status()
        .expect("invoke twanga");
    assert!(status.success(), "twanga convert exit code: {status}");

    // The cross-format conversion: MusicXML in → alphaTex out,
    // re-parseable, metadata preserved.
    let written = fs::read_to_string(&out).expect("read output");
    let parsed = twanga_tabs::alphatex::parse(&written).expect("parse output");
    assert_eq!(parsed.title.as_deref(), Some("Twinkle Twinkle Little Star"));
    assert_eq!(parsed.tempo, 100);
    // 4-string reentrant uke tuning came across cleanly.
    assert_eq!(parsed.tuning_names, vec!["A4", "E4", "C4", "G4"]);
    // 8 columns in the MusicXML fixture.
    assert_eq!(parsed.columns.len(), 8);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn convert_mxl_to_alphatex_writes_valid_alphatex() {
    // No .mxl fixture committed to the repo — `.mxl` is just zipped
    // MusicXML, so we build one on the fly from the existing
    // `.musicxml` fixture using `twanga_tabs::musicxml::write_mxl_bytes`
    // (the symmetric inverse of `parse_mxl`). Keeps a single source
    // of truth for the fixture content and exercises the binary's
    // `.mxl` → alphaTex path through clap → file IO → conversion
    // pipeline, the same surface a user hits.
    let dir = scratch("convert-mxl");
    let mxl_path = dir.join("input.mxl");
    let out = dir.join("out.alphatex");

    let xml = fs::read_to_string(fixture("twinkle-twinkle-uke.musicxml")).expect("read musicxml");
    let mxl_bytes = twanga_tabs::musicxml::write_mxl_bytes(&xml);
    fs::write(&mxl_path, &mxl_bytes).expect("write mxl");

    let status = Command::new(binary())
        .arg("convert")
        .arg(&mxl_path)
        .arg("--out")
        .arg(&out)
        .status()
        .expect("invoke twanga");
    assert!(status.success(), "twanga convert (mxl): {status}");

    // Same expected shape as the raw-MusicXML conversion — the
    // .mxl wrapper changes the transport, not the parsed result.
    let written = fs::read_to_string(&out).expect("read output");
    let parsed = twanga_tabs::alphatex::parse(&written).expect("parse output");
    assert_eq!(parsed.title.as_deref(), Some("Twinkle Twinkle Little Star"));
    assert_eq!(parsed.tempo, 100);
    assert_eq!(parsed.tuning_names, vec!["A4", "E4", "C4", "G4"]);
    assert_eq!(parsed.columns.len(), 8);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn convert_ascii_tab_to_alphatex_writes_valid_alphatex() {
    // ASCII tab fixture is committed as text. End-to-end test: real
    // .tab on disk → `twanga convert` → re-parseable alphaTex with
    // the right tuning pinned + melody intact.
    let dir = scratch("convert-ascii");
    let out = dir.join("out.alphatex");

    let status = Command::new(binary())
        .arg("convert")
        .arg(fixture("twinkle-twinkle.tab"))
        .arg("--out")
        .arg(&out)
        .status()
        .expect("invoke twanga");
    assert!(status.success(), "twanga convert (ascii): {status}");

    let written = fs::read_to_string(&out).expect("read output");
    let parsed = twanga_tabs::alphatex::parse(&written).expect("parse output");
    // Tuning pinned to standard guitar via exact label match.
    assert_eq!(parsed.tuning_names.len(), 6);
    // 7 columns from the melody.
    assert_eq!(parsed.columns.len(), 7);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn convert_abc_to_alphatex_writes_valid_alphatex() {
    // ABC fixture is committed as text under tests/fixtures/, same
    // shape as the .alphatex / .musicxml ones. End-to-end test: real
    // .abc on disk → `twanga convert` → re-parseable alphaTex.
    let dir = scratch("convert-abc");
    let out = dir.join("out.alphatex");

    let status = Command::new(binary())
        .arg("convert")
        .arg(fixture("twinkle-twinkle.abc"))
        .arg("--out")
        .arg(&out)
        .status()
        .expect("invoke twanga");
    assert!(status.success(), "twanga convert (abc): {status}");

    let written = fs::read_to_string(&out).expect("read output");
    let parsed = twanga_tabs::alphatex::parse(&written).expect("parse output");
    assert_eq!(parsed.title.as_deref(), Some("Twinkle Twinkle Little Star"));
    assert_eq!(parsed.tempo, 100);
    // ABC's K:C with no tuning info → parser placed on standard
    // guitar (6 strings).
    assert_eq!(parsed.tuning_names.len(), 6);
    // 7 columns (C C G G | A A G2).
    assert_eq!(parsed.columns.len(), 7);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn convert_midi_to_alphatex_writes_valid_alphatex() {
    // No .mid fixture committed to the repo — build one on the fly
    // via `twanga_tabs::midi::write_smf_bytes` (the symmetric inverse
    // of `midi::parse`). Same posture as the .mxl integration test.
    let dir = scratch("convert-midi");
    let mid_path = dir.join("input.mid");
    let out = dir.join("out.alphatex");

    // Twinkle's first phrase: C C G G A A G — quarter notes.
    let notes: Vec<(u8, u32)> = vec![
        (60, 4),
        (60, 4),
        (67, 4),
        (67, 4),
        (69, 4),
        (69, 4),
        (67, 4),
    ];
    let mid_bytes =
        twanga_tabs::midi::write_smf_bytes(&notes, Some("Twinkle Twinkle Little Star"), Some(100));
    fs::write(&mid_path, &mid_bytes).expect("write mid");

    let status = Command::new(binary())
        .arg("convert")
        .arg(&mid_path)
        .arg("--out")
        .arg(&out)
        .status()
        .expect("invoke twanga");
    assert!(status.success(), "twanga convert (midi): {status}");

    // Output should re-parse as alphaTex; MIDI defaults to standard
    // guitar so the tuning header should be EADGBE in string-1-first
    // (high E) order.
    let written = fs::read_to_string(&out).expect("read output");
    let parsed = twanga_tabs::alphatex::parse(&written).expect("parse output");
    assert_eq!(parsed.title.as_deref(), Some("Twinkle Twinkle Little Star"));
    assert_eq!(parsed.tempo, 100);
    assert_eq!(parsed.tuning_names.len(), 6);
    assert_eq!(parsed.columns.len(), 7);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn convert_with_explicit_from_flag_overrides_extension() {
    // Save the alphaTex fixture under a `.txt` extension and force
    // `--from alphatex`. The conversion should succeed where
    // extension-based detection would (correctly) have picked
    // alphaTex anyway — but the assertion is that `--from`
    // overrides whatever the extension would have said, including
    // when they agree.
    let dir = scratch("convert-from-flag");
    let mis_extended = dir.join("input.txt");
    fs::copy(fixture("twinkle-twinkle-uke.alphatex"), &mis_extended).expect("seed input");
    let out = dir.join("out.alphatex");

    let status = Command::new(binary())
        .arg("convert")
        .arg(&mis_extended)
        .arg("--out")
        .arg(&out)
        .arg("--from")
        .arg("alphatex")
        .status()
        .expect("invoke twanga");
    assert!(status.success(), "twanga convert --from alphatex: {status}");
    assert!(out.exists(), "convert with --from didn't write output");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn convert_rejects_unknown_extension_with_unsniffable_content() {
    // No extension AND content that doesn't match any known format
    // (no alphaTex backslash directives, no ASCII-tab string-label
    // lines) → can't infer format → expect a non-zero exit. The
    // unit tests cover the exact error message shape.
    let dir = scratch("convert-unsniffable");
    let bare = dir.join("garbage_no_extension");
    fs::write(
        &bare,
        "this is just prose, no directives, no tab lines, nothing",
    )
    .expect("seed");
    let out = dir.join("out.alphatex");

    let status = Command::new(binary())
        .arg("convert")
        .arg(&bare)
        .arg("--out")
        .arg(&out)
        .status()
        .expect("invoke twanga");
    assert!(
        !status.success(),
        "convert should exit non-zero on unknown extension + unsniffable content"
    );
    assert!(
        !out.exists(),
        "convert shouldn't write output on a parse-setup failure"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn convert_sniffs_alphatex_in_txt_file() {
    // `.txt` is no longer mapped to a format by extension — sniffer
    // recognises alphaTex's backslash directives and routes through.
    let dir = scratch("convert-sniff-alphatex-txt");
    let mis_extended = dir.join("song.txt");
    fs::copy(fixture("twinkle-twinkle-uke.alphatex"), &mis_extended).expect("seed");
    let out = dir.join("out.alphatex");

    let status = Command::new(binary())
        .arg("convert")
        .arg(&mis_extended)
        .arg("--out")
        .arg(&out)
        .status()
        .expect("invoke twanga");
    assert!(
        status.success(),
        "alphatex content in .txt should sniff cleanly"
    );
    assert!(out.exists(), "output should be written");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn convert_sniffs_ascii_tab_in_txt_file() {
    // Same sniff path, ASCII-tab side — `.tab` extension is canonical
    // but a tab pasted into a `.txt` should still work without
    // requiring `--from ascii`.
    let dir = scratch("convert-sniff-ascii-txt");
    let mis_extended = dir.join("song.txt");
    fs::copy(fixture("twinkle-twinkle.tab"), &mis_extended).expect("seed");
    let out = dir.join("out.alphatex");

    let status = Command::new(binary())
        .arg("convert")
        .arg(&mis_extended)
        .arg("--out")
        .arg(&out)
        .status()
        .expect("invoke twanga");
    assert!(
        status.success(),
        "ASCII tab content in .txt should sniff cleanly"
    );
    assert!(out.exists());

    let _ = fs::remove_dir_all(&dir);
}
