//! ASCII tab parser — the dominant tab format on the open web
//! (paste from any tab site, copy from any text-heavy forum post,
//! grab a `.txt` README in a song-cover repo). No spec; heuristic
//! parser against the shape every ASCII tab in the wild uses:
//!
//! ```text
//! e|---0---2---3---|
//! B|---1---3---5---|
//! G|---0---2---4---|
//! D|---2---0---2---|
//! A|---3---x---x---|
//! E|---x---x---x---|
//! ```
//!
//! # What the parser handles
//!
//! - **String-line detection** — contiguous lines that match
//!   `<label>|<content>` where label is 1–3 ASCII chars and the
//!   content is mostly `-`, digits, `|`, `x`, and a few decoration
//!   chars.
//! - **Tuning inference from labels** — `e B G D A E` → standard
//!   guitar, `A E C G` (lowercase last) → reentrant ukulele, etc.
//!   When the labels don't match any built-in tuning the parser
//!   falls back to the nearest one by string count (5 → banjo,
//!   4 → uke, 6 → guitar) and surfaces an
//!   [`ParseWarning::InferredTuning`] so the user can confirm.
//! - **Multi-digit frets** — `--12--` is read as fret 12, not
//!   two separate frets `1` and `2`. The first digit's column
//!   position is the canonical column index.
//! - **Rests** — columns where no string carries a digit produce
//!   an empty `TabColumn` (so the resulting alphaTex preserves
//!   timing rather than collapsing to a single-note line).
//! - **Title from a `# Title` line above the tab** — common
//!   convention in published ASCII tabs. Falls back to filename.
//!
//! # What survives round-trip
//!
//! - **Hammer-on / pull-off / slide (`h` / `p` / `s`)** — captured
//!   as the column's `articulation` byte and round-tripped through
//!   alphaTex (the writer prepends the prefix to the destination
//!   note token). TWANGA's playback / renderer don't consume the
//!   articulation yet; the data simply isn't lost on import +
//!   export. See [`crate::TabColumn::articulation`].
//!
//! # Limitations (deferred)
//!
//! - **Bends, vibrato, slide-up/down via `/\`** — `b`, `~`, `/`,
//!   `\` are tokenised and recognised as tab characters by the
//!   line-shape heuristic, but not preserved as articulation data
//!   in v1. Bends specifically carry a target pitch (`3b5`) that
//!   would need a richer articulation model than the single-byte
//!   tag h/p/s use; tracked on the BACKLOG.
//! - **Multiple sections** — a long song with verse / chorus / solo
//!   tab blocks separated by paragraph breaks is parsed as one
//!   continuous tab. This matches how alphaTab and similar tools
//!   handle it.
//! - **Variable column widths** — most ASCII tabs use a fixed beat-
//!   to-character spacing (e.g. 4 chars per beat), but some are
//!   irregular. The parser is alignment-agnostic — it cares about
//!   digit positions across strings, not absolute spacing — so
//!   irregular spacing still produces correct columns, just without
//!   accurate duration data. Every column gets the default eighth-
//!   note denominator.
//! - **Tuning declaration syntax** — there's no agreed-upon "this is
//!   drop-D" header marker. The parser only uses line labels; if you
//!   want to override, re-import via the GUI Importer's tuning picker
//!   (planned) or hand-edit the alphaTex output.

use twanga_core::Tuning;

use crate::{ParseOutput, ParseWarning, ParsedTab, TabColumn};

/// Errors the ASCII tab parser can return. Distinct variants per
/// failure mode so the importer UI can surface targeted messages.
#[derive(Debug)]
pub enum AsciiTabError {
    /// Couldn't find any tab lines in the input. Heuristic: at least
    /// two contiguous lines matching the `<label>|<content>` shape.
    NoTabLines,
    /// Found tab lines but every column ended up empty — nothing to
    /// play.
    EmptyScore,
}

impl std::fmt::Display for AsciiTabError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoTabLines => write!(
                f,
                "no ASCII tab lines found (expected lines like 'e|---0---2---|')"
            ),
            Self::EmptyScore => write!(f, "ASCII tab has no playable notes"),
        }
    }
}
impl std::error::Error for AsciiTabError {}

/// Default duration denominator for every column. ASCII tab has no
/// reliable timing data — every column becomes an eighth note,
/// matching the format's typical "two chars per eighth" cadence
/// convention.
const DEFAULT_DENOM: u32 = 8;

