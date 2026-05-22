//! Format detection + conversion pipeline + library write for
//! `twanga import` and `twanga convert`.
//!
//! Both subcommands share the same underlying flow:
//!
//!   read bytes → detect format → parse to `ParsedTab` → serialize as
//!   alphaTex
//!
//! `convert` writes the result to an explicit `--out` path (stateless,
//! no library involvement). `import` writes to the user's library
//! directory under a derived filename, mirroring what the GUI Importer
//! does. Both go through the same `convert_to_alphatex` helper so the
//! conversion logic itself has a single test surface.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use twanga_core::Capo;
use twanga_tabs::{ParseWarning, ParsedTab, abc, alphatex, ascii_tab, midi, musicxml};

use crate::bundled;
use crate::slugify;

/// Source formats the importer / converter understands. Detected from
/// the input's file extension unless the user passes `--from`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    /// alphaTex (TWANGA's native format — `.alphatex` files).
    AlphaTex,
    /// MusicXML, raw XML (`.musicxml` or `.xml`).
    MusicXml,
    /// MusicXML, zipped (`.mxl` — the MuseScore default export).
    Mxl,
    /// Standard MIDI File (`.mid` / `.midi`). Pitches placed on the
    /// default tuning (standard guitar) at import time; user can
    /// retune at playback.
    Midi,
    /// ABC notation (`.abc`). Folk / traditional text-based notation
    /// format; same pitch-only constraint as MIDI, so the importer
    /// places notes on the default tuning and surfaces an
    /// `InferredTuning` warning.
    Abc,
    /// ASCII tab (`.tab`, or `.txt` with `--from ascii`). Heuristic
    /// parser — string-line detection, tuning inferred from line
    /// labels with nearest-match fallback against the built-in
    /// registry.
    AsciiTab,
}

impl SourceFormat {
    /// Parse a user-supplied `--from` value into a `SourceFormat`.
    /// Accepts the canonical lowercase names (`alphatex`, `musicxml`,
    /// `mxl`, `midi`) — clap's `value_parser` rejects anything else
    /// before we reach here, so the panicking match arm is
    /// unreachable in practice but kept defensive for completeness.
    pub fn parse_flag(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "alphatex" => Ok(Self::AlphaTex),
            "musicxml" | "xml" => Ok(Self::MusicXml),
            "mxl" => Ok(Self::Mxl),
            "midi" | "mid" => Ok(Self::Midi),
            "abc" => Ok(Self::Abc),
            "ascii" | "ascii-tab" | "tab" => Ok(Self::AsciiTab),
            other => Err(anyhow!(
                "unknown --from value '{other}' (expected alphatex / musicxml / mxl / midi / abc / ascii)"
            )),
        }
    }

    /// Detect from a file path's extension. Returns `None` for
    /// unrecognised extensions AND for ambiguous ones (`.txt` could
    /// be either alphaTex or ASCII tab) so callers can fall back to
    /// content sniffing or surface a clear error.
    pub fn from_extension(path: &Path) -> Option<Self> {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)?;
        match ext.as_str() {
            "alphatex" => Some(Self::AlphaTex),
            "musicxml" | "xml" => Some(Self::MusicXml),
            "mxl" => Some(Self::Mxl),
            "mid" | "midi" => Some(Self::Midi),
            "abc" => Some(Self::Abc),
            "tab" => Some(Self::AsciiTab),
            _ => None,
        }
    }

    /// Sniff the format by content shape. Used as a fallback for
    /// ambiguous-extension files (`.txt`) and for files with no
    /// extension at all. Recognises alphaTex by its `\<directive>`
    /// header lines (`\title`, `\tempo`, `\tuning`, `\subtitle`) and
    /// ASCII tab by its string-label-pipe-content shape. Returns
    /// `None` if neither pattern dominates the head of the input —
    /// the caller surfaces a clear "couldn't detect" error.
    ///
    /// Window size is 8 KiB rather than 1 KiB so the heuristic
    /// survives realistic ASCII-tab headers — public-domain tabs on
    /// the open web often carry multi-paragraph credit / license /
    /// transcriber-notes blocks before the first string-line, and a
    /// narrow window can miss the tab entirely.
    pub fn sniff_content(bytes: &[u8]) -> Option<Self> {
        let head = std::str::from_utf8(&bytes[..bytes.len().min(8192)]).ok()?;
        // alphaTex heuristic — any backslash directive on a non-
        // comment line within the first 1 KiB.
        let looks_alphatex = head.lines().any(|l| {
            let l = l.trim_start();
            l.starts_with("\\title")
                || l.starts_with("\\tempo")
                || l.starts_with("\\tuning")
                || l.starts_with("\\subtitle")
                || l.starts_with("\\capo")
        });
        if looks_alphatex {
            return Some(Self::AlphaTex);
        }
        // ASCII tab heuristic — at least two lines matching the
        // `<short-label>|<content>` shape. Reuses the parser's own
        // detection so the sniffer and the parser agree on what
        // counts as ASCII tab.
        let tab_lines = head
            .lines()
            .filter(|l| twanga_tabs::ascii_tab::looks_like_tab_line(l))
            .count();
        if tab_lines >= 2 {
            return Some(Self::AsciiTab);
        }
        None
    }
}

