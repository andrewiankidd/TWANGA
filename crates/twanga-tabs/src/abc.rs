//! ABC notation (`.abc`) parser — the dominant text-based format in
//! folk / traditional music circles (banjo, mando, fiddle, uke
//! communities especially). Open specification by Chris Walshaw; no
//! licensing concerns.
//!
//! # Scope
//!
//! This is a **deliberately minimal** parser covering the subset of
//! ABC that folk-tune notation actually uses:
//!
//! - **Header fields** — `T:` (title), `M:` (meter, parsed but not
//!   used in TWANGA's column model), `L:` (default note length),
//!   `Q:` (tempo), `K:` (key — terminates the header and supplies
//!   the accidental defaults for unmarked notes)
//! - **Notes** — pitch letters `C-B` (octave 4 = middle-C octave)
//!   and `c-b` (octave 5), octave shifts `,` / `'`, accidentals
//!   `^` / `_` / `=`, and durations `N`, `N/M`, `/`, `/N`, `>` /
//!   `<` (broken rhythms are simplified — both notes get the
//!   midpoint duration, surfacing an [`ParseWarning::IrregularDuration`])
//! - **Rests** — `z` (single rest) and `Z` (multi-measure rest;
//!   collapsed to one rest column)
//! - **Bar lines** — `|`, `||`, `:|`, `|:`, `[1`, `[2` are tokenised
//!   but not represented in the output (TWANGA's `ParsedTab` carries
//!   no bar structure; columns are sequential)
//!
//! # Limitations (deferred)
//!
//! - **Chords** — `[CEG]` chord notation isn't supported in v1; a
//!   chord becomes a sequence of single notes (the first one) and
//!   emits an `IrregularDuration` warning. Folk tunes are
//!   overwhelmingly monophonic, so this is a small loss.
//! - **Multi-voice** — `V:` voice declarations are silently
//!   ignored; only the first voice's notes are collected.
//! - **Tuplets** — `(3CDE` triplet notation is partially supported
//!   (the `(3` prefix is recognised and the three notes that
//!   follow get their durations multiplied by 2/3 then snapped to
//!   the nearest power-of-2, surfacing an `IrregularDuration`).
//! - **Decorations / ornaments / lyrics** — `~M!trill!` etc are
//!   stripped and ignored.
//!
//! # Tuning
//!
//! ABC carries no instrument-tuning information. The parser places
//! every pitch on the default tuning (standard 6-string guitar)
//! and emits an [`ParseWarning::InferredTuning`] so the importer UI
//! can flag the guess. Users can retune at playback via
//! `--tuning <slug>`.

use twanga_core::{MidiNote, Tuning};

use crate::{ParseOutput, ParseWarning, ParsedTab, TabColumn, snap_to_power_of_two};

/// Errors the ABC parser can return. Distinct variants per failure
/// mode so the importer UI can surface targeted messages.
#[derive(Debug)]
pub enum AbcError {
    /// File didn't contain a `K:` header — ABC's hard requirement
    /// that defines accidentals + terminates the header section.
    /// Without it we can't disambiguate `F` (could be F or F# in G
    /// major).
    MissingKey,
    /// File parsed but produced no playable notes. Most likely an
    /// empty tune body or all-rest input.
    EmptyScore,
    /// Header field has malformed content (e.g. `Q:not-a-number`).
    /// Wraps the offending line so the user can locate it.
    BadHeader(String),
}

impl std::fmt::Display for AbcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingKey => write!(
                f,
                "ABC file has no K: header — required by the format spec to define accidentals"
            ),
            Self::EmptyScore => write!(f, "ABC tune body has no playable notes"),
            Self::BadHeader(line) => write!(f, "malformed ABC header line: {line}"),
        }
    }
}
impl std::error::Error for AbcError {}

/// Maximum fret position the pitch-to-fret matcher will reach for.
const MAX_FRET: u8 = 20;

/// Default tempo when `Q:` is missing. ABC convention is "moderate"
/// when unspecified, ~120 BPM; matches other parsers.
const DEFAULT_TEMPO: u32 = 120;

/// Default note length when `L:` is missing. ABC's own default: if
/// the meter is 3/4 or simpler use 1/8; if more complex use 1/16.
/// We always use 1/8 for simplicity — close enough for the import
/// flow's quantisation, and the user can override at the source.
const DEFAULT_NOTE_LENGTH_DENOM: u32 = 8;

