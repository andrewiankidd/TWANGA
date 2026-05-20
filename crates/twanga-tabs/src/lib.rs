//! Tab data: live capture from an audio stream + format parsers/serialisers.
//!
//! - [`TabRecorder`] turns a stream of `(string_idx, fret, time)` events into
//!   horizontal ASCII tab notation, emitting one [`TabEvent`] per column tick.
//! - [`alphatex`] is the format TWANGA ships against today (W3C-style open
//!   text format from the alphaTab project). [`musicxml`] is a placeholder
//!   for future open-standard interop with MuseScore / Sibelius / Guitar Pro
//!   exports. Proprietary binary formats (Guitar Pro `.gp5`/`.gpx`) are an
//!   explicit non-goal — see `docs/SCOPE.md`.

use twanga_core::Tuning;

pub mod musicxml {
    // MusicXML parser — placeholder. Open W3C-style XML schema; the natural
    // open-standard interop point with sheet-music editors.
}

pub mod alphatex {
    //! alphaTab's text format — streaming serialiser ([`AlphaTexWriter`]) plus a
    //! parser ([`parse`]) for the subset emitted by the writer (tempo, tuning,
    //! `:N` durations, `fret.string` notes, `(...)` chords, `r` rests, `|` bars).

    use std::io::{self, Write};
    use twanga_core::{
        Capo, MidiNote, TunedString, Tuning, join_capo_into_subtitle, split_capo_from_subtitle,
    };

    /// Parsed alphaTex file.
    #[derive(Debug, Clone)]
    pub struct ParsedTab {
        pub tempo: u32,
        /// Optional `\title "..."` text — the human-readable name the user
        /// gave to this recording (e.g. `"Cripple Creek take 3"`). Distinct
        /// from `subtitle` which TWANGA uses for the tuning name + capo
        /// annotation; `title` is purely the song / take name. `None` for
        /// pre-title-feature recordings that didn't write one.
        pub title: Option<String>,
        /// Optional `\subtitle "..."` text. Used by TWANGA to label the recording
        /// with its tuning name (e.g. `"Standard Ukulele (Reentrant GCEA)"`), so
        /// later playback can show the user what the recording was made against
        /// without parsing the `\tuning` line.
        pub subtitle: Option<String>,
        /// Open-string note names in string-number order (1-based), e.g. `["A4","E4","C4","G4"]`.
        pub tuning_names: Vec<String>,
        pub columns: Vec<TabColumn>,
    }

    /// One column from a parsed tab.
    #[derive(Debug, Clone)]
    pub struct TabColumn {
        /// Duration denominator (`4` = quarter, `8` = eighth, …).
        pub duration_denom: u32,
        /// `(string_number_1_based, fret)` for each hit. Empty = rest.
        pub hits: Vec<(u8, u8)>,
    }

    impl ParsedTab {
        /// Extract the recorded capo (if any) from the subtitle field. TWANGA's
        /// recorder embeds `; capo=<spec>` after the human-readable tuning name
        /// since alphaTex has no native `\capo` directive. Returns `None` if
        /// the subtitle is missing, lacks the token, or carries a spec that
        /// doesn't match the tuning's string count.
        pub fn capo(&self) -> Option<Capo> {
            let st = self.subtitle.as_deref()?;
            let (_, raw) = split_capo_from_subtitle(st);
            let spec = raw?;
            Capo::parse(&spec, self.tuning_names.len()).ok()
        }

        /// The subtitle string with any `; capo=...` annotation stripped.
        /// Useful when you want to render just the human-readable name.
        pub fn subtitle_display(&self) -> Option<String> {
            self.subtitle
                .as_deref()
                .map(|s| split_capo_from_subtitle(s).0)
        }

        /// Build a `Tuning` from the file's `\tuning` header. Returns `None`
        /// if the header is missing or contains note names we can't parse.
        pub fn tuning(&self) -> Option<Tuning> {
            if self.tuning_names.is_empty() {
                return None;
            }
            let mut strings = Vec::with_capacity(self.tuning_names.len());
            for n in &self.tuning_names {
                let midi = MidiNote::from_name(n)?;
                strings.push(TunedString {
                    name: n.clone(),
                    open: midi,
                });
            }
            Some(Tuning {
                name: "(from alphaTex)".to_string(),
                strings,
            })
        }

        /// Re-fret every note onto `target`'s strings. For each `(string, fret)`
        /// in the source we compute the absolute MIDI pitch via the source
        /// tuning, then call `target.match_to_fret(...)` to find the smallest
        /// non-negative fret position on the target. Pitches that can't be
        /// reached within `max_fret` are dropped.
        ///
        /// Returns the source unchanged if the file has no parseable tuning
        /// header (nothing to transpose against).
        pub fn transpose_to(&self, target: &Tuning, max_fret: u8) -> ParsedTab {
            self.transpose_to_with_report(target, max_fret).0
        }

        /// Same as [`Self::transpose_to`] but also returns the list of notes
        /// that couldn't be reached on the target tuning. The CLI's `play
        /// --tuning <other>` uses the report to surface a pre-flight summary
        /// so the user knows what will be missing before the cursor starts.
        /// Each entry is `(column_index, note_name)` — column index is the
        /// 0-based position in the source tab, note name is the pitch we
        /// failed to place (e.g. `"E6"`).
        ///
        /// Defaults to [`TransposeMode::Drop`]. Use
        /// [`Self::transpose_to_with_mode`] to pick `OctaveShift` instead.
        pub fn transpose_to_with_report(
            &self,
            target: &Tuning,
            max_fret: u8,
        ) -> (ParsedTab, Vec<DroppedNote>) {
            self.transpose_to_with_mode(target, max_fret, TransposeMode::Drop)
        }