/// Result of a conversion. `alphatex` is the file contents ready to
/// write; `warnings` is whatever the source parser surfaced
/// (alphaTex parsing is strict so it either succeeds clean or errors
/// out; the other formats may produce non-fatal observations).
#[derive(Debug, Clone)]
pub struct Conversion {
    pub tab: ParsedTab,
    pub alphatex: String,
    pub warnings: Vec<ParseWarning>,
}

/// Convert `input` (the raw file bytes) of the given `format` into an
/// alphaTex string. Tab metadata (title, tempo, tuning, capo) survives
/// the round-trip; per-format warnings (MusicXML's irregular-duration
/// / unreachable-note observations) propagate to the caller.
///
/// When the source format already IS alphaTex we still parse-then-
/// serialise: it normalises whitespace, ensures the file is well-
/// formed, and means the importer never lands a half-valid file in
/// the user's library. Slightly more expensive than a passthrough but
/// the correctness guarantee is worth it.
pub fn convert_to_alphatex(input: &[u8], format: SourceFormat) -> Result<Conversion> {
    match format {
        SourceFormat::AlphaTex => {
            let text = std::str::from_utf8(input)
                .map_err(|e| anyhow!("alphaTex file is not valid UTF-8: {e}"))?;
            let parsed = alphatex::parse(text).map_err(|e| anyhow!("alphaTex parse: {e}"))?;
            let alphatex = serialise(&parsed)?;
            Ok(Conversion {
                tab: parsed,
                alphatex,
                warnings: Vec::new(),
            })
        }
        SourceFormat::MusicXml => {
            let text = std::str::from_utf8(input)
                .map_err(|e| anyhow!("MusicXML file is not valid UTF-8: {e}"))?;
            let out = musicxml::parse(text).map_err(|e| anyhow!("MusicXML parse: {e}"))?;
            let alphatex = serialise(&out.tab)?;
            Ok(Conversion {
                tab: out.tab,
                alphatex,
                warnings: out.warnings,
            })
        }
        SourceFormat::Mxl => {
            let out = musicxml::parse_mxl(input).map_err(|e| anyhow!("MXL parse: {e}"))?;
            let alphatex = serialise(&out.tab)?;
            Ok(Conversion {
                tab: out.tab,
                alphatex,
                warnings: out.warnings,
            })
        }
        SourceFormat::Midi => {
            let out = midi::parse(input).map_err(|e| anyhow!("MIDI parse: {e}"))?;
            let alphatex = serialise(&out.tab)?;
            Ok(Conversion {
                tab: out.tab,
                alphatex,
                warnings: out.warnings,
            })
        }
        SourceFormat::Abc => {
            let text = std::str::from_utf8(input)
                .map_err(|e| anyhow!("ABC file is not valid UTF-8: {e}"))?;
            let out = abc::parse(text).map_err(|e| anyhow!("ABC parse: {e}"))?;
            let alphatex = serialise(&out.tab)?;
            Ok(Conversion {
                tab: out.tab,
                alphatex,
                warnings: out.warnings,
            })
        }
        SourceFormat::AsciiTab => {
            let text = std::str::from_utf8(input)
                .map_err(|e| anyhow!("ASCII tab file is not valid UTF-8: {e}"))?;
            let out = ascii_tab::parse(text).map_err(|e| anyhow!("ASCII tab parse: {e}"))?;
            let alphatex = serialise(&out.tab)?;
            Ok(Conversion {
                tab: out.tab,
                alphatex,
                warnings: out.warnings,
            })
        }
    }
}