/// Parse an ABC notation document (raw text).
pub fn parse(text: &str) -> Result<ParseOutput, AbcError> {
    let mut state = HeaderState::default();
    let mut body_lines: Vec<&str> = Vec::new();
    let mut in_body = false;

    // ── Header phase — `X:Y` lines until `K:` is seen (which
    //    terminates the header per ABC spec). After K:, every
    //    remaining line is body.
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('%') {
            // Skip blank lines and comments (% starts a line comment).
            continue;
        }
        if in_body {
            body_lines.push(line);
            continue;
        }
        if let Some((field, value)) = parse_header_line(line) {
            apply_header(&mut state, field, value)?;
            if field == 'K' {
                in_body = true;
            }
        } else {
            // Non-header line before K: — treat as body (some loose
            // ABC files put notes inline without explicit K:).
            body_lines.push(line);
        }
    }

    state.key.ok_or(AbcError::MissingKey)?;

    // ── Body phase — walk the joined body text token-by-token.
    let body = body_lines.join(" ");
    let target_tuning = Tuning::standard_guitar();
    let mut columns: Vec<TabColumn> = Vec::new();
    let mut warnings: Vec<ParseWarning> = Vec::new();
    let key_accidentals = state.key.unwrap_or(KeyAccidentals::c_major());

    let mut tokens = TokenStream::new(&body);
    let mut tuplet_remaining: u32 = 0;
    while let Some(tok) = tokens.next() {
        match tok {
            Token::Note {
                accidental,
                letter,
                octave_shift,
                duration,
            } => {
                let pitch = note_to_midi(letter, accidental, octave_shift, &key_accidentals);
                let mut denom = duration_to_denom(duration, state.length_denom);
                let mut rounded = false;
                if tuplet_remaining > 0 {
                    // Triplet: each note gets 2/3 its written value.
                    // Snap to nearest power-of-2 → warn.
                    let scaled = denom as f64 * 1.5;
                    let (snapped, _) = snap_to_power_of_two(scaled);
                    denom = snapped;
                    rounded = true;
                    tuplet_remaining -= 1;
                }
                let column_index = columns.len();
                if rounded {
                    warnings.push(ParseWarning::IrregularDuration {
                        column_index,
                        raw_duration: "triplet".into(),
                    });
                }
                match pitch.and_then(|p| crate::place_pitch(&target_tuning, p, MAX_FRET)) {
                    Some(hit) => {
                        columns.push(TabColumn {
                            duration_denom: denom,
                            hits: vec![hit],
                            articulation: None,
                        });
                    }
                    None => {
                        let note_name = pitch
                            .map(|p| MidiNote(p).name())
                            .unwrap_or_else(|| "?".into());
                        warnings.push(ParseWarning::UnreachableNote {
                            column_index,
                            note: note_name,
                        });
                        // Still emit a rest column so the timing
                        // doesn't drift — losing the pitch is
                        // recoverable, losing the beat isn't.
                        columns.push(TabColumn {
                            duration_denom: denom,
                            hits: Vec::new(),
                            articulation: None,
                        });
                    }
                }
            }
            Token::Rest { duration } => {
                let denom = duration_to_denom(duration, state.length_denom);
                columns.push(TabColumn {
                    duration_denom: denom,
                    hits: Vec::new(),
                    articulation: None,
                });
            }
            Token::TupletStart(n) => {
                tuplet_remaining = n;
            }
            Token::BarLine | Token::Ignored => {}
        }
    }

    if columns.is_empty() {
        return Err(AbcError::EmptyScore);
    }
    // MIDI-style "no tuning declared" warning — same posture as the
    // MIDI parser so the importer UI surfaces an InferredTuning
    // badge consistently across pitch-only formats.
    warnings.push(ParseWarning::InferredTuning {
        source_tuning: Vec::new(),
        matched_name: target_tuning.name.clone(),
    });

    let tuning_names: Vec<String> = target_tuning
        .strings
        .iter()
        .map(|s| s.name.clone())
        .collect();

    Ok(ParseOutput {
        tab: ParsedTab {
            tempo: state.tempo.unwrap_or(DEFAULT_TEMPO),
            title: state.title,
            subtitle: Some(target_tuning.name.clone()),
            tuning_names,
            columns,
        },
        warnings,
    })
}