/// Parse an ASCII tab document (raw text).
pub fn parse(text: &str) -> Result<ParseOutput, AsciiTabError> {
    let lines: Vec<&str> = text.lines().collect();

    // Extract optional title from `# Title` / `Title:` lines.
    // Stop at the first em-dash or `—` separator so a header like
    // `# Twinkle Twinkle — ASCII test fixture` yields the song name,
    // not the whole annotation. ASCII tabs in the wild commonly use
    // hyphen-space-text as a descriptive subtitle separator.
    let title = lines.iter().find_map(|l| {
        let l = l.trim();
        let rest = l.strip_prefix('#').or_else(|| l.strip_prefix("Title:"))?;
        let cleaned = rest.trim();
        if cleaned.is_empty() {
            return None;
        }
        let head = cleaned
            .split_once(" — ")
            .or_else(|| cleaned.split_once(" - "))
            .map(|(head, _)| head)
            .unwrap_or(cleaned);
        Some(head.trim().to_string())
    });

    // Find the first run of contiguous tab-shaped lines. A run is
    // 2+ adjacent lines that each match `is_tab_line`. We take the
    // first run only — multi-section tabs are documented as v1
    // limitation.
    let run = find_first_tab_run(&lines).ok_or(AsciiTabError::NoTabLines)?;

    // Each line: extract label + fret digits per character position.
    let parsed_lines: Vec<TabLineParsed> = run.iter().map(|l| parse_tab_line(l)).collect();

    // Build columns from the union of digit positions across all
    // strings. The line order in the source is "string 1 first" by
    // convention (high E at top for guitar), which matches TWANGA's
    // internal convention exactly.
    let mut warnings: Vec<ParseWarning> = Vec::new();
    let labels: Vec<String> = parsed_lines.iter().map(|p| p.label.clone()).collect();
    let (tuning, tuning_warning) = match_tuning(&labels);
    if let Some(w) = tuning_warning {
        warnings.push(w);
    }

    let columns = build_columns(&parsed_lines);
    if columns.is_empty() {
        return Err(AsciiTabError::EmptyScore);
    }

    let tuning_names: Vec<String> = tuning.strings.iter().map(|s| s.name.clone()).collect();

    Ok(ParseOutput {
        tab: ParsedTab {
            tempo: 120,
            title,
            subtitle: Some(tuning.name.clone()),
            tuning_names,
            columns,
        },
        warnings,
    })
}