/// Serialise a `ParsedTab` back to alphaTex via the canonical
/// `AlphaTexWriter`. Same writer the recorder uses, so files written
/// here are bit-for-bit identical to recordings that produce the
/// same notes — the importer doesn't fork the format.
///
/// The first column's `duration_denom` is taken as the file's global
/// resolution. Mixed-denominator scores (some quarter, some eighth)
/// will compress to the first value; this is a known limitation of
/// the current `AlphaTexWriter` and matches the recorder's behaviour.
fn serialise(parsed: &ParsedTab) -> Result<String> {
    let tuning = parsed
        .tuning()
        .ok_or_else(|| anyhow!("source has no parseable tuning — re-import via the GUI Importer to assign one, or pass --tuning"))?;
    let capo = parsed
        .capo()
        .unwrap_or_else(|| Capo::none(tuning.strings.len()));
    let denom = parsed
        .columns
        .first()
        .map(|c| c.duration_denom.max(1))
        .unwrap_or(8);

    let mut buf: Vec<u8> = Vec::new();
    {
        let mut writer = alphatex::AlphaTexWriter::new(
            &mut buf,
            &tuning,
            &capo,
            parsed.tempo,
            denom,
            parsed.title.as_deref(),
        )
        .context("AlphaTexWriter::new")?;
        let string_count = tuning.strings.len();
        for col in &parsed.columns {
            writer
                .write_column_with_articulation(
                    &column_to_marks(col, string_count),
                    col.articulation,
                )
                .context("AlphaTexWriter::write_column_with_articulation")?;
        }
        writer.finalize().context("AlphaTexWriter::finalize")?;
    }
    String::from_utf8(buf).map_err(|e| anyhow!("alphaTex output is not valid UTF-8: {e}"))
}

/// Convert a `TabColumn` (a `Vec<(string, fret)>`) to the
/// `&[Option<u8>]` shape `AlphaTexWriter::write_column` expects.
/// `string_count` is the tuning's string count — out-of-range string
/// numbers are silently dropped (matches the recorder's tolerance
/// for malformed source data).
fn column_to_marks(col: &twanga_tabs::TabColumn, string_count: usize) -> Vec<Option<u8>> {
    let mut marks = vec![None; string_count];
    for (string, fret) in &col.hits {
        let idx = (*string as usize).saturating_sub(1);
        if idx < marks.len() {
            marks[idx] = Some(*fret);
        }
    }
    marks
}

// ──────────────────────────── CLI entry points ─────────────────────────────

/// Body of `twanga convert <input> --out <out> [--from <format>]`.
/// Stateless: input file in, alphaTex file out, no library
/// involvement.
pub fn run_convert(input: PathBuf, out: PathBuf, from: Option<String>) -> Result<()> {
    let bytes = fs::read(&input).with_context(|| format!("failed to read {}", input.display()))?;
    let format = resolve_format(&input, from.as_deref(), &bytes)?;
    let conversion = convert_to_alphatex(&bytes, format)?;
    fs::write(&out, &conversion.alphatex)
        .with_context(|| format!("failed to write {}", out.display()))?;
    report_warnings(&conversion.warnings);
    println!(
        "converted: {} ({}) -> {}",
        input.display(),
        format_label(format),
        out.display()
    );
    Ok(())
}