/// Accumulator for header fields we care about — populated by
/// `apply_header` during the header phase. Anything not declared
/// in the source defaults to a sensible value when used.
#[derive(Debug, Default)]
struct HeaderState {
    title: Option<String>,
    tempo: Option<u32>,
    length_denom: u32,
    key: Option<KeyAccidentals>,
}

/// Recognise a header line of the shape `X:value`. Returns `(field,
/// value)` for valid lines, `None` otherwise (so the caller can
/// treat the line as body).
fn parse_header_line(line: &str) -> Option<(char, &str)> {
    let mut chars = line.chars();
    let first = chars.next()?;
    let second = chars.next()?;
    if !first.is_ascii_alphabetic() || second != ':' {
        return None;
    }
    let rest = &line[2..];
    Some((first, rest.trim()))
}

fn apply_header(state: &mut HeaderState, field: char, value: &str) -> Result<(), AbcError> {
    match field {
        // Multiple T: lines = primary title + alternate names; keep
        // the first one (the guard makes the assignment a no-op on
        // subsequent T: lines).
        'T' if state.title.is_none() => {
            state.title = Some(value.to_string());
        }
        'Q' => {
            // Q:120 or Q:1/4=120 (note=BPM). Pull the trailing
            // integer in either case.
            let bpm = value
                .rsplit('=')
                .next()
                .unwrap_or(value)
                .trim()
                .parse::<u32>()
                .map_err(|_| AbcError::BadHeader(format!("Q:{value}")))?;
            state.tempo = Some(bpm);
        }
        'L' => {
            // L:1/8, L:1/4, etc — we want the denominator.
            let denom = value
                .trim()
                .trim_start_matches("1/")
                .parse::<u32>()
                .map_err(|_| AbcError::BadHeader(format!("L:{value}")))?;
            state.length_denom = denom.max(1);
        }
        'K' => {
            state.key = Some(KeyAccidentals::parse(value));
        }
        _ => {
            // Unknown header — silently accepted (ABC has dozens of
            // optional fields like A:area, C:composer, R:rhythm,
            // O:origin, etc — TWANGA doesn't use them but they
            // shouldn't fail the parse).
        }
    }
    Ok(())
}

/// Accidentals defined by the key signature for the seven note
/// letters. Stored as semitone offsets (-1 flat, 0 natural, +1
/// sharp) indexed by `letter - 'A'`. The body parser overrides
/// these per-note when an explicit `^`/`_`/`=` is present, then
/// resets at bar lines — but TWANGA's per-bar accidental reset
/// isn't critical for the column model since we operate on raw
/// pitches and the user's intent is preserved.
#[derive(Debug, Clone, Copy)]
struct KeyAccidentals {
    semitones: [i8; 7],
}

impl KeyAccidentals {
    fn c_major() -> Self {
        Self { semitones: [0; 7] }
    }

    fn parse(value: &str) -> Self {
        // Common keys mapped to their sharps/flats. We're not
        // building a music-theory library here — covering the
        // 14 standard keys (7 majors + 7 minors that share key
        // signatures) is enough for folk tunes; anything else
        // falls back to C major.
        let v = value.trim().to_ascii_uppercase();
        // Strip "m" / "minor" / "MIN" suffixes and remap to the
        // relative major.
        let (root, is_minor) = if let Some(prefix) = v
            .strip_suffix("MIN")
            .or_else(|| v.strip_suffix("MINOR"))
            .or_else(|| v.strip_suffix('M'))
        {
            (prefix.trim_end().to_string(), true)
        } else {
            (v.clone(), false)
        };

        // Convert minor to relative major (same key sig).
        let major_root = if is_minor {
            minor_to_relative_major(&root)
        } else {
            root
        };

        // Map major root → number of sharps (positive) or flats
        // (negative). Order of sharps: F C G D A E B. Order of
        // flats: B E A D G C F.
        let sharps: i32 = match major_root.as_str() {
            "C" => 0,
            "G" => 1,
            "D" => 2,
            "A" => 3,
            "E" => 4,
            "B" => 5,
            "F#" => 6,
            "C#" => 7,
            "F" => -1,
            "BB" | "B♭" => -2,
            "EB" | "E♭" => -3,
            "AB" | "A♭" => -4,
            "DB" | "D♭" => -5,
            "GB" | "G♭" => -6,
            "CB" | "C♭" => -7,
            _ => 0,
        };
        let mut semitones = [0i8; 7];
        // Sharps: F, C, G, D, A, E, B in order
        const SHARP_ORDER: [char; 7] = ['F', 'C', 'G', 'D', 'A', 'E', 'B'];
        const FLAT_ORDER: [char; 7] = ['B', 'E', 'A', 'D', 'G', 'C', 'F'];
        if sharps > 0 {
            for &c in SHARP_ORDER.iter().take(sharps as usize) {
                semitones[(c as u8 - b'A') as usize] = 1;
            }
        } else if sharps < 0 {
            for &c in FLAT_ORDER.iter().take((-sharps) as usize) {
                semitones[(c as u8 - b'A') as usize] = -1;
            }
        }
        Self { semitones }
    }