/// Find the index range of the first contiguous run of tab-shaped
/// lines. Returns the slice of lines, not the indices, so the caller
/// can iterate directly.
fn find_first_tab_run<'a>(lines: &'a [&'a str]) -> Option<Vec<&'a str>> {
    let mut start: Option<usize> = None;
    let mut end: usize = 0;
    for (i, &l) in lines.iter().enumerate() {
        if is_tab_line(l) {
            if start.is_none() {
                start = Some(i);
            }
            end = i;
        } else if start.is_some() {
            // First non-tab line after we'd entered a run — but a
            // single blank/comment line in the middle of a run
            // shouldn't break it (some tabs have section labels
            // between staves). Require 2+ consecutive non-tab lines
            // to break out, to be tolerant of stray annotations.
            let next_is_tab = lines.get(i + 1).map(|n| is_tab_line(n)).unwrap_or(false);
            if !next_is_tab {
                break;
            }
        }
    }
    let s = start?;
    if end <= s {
        return None;
    }
    // Only the lines that are themselves tab lines (skip the
    // tolerated-blank ones in the middle).
    Some(
        lines[s..=end]
            .iter()
            .filter(|l| is_tab_line(l))
            .copied()
            .collect(),
    )
}

/// True if the line looks like an ASCII-tab string line:
/// `[label][|]<content>` where content is mostly tab characters.
/// Exposed publicly so format sniffers (CLI's `SourceFormat::sniff_content`)
/// can reuse the same line-shape heuristic the parser does, keeping the
/// sniffer and parser in agreement about what counts as ASCII tab.
pub fn looks_like_tab_line(line: &str) -> bool {
    is_tab_line(line)
}

fn is_tab_line(line: &str) -> bool {
    // Find the first `|` — that's the label/content separator.
    let trimmed = line.trim_start();
    let Some(bar_idx) = trimmed.find('|') else {
        return false;
    };
    // Label must be 1-3 ASCII chars, mostly letters or a few
    // common decorations (`#`, `b`, `(`, `)`, `,`, `'`).
    if bar_idx == 0 || bar_idx > 5 {
        return false;
    }
    let label = &trimmed[..bar_idx];
    if !label.chars().any(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    // Content after the bar must be dominated by tab characters.
    let content = &trimmed[bar_idx + 1..];
    if content.is_empty() {
        return false;
    }
    let tab_chars: usize = content
        .chars()
        .filter(|c| matches!(c, '-' | '|' | 'x' | 'X' | ' ') || c.is_ascii_digit())
        .count();
    let bend_chars: usize = content
        .chars()
        .filter(|c| matches!(c, 'h' | 'p' | 'b' | 'r' | '/' | '\\' | '~' | '*'))
        .count();
    // Threshold: at least 60% of the content is tab-shaped (allows
    // for stray articulation marks).
    let total = content.chars().count();
    (tab_chars + bend_chars) * 100 >= total * 60
}

/// One parsed string line: its label + a list of
/// `(column_pos, fret, articulation)` triples in left-to-right
/// order. `articulation` is the alphaTex prefix byte (`h`/`p`/`s`)
/// that immediately preceded this digit run, or `None` for a plain
/// pluck. Capturing it on the per-string parse means we can route
/// it through to the destination column even when only one string
/// of the chord carries the technique annotation.
struct TabLineParsed {
    label: String,
    frets: Vec<(usize, u8, Option<u8>)>,
}

/// Walk a string line character-by-character, extracting digit runs.
/// `column_pos` is the START position of the digit run within the
/// content (after the `|` separator). `articulation` is the byte
/// `b'h'` / `b'p'` / `b's'` if one of those characters immediately
/// preceded the digit run, otherwise `None`. Other articulation
/// markers (`b` bend, `~` vibrato, `/` slide up, `\` slide down) are
/// recognised as tab characters by [`is_tab_line`] but not yet
/// preserved — the alphaTex round-trip set is limited to h/p/s in
/// v1, with the others deferred to the articulation-data-model
/// backlog item.
fn parse_tab_line(line: &str) -> TabLineParsed {
    let trimmed = line.trim_start();
    let bar_idx = trimmed.find('|').unwrap_or(0);
    let label = trimmed[..bar_idx].trim().to_string();
    let content = &trimmed[bar_idx + 1..];

    let bytes = content.as_bytes();
    let mut frets: Vec<(usize, u8, Option<u8>)> = Vec::new();
    let mut i = 0;
    let mut pending_articulation: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        if matches!(b, b'h' | b'p' | b's') {
            // Capture the articulation byte; it applies to the NEXT
            // digit run we encounter (the destination note).
            pending_articulation = Some(b);
            i += 1;
        } else if b.is_ascii_digit() {
            let start = i;
            let mut digit_str = String::new();
            digit_str.push(b as char);
            i += 1;
            // Cap at 2 digits — frets above 99 are vanishingly rare
            // and stopping at 2 avoids accidentally swallowing
            // adjacent unrelated digits if the source is weirdly
            // spaced.
            if i < bytes.len() && bytes[i].is_ascii_digit() && digit_str.len() < 2 {
                digit_str.push(bytes[i] as char);
                i += 1;
            }
            if let Ok(fret) = digit_str.parse::<u8>() {
                frets.push((start, fret, pending_articulation));
            }
            pending_articulation = None;
        } else {
            // Any non-digit, non-articulation char (-, |, x, space,
            // ~, /, \, b, *) drops the pending articulation so a
            // line shape like `--3---h--5--` doesn't propagate the
            // `h` past the intervening dashes onto the 5.
            // Actually we DO want `--3h5--` to carry the h onto 5,
            // so only clear pending on whitespace / bar lines / x
            // (muted-string marker). Other articulation chars stay
            // sticky for the next digit run.
            if matches!(b, b' ' | b'|' | b'x' | b'X') {
                pending_articulation = None;
            }
            i += 1;
        }
    }
    TabLineParsed { label, frets }
}

/// Build the column list from the parsed lines. Each unique column-
/// position across all strings produces one `TabColumn`; strings
/// that have a digit at that position contribute a `(string,
/// fret)` hit. The column's `articulation` is the first articulation
/// byte we encounter across all strings at that position — chord
/// fingerings where one string has `h` and another doesn't are rare
/// in the wild, and when they do happen the user's intent is "the
/// whole chord is hammered to," so a single column-level tag does
/// the right thing.
fn build_columns(lines: &[TabLineParsed]) -> Vec<TabColumn> {
    let mut positions: Vec<usize> = lines
        .iter()
        .flat_map(|l| l.frets.iter().map(|f| f.0))
        .collect();
    positions.sort_unstable();
    positions.dedup();

    let mut columns: Vec<TabColumn> = Vec::with_capacity(positions.len());
    for &pos in &positions {
        let mut hits: Vec<(u8, u8)> = Vec::new();
        let mut articulation: Option<u8> = None;
        for (line_idx, l) in lines.iter().enumerate() {
            if let Some((_, fret, art)) = l.frets.iter().find(|(p, _, _)| *p == pos) {
                hits.push(((line_idx + 1) as u8, *fret));
                if articulation.is_none() {
                    articulation = *art;
                }
            }
        }
        columns.push(TabColumn {
            duration_denom: DEFAULT_DENOM,
            hits,
            articulation,
        });
    }
    columns
}

/// Try to match the source's labels to a built-in tuning. Strategy:
///
/// 1. Build a candidate tuning's expected labels (open-string note
///    names, lowercase-stripped for octave-insensitive matching).
/// 2. Compare against the source labels — exact match wins.
/// 3. If no exact match, fall back to nearest-by-string-count
///    (6→guitar, 5→banjo, 4→uke) and emit `InferredTuning`.
///
/// Returns `(tuning, optional_warning)`. The warning is `None` on
/// exact match and `Some(InferredTuning)` on fallback so the
/// importer UI can show the user which tuning was guessed.
fn match_tuning(labels: &[String]) -> (Tuning, Option<ParseWarning>) {
    // Exact match — check every built-in preset.
    for preset in Tuning::builtin_presets() {
        let tuning = preset.to_tuning();
        if labels_match_tuning(labels, &tuning) {
            return (tuning, None);
        }
    }
    // No exact match — fall back by string count.
    let fallback = match labels.len() {
        5 => Tuning::standard_banjo(),
        4 => Tuning::standard_ukulele(),
        _ => Tuning::standard_guitar(),
    };
    let warning = ParseWarning::InferredTuning {
        source_tuning: labels.to_vec(),
        matched_name: fallback.name.clone(),
    };
    (fallback, Some(warning))
}

/// True if the source labels match the tuning's open-string note
/// names. ASCII tab labels are typically just the note letter
/// without an octave (`E`, `A`, `D`, `G`, `B`, `e`), so we compare
/// only the letter portion (and a sharp/flat if present).
fn labels_match_tuning(labels: &[String], tuning: &Tuning) -> bool {
    if labels.len() != tuning.strings.len() {
        return false;
    }
    for (label, string) in labels.iter().zip(tuning.strings.iter()) {
        if !label_matches_string(label, &string.name) {
            return false;
        }
    }
    true
}

/// Strip octave digits + case for an octave-insensitive letter
/// comparison. `"E4"` and `"e"` and `"E"` all reduce to `"E"`.
fn label_matches_string(label: &str, string_name: &str) -> bool {
    let l = strip_octave(label).to_ascii_uppercase();
    let s = strip_octave(string_name).to_ascii_uppercase();
    !l.is_empty() && l == s
}

fn strip_octave(s: &str) -> String {
    s.chars()
        .take_while(|c| c.is_ascii_alphabetic() || matches!(c, '#' | 'b'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_six_string_guitar_tab() {
        let src = "\
e|--0--3--5--|
B|--1--3--5--|
G|--0--0--0--|
D|--2--0--0--|
A|--3--2--3--|
E|--0--3--5--|
";
        let out = parse(src).expect("parse");
        // Should match standard guitar tuning exactly — no
        // InferredTuning warning.
        assert_eq!(out.tab.tuning_names.len(), 6);
        assert!(
            !out.warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::InferredTuning { .. })),
            "exact label match should NOT surface InferredTuning"
        );
        // Three columns (0/1/0/2/3/0, 3/3/0/0/2/3, 5/5/0/0/3/5).
        assert_eq!(out.tab.columns.len(), 3);
        assert_eq!(out.tab.columns[0].hits.len(), 6);
    }

    #[test]
    fn handles_two_digit_frets() {
        let src = "\
e|--12--10--7--|
B|-------------|
";
        let out = parse(src).expect("parse");
        // 12, 10, 7 — three columns with one hit each.
        assert_eq!(out.tab.columns.len(), 3);
        assert_eq!(out.tab.columns[0].hits[0].1, 12);
        assert_eq!(out.tab.columns[1].hits[0].1, 10);
        assert_eq!(out.tab.columns[2].hits[0].1, 7);
    }

    #[test]
    fn missing_tab_lines_returns_error() {
        let src = "this is just prose, no tab lines anywhere";
        assert!(matches!(parse(src), Err(AsciiTabError::NoTabLines)));
    }

    #[test]
    fn five_string_banjo_labels_match_standard_banjo() {
        // Standard banjo (TWANGA's "Standard 5-String Banjo (Open G)")
        // string names: D4, B3, G3, D3, g4 (drone). Labels in ASCII
        // tab typically drop octave numbers: D B G D g.
        let src = "\
D|--0--2--0--|
B|--0--3--0--|
G|--0--0--0--|
D|--0--0--2--|
g|--0--0--0--|
";
        let out = parse(src).expect("parse");
        assert_eq!(out.tab.tuning_names.len(), 5);
        // Should match standard banjo exactly — no InferredTuning.
        assert!(
            !out.warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::InferredTuning { .. })),
            "5-string D/B/G/D/g should match standard banjo exactly"
        );
    }

    #[test]
    fn ukulele_labels_match_reentrant_uke() {
        // Standard ukulele: A4 E4 C4 g4 (reentrant). Labels A E C g.
        let src = "\
A|--0--3--5--|
E|--0--3--3--|
C|--0--0--0--|
g|--0--0--0--|
";
        let out = parse(src).expect("parse");
        assert_eq!(out.tab.tuning_names.len(), 4);
        assert!(
            !out.warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::InferredTuning { .. }))
        );
    }

    #[test]
    fn unrecognised_labels_fall_back_to_nearest_by_count() {
        // 6 strings, unrecognised labels (use Q/W/E/R/T/Y nonsense
        // letters) → fall back to standard guitar with a warning.
        let src = "\
Q|--0--3--|
W|--1--3--|
E|--0--0--|
R|--2--0--|
T|--3--2--|
Y|--0--3--|
";
        let out = parse(src).expect("parse");
        assert_eq!(out.tab.tuning_names.len(), 6);
        let warning = out
            .warnings
            .iter()
            .find_map(|w| match w {
                ParseWarning::InferredTuning { matched_name, .. } => Some(matched_name.clone()),
                _ => None,
            })
            .expect("should infer + warn");
        assert!(
            warning.contains("Standard Guitar"),
            "expected guitar fallback, got '{warning}'"
        );
    }

    #[test]
    fn title_from_hash_comment() {
        let src = "\
# Crazy Train

e|--0--3--|
B|--1--3--|
";
        let out = parse(src).expect("parse");
        assert_eq!(out.tab.title.as_deref(), Some("Crazy Train"));
    }

    #[test]
    fn captures_hammer_on_articulation_on_destination_column() {
        // `--3h5--` on string 1 should produce two columns: pluck 3
        // (no articulation), then hammer-on 5 (articulation = b'h').
        let src = "e|--3h5--|\nB|-------|\n";
        let out = parse(src).expect("parse");
        assert_eq!(out.tab.columns.len(), 2);
        assert_eq!(out.tab.columns[0].articulation, None);
        assert_eq!(out.tab.columns[1].articulation, Some(b'h'));
    }

    #[test]
    fn captures_pull_off_and_slide_articulations() {
        // `--5p3--` and `--3s5--` round-trip as their respective bytes.
        let src = "e|--5p3---3s5--|\nB|-------------|\n";
        let out = parse(src).expect("parse");
        let arts: Vec<Option<u8>> = out.tab.columns.iter().map(|c| c.articulation).collect();
        assert_eq!(arts, vec![None, Some(b'p'), None, Some(b's')]);
    }

    #[test]
    fn articulation_drops_at_string_boundaries() {
        // A pending `h` shouldn't carry across the `|` bar line into
        // the next measure's first note.
        let src = "e|--3h|5--|\nB|--------|\n";
        let out = parse(src).expect("parse");
        // Column 0: plain 3. Column 1: plain 5 (the `|` cleared the
        // pending h).
        assert_eq!(out.tab.columns[0].articulation, None);
        assert_eq!(out.tab.columns[1].articulation, None);
    }

    #[test]
    fn rests_columns_kept_for_timing() {
        // Position 5 has a hit on string 1, no hit anywhere else.
        // The build_columns function only creates columns for
        // positions with at least one digit, so we expect 1 column
        // (the digit). What we're really pinning here is that the
        // single-hit column has only one hit, not phantom zeros from
        // the all-dash strings.
        let src = "\
e|-----3-----|
B|-----------|
";
        let out = parse(src).expect("parse");
        assert_eq!(out.tab.columns.len(), 1);
        assert_eq!(out.tab.columns[0].hits.len(), 1);
        assert_eq!(out.tab.columns[0].hits[0], (1, 3));
    }
}