/// Body of `twanga import <input> [--from <format>] [--title <text>]`.
/// Resolves the user library dir (via `twanga-paths`), derives a
/// timestamped filename, and writes the converted alphaTex there.
/// Mirrors what the GUI Importer does so a CLI import and a GUI
/// import land identically on disk.
pub fn run_import(
    input: PathBuf,
    from: Option<String>,
    title_override: Option<String>,
) -> Result<()> {
    let bytes = fs::read(&input).with_context(|| format!("failed to read {}", input.display()))?;
    let format = resolve_format(&input, from.as_deref(), &bytes)?;
    let conversion = convert_to_alphatex(&bytes, format)?;

    // Effective title: explicit --title wins, then the source's title,
    // then "imported" as a fallback. The filename slug derives from
    // whichever is set.
    let effective_title = title_override
        .or_else(|| conversion.tab.title.clone())
        .unwrap_or_else(|| "imported".to_string());

    let dir = bundled::library_dir_path();
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create library dir {}", dir.display()))?;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let slug = slugify(&effective_title);
    let filename = if slug.is_empty() {
        format!("imported-{ts}.alphatex")
    } else {
        format!("{slug}-{ts}.alphatex")
    };
    let path = dir.join(&filename);
    fs::write(&path, &conversion.alphatex)
        .with_context(|| format!("failed to write {}", path.display()))?;

    report_warnings(&conversion.warnings);
    println!(
        "imported: {} ({}) -> {}",
        input.display(),
        format_label(format),
        path.display()
    );
    Ok(())
}

/// Resolve the source format. Priority order:
/// 1. Explicit `--from` flag wins unconditionally.
/// 2. Otherwise infer from path extension via
///    [`SourceFormat::from_extension`].
/// 3. If the extension is missing or ambiguous (`.txt` is the
///    notable case — could be either alphaTex or ASCII tab),
///    fall back to content sniffing via
///    [`SourceFormat::sniff_content`].
/// 4. If none of those resolve, hard-error with the supported-format
///    list so the user knows which `--from` values are valid.
fn resolve_format(input: &Path, from: Option<&str>, content: &[u8]) -> Result<SourceFormat> {
    if let Some(f) = from {
        return SourceFormat::parse_flag(f);
    }
    if let Some(fmt) = SourceFormat::from_extension(input) {
        return Ok(fmt);
    }
    if let Some(fmt) = SourceFormat::sniff_content(content) {
        return Ok(fmt);
    }
    Err(anyhow!(
        "can't infer source format from '{}'. Pass --from alphatex / musicxml / mxl / midi / abc / ascii explicitly.",
        input.display()
    ))
}

/// Stderr-printed warning summary. Matches the CLI's habit of using
/// `eprintln!` for non-fatal observations so machine-readable stdout
/// (e.g. piping `twanga convert ... > out.alphatex`) stays clean.
fn report_warnings(warnings: &[ParseWarning]) {
    if warnings.is_empty() {
        return;
    }
    eprintln!("import: {} parse warning(s):", warnings.len());
    for w in warnings {
        match w {
            ParseWarning::IrregularDuration {
                column_index,
                raw_duration,
            } => {
                eprintln!(
                    "  col {column_index}: irregular duration '{raw_duration}' rounded to nearest power of 2"
                );
            }
            ParseWarning::UnreachableNote { column_index, note } => {
                eprintln!(
                    "  col {column_index}: note {note} unreachable on staff tuning — dropped"
                );
            }
            ParseWarning::MissingStringTuning { referenced_string } => {
                eprintln!(
                    "  string {referenced_string} referenced but no <staff-tuning> declared — string mapping may be wrong"
                );
            }
            ParseWarning::SkippedTrack { index, name } => {
                eprintln!(
                    "  track {index} ('{name}') skipped — only the first note-bearing track is imported"
                );
            }
            ParseWarning::InferredTuning { matched_name, .. } => {
                eprintln!(
                    "  source had no declared tuning — placed notes on '{matched_name}'; retune at playback with --tuning <slug>"
                );
            }
        }
    }
}