    fn for_letter(&self, letter: char) -> i8 {
        let idx = (letter.to_ascii_uppercase() as u8).wrapping_sub(b'A') as usize;
        self.semitones.get(idx).copied().unwrap_or(0)
    }
}

fn minor_to_relative_major(minor_root: &str) -> String {
    // Minor → relative major is +3 semitones, but we want the
    // letter-equivalent. Hardcode the 14 standard minors.
    match minor_root {
        "A" => "C".into(),
        "E" => "G".into(),
        "B" => "D".into(),
        "F#" => "A".into(),
        "C#" => "E".into(),
        "G#" => "B".into(),
        "D#" => "F#".into(),
        "A#" => "C#".into(),
        "D" => "F".into(),
        "G" => "BB".into(),
        "C" => "EB".into(),
        "F" => "AB".into(),
        "BB" => "DB".into(),
        "EB" => "GB".into(),
        "AB" => "CB".into(),
        _ => "C".into(),
    }
}

/// One body token. The parser walks the body producing these in
/// order; the column-building loop consumes them.
#[derive(Debug, Clone)]
enum Token {
    Note {
        accidental: Option<i8>, // -1 flat, 0 natural, +1 sharp; None = use key sig
        letter: char,
        octave_shift: i32,
        /// Raw duration components: `(num, denom)` where the
        /// effective duration is `num / denom * default_length`.
        duration: (u32, u32),
    },
    Rest {
        duration: (u32, u32),
    },
    BarLine,
    TupletStart(u32),
    Ignored,
}