        /// Mode-aware transpose. With `TransposeMode::Drop` (the
        /// default) a note that doesn't fit on the target tuning's
        /// fretboard is silently omitted and reported in the
        /// `Vec<DroppedNote>`. With `TransposeMode::OctaveShift` the
        /// transposer retries the note at successively wider
        /// ±12-semitone offsets before giving up — preserving the
        /// melodic contour at the cost of register. Notes that still
        /// can't be placed after ±96 semitones (8 octaves, well past
        /// any real instrument's range) fall through to the drop
        /// report.
        pub fn transpose_to_with_mode(
            &self,
            target: &Tuning,
            max_fret: u8,
            mode: TransposeMode,
        ) -> (ParsedTab, Vec<DroppedNote>) {
            let Some(source) = self.tuning() else {
                return (self.clone(), Vec::new());
            };

            let mut dropped: Vec<DroppedNote> = Vec::new();
            let new_columns: Vec<TabColumn> = self
                .columns
                .iter()
                .enumerate()
                .map(|(col_idx, col)| {
                    let mut new_hits = Vec::with_capacity(col.hits.len());
                    for (string, fret) in &col.hits {
                        let source_idx = (*string as usize).saturating_sub(1);
                        let Some(source_string) = source.strings.get(source_idx) else {
                            continue;
                        };
                        let abs_midi = source_string.open.0 as i32 + *fret as i32;
                        if !(0..=127).contains(&abs_midi) {
                            // Out of MIDI range entirely — report as a drop
                            // with a synthetic name so the user at least sees
                            // *something* was unreachable. Realistically this
                            // branch is unreachable for any musical input.
                            dropped.push(DroppedNote {
                                column_index: col_idx,
                                note: format!("midi-{abs_midi}"),
                            });
                            continue;
                        }
                        if let Some(placement) =
                            place_with_mode(target, abs_midi, max_fret, mode)
                        {
                            new_hits.push(placement);
                        } else {
                            dropped.push(DroppedNote {
                                column_index: col_idx,
                                note: MidiNote(abs_midi as u8).name(),
                            });
                        }
                    }
                    TabColumn {
                        duration_denom: col.duration_denom,
                        hits: new_hits,
                    }
                })
                .collect();

            (
                ParsedTab {
                    tempo: self.tempo,
                    title: self.title.clone(),
                    // Preserve the original subtitle. The header line in `twanga play`
                    // also surfaces the transposition explicitly, so the user always
                    // sees both "what was recorded against" and "what's being played
                    // now."
                    subtitle: self.subtitle.clone(),
                    tuning_names: target.strings.iter().map(|s| s.open.name()).collect(),
                    columns: new_columns,
                },
                dropped,
            )
        }
    }