fn format_label(f: SourceFormat) -> &'static str {
    match f {
        SourceFormat::AlphaTex => "alphatex",
        SourceFormat::MusicXml => "musicxml",
        SourceFormat::Mxl => "mxl",
        SourceFormat::Midi => "midi",
        SourceFormat::Abc => "abc",
        SourceFormat::AsciiTab => "ascii-tab",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_detection_covers_known_suffixes() {
        assert_eq!(
            SourceFormat::from_extension(Path::new("foo.alphatex")),
            Some(SourceFormat::AlphaTex)
        );
        assert_eq!(
            SourceFormat::from_extension(Path::new("foo.musicxml")),
            Some(SourceFormat::MusicXml)
        );
        assert_eq!(
            SourceFormat::from_extension(Path::new("foo.xml")),
            Some(SourceFormat::MusicXml)
        );
        assert_eq!(
            SourceFormat::from_extension(Path::new("foo.mxl")),
            Some(SourceFormat::Mxl)
        );
        assert_eq!(
            SourceFormat::from_extension(Path::new("foo.mid")),
            Some(SourceFormat::Midi)
        );
        assert_eq!(
            SourceFormat::from_extension(Path::new("foo.midi")),
            Some(SourceFormat::Midi)
        );
        assert_eq!(
            SourceFormat::from_extension(Path::new("foo.abc")),
            Some(SourceFormat::Abc)
        );
        assert_eq!(
            SourceFormat::from_extension(Path::new("foo.tab")),
            Some(SourceFormat::AsciiTab)
        );
        // Case-insensitive
        assert_eq!(
            SourceFormat::from_extension(Path::new("FOO.ALPHATEX")),
            Some(SourceFormat::AlphaTex)
        );
        // Unknown
        assert_eq!(SourceFormat::from_extension(Path::new("foo.pdf")), None);
        assert_eq!(SourceFormat::from_extension(Path::new("foo")), None);
        // `.txt` is intentionally ambiguous — extension detection
        // returns None and the resolver falls back to content
        // sniffing via `SourceFormat::sniff_content`.
        assert_eq!(SourceFormat::from_extension(Path::new("foo.txt")), None);
    }

    #[test]
    fn sniff_content_recognises_alphatex_directives() {
        // Any backslash directive in the head qualifies.
        for src in &[
            "\\title \"Hello\"\n\\tempo 120\n",
            "% comment\n\\tuning E4 B3 G3 D3 A2 E2\n",
            "\\tempo 100\n.\n:4 0.6 |\n",
        ] {
            assert_eq!(
                SourceFormat::sniff_content(src.as_bytes()),
                Some(SourceFormat::AlphaTex),
                "should sniff alphaTex: {src:?}"
            );
        }
    }

    #[test]
    fn sniff_content_recognises_ascii_tab_shape() {
        let src = "e|--0--3--5--|\nB|--1--3--5--|\nG|--0--2--4--|\n";
        assert_eq!(
            SourceFormat::sniff_content(src.as_bytes()),
            Some(SourceFormat::AsciiTab)
        );
    }

    #[test]
    fn sniff_content_finds_tab_past_long_comment_header() {
        // Pad with 4 KiB of comment-shaped content before the first
        // tab line. The previous 1 KiB sniff window would have missed
        // the actual tab; the 8 KiB window catches it. Regression
        // pin: any future window-shrink will break this test.
        let mut src = String::with_capacity(8 * 1024);
        for _ in 0..50 {
            src.push_str("# this is a long credit / license / transcriber-notes header line\n");
        }
        src.push_str("e|--0--3--5--|\nB|--1--3--5--|\nG|--0--2--4--|\n");
        assert_eq!(
            SourceFormat::sniff_content(src.as_bytes()),
            Some(SourceFormat::AsciiTab)
        );
    }

    #[test]
    fn sniff_content_returns_none_for_unrecognisable() {
        // Plain prose with no directives, no tab-shaped lines.
        let src = "just some words that aren't tab or alphatex\nmore prose\n";
        assert_eq!(SourceFormat::sniff_content(src.as_bytes()), None);
    }

    #[test]
    fn parse_flag_accepts_canonical_names() {
        assert_eq!(
            SourceFormat::parse_flag("alphatex").unwrap(),
            SourceFormat::AlphaTex
        );
        assert_eq!(
            SourceFormat::parse_flag("MUSICXML").unwrap(),
            SourceFormat::MusicXml
        );
        assert_eq!(
            SourceFormat::parse_flag("xml").unwrap(),
            SourceFormat::MusicXml
        );
        assert_eq!(SourceFormat::parse_flag("mxl").unwrap(), SourceFormat::Mxl);
        assert_eq!(
            SourceFormat::parse_flag("midi").unwrap(),
            SourceFormat::Midi
        );
        assert_eq!(SourceFormat::parse_flag("mid").unwrap(), SourceFormat::Midi);
        assert_eq!(SourceFormat::parse_flag("abc").unwrap(), SourceFormat::Abc);
        assert_eq!(
            SourceFormat::parse_flag("ascii").unwrap(),
            SourceFormat::AsciiTab
        );
        assert_eq!(
            SourceFormat::parse_flag("tab").unwrap(),
            SourceFormat::AsciiTab
        );
        // Unknown
        assert!(SourceFormat::parse_flag("guitarpro").is_err());
        assert!(SourceFormat::parse_flag("").is_err());
    }

    /// End-to-end conversion of a hand-written MusicXML fixture
    /// through to an alphaTex string. Doesn't go through the
    /// filesystem — the convert_to_alphatex function is byte-in /
    /// byte-out so we can test it directly.
    #[test]
    fn musicxml_round_trips_to_alphatex() {
        const XML: &str = r#"<?xml version="1.0"?>
<score-partwise version="3.1">
  <work><work-title>Round Trip</work-title></work>
  <part id="P1">
    <measure number="1">
      <attributes>
        <divisions>4</divisions>
        <staff-details>
          <staff-tuning line="1"><tuning-step>E</tuning-step><tuning-octave>2</tuning-octave></staff-tuning>
          <staff-tuning line="2"><tuning-step>A</tuning-step><tuning-octave>2</tuning-octave></staff-tuning>
          <staff-tuning line="3"><tuning-step>D</tuning-step><tuning-octave>3</tuning-octave></staff-tuning>
          <staff-tuning line="4"><tuning-step>G</tuning-step><tuning-octave>3</tuning-octave></staff-tuning>
          <staff-tuning line="5"><tuning-step>B</tuning-step><tuning-octave>3</tuning-octave></staff-tuning>
          <staff-tuning line="6"><tuning-step>E</tuning-step><tuning-octave>4</tuning-octave></staff-tuning>
        </staff-details>
      </attributes>
      <sound tempo="100"/>
      <note>
        <pitch><step>E</step><octave>2</octave></pitch>
        <duration>4</duration>
        <technical><string>1</string><fret>0</fret></technical>
      </note>
    </measure>
  </part>
</score-partwise>"#;
        let out = convert_to_alphatex(XML.as_bytes(), SourceFormat::MusicXml).expect("convert");
        // The output should re-parse as alphaTex and preserve the
        // tempo + title.
        let reparsed = alphatex::parse(&out.alphatex).expect("re-parse");
        assert_eq!(reparsed.tempo, 100);
        assert_eq!(reparsed.title.as_deref(), Some("Round Trip"));
        // Single column, single hit. AlphaTex string indexing matches
        // MusicXML's after the parser's inversion: string=6 in TWANGA
        // (high E in MusicXML's line=1 / low E reversal becomes
        // high-string in TWANGA's top-down convention).
        assert_eq!(reparsed.columns.len(), 1);
        assert_eq!(reparsed.columns[0].hits, vec![(6, 0)]);
    }

    #[test]
    fn alphatex_passthrough_normalises_and_validates() {
        // alphaTex source → alphaTex output. The convert path should
        // accept a valid file unchanged-in-meaning even if whitespace
        // shifts slightly through the writer.
        let src = "\\title \"Hello\"\n\\subtitle \"Standard Guitar (EADGBE)\"\n\\tempo 120\n\\tuning E4 B3 G3 D3 A2 E2\n\n.\n:4 0.6 0.5 0.4 |\n";
        let out = convert_to_alphatex(src.as_bytes(), SourceFormat::AlphaTex).expect("convert");
        let reparsed = alphatex::parse(&out.alphatex).expect("re-parse");
        assert_eq!(reparsed.title.as_deref(), Some("Hello"));
        assert_eq!(reparsed.tempo, 120);
        assert_eq!(reparsed.columns.len(), 3);
    }

    #[test]
    fn convert_rejects_invalid_utf8_for_text_formats() {
        let bytes = b"\xff\xfe not utf-8";
        assert!(convert_to_alphatex(bytes, SourceFormat::AlphaTex).is_err());
        assert!(convert_to_alphatex(bytes, SourceFormat::MusicXml).is_err());
    }
}