/// Streaming tokeniser for the body. Walks characters, emitting
/// `Token`s. Handles the ABC quirks: durations after the note
/// letter, octave marks before or after, prefixes like `^` for
/// sharps.
struct TokenStream<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> TokenStream<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            bytes: s.as_bytes(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn skip_whitespace(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn next(&mut self) -> Option<Token> {
        self.skip_whitespace();
        let b = self.peek()?;
        match b {
            // Accidentals or notes
            b'^' | b'_' | b'=' | b'A'..=b'G' | b'a'..=b'g' => Some(self.read_note()),
            b'z' | b'Z' | b'x' => {
                self.advance();
                let duration = self.read_duration();
                Some(Token::Rest { duration })
            }
            b'|' | b':' => {
                self.advance();
                Some(Token::BarLine)
            }
            b'(' => {
                // Tuplet start — `(3` etc.
                self.advance();
                if let Some(d) = self.peek()
                    && d.is_ascii_digit()
                {
                    self.advance();
                    let n = (d - b'0') as u32;
                    return Some(Token::TupletStart(n));
                }
                Some(Token::Ignored)
            }
            b'[' | b']' => {
                // Chord brackets — skip the bracket and the first
                // note that follows is the only one we'll pick up.
                // (Full chord support is a v2 feature.)
                self.advance();
                Some(Token::Ignored)
            }
            b'"' => {
                // Chord symbol string ("Am7") — skip to closing quote.
                self.advance();
                while let Some(b) = self.advance() {
                    if b == b'"' {
                        break;
                    }
                }
                Some(Token::Ignored)
            }
            b'!' => {
                // Decoration / ornament — skip to closing !.
                self.advance();
                while let Some(b) = self.advance() {
                    if b == b'!' {
                        break;
                    }
                }
                Some(Token::Ignored)
            }
            _ => {
                self.advance();
                Some(Token::Ignored)
            }
        }
    }

    fn read_note(&mut self) -> Token {
        let mut accidental: Option<i8> = None;
        // Leading accidentals
        loop {
            match self.peek() {
                Some(b'^') => {
                    self.advance();
                    accidental = Some(accidental.unwrap_or(0) + 1);
                }
                Some(b'_') => {
                    self.advance();
                    accidental = Some(accidental.unwrap_or(0) - 1);
                }
                Some(b'=') => {
                    self.advance();
                    accidental = Some(0);
                }
                _ => break,
            }
        }
        let letter = match self.advance() {
            Some(b) => b as char,
            None => return Token::Ignored,
        };
        // Octave marks after the letter
        let mut octave_shift = if letter.is_ascii_lowercase() { 1 } else { 0 };
        loop {
            match self.peek() {
                Some(b',') => {
                    self.advance();
                    octave_shift -= 1;
                }
                Some(b'\'') => {
                    self.advance();
                    octave_shift += 1;
                }
                _ => break,
            }
        }
        let duration = self.read_duration();
        Token::Note {
            accidental,
            letter: letter.to_ascii_uppercase(),
            octave_shift,
            duration,
        }
    }

    /// Read the optional duration suffix after a note. Forms:
    /// - `` (empty) → (1, 1)
    /// - `2`, `3`, … → (n, 1)
    /// - `/2`, `/3`, … → (1, n)
    /// - `/` (alone) → (1, 2) — ABC shorthand for `/2`
    /// - `3/2`, etc → (n, m)
    fn read_duration(&mut self) -> (u32, u32) {
        let num = self.read_uint();
        if self.peek() == Some(b'/') {
            self.advance();
            let denom = self.read_uint_or_default(2);
            (num.unwrap_or(1), denom)
        } else {
            (num.unwrap_or(1), 1)
        }
    }

    fn read_uint(&mut self) -> Option<u32> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start {
            None
        } else {
            std::str::from_utf8(&self.bytes[start..self.pos])
                .ok()
                .and_then(|s| s.parse().ok())
        }
    }

    fn read_uint_or_default(&mut self, default: u32) -> u32 {
        self.read_uint().unwrap_or(default)
    }
}