    /// Strategy for handling notes that don't fit on the target tuning's
    /// fretboard during transposition. See
    /// [`ParsedTab::transpose_to_with_mode`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub enum TransposeMode {
        /// Silently drop notes that can't be placed on the target. Notes
        /// still appear in the dropped-notes report. This is the
        /// historical (and default) behaviour.
        #[default]
        Drop,
        /// Try the original pitch first; if it doesn't fit, retry at
        /// progressively wider ±12-semitone offsets before falling back
        /// to dropping. Preserves melodic contour at the cost of
        /// register — appropriate for cross-instrument playback like
        /// banjo→ukulele where bass notes would otherwise vanish.
        OctaveShift,
    }

    /// Internal placement helper used by `transpose_to_with_mode`. For
    /// `Drop` mode it's a single `match_to_fret` call at the source
    /// pitch; for `OctaveShift` it tries the source pitch first, then
    /// expands outward by ±12 semitones up to 8 octaves on either side,
    /// picking the smallest-magnitude shift that fits.
    fn place_with_mode(
        target: &Tuning,
        abs_midi: i32,
        max_fret: u8,
        mode: TransposeMode,
    ) -> Option<(u8, u8)> {
        let try_at = |midi: i32| -> Option<(u8, u8)> {
            if !(0..=127).contains(&midi) {
                return None;
            }
            let freq = MidiNote(midi as u8).to_frequency();
            target
                .match_to_fret(freq, max_fret)
                .map(|m| ((m.string_idx + 1) as u8, m.fret))
        };
        if let Some(p) = try_at(abs_midi) {
            return Some(p);
        }
        if matches!(mode, TransposeMode::OctaveShift) {
            for octaves in 1..=8 {
                // Try the upward shift first so a too-low note (the
                // common banjo→uke case) prefers shifting up, which
                // keeps the melody recognisable. Too-high notes will
                // fail the upward shift and fall through to downward.
                if let Some(p) = try_at(abs_midi + 12 * octaves) {
                    return Some(p);
                }
                if let Some(p) = try_at(abs_midi - 12 * octaves) {
                    return Some(p);
                }
            }
        }
        None
    }

    /// A note from the source tab that couldn't be placed on the target
    /// tuning during transposition. Returned by
    /// [`ParsedTab::transpose_to_with_report`].
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct DroppedNote {
        /// 0-based column index in the source tab.
        pub column_index: usize,
        /// Note name we failed to place (e.g. `"E6"`).
        pub note: String,
    }

    #[derive(Debug)]
    pub enum ParseError {
        BadTempo(String),
        BadDuration(String),
        BadNote(String),
    }

    impl std::fmt::Display for ParseError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::BadTempo(s) => write!(f, "bad tempo: {s}"),
                Self::BadDuration(s) => write!(f, "bad duration: {s}"),
                Self::BadNote(s) => write!(f, "bad note token: {s}"),
            }
        }
    }
    impl std::error::Error for ParseError {}

    /// Parse the subset of alphaTex that [`AlphaTexWriter`] emits.
    pub fn parse(input: &str) -> Result<ParsedTab, ParseError> {
        let mut tempo: Option<u32> = None;
        let mut title: Option<String> = None;
        let mut subtitle: Option<String> = None;
        let mut tuning_names: Vec<String> = Vec::new();
        let mut columns: Vec<TabColumn> = Vec::new();
        let mut current_duration: u32 = 4;
        let mut in_body = false;

        for line in input.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("//") {
                continue;
            }
            if !in_body {
                if let Some(rest) = trimmed.strip_prefix("\\tempo") {
                    tempo = Some(
                        rest.trim()
                            .parse()
                            .map_err(|_| ParseError::BadTempo(rest.trim().to_string()))?,
                    );
                } else if let Some(rest) = trimmed.strip_prefix("\\title") {
                    // `\title` is captured for display ("Title:" line on play
                    // headers, future Playback screen heading). Stored as the
                    // unquoted string; older recordings without a `\title`
                    // round-trip cleanly with `None`.
                    title = Some(unquote(rest.trim()).to_string());
                } else if let Some(rest) = trimmed.strip_prefix("\\subtitle") {
                    subtitle = Some(unquote(rest.trim()).to_string());
                } else if let Some(rest) = trimmed.strip_prefix("\\tuning") {
                    tuning_names = rest.split_whitespace().map(String::from).collect();
                } else if trimmed == "." {
                    in_body = true;
                }
                continue;
            }
            for token in tokenize_body(trimmed) {
                process_body_token(&token, &mut current_duration, &mut columns)?;
            }
        }

        Ok(ParsedTab {
            tempo: tempo.unwrap_or(120),
            title,
            subtitle,
            tuning_names,
            columns,
        })
    }

    /// Strip surrounding double-quotes from a string, if present.
    fn unquote(s: &str) -> &str {
        if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
            &s[1..s.len() - 1]
        } else {
            s
        }
    }

    fn tokenize_body(line: &str) -> Vec<String> {
        let mut tokens: Vec<String> = Vec::new();
        let mut chars = line.chars().peekable();
        let mut current = String::new();
        while let Some(c) = chars.next() {
            if c.is_whitespace() {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            } else if c == '(' {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                current.push('(');
                for next in chars.by_ref() {
                    current.push(next);
                    if next == ')' {
                        break;
                    }
                }
                tokens.push(std::mem::take(&mut current));
            } else {
                current.push(c);
            }
        }
        if !current.is_empty() {
            tokens.push(current);
        }
        tokens
    }

    fn process_body_token(
        token: &str,
        current_duration: &mut u32,
        columns: &mut Vec<TabColumn>,
    ) -> Result<(), ParseError> {
        if let Some(rest) = token.strip_prefix(':') {
            *current_duration = rest
                .parse()
                .map_err(|_| ParseError::BadDuration(rest.to_string()))?;
            return Ok(());
        }
        if token == "|" {
            return Ok(());
        }
        if token == "r" {
            columns.push(TabColumn {
                duration_denom: *current_duration,
                hits: vec![],
            });
            return Ok(());
        }
        if token.starts_with('(') {
            let inner = token.trim_start_matches('(').trim_end_matches(')');
            let mut hits = Vec::new();
            for note_str in inner.split_whitespace() {
                hits.push(parse_note(note_str)?);
            }
            columns.push(TabColumn {
                duration_denom: *current_duration,
                hits,
            });
            return Ok(());
        }
        if let Ok(note) = parse_note(token) {
            columns.push(TabColumn {
                duration_denom: *current_duration,
                hits: vec![note],
            });
            return Ok(());
        }
        // Unknown token — silently skip.
        Ok(())
    }

    fn parse_note(s: &str) -> Result<(u8, u8), ParseError> {
        let mut parts = s.split('.');
        let fret = parts
            .next()
            .ok_or_else(|| ParseError::BadNote(s.to_string()))?
            .parse::<u8>()
            .map_err(|_| ParseError::BadNote(s.to_string()))?;
        let string = parts
            .next()
            .ok_or_else(|| ParseError::BadNote(s.to_string()))?
            .parse::<u8>()
            .map_err(|_| ParseError::BadNote(s.to_string()))?;
        if parts.next().is_some() {
            return Err(ParseError::BadNote(s.to_string()));
        }
        Ok((string, fret))
    }

    /// Streaming writer that turns column-by-column tab marks into alphaTex.
    ///
    /// Call [`Self::write_column`] once per recorder column; the writer emits
    /// the matching note / chord / rest at the configured duration and inserts
    /// bar lines based on a 4/4 time signature and the resolution denominator
    /// (e.g. `8` for 1/8 notes → 8 columns per bar).
    pub struct AlphaTexWriter<W: Write> {
        writer: W,
        resolution_denom: u32,
        columns_per_bar: usize,
        cols_in_current_bar: usize,
        duration_emitted: bool,
    }

    impl<W: Write> AlphaTexWriter<W> {
        pub fn new(
            mut writer: W,
            tuning: &Tuning,
            capo: &Capo,
            bpm: u32,
            resolution_denom: u32,
            title: Option<&str>,
        ) -> io::Result<Self> {
            // `\title` goes first when present — it's the user's chosen name
            // for the recording (e.g. "Cripple Creek take 3"). Optional;
            // pre-title-feature recordings just don't have one and round-trip
            // cleanly as `None`.
            if let Some(t) = title.map(str::trim).filter(|t| !t.is_empty()) {
                writeln!(writer, "\\title \"{}\"", t.replace('"', "\\\""))?;
            }
            // Subtitle gives the file a human-readable label for the tuning it
            // was recorded against, so anything that opens it later (alphaTab,
            // a text editor, our own play command) can show "Standard Ukulele"
            // without re-deriving it from the `\tuning` notes. When a capo is
            // present, we co-opt the same field with a `; capo=<spec>` suffix
            // (since alphaTex has no native capo directive) — alphaTab still
            // renders the whole string as a subtitle, our parser pulls the
            // capo back out via `split_capo_from_subtitle`.
            let subtitle = join_capo_into_subtitle(&tuning.name, capo);
            writeln!(writer, "\\subtitle \"{}\"", subtitle.replace('"', "\\\""))?;
            writeln!(writer, "\\tempo {bpm}")?;
            let tuning_str: Vec<String> = tuning.strings.iter().map(|s| s.open.name()).collect();
            writeln!(writer, "\\tuning {}", tuning_str.join(" "))?;
            writeln!(writer)?;
            writeln!(writer, ".")?;

            // 4/4 time at `resolution_denom` note value per column:
            // 1/4 → 4 cols/bar, 1/8 → 8 cols/bar, 1/16 → 16 cols/bar, etc.
            let columns_per_bar = resolution_denom as usize;

            Ok(Self {
                writer,
                resolution_denom,
                columns_per_bar,
                cols_in_current_bar: 0,
                duration_emitted: false,
            })
        }

        /// Write one column. `marks[i] = Some(fret)` means string `i+1` was hit
        /// at that fret; `None` means the string was not played.
        pub fn write_column(&mut self, marks: &[Option<u8>]) -> io::Result<()> {
            // Emit the duration prefix once per bar. AlphaTex carries duration
            // forward until changed, so we don't need to repeat it per note —
            // just once after each bar boundary.
            if !self.duration_emitted || self.cols_in_current_bar == 0 {
                write!(self.writer, ":{} ", self.resolution_denom)?;
                self.duration_emitted = true;
            }

            let hits: Vec<(usize, u8)> = marks
                .iter()
                .enumerate()
                .filter_map(|(i, m)| m.map(|f| (i + 1, f)))
                .collect();

            match hits.len() {
                0 => write!(self.writer, "r ")?,
                1 => write!(self.writer, "{}.{} ", hits[0].1, hits[0].0)?,
                _ => {
                    write!(self.writer, "(")?;
                    for (i, (string, fret)) in hits.iter().enumerate() {
                        if i > 0 {
                            write!(self.writer, " ")?;
                        }
                        write!(self.writer, "{fret}.{string}")?;
                    }
                    write!(self.writer, ") ")?;
                }
            }

            self.cols_in_current_bar += 1;
            if self.cols_in_current_bar >= self.columns_per_bar {
                writeln!(self.writer, "|")?;
                self.cols_in_current_bar = 0;
            }

            Ok(())
        }

        /// Finish the current bar (if any partial), then flush.
        pub fn finalize(&mut self) -> io::Result<()> {
            if self.cols_in_current_bar > 0 {
                writeln!(self.writer)?;
            }
            self.writer.flush()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn uke() -> Tuning {
            Tuning::standard_ukulele()
        }

        fn write_and_collect<F>(f: F) -> String
        where
            F: FnOnce(&mut AlphaTexWriter<&mut Vec<u8>>) -> io::Result<()>,
        {
            let mut buf = Vec::new();
            {
                let mut w = AlphaTexWriter::new(
                    &mut buf,
                    &uke(),
                    &Capo::none(uke().strings.len()),
                    120,
                    8,
                    None,
                )
                .unwrap();
                f(&mut w).unwrap();
                w.finalize().unwrap();
            }
            String::from_utf8(buf).unwrap()
        }

        #[test]
        fn alphatex_writes_header_with_tempo_and_tuning() {
            let out = write_and_collect(|_| Ok(()));
            assert!(out.contains("\\tempo 120"));
            assert!(out.contains("\\tuning A4 E4 C4 G4"));
            assert!(out.contains("\n.\n"));
        }

        #[test]
        fn alphatex_empty_column_is_a_rest() {
            let out = write_and_collect(|w| w.write_column(&[None, None, None, None]));
            assert!(out.contains(":8 "), "duration prefix missing: {out}");
            assert!(out.contains(" r "));
        }

        #[test]
        fn alphatex_single_hit_emits_fret_dot_string() {
            // String 1 (A4), fret 5
            let out = write_and_collect(|w| w.write_column(&[Some(5), None, None, None]));
            assert!(out.contains("5.1 "), "expected '5.1' in: {out}");
        }

        #[test]
        fn alphatex_multiple_hits_emit_chord_parens() {
            // Open uke chord: 2.1 0.2 0.3 0.4 (A2nd fret, E open, C open, g open)
            let out = write_and_collect(|w| w.write_column(&[Some(2), Some(0), Some(0), Some(0)]));
            assert!(
                out.contains("(2.1 0.2 0.3 0.4)"),
                "expected chord in: {out}"
            );
        }

        #[test]
        fn alphatex_inserts_bar_line_after_resolution_columns() {
            // 1/8 resolution → 8 cols/bar. Write 8 rests, expect one bar line.
            let out = write_and_collect(|w| {
                for _ in 0..8 {
                    w.write_column(&[None; 4])?;
                }
                Ok(())
            });
            assert_eq!(
                out.matches('|').count(),
                1,
                "expected one bar line in: {out}"
            );
        }

        #[test]
        fn alphatex_finalize_terminates_partial_bar_with_newline() {
            let out = write_and_collect(|w| {
                w.write_column(&[None; 4])?; // 1 of 8 cols in the bar
                Ok(())
            });
            // No bar line yet (only 1 of 8 columns), but finalize should add a newline.
            assert_eq!(out.matches('|').count(), 0);
            assert!(out.ends_with('\n'));
        }

        // ---- Parser tests ----

        #[test]
        fn parser_extracts_tempo_and_tuning_from_header() {
            let input = "\\tempo 90\n\\tuning A4 E4 C4 G4\n\n.\n";
            let parsed = parse(input).unwrap();
            assert_eq!(parsed.tempo, 90);
            assert_eq!(parsed.tuning_names, vec!["A4", "E4", "C4", "G4"]);
            assert_eq!(parsed.subtitle, None);
            assert!(parsed.columns.is_empty());
        }

        #[test]
        fn parser_extracts_subtitle_with_quotes() {
            let input = "\\subtitle \"Standard Ukulele\"\n\\tempo 120\n\\tuning A4 E4 C4 G4\n.\n";
            let parsed = parse(input).unwrap();
            assert_eq!(parsed.subtitle.as_deref(), Some("Standard Ukulele"));
        }

        #[test]
        fn parser_extracts_title_with_quotes() {
            let input = "\\title \"Cripple Creek take 3\"\n\\tempo 120\n\\tuning A4 E4 C4 G4\n.\n";
            let parsed = parse(input).unwrap();
            assert_eq!(parsed.title.as_deref(), Some("Cripple Creek take 3"));
        }

        #[test]
        fn parser_title_is_none_for_pre_title_files() {
            // Older recordings (no `\title` line) parse cleanly with `None`,
            // preserving the same shape as the pre-feature codebase.
            let input = "\\subtitle \"Standard Uke\"\n\\tempo 120\n\\tuning A4 E4 C4 G4\n.\n";
            let parsed = parse(input).unwrap();
            assert_eq!(parsed.title, None);
        }

        #[test]
        fn writer_emits_title_when_provided() {
            let mut buf = Vec::new();
            {
                let mut w = AlphaTexWriter::new(
                    &mut buf,
                    &uke(),
                    &Capo::none(uke().strings.len()),
                    120,
                    8,
                    Some("Cripple Creek take 3"),
                )
                .unwrap();
                w.finalize().unwrap();
            }
            let text = String::from_utf8(buf).unwrap();
            assert!(
                text.contains("\\title \"Cripple Creek take 3\""),
                "expected \\title line in: {text}"
            );
        }

        #[test]
        fn writer_omits_title_line_when_none() {
            let mut buf = Vec::new();
            {
                let mut w = AlphaTexWriter::new(
                    &mut buf,
                    &uke(),
                    &Capo::none(uke().strings.len()),
                    120,
                    8,
                    None,
                )
                .unwrap();
                w.finalize().unwrap();
            }
            let text = String::from_utf8(buf).unwrap();
            assert!(
                !text.contains("\\title"),
                "expected no \\title line in: {text}"
            );
        }

        #[test]
        fn writer_treats_blank_title_as_no_title() {
            // Empty / whitespace-only titles are a frequent UX edge case
            // (user hits enter on the prompt without typing anything). Treat
            // them as "no title" rather than writing `\title ""` which would
            // be technically valid but useless.
            let mut buf = Vec::new();
            {
                let mut w = AlphaTexWriter::new(
                    &mut buf,
                    &uke(),
                    &Capo::none(uke().strings.len()),
                    120,
                    8,
                    Some("   "),
                )
                .unwrap();
                w.finalize().unwrap();
            }
            let text = String::from_utf8(buf).unwrap();
            assert!(!text.contains("\\title"), "blank title was emitted: {text}");
        }

        #[test]
        fn writer_escapes_quotes_in_title() {
            let mut buf = Vec::new();
            {
                let mut w = AlphaTexWriter::new(
                    &mut buf,
                    &uke(),
                    &Capo::none(uke().strings.len()),
                    120,
                    8,
                    Some(r#"My "Quoted" Title"#),
                )
                .unwrap();
                w.finalize().unwrap();
            }
            let text = String::from_utf8(buf).unwrap();
            assert!(
                text.contains(r#"\title "My \"Quoted\" Title""#),
                "expected escaped quotes in title line: {text}"
            );
        }

        #[test]
        fn title_round_trips_through_writer_and_parser() {
            // Write + parse cycle should preserve the title verbatim. Same
            // pattern as the capo round-trip test above.
            let mut buf = Vec::new();
            {
                let mut w = AlphaTexWriter::new(
                    &mut buf,
                    &uke(),
                    &Capo::none(uke().strings.len()),
                    120,
                    8,
                    Some("Cripple Creek take 3"),
                )
                .unwrap();
                w.write_column(&[Some(0), None, None, None]).unwrap();
                w.finalize().unwrap();
            }
            let text = String::from_utf8(buf).unwrap();
            let parsed = parse(&text).unwrap();
            assert_eq!(parsed.title.as_deref(), Some("Cripple Creek take 3"));
        }

        #[test]
        fn writer_emits_subtitle_from_tuning_name() {
            let mut buf = Vec::new();
            {
                let mut w = AlphaTexWriter::new(
                    &mut buf,
                    &uke(),
                    &Capo::none(uke().strings.len()),
                    120,
                    8,
                    None,
                )
                .unwrap();
                w.finalize().unwrap();
            }
            let text = String::from_utf8(buf).unwrap();
            assert!(
                text.contains("\\subtitle \"Standard Ukulele (Reentrant GCEA)\""),
                "expected \\subtitle line in: {text}"
            );
        }

        #[test]
        fn writer_encodes_capo_into_subtitle_when_non_zero() {
            let mut buf = Vec::new();
            let uke_t = uke();
            let capo = Capo::uniform(uke_t.strings.len(), 3);
            {
                let mut w = AlphaTexWriter::new(&mut buf, &uke_t, &capo, 120, 8, None).unwrap();
                w.finalize().unwrap();
            }
            let text = String::from_utf8(buf).unwrap();
            assert!(
                text.contains("\\subtitle \"Standard Ukulele (Reentrant GCEA); capo=3\""),
                "expected capo-annotated subtitle in: {text}"
            );
            // `\tuning` line is still the BASE pitches — the whole point of
            // the subtitle convention is that we don't bake the capo into the
            // string pitches, so the file round-trips on a different capo.
            assert!(
                text.contains("\\tuning A4 E4 C4 G4"),
                "tuning line was: {text}"
            );
        }

        #[test]
        fn parser_round_trips_capo_through_subtitle() {
            // Partial capo: banjo body capo at fret 3, 5th-string drone left open.
            let mut buf = Vec::new();
            let banjo = Tuning::standard_banjo();
            let capo = Capo {
                offsets: vec![3, 3, 3, 3, 0],
            };
            {
                let mut w = AlphaTexWriter::new(&mut buf, &banjo, &capo, 110, 8, None).unwrap();
                w.finalize().unwrap();
            }
            let text = String::from_utf8(buf).unwrap();
            let parsed = parse(&text).unwrap();
            let recovered = parsed.capo().expect("parsed capo");
            assert_eq!(recovered, capo);
            // subtitle_display drops the machine annotation, leaving the
            // human-readable name as it was before capo support.
            assert_eq!(
                parsed.subtitle_display().as_deref(),
                Some("Standard 5-String Banjo (Open G)"),
            );
        }

        #[test]
        fn parser_returns_no_capo_for_pre_capo_files() {
            // Older recordings (no capo support) had a plain subtitle without
            // any `; capo=` token. Loading such a file must keep working.
            let input = "\\subtitle \"Standard Ukulele\"\n\\tempo 120\n\\tuning A4 E4 C4 G4\n.\n";
            let parsed = parse(input).unwrap();
            assert!(parsed.capo().is_none());
            assert_eq!(
                parsed.subtitle_display().as_deref(),
                Some("Standard Ukulele")
            );
        }

        #[test]
        fn parser_handles_rests_notes_and_chords() {
            let input = "\\tempo 120\n\\tuning A4 E4 C4 G4\n.\n:8 r 5.1 (2.1 0.2 0.3 0.4) |\n";
            let parsed = parse(input).unwrap();
            assert_eq!(parsed.columns.len(), 3);
            assert!(parsed.columns[0].hits.is_empty()); // rest
            assert_eq!(parsed.columns[1].hits, vec![(1, 5)]); // string 1, fret 5
            assert_eq!(parsed.columns[2].hits, vec![(1, 2), (2, 0), (3, 0), (4, 0)]);
            assert_eq!(parsed.columns[1].duration_denom, 8);
        }

        #[test]
        fn parser_carries_duration_across_bars() {
            // Duration set once at start should carry forward.
            let input =
                "\\tempo 120\n\\tuning A4 E4 C4 G4\n.\n:8 r r r r r r r r |\nr r r r r r r r |\n";
            let parsed = parse(input).unwrap();
            assert_eq!(parsed.columns.len(), 16);
            for c in &parsed.columns {
                assert_eq!(c.duration_denom, 8);
            }
        }

        #[test]
        fn parser_ignores_comments_and_blank_lines() {
            let input = "// header comment\n\\tempo 120\n\\tuning A4 E4 C4 G4\n\n.\n// body comment\n:8 r r |\n";
            let parsed = parse(input).unwrap();
            assert_eq!(parsed.tempo, 120);
            assert_eq!(parsed.columns.len(), 2);
        }

        #[test]
        fn parsed_tab_tuning_recovers_uke_strings() {
            let input = "\\tempo 120\n\\tuning A4 E4 C4 G4\n.\n";
            let parsed = parse(input).unwrap();
            let t = parsed.tuning().expect("tuning should parse");
            assert_eq!(t.strings.len(), 4);
            assert_eq!(t.strings[0].open, twanga_core::MidiNote(69)); // A4
            assert_eq!(t.strings[3].open, twanga_core::MidiNote(67)); // G4
        }

        #[test]
        fn transpose_to_same_tuning_preserves_hits() {
            let uke = Tuning::standard_ukulele();
            let input = "\\tempo 120\n\\tuning A4 E4 C4 G4\n.\n:8 0.3 2.3 0.2 1.2 |\n";
            let parsed = parse(input).unwrap();
            let transposed = parsed.transpose_to(&uke, 20);
            // Same hits on same strings/frets after transposing onto same tuning.
            for (orig, new) in parsed.columns.iter().zip(transposed.columns.iter()) {
                assert_eq!(orig.hits, new.hits);
            }
        }

        #[test]
        fn transpose_uke_c4_to_banjo_preserves_pitch() {
            // C4 on uke is string 3 (C4 open) fret 0.
            // On banjo standard tuning (D4 B3 G3 D3 g4-drone), C4 = MIDI 60.
            // B3 = MIDI 59, so C4 is B3 fret 1. That's the smallest fret choice.
            let banjo = Tuning::standard_banjo();
            let input = "\\tempo 120\n\\tuning A4 E4 C4 G4\n.\n:8 0.3 |\n";
            let parsed = parse(input).unwrap();
            let transposed = parsed.transpose_to(&banjo, 20);
            assert_eq!(transposed.columns.len(), 1);
            let hits = &transposed.columns[0].hits;
            assert_eq!(hits.len(), 1);
            let (string, fret) = hits[0];
            let banjo_open = banjo.strings[(string - 1) as usize].open;
            assert_eq!(
                banjo_open.0 as u16 + fret as u16,
                60,
                "absolute pitch should still be MIDI 60 (C4)"
            );
        }

        #[test]
        fn transpose_drops_out_of_range_notes() {
            // Source: a note with fret 50 (way past any sensible max_fret).
            // With max_fret=20 on the target, this should be dropped.
            let banjo = Tuning::standard_banjo();
            let input = "\\tempo 120\n\\tuning A4 E4 C4 G4\n.\n:8 50.1 |\n";
            let parsed = parse(input).unwrap();
            let transposed = parsed.transpose_to(&banjo, 20);
            assert!(transposed.columns[0].hits.is_empty());
        }

        #[test]
        fn transpose_updates_tuning_names_header() {
            let banjo = Tuning::standard_banjo();
            let input = "\\tempo 120\n\\tuning A4 E4 C4 G4\n.\n:8 0.3 |\n";
            let parsed = parse(input).unwrap();
            let transposed = parsed.transpose_to(&banjo, 20);
            // After transposition, the tuning header reflects the target.
            assert_eq!(transposed.tuning_names, vec!["D4", "B3", "G3", "D3", "G4"],);
        }

        #[test]
        fn transpose_to_with_report_returns_empty_drops_when_all_notes_fit() {
            let banjo = Tuning::standard_banjo();
            let input = "\\tempo 120\n\\tuning A4 E4 C4 G4\n.\n:8 0.3 |\n";
            let parsed = parse(input).unwrap();
            let (transposed, dropped) = parsed.transpose_to_with_report(&banjo, 20);
            assert!(dropped.is_empty(), "expected no drops, got {dropped:?}");
            assert!(!transposed.columns[0].hits.is_empty());
        }

        #[test]
        fn transpose_to_with_report_reports_unreachable_pitches() {
            // E2 is below the uke's playable range (lowest open is C4 = MIDI
            // 60; E2 = MIDI 40, four octaves down). Transposing a guitar tab
            // that hits the low-E string open should report a drop.
            let uke = uke();
            // Standard-guitar low-E (string 6) played open: `0.6` at 1/8.
            let input = "\\tempo 120\n\\tuning E4 B3 G3 D3 A2 E2\n.\n:8 0.6 |\n";
            let parsed = parse(input).unwrap();
            let (transposed, dropped) = parsed.transpose_to_with_report(&uke, 20);
            assert_eq!(dropped.len(), 1, "expected one drop, got {dropped:?}");
            assert_eq!(dropped[0].column_index, 0);
            assert_eq!(dropped[0].note, "E2");
            // Dropped notes don't appear in the transposed output.
            assert!(transposed.columns[0].hits.is_empty());
        }

        #[test]
        fn transpose_octave_shift_keeps_too_low_notes_an_octave_up() {
            // Same setup as `transpose_to_with_report_reports_unreachable_pitches`:
            // a guitar low-E (E2 = MIDI 40) being transposed onto a uke,
            // which can't reach below C4 (MIDI 60). With `OctaveShift` we
            // expect the note to come back at E3 (one octave up = still
            // below uke range) or E4 (two octaves up = playable on the C
            // string at fret 4). The placement algorithm prefers the
            // smallest shift that fits, so E4 wins.
            let uke = uke();
            let input = "\\tempo 120\n\\tuning E4 B3 G3 D3 A2 E2\n.\n:8 0.6 |\n";
            let parsed = parse(input).unwrap();
            let (transposed, dropped) =
                parsed.transpose_to_with_mode(&uke, 20, TransposeMode::OctaveShift);
            assert!(dropped.is_empty(), "octave-shift should rescue E2 onto uke");
            let hits = &transposed.columns[0].hits;
            assert_eq!(hits.len(), 1);
            let (string, fret) = hits[0];
            let uke_open = uke.strings[(string - 1) as usize].open;
            let placed_midi = uke_open.0 as i32 + fret as i32;
            // E2 = MIDI 40. The shift should be a multiple of 12 above
            // that — so MIDI 52 (E3, still below uke range — won't fit)
            // or MIDI 64 (E4, the smallest shift that lands inside the
            // uke's playable range). The algorithm picks the smallest
            // shift that fits, so we should get 64.
            assert_eq!(
                placed_midi, 64,
                "expected E4 (MIDI 64) — the smallest octave-shift that fits"
            );
        }

        #[test]
        fn transpose_octave_shift_keeps_too_high_notes_an_octave_down() {
            // Inverse of the above: a guitar note far above the source's
            // range, transposed onto a smaller instrument whose top is
            // below the source pitch. We use a guitar tab with a high
            // fret on the high E string (E4 + 24 = MIDI 88 = E6) and
            // transpose onto a tuning whose highest reachable pitch is
            // below E6. With OctaveShift we expect the note to come
            // back an octave or two lower.
            let uke = uke();
            // Guitar high E string + fret 24 = E6 = MIDI 88. Uke's
            // highest open is A4 (MIDI 69); A4 + max_fret(=20) = MIDI 89,
            // so E6 is technically reachable on a 20-fret uke. Use a
            // narrower max_fret on the target to force the shift.
            let input = "\\tempo 120\n\\tuning E4 B3 G3 D3 A2 E2\n.\n:8 24.1 |\n";
            let parsed = parse(input).unwrap();
            let (transposed, dropped) =
                parsed.transpose_to_with_mode(&uke, 12, TransposeMode::OctaveShift);
            assert!(dropped.is_empty(), "octave-shift should rescue E6");
            let hits = &transposed.columns[0].hits;
            assert_eq!(hits.len(), 1);
            let (string, fret) = hits[0];
            let uke_open = uke.strings[(string - 1) as usize].open;
            let placed_midi = uke_open.0 as i32 + fret as i32;
            // 88 - 12*k for some k > 0. With max_fret=12 and uke open
            // strings A4/E4/C4/G4 (MIDI 69/64/60/67), top reachable is
            // 69+12 = 81. So we need a shift of at least 12 down (→ 76,
            // fits on A4 string + 7) — and the algorithm picks the
            // smallest. So we should get 76.
            assert_eq!(
                placed_midi, 76,
                "expected E5 (MIDI 76) — one octave below the source E6"
            );
        }

        #[test]
        fn transpose_octave_shift_leaves_in_range_notes_alone() {
            // A note that already fits should not be shifted just because
            // the mode is OctaveShift. C4 on a uke fits at C-string open.
            let uke = uke();
            let input = "\\tempo 120\n\\tuning A4 E4 C4 G4\n.\n:8 0.3 |\n";
            let parsed = parse(input).unwrap();
            let (transposed, dropped) =
                parsed.transpose_to_with_mode(&uke, 20, TransposeMode::OctaveShift);
            assert!(dropped.is_empty());
            let hits = &transposed.columns[0].hits;
            assert_eq!(hits.len(), 1);
            let (string, fret) = hits[0];
            let uke_open = uke.strings[(string - 1) as usize].open;
            assert_eq!(
                uke_open.0 as i32 + fret as i32,
                60,
                "C4 = MIDI 60 should land at its original pitch, not shifted"
            );
        }

        #[test]
        fn transpose_octave_shift_still_drops_truly_unreachable() {
            // A note that even ±8 octaves can't rescue (e.g. fret 50
            // past max_fret on every octave we try). With Drop mode we'd
            // see one drop entry; with OctaveShift we should also see
            // one drop because the placement is still impossible —
            // shifting an unreachable note up/down by 12 doesn't help
            // when the source fret is just nonsense.
            let uke = uke();
            let input = "\\tempo 120\n\\tuning A4 E4 C4 G4\n.\n:8 50.1 |\n";
            let parsed = parse(input).unwrap();
            let (transposed, dropped) =
                parsed.transpose_to_with_mode(&uke, 20, TransposeMode::OctaveShift);
            // 50 frets up from A4 (MIDI 69) is MIDI 119 = B8 (still in
            // MIDI range). After enough downward octave shifts that lands
            // back in playable range, so this one SHOULD recover. Use a
            // pitch that can't be reached no matter how many octaves you
            // shift: nothing — every MIDI value is reachable at some
            // octave on at least one string of a 20-fret uke. So this
            // case actually tests that OctaveShift doesn't break — it
            // should recover where Drop would not.
            //
            // Verify by comparing Drop vs OctaveShift on the same input:
            // Drop reports the note as dropped (it's at A4+50 = MIDI 119
            // = B8, and matching at exactly that pitch fails on a 20-fret
            // uke whose top reachable is MIDI 89). OctaveShift should
            // place it.
            assert!(
                dropped.is_empty() || !transposed.columns[0].hits.is_empty(),
                "OctaveShift should rescue some notes Drop would lose"
            );

            let (_drop_t, drop_dropped) = parsed.transpose_to_with_mode(
                &uke,
                20,
                TransposeMode::Drop,
            );
            // Drop mode definitely loses it (B8 isn't reachable on a
            // 20-fret uke at the source pitch).
            assert_eq!(drop_dropped.len(), 1);
        }

        #[test]
        fn parser_round_trips_with_writer() {
            // Write some content, then parse it back, and check we recover what we wrote.
            let mut buf = Vec::new();
            let uke = uke();
            {
                let mut w = AlphaTexWriter::new(
                    &mut buf,
                    &uke,
                    &Capo::none(uke.strings.len()),
                    100,
                    8,
                    None,
                )
                .unwrap();
                w.write_column(&[Some(0), None, None, None]).unwrap(); // A open
                w.write_column(&[None, None, Some(3), None]).unwrap(); // C fret 3
                w.write_column(&[None; 4]).unwrap(); // rest
                w.write_column(&[Some(2), Some(0), Some(0), Some(0)])
                    .unwrap(); // chord
                w.finalize().unwrap();
            }
            let text = String::from_utf8(buf).unwrap();
            let parsed = parse(&text).unwrap();
            assert_eq!(parsed.tempo, 100);
            assert_eq!(parsed.tuning_names, vec!["A4", "E4", "C4", "G4"]);
            assert_eq!(parsed.columns.len(), 4);
            assert_eq!(parsed.columns[0].hits, vec![(1, 0)]);
            assert_eq!(parsed.columns[1].hits, vec![(3, 3)]);
            assert!(parsed.columns[2].hits.is_empty());
            assert_eq!(parsed.columns[3].hits, vec![(1, 2), (2, 0), (3, 0), (4, 0)]);
        }
    }
}

/// One event from the live tab recorder.
///
/// Both variants carry `column_marks` — the per-string fret values for the
/// single column that was just committed. Consumers writing to a structured
/// format (e.g. alphaTex) use `column_marks` to emit one note/chord/rest per
/// event; consumers driving a live ASCII display use `rows`.
#[derive(Debug, Clone)]
pub enum TabEvent {
    /// A column was just committed; the block is still in progress.
    ColumnTick {
        rows: Vec<String>,
        column_marks: Vec<Option<u8>>,
    },
    /// The current block just filled. Renderers commit (no further refresh)
    /// and start a fresh block.
    BlockComplete {
        rows: Vec<String>,
        column_marks: Vec<Option<u8>>,
    },
}

/// Streaming tab recorder.
///
/// Time is divided into fixed-duration columns. Each column shows which
/// strings were hit during that slice and at what fret: digit 0-9 for that
/// fret number, `+` for fret 10+, `-` for no hit. Once `columns_per_block`
/// columns have been recorded, the recorder finalises the current block and
/// starts a fresh one.
///
/// Decoupled from any pitch detector: callers feed it `record_hit(idx, fret)`
/// for each detected event and `advance(samples)` for elapsed time, then
/// handle the returned [`TabEvent`]s.
pub struct TabRecorder {
    string_names: Vec<String>,
    samples_per_column: usize,
    columns_per_block: usize,
    name_width: usize,
    /// Last-recorded fret per string for the in-progress column. `None` = no hit yet.
    column_marks: Vec<Option<u8>>,
    /// Completed columns: outer index is string, inner index is column position.
    completed_columns: Vec<Vec<Option<u8>>>,
    samples_in_current_column: usize,
}

impl TabRecorder {
    pub fn new(
        tuning: &Tuning,
        sample_rate: u32,
        ms_per_column: u32,
        columns_per_block: usize,
    ) -> Self {
        let samples_per_column = ((sample_rate as u64) * (ms_per_column as u64) / 1000) as usize;
        let name_width = tuning
            .strings
            .iter()
            .map(|s| s.name.len())
            .max()
            .unwrap_or(0);
        let n = tuning.strings.len();
        Self {
            string_names: tuning.strings.iter().map(|s| s.name.clone()).collect(),
            samples_per_column: samples_per_column.max(1),
            columns_per_block: columns_per_block.max(1),
            name_width,
            column_marks: vec![None; n],
            completed_columns: (0..n)
                .map(|_| Vec::with_capacity(columns_per_block))
                .collect(),
            samples_in_current_column: 0,
        }
    }

    pub fn string_count(&self) -> usize {
        self.string_names.len()
    }

    /// 0-based string index whose name matches exactly.
    pub fn string_index_for_name(&self, name: &str) -> Option<usize> {
        self.string_names.iter().position(|n| n == name)
    }

    /// Record a hit on `string_idx` at the given fret within the current column.
    /// Multiple hits in the same column overwrite each other — last wins.
    pub fn record_hit(&mut self, string_idx: usize, fret: u8) {
        if let Some(m) = self.column_marks.get_mut(string_idx) {
            *m = Some(fret);
        }
    }

    /// Advance internal time. Returns any column ticks and block-completes
    /// that happened during this advance.
    pub fn advance(&mut self, samples: usize) -> Vec<TabEvent> {
        self.samples_in_current_column += samples;
        let mut events = Vec::new();
        while self.samples_in_current_column >= self.samples_per_column {
            self.samples_in_current_column -= self.samples_per_column;

            // Snapshot the marks being committed before resetting for the
            // next column. Consumers writing to structured formats (alphaTex,
            // etc.) need these per-column.
            let column_marks = self.column_marks.clone();

            for (s, mark) in self.column_marks.iter().enumerate() {
                self.completed_columns[s].push(*mark);
            }
            self.column_marks.fill(None);

            let rows = self.render_rows();
            if self.completed_columns[0].len() >= self.columns_per_block {
                events.push(TabEvent::BlockComplete { rows, column_marks });
                for col in &mut self.completed_columns {
                    col.clear();
                }
            } else {
                events.push(TabEvent::ColumnTick { rows, column_marks });
            }
        }
        events
    }

    fn render_rows(&self) -> Vec<String> {
        self.string_names
            .iter()
            .zip(self.completed_columns.iter())
            .map(|(name, marks)| {
                let padded = format!("{:<width$}", name, width = self.name_width);
                let content: String = marks.iter().map(|m| fret_char(*m)).collect();
                format!("{padded} | {content}")
            })
            .collect()
    }
}

fn fret_char(mark: Option<u8>) -> char {
    match mark {
        None => '-',
        Some(n) if n <= 9 => char::from_digit(n as u32, 10).unwrap(),
        Some(_) => '+',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uke() -> Tuning {
        Tuning::standard_ukulele()
    }

    fn tab_after(rows: &[String], string_idx: usize) -> String {
        rows[string_idx].split(" | ").nth(1).unwrap().to_string()
    }

    fn rows_of(event: &TabEvent) -> &[String] {
        match event {
            TabEvent::ColumnTick { rows, .. } => rows,
            TabEvent::BlockComplete { rows, .. } => rows,
        }
    }

    #[test]
    fn recorder_emits_one_event_per_column_tick() {
        let t = uke();
        let mut r = TabRecorder::new(&t, 1000, 10, 4);
        let events = r.advance(30);
        assert_eq!(events.len(), 3);
        for e in &events {
            assert!(matches!(e, TabEvent::ColumnTick { .. }));
        }
    }

    #[test]
    fn recorder_emits_block_complete_on_last_column_of_block() {
        let t = uke();
        let mut r = TabRecorder::new(&t, 1000, 10, 4);
        let events = r.advance(40);
        assert_eq!(events.len(), 4);
        assert!(matches!(events[3], TabEvent::BlockComplete { .. }));
    }

    #[test]
    fn recorder_renders_fret_digits_per_column() {
        let t = uke();
        let mut r = TabRecorder::new(&t, 1000, 10, 4);

        r.record_hit(0, 0); // A string, open, col 1
        let _ = r.advance(10);
        let _ = r.advance(10); // col 2 — no hits
        r.record_hit(1, 3); // E string, fret 3, col 3
        let _ = r.advance(10);
        let events = r.advance(10); // col 4 — fills block

        let last = events.last().unwrap();
        let rows = rows_of(last);
        assert_eq!(tab_after(rows, 0), "0---");
        assert_eq!(tab_after(rows, 1), "--3-");
        assert_eq!(tab_after(rows, 2), "----");
    }

    #[test]
    fn recorder_renders_double_digit_frets_as_plus() {
        let t = uke();
        let mut r = TabRecorder::new(&t, 1000, 10, 1);
        r.record_hit(0, 12);
        let events = r.advance(10);
        let rows = rows_of(&events[0]);
        assert_eq!(tab_after(rows, 0), "+");
    }

    #[test]
    fn recorder_string_index_for_name_resolves_uke_names() {
        let t = uke();
        let r = TabRecorder::new(&t, 1000, 10, 4);
        assert_eq!(r.string_index_for_name("A4"), Some(0));
        assert_eq!(r.string_index_for_name("g4 (reentrant)"), Some(3));
        assert_eq!(r.string_index_for_name("nope"), None);
    }

    #[test]
    fn recorder_last_hit_in_column_wins() {
        let t = uke();
        let mut r = TabRecorder::new(&t, 1000, 10, 1);
        r.record_hit(0, 1);
        r.record_hit(0, 7);
        r.record_hit(0, 3);
        let events = r.advance(10);
        let rows = rows_of(&events[0]);
        assert_eq!(tab_after(rows, 0), "3");
    }

    #[test]
    fn recorder_handles_multi_block_advances() {
        let t = uke();
        let mut r = TabRecorder::new(&t, 1000, 10, 2);
        let events = r.advance(50);
        let blocks = events
            .iter()
            .filter(|e| matches!(e, TabEvent::BlockComplete { .. }))
            .count();
        assert_eq!(blocks, 2);
    }

    #[test]
    fn recorder_pads_string_names_to_align_pipes() {
        let t = uke();
        let mut r = TabRecorder::new(&t, 1000, 10, 1);
        let events = r.advance(10);
        let rows = rows_of(&events[0]);
        let pipe_positions: Vec<usize> = rows
            .iter()
            .map(|r| r.find(" | ").expect("has separator"))
            .collect();
        assert!(pipe_positions.windows(2).all(|w| w[0] == w[1]));
    }
}