/// Map an ABC note (letter + accidental + octave shift) to a MIDI
/// number. `key_accidentals` supplies the implicit accidental when
/// the parser saw no explicit `^`/`_`/`=`. Returns `None` if the
/// resulting MIDI value is out of range (0..=127).
fn note_to_midi(
    letter: char,
    accidental: Option<i8>,
    octave_shift: i32,
    key_accidentals: &KeyAccidentals,
) -> Option<u8> {
    let base_semitones: i32 = match letter {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    // ABC uppercase letters sit in the middle-C octave: C = C4 = MIDI 60.
    // Lowercase letters (already accounted for via octave_shift=+1 in
    // read_note) bump us up an octave.
    let octave = 4 + octave_shift;
    // 12 * (octave + 1) is C-1 to C0 to C1 ... — MIDI's formula.
    let midi = 12 * (octave + 1) + base_semitones;
    let alter = match accidental {
        Some(a) => a as i32,
        None => key_accidentals.for_letter(letter) as i32,
    };
    let total = midi + alter;
    if (0..=127).contains(&total) {
        Some(total as u8)
    } else {
        None
    }
}

/// Convert an ABC duration tuple to a TWANGA denominator. The
/// effective duration in beats is `num / denom * (1 / L)`, so the
/// effective TWANGA denom is `L * denom / num`.
fn duration_to_denom(duration: (u32, u32), length_denom: u32) -> u32 {
    let l = if length_denom == 0 {
        DEFAULT_NOTE_LENGTH_DENOM
    } else {
        length_denom
    };
    let (num, denom) = duration;
    if num == 0 {
        return l;
    }
    let raw = (l as f64 * denom as f64) / num as f64;
    let (snapped, _) = snap_to_power_of_two(raw);
    snapped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_tune_in_c_major() {
        // The simplest possible ABC tune: title, default length,
        // key, then three quarter notes.
        let src = "T:Three Notes\nL:1/4\nK:C\nCDE\n";
        let out = parse(src).expect("parse");
        assert_eq!(out.tab.title.as_deref(), Some("Three Notes"));
        assert_eq!(out.tab.columns.len(), 3);
        assert_eq!(out.tab.columns[0].duration_denom, 4);
        // C, D, E — three single-hit columns.
        for col in &out.tab.columns {
            assert_eq!(col.hits.len(), 1);
        }
    }

    #[test]
    fn missing_key_returns_error() {
        let src = "T:No Key\nL:1/4\nCDE\n";
        assert!(matches!(parse(src), Err(AbcError::MissingKey)));
    }

    #[test]
    fn key_signature_applies_implicit_sharps() {
        // G major has F# as its key signature. An unmarked `F` in
        // the body should parse as F#, not natural F.
        let g_major = "T:F Sharp Test\nL:1/4\nK:G\nF\n";
        let out_g = parse(g_major).expect("parse G");
        // F# is MIDI 66. On standard guitar: E4(64)+2 = string 1 fret 2.
        // Or B3(59)+7 = string 2 fret 7. Lowest is fret 2.
        assert_eq!(out_g.tab.columns[0].hits, vec![(1, 2)]);

        let c_major = "T:F Natural Test\nL:1/4\nK:C\nF\n";
        let out_c = parse(c_major).expect("parse C");
        // F natural is MIDI 65. On standard guitar: E4(64)+1 = string 1 fret 1.
        assert_eq!(out_c.tab.columns[0].hits, vec![(1, 1)]);
    }

    #[test]
    fn explicit_accidental_overrides_key_signature() {
        // In G major (F# implicit), `=F` should be F natural.
        let src = "T:Override\nL:1/4\nK:G\n=F\n";
        let out = parse(src).expect("parse");
        assert_eq!(out.tab.columns[0].hits, vec![(1, 1)]);
    }

    #[test]
    fn octave_marks_shift_pitch_correctly() {
        // C, c, C, are three different octaves. C = C4 (MIDI 60),
        // c = C5 (MIDI 72), C, = C3 (MIDI 48). All should produce
        // distinct hits.
        let src = "T:Octaves\nL:1/4\nK:C\nCcC,\n";
        let out = parse(src).expect("parse");
        assert_eq!(out.tab.columns.len(), 3);
        // The three hits must be at different fret positions
        // (different MIDI values placed on standard guitar).
        let hits: Vec<(u8, u8)> = out.tab.columns.iter().map(|c| c.hits[0]).collect();
        assert!(hits[0] != hits[1] && hits[1] != hits[2]);
    }

    #[test]
    fn duration_modifiers_change_denominator() {
        // L:1/4 base. `C2` = half note (denom 2). `C/2` = eighth (denom 8).
        let src = "T:Durations\nL:1/4\nK:C\nC2 C/2\n";
        let out = parse(src).expect("parse");
        assert_eq!(out.tab.columns.len(), 2);
        assert_eq!(out.tab.columns[0].duration_denom, 2);
        assert_eq!(out.tab.columns[1].duration_denom, 8);
    }

    #[test]
    fn rests_produce_empty_columns() {
        let src = "T:Rests\nL:1/4\nK:C\nC z C\n";
        let out = parse(src).expect("parse");
        assert_eq!(out.tab.columns.len(), 3);
        assert_eq!(out.tab.columns[1].hits.len(), 0);
    }

    #[test]
    fn tempo_header_is_extracted() {
        let src = "T:Fast\nQ:1/4=180\nL:1/4\nK:C\nC\n";
        let out = parse(src).expect("parse");
        assert_eq!(out.tab.tempo, 180);
    }

    #[test]
    fn bar_lines_are_ignored() {
        let src = "T:Bars\nL:1/4\nK:C\nC | D | E |\n";
        let out = parse(src).expect("parse");
        assert_eq!(out.tab.columns.len(), 3);
    }

    #[test]
    fn inferred_tuning_warning_is_always_present() {
        let src = "T:Anything\nL:1/4\nK:C\nC\n";
        let out = parse(src).expect("parse");
        assert!(
            out.warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::InferredTuning { .. }))
        );
    }
}
