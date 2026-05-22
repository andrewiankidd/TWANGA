//! MusicXML parser — converts MusicXML 3.1 partwise scores (the
//! format MuseScore / Sibelius / Guitar Pro all export) into the same
//! [`ParsedTab`] shape the alphaTex parser produces, so downstream
//! renderers + playback code consume both formats uniformly.
//!
//! # Scope
//!
//! The parser targets `score-partwise` (the more common variant —
//! parts contain measures contain notes); `score-timewise` is the
//! transpose of that and isn't covered by this MVP. Extending to
//! timewise is a follow-up if a user shows up with one.
//!
//! Element coverage:
//!
//! - `<work><work-title>` → [`ParsedTab::title`]
//! - `<identification><creator type="composer">` → [`ParsedTab::subtitle`]
//!   (without the `; capo=…` machine annotation — that gets appended
//!   if a capo is present)
//! - `<sound tempo="…"/>` → [`ParsedTab::tempo`] (first occurrence wins)
//! - `<staff-details><staff-tuning>` → [`ParsedTab::tuning_names`]
//! - `<staff-details><capo>N</capo>` → uniform capo, encoded into the
//!   subtitle as `; capo=N` so the result round-trips through
//!   alphaTex without losing the capo state
//! - `<note>` with `<chord/>` → appended to the previous column's
//!   `hits` (chord member)
//! - `<note>` with `<technical><string>` + `<technical><fret>` →
//!   placed directly at the given fret on that string (with
//!   MusicXML's string numbering inverted to TWANGA's `string 1 =
//!   top` convention)
//! - `<note>` with `<pitch>` only → matched against the staff tuning
//!   via `Tuning::match_to_fret`, same logic the recorder uses
//! - `<note>` with `<rest>` → empty column (rest beat)
//!
//! `.mxl` (zipped MusicXML) is supported via [`parse_mxl`]. The zip
//! is expected to follow the MusicXML container spec (an inner
//! `META-INF/container.xml` pointing at the actual score file); we
//! fall back to "first `.xml` / `.musicxml` entry in the archive"
//! when the container manifest is missing, which covers older
//! exporters.
//!
//! # Limitations
//!
//! - Durations not on a clean power-of-2 division (dotted notes,
//!   triplets) are rounded UP to the next power-of-2 denominator
//!   (a dotted half → half). Not lossless; the alternative is to
//!   reject the import outright, which is worse UX. Surfaced via
//!   [`ParseWarning::IrregularDuration`] so the importer can show a
//!   "N notes had non-standard durations" preflight summary.
//! - Multi-voice / multi-staff parts: only the first staff of the
//!   first part is read. Documented limitation.
//! - Time / key signature changes mid-score: ignored (TWANGA's tab
//!   model doesn't have time signature anyway — we serialise notes
//!   in column-by-column order).

use std::io::Cursor;

use quick_xml::Reader;
use quick_xml::events::Event;
use twanga_core::{Capo, MidiNote, TunedString, join_capo_into_subtitle};

use crate::{ParseOutput, ParseWarning, ParsedTab, TabColumn, snap_to_power_of_two};

/// Errors the parser can return. Distinct variants per failure mode
/// so the importer UI can surface targeted messages rather than a
/// generic "import failed".
#[derive(Debug)]
pub enum MusicXmlError {
    /// XML wasn't well-formed at all. Wraps the quick-xml error.
    BadXml(String),
    /// Well-formed XML, but not a MusicXML partwise score (no
    /// `<score-partwise>` root). Could be `<score-timewise>` (not
    /// supported), or another XML format entirely.
    NotPartwise,
    /// `.mxl` archive couldn't be opened or didn't contain an XML
    /// payload.
    BadArchive(String),
    /// The score has no `<part>` elements with playable notes — we
    /// have nothing to render.
    EmptyScore,
    /// A `<staff-tuning>` element referenced a pitch we couldn't
    /// parse (e.g. unknown step or out-of-range octave).
    BadTuning(String),
}

impl std::fmt::Display for MusicXmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadXml(s) => write!(f, "XML parse error: {s}"),
            Self::NotPartwise => write!(
                f,
                "not a MusicXML partwise score (looking for <score-partwise> root)"
            ),
            Self::BadArchive(s) => write!(f, "MXL archive read failed: {s}"),
            Self::EmptyScore => write!(f, "MusicXML has no playable notes"),
            Self::BadTuning(s) => write!(f, "could not parse staff tuning: {s}"),
        }
    }
}
impl std::error::Error for MusicXmlError {}

/// Maximum fret position the pitch-to-fret matcher will reach for.
/// Same constant the recorder uses; out-of-range notes become
/// [`ParseWarning::UnreachableNote`] rather than failing the import.
const MAX_FRET: u8 = 20;

/// Default tempo applied when the score doesn't include a `<sound
/// tempo="…"/>` element. Matches the alphaTex parser's default.
const DEFAULT_TEMPO: u32 = 120;

/// Parse a MusicXML document (raw XML text).
pub fn parse(xml: &str) -> Result<ParseOutput, MusicXmlError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    parse_inner(&mut reader)
}

/// Parse an MXL archive (zipped MusicXML container). Reads the inner
/// XML payload via the MusicXML container spec or — when the
/// `META-INF/container.xml` manifest is missing — falls back to "the
/// first `.xml` / `.musicxml` entry in the archive."
pub fn parse_mxl(bytes: &[u8]) -> Result<ParseOutput, MusicXmlError> {
    let cursor = Cursor::new(bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| MusicXmlError::BadArchive(e.to_string()))?;
    let xml = read_mxl_payload(&mut archive)?;
    parse(&xml)
}

/// Pack a MusicXML payload into the bytes of a well-formed `.mxl`
/// archive (DEFLATE-compressed zip with a `META-INF/container.xml`
/// manifest pointing at the payload). Symmetric inverse of
/// [`parse_mxl`] — bytes written here round-trip back through it.
///
/// Used by tests that need a real `.mxl` to feed the CLI / parser
/// without committing a binary blob to the repo, and available for
/// any caller that needs to emit the MXL transport format from an
/// in-memory MusicXML string.
pub fn write_mxl_bytes(musicxml_payload: &str) -> Vec<u8> {
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    let mut buf: Vec<u8> = Vec::new();
    {
        let mut zip = ZipWriter::new(Cursor::new(&mut buf));
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        let manifest = r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<container>
  <rootfiles>
    <rootfile full-path="score.xml" media-type="application/vnd.recordare.musicxml+xml"/>
  </rootfiles>
</container>"#;
        zip.start_file("META-INF/container.xml", opts)
            .expect("mxl manifest entry");
        std::io::Write::write_all(&mut zip, manifest.as_bytes()).expect("write mxl manifest");

        zip.start_file("score.xml", opts)
            .expect("mxl payload entry");
        std::io::Write::write_all(&mut zip, musicxml_payload.as_bytes())
            .expect("write mxl payload");

        zip.finish().expect("finish mxl archive");
    }
    buf
}

/// Try the MusicXML container manifest first; on miss, fall through
/// to first plausible entry. Producing a clean error rather than
/// silently picking the wrong file matters here — a misparse can
/// look like an empty score and confuse the user.
fn read_mxl_payload(archive: &mut zip::ZipArchive<Cursor<&[u8]>>) -> Result<String, MusicXmlError> {
    use std::io::Read;

    // 1. Container manifest. Path is fixed per the MusicXML spec.
    // We read the manifest into an owned String inside its own
    // scope so the archive borrow is released before we re-borrow
    // for the payload — Rust's borrow checker can't see across
    // method calls otherwise.
    let rootfile: Option<String> = {
        let manifest_text = archive
            .by_name("META-INF/container.xml")
            .ok()
            .and_then(|mut m| {
                let mut buf = String::new();
                m.read_to_string(&mut buf).ok().map(|_| buf)
            });
        manifest_text.as_deref().and_then(parse_container_rootfile)
    };
    if let Some(name) = rootfile {
        let mut payload = archive
            .by_name(&name)
            .map_err(|e| MusicXmlError::BadArchive(format!("rootfile '{name}': {e}")))?;
        let mut out = String::new();
        payload
            .read_to_string(&mut out)
            .map_err(|e| MusicXmlError::BadArchive(e.to_string()))?;
        return Ok(out);
    }

    // 2. Fallback — first non-META-INF .xml/.musicxml entry.
    let names: Vec<String> = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|n| !n.starts_with("META-INF/"))
        .filter(|n| n.ends_with(".xml") || n.ends_with(".musicxml"))
        .collect();
    let first = names.into_iter().next().ok_or_else(|| {
        MusicXmlError::BadArchive(
            "no .xml or .musicxml entry found in archive (and no container manifest)".into(),
        )
    })?;
    let mut payload = archive
        .by_name(&first)
        .map_err(|e| MusicXmlError::BadArchive(e.to_string()))?;
    let mut out = String::new();
    payload
        .read_to_string(&mut out)
        .map_err(|e| MusicXmlError::BadArchive(e.to_string()))?;
    Ok(out)
}

/// Pluck the `full-path` attribute out of a MusicXML container's
/// rootfile element. Minimal XML scan — the container.xml is small
/// (~200 bytes) and has a well-known shape; using the full quick-xml
/// state machine here would be overkill.
fn parse_container_rootfile(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) if e.name().as_ref() == b"rootfile" => {
                for attr in e.attributes().with_checks(false).flatten() {
                    if attr.key.as_ref() == b"full-path" {
                        return std::str::from_utf8(&attr.value).ok().map(str::to_string);
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
}

// ───────────────────────────── core parser ────────────────────────────

/// Internal scratch state we accumulate while streaming through the
/// XML events. Promoted to a struct (rather than free locals) so the
/// dispatch loop stays readable.
#[derive(Debug, Default)]
struct ParserState {
    title: Option<String>,
    composer: Option<String>,
    tempo: Option<u32>,
    /// MusicXML's `<divisions>` element — divisions per quarter note,
    /// used to convert each note's `<duration>` into a TWANGA
    /// denominator. Defaults to 1 if the file omits it (which is
    /// technically against spec but seen in malformed exports).
    divisions: u32,
    /// `<staff-tuning>` collected in MusicXML order (line=1 first,
    /// which is the LOWEST string in MusicXML convention). We invert
    /// before storing in [`ParsedTab`] so string 1 is the highest
    /// (TWANGA convention, matches alphaTex).
    staff_tuning_lines: Vec<TunedString>,
    /// Uniform capo from `<capo>N</capo>` if present.
    capo_fret: Option<u8>,
    /// Output columns we're building up as we walk the score.
    columns: Vec<TabColumn>,
    warnings: Vec<ParseWarning>,
    /// `id` attribute of the FIRST `<part>` we encountered. Any
    /// later `<part>` is ignored — multi-part scores (e.g. four
    /// stacked guitar parts) would otherwise concatenate every
    /// part's tuning + notes into one tab, which is gibberish. The
    /// module docs commit to "only the first staff of the first
    /// part is read"; this field enforces it.
    first_part_id: Option<String>,
    /// True while we're inside the first `<part>` (i.e. data should
    /// be collected). False when we've moved past the first part
    /// into subsequent ones we'll ignore.
    in_first_part: bool,
}

fn parse_inner(reader: &mut Reader<&[u8]>) -> Result<ParseOutput, MusicXmlError> {
    let mut state = ParserState::default();
    let mut path: Vec<String> = Vec::new();
    let mut buf = Vec::new();
    let mut saw_partwise_root = false;
    let mut current_text = String::new();
    // Per-note scratch — reset on each <note> start.
    let mut note_scratch = NoteScratch::default();

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => return Err(MusicXmlError::BadXml(e.to_string())),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = std::str::from_utf8(e.name().as_ref())
                    .unwrap_or("")
                    .to_string();
                if name == "score-partwise" {
                    saw_partwise_root = true;
                } else if name == "score-timewise" {
                    return Err(MusicXmlError::NotPartwise);
                } else if name == "part" {
                    // First `<part>` we see is the one we keep. The
                    // `id` attribute distinguishes; we only collect
                    // data while we're inside the matching part.
                    // `<part-list>` declares parts up-front but has
                    // no `<part>` children — only the actual
                    // `<part>` elements at score-partwise level do.
                    // The `part-list` path-frame ancestor check
                    // gates which kind of `<part>` we're seeing.
                    let inside_part_list = path.iter().any(|p| p == "part-list");
                    if !inside_part_list {
                        let id = e
                            .attributes()
                            .with_checks(false)
                            .flatten()
                            .find(|a| a.key.as_ref() == b"id")
                            .and_then(|a| std::str::from_utf8(&a.value).ok().map(str::to_string))
                            .unwrap_or_default();
                        if state.first_part_id.is_none() {
                            state.first_part_id = Some(id.clone());
                        }
                        state.in_first_part = state
                            .first_part_id
                            .as_deref()
                            .map(|first| first == id)
                            .unwrap_or(false);
                    }
                } else if name == "sound" && state.tempo.is_none() {
                    for attr in e.attributes().with_checks(false).flatten() {
                        if attr.key.as_ref() == b"tempo" {
                            if let Ok(s) = std::str::from_utf8(&attr.value) {
                                if let Ok(t) = s.trim().parse::<f64>() {
                                    let rounded = t.round() as i64;
                                    if (20..=400).contains(&rounded) {
                                        state.tempo = Some(rounded as u32);
                                    }
                                }
                            }
                        }
                    }
                } else if name == "note" {
                    note_scratch = NoteScratch::default();
                }
                path.push(name);
                current_text.clear();
            }
            Ok(Event::Empty(e)) => {
                // Self-closing tags carry meaning when they're
                // `<chord/>` or `<rest/>` — record on the current
                // note's scratch.
                let qname = e.name();
                let name = std::str::from_utf8(qname.as_ref()).unwrap_or("");
                if name == "chord" {
                    note_scratch.is_chord_member = true;
                } else if name == "rest" {
                    note_scratch.is_rest = true;
                } else if name == "sound" && state.tempo.is_none() {
                    for attr in e.attributes().with_checks(false).flatten() {
                        if attr.key.as_ref() == b"tempo" {
                            if let Ok(s) = std::str::from_utf8(&attr.value) {
                                if let Ok(t) = s.trim().parse::<f64>() {
                                    let rounded = t.round() as i64;
                                    if (20..=400).contains(&rounded) {
                                        state.tempo = Some(rounded as u32);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Ok(Event::Text(t)) => {
                // `unescape` decodes `&amp;` / `&lt;` / etc. into
                // their literal characters — the right behaviour for
                // text content like `<work-title>Rock &amp; Roll</work-title>`.
                if let Ok(s) = t.unescape() {
                    current_text.push_str(&s);
                }
            }
            Ok(Event::End(_)) => {
                let closing = path.pop().unwrap_or_default();
                handle_element_close(
                    &closing,
                    &path,
                    &current_text,
                    &mut state,
                    &mut note_scratch,
                )?;
                current_text.clear();
            }
            _ => {}
        }
        buf.clear();
    }

    if !saw_partwise_root {
        return Err(MusicXmlError::NotPartwise);
    }
    if state.columns.is_empty() {
        return Err(MusicXmlError::EmptyScore);
    }

    // Build the resulting ParsedTab. MusicXML lists tuning lines
    // bottom-up (line=1 is the lowest string); TWANGA lists string 1
    // at the top. Reverse before serialising names.
    let mut tuning_strings = state.staff_tuning_lines;
    tuning_strings.reverse();

    let tuning_names: Vec<String> = tuning_strings.iter().map(|s| s.name.clone()).collect();

    // Compose the subtitle: prefer the composer when present, else
    // the staff-tuning name. Append `; capo=N` if a capo is set so
    // the file round-trips through alphaTex's `\subtitle` convention.
    let base_subtitle = match (&state.composer, tuning_strings.is_empty()) {
        (Some(c), _) => Some(c.clone()),
        (None, false) => Some(format!(
            "MusicXML import ({} strings)",
            tuning_strings.len()
        )),
        (None, true) => None,
    };
    let subtitle = match (base_subtitle, state.capo_fret) {
        (Some(s), Some(fret)) if fret > 0 => {
            // `join_capo_into_subtitle` expects a `Capo` typed
            // against a known string count — build a uniform capo
            // against the staff tuning we just inverted. When the
            // score had no `<staff-tuning>` at all (pitch-only
            // notation), fall back to a placeholder length of 1
            // so the capo doesn't collapse to "no-op" via
            // `Capo::is_none`. The user will pick a real tuning in
            // the Importer UI; the capo annotation survives so the
            // round-trip preserves their intent.
            let string_count = tuning_strings.len().max(1);
            let capo = Capo::uniform(string_count, fret as i32);
            Some(join_capo_into_subtitle(&s, &capo))
        }
        (s, _) => s,
    };

    Ok(ParseOutput {
        tab: ParsedTab {
            tempo: state.tempo.unwrap_or(DEFAULT_TEMPO),
            title: state.title,
            subtitle,
            tuning_names,
            columns: state.columns,
        },
        warnings: state.warnings,
    })
}

#[derive(Debug, Default)]
struct NoteScratch {
    is_chord_member: bool,
    is_rest: bool,
    // Explicit string + fret from `<technical>`. MusicXML's string
    // numbering is bottom-up (string 1 = lowest); we invert when
    // applying.
    explicit_string: Option<u8>,
    explicit_fret: Option<u8>,
    // Pitch (used only when `<technical><fret>` isn't present, so
    // we can infer placement against the staff tuning).
    pitch_step: Option<char>,
    pitch_alter: i32,
    pitch_octave: Option<i32>,
    // Duration in MusicXML divisions.
    duration: Option<u32>,
    // Original duration string verbatim, used for warning surfacing
    // when we have to round.
    raw_duration: String,
    // Staff-tuning being accumulated (step + octave). The `line`
    // attribute on `<staff-tuning>` is read positionally — we use
    // the order entries appear in the XML to assign line numbers,
    // matching MusicXML's convention that lines are listed
    // bottom-up. Captured at element close time, no per-tag scratch
    // needed.
    tuning_step: Option<char>,
    tuning_alter: i32,
    tuning_octave: Option<i32>,
}

/// Per-element close handler. Splits out of the main loop so the
/// dispatch table is readable.
fn handle_element_close(
    closing: &str,
    parent_path: &[String],
    text: &str,
    state: &mut ParserState,
    note: &mut NoteScratch,
) -> Result<(), MusicXmlError> {
    let parent = parent_path.last().map(String::as_str).unwrap_or("");
    let grandparent = if parent_path.len() >= 2 {
        parent_path[parent_path.len() - 2].as_str()
    } else {
        ""
    };
    match closing {
        // ── Metadata ──
        // Score-level metadata (`<work>` / `<identification>`) appears
        // OUTSIDE any `<part>`, so the first-part gate doesn't apply.
        "work-title" if parent == "work" => {
            state.title = some_if_nonempty(text);
        }
        "creator" if parent == "identification" && state.composer.is_none() => {
            // <creator type="composer">Bach</creator> — we capture
            // any creator type since users tag inconsistently;
            // the first creator wins so we don't churn subtitle
            // text on every subsequent `<creator>` element.
            state.composer = some_if_nonempty(text);
        }
        // Part-scoped metadata (`<attributes>` lives inside `<part>`).
        // Gate on `in_first_part` so later parts' divisions / capo
        // settings don't overwrite the first part's. This is the
        // "only first part is read" promise from the module docs.
        "divisions" if parent == "attributes" && state.in_first_part => {
            if let Ok(d) = text.trim().parse::<u32>() {
                if d > 0 {
                    state.divisions = d;
                }
            }
        }
        "capo" if parent == "staff-details" && state.in_first_part => {
            if let Ok(f) = text.trim().parse::<u8>() {
                state.capo_fret = Some(f);
            }
        }
        // ── Tuning lines ──
        "tuning-step" if parent == "staff-tuning" => {
            note.tuning_step = text.trim().chars().next();
        }
        "tuning-alter" if parent == "staff-tuning" => {
            note.tuning_alter = text.trim().parse().unwrap_or(0);
        }
        "tuning-octave" if parent == "staff-tuning" => {
            note.tuning_octave = text.trim().parse().ok();
        }
        "staff-tuning" if state.in_first_part => {
            // Stable line attribute lives on the start tag; we
            // recorded nothing for it because we'd need to capture
            // attributes at start time. Use list length + 1 as the
            // line so order in the file == order in our vec (which
            // we'll invert before emitting). Gated on `in_first_part`
            // — multi-part scores would otherwise pile every guitar's
            // tuning onto one combined list.
            let line = state.staff_tuning_lines.len() as u8 + 1;
            let step = note
                .tuning_step
                .take()
                .ok_or_else(|| MusicXmlError::BadTuning(format!("line {line}: missing step")))?;
            let oct = note
                .tuning_octave
                .take()
                .ok_or_else(|| MusicXmlError::BadTuning(format!("line {line}: missing octave")))?;
            let alter = std::mem::take(&mut note.tuning_alter);
            let midi = step_alter_octave_to_midi(step, alter, oct)
                .ok_or_else(|| MusicXmlError::BadTuning(format!("line {line}: out of range")))?;
            state.staff_tuning_lines.push(TunedString {
                name: MidiNote(midi).name(),
                open: MidiNote(midi),
                fret_offset: 0,
            });
            note.tuning_step = None;
            note.tuning_octave = None;
            note.tuning_alter = 0;
        }
        // ── Note bits ──
        "step" if grandparent == "note" && parent == "pitch" => {
            note.pitch_step = text.trim().chars().next();
        }
        "alter" if grandparent == "note" && parent == "pitch" => {
            note.pitch_alter = text.trim().parse().unwrap_or(0);
        }
        "octave" if grandparent == "note" && parent == "pitch" => {
            note.pitch_octave = text.trim().parse().ok();
        }
        "string" if grandparent == "note" && parent == "technical" => {
            note.explicit_string = text.trim().parse().ok();
        }
        "fret" if grandparent == "note" && parent == "technical" => {
            note.explicit_fret = text.trim().parse().ok();
        }
        "duration" if parent == "note" => {
            note.raw_duration = text.trim().to_string();
            note.duration = text.trim().parse().ok();
        }
        "note" => {
            // Only commit notes from the first part — same "first
            // part is read" rule as the metadata above. Notes from
            // later parts (e.g. a second guitar tabbed in the same
            // file) are silently skipped; their data is still
            // scratch-cleared so the next note starts fresh.
            if state.in_first_part {
                commit_note(state, note);
            }
            *note = NoteScratch::default();
        }
        _ => {}
    }
    Ok(())
}

/// Build a column (or append to the last one for a chord member) out
/// of the accumulated [`NoteScratch`] and reset for the next note.
fn commit_note(state: &mut ParserState, note: &NoteScratch) {
    // Duration → TWANGA denominator. divisions=Q means quarter=Q,
    // so denom = 4 * Q / duration. Snap to a clean power of 2; if
    // we have to round, surface a warning.
    let duration = note.duration.unwrap_or(state.divisions.max(1));
    let q = state.divisions.max(1);
    let raw_denom = (4 * q) as f64 / duration as f64;
    let (denom, rounded) = snap_to_power_of_two(raw_denom);
    if rounded {
        state.warnings.push(ParseWarning::IrregularDuration {
            column_index: state.columns.len(),
            raw_duration: note.raw_duration.clone(),
        });
    }

    if note.is_rest {
        state.columns.push(TabColumn {
            duration_denom: denom,
            hits: Vec::new(),
            articulation: None,
        });
        return;
    }

    // Resolve string+fret for the played note.
    let placement = if let (Some(s), Some(f)) = (note.explicit_string, note.explicit_fret) {
        // MusicXML strings count from the lowest (string 1 = low E
        // on guitar). TWANGA string 1 = highest. Invert against the
        // recorded staff-tuning size; if we don't know the count
        // yet (no <staff-tuning>), keep the value as-is and emit a
        // warning so the importer surfaces the mismatch.
        let count = state.staff_tuning_lines.len() as u8;
        if count == 0 {
            state.warnings.push(ParseWarning::MissingStringTuning {
                referenced_string: s,
            });
            Some((s, f))
        } else if s >= 1 && s <= count {
            Some((count - s + 1, f))
        } else {
            state.warnings.push(ParseWarning::MissingStringTuning {
                referenced_string: s,
            });
            None
        }
    } else if let (Some(step), Some(oct)) = (note.pitch_step, note.pitch_octave) {
        // No explicit fret: match against the staff tuning, same
        // algorithm the recorder uses for live pitch detection.
        match infer_placement(&state.staff_tuning_lines, step, note.pitch_alter, oct) {
            Some(p) => Some(p),
            None => {
                state.warnings.push(ParseWarning::UnreachableNote {
                    column_index: state.columns.len(),
                    note: step_alter_octave_to_midi(step, note.pitch_alter, oct)
                        .map(|m| MidiNote(m).name())
                        .unwrap_or_else(|| "?".into()),
                });
                None
            }
        }
    } else {
        // Note without pitch or technical info — treat as a rest
        // rather than fail the whole import. Captures malformed
        // exports that omit `<rest/>`.
        None
    };

    if note.is_chord_member {
        if let Some(last) = state.columns.last_mut() {
            if let Some(p) = placement {
                last.hits.push(p);
                return;
            }
            // No placement for a chord member is a no-op (don't
            // append a fresh column for it).
            return;
        }
        // Chord marker on the very first note is malformed; fall
        // through and treat as a fresh column.
    }
    state.columns.push(TabColumn {
        duration_denom: denom,
        hits: placement.map(|p| vec![p]).unwrap_or_default(),
        articulation: None,
    });
}

/// Match a pitch (step + alter + octave) against the staff tuning,
/// returning the smallest non-negative fret position. Identical
/// algorithm to `Tuning::match_to_fret` modulo the input shape —
/// re-implemented here on raw `TunedString`s rather than going through
/// the `Tuning` type because the tuning may not be complete at the
/// time we hit our first note.
fn infer_placement(
    strings: &[TunedString],
    step: char,
    alter: i32,
    octave: i32,
) -> Option<(u8, u8)> {
    if strings.is_empty() {
        return None;
    }
    let target = step_alter_octave_to_midi(step, alter, octave)?;
    let mut best: Option<(u8, u8)> = None;
    for (idx, s) in strings.iter().enumerate() {
        let delta = target as i32 - s.open.0 as i32 - s.fret_offset as i32;
        if (0..=MAX_FRET as i32).contains(&delta) {
            let fret = delta as u8;
            let twanga_string = strings.len() as u8 - idx as u8;
            best = match best {
                None => Some((twanga_string, fret)),
                Some((_, prev)) if fret < prev => Some((twanga_string, fret)),
                Some(prev) => Some(prev),
            };
        }
    }
    best
}

/// Convert MusicXML's step+alter+octave triple to a MIDI number.
/// `step` is one of `A` `B` `C` `D` `E` `F` `G`; `alter` is sharps
/// (positive) / flats (negative); `octave` is the standard scientific
/// pitch notation octave (middle C = C4 = MIDI 60).
fn step_alter_octave_to_midi(step: char, alter: i32, octave: i32) -> Option<u8> {
    let step_semitones = match step.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    // MIDI: C-1 = 0, C0 = 12, C4 = 60. Formula: 12 * (octave + 1) + step.
    let midi = 12 * (octave + 1) + step_semitones + alter;
    if (0..=127).contains(&midi) {
        Some(midi as u8)
    } else {
        None
    }
}

fn some_if_nonempty(s: &str) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

// Suppress an unused-import warning on the `Capo` type — referenced
// only in doc comments at the moment, but the parser will likely want
// the type directly when we add explicit capo round-tripping into the
// ParsedTab struct (rather than via subtitle annotation). Keep the
// import so a future addition doesn't have to re-justify it.
#[allow(dead_code)]
fn _ensure_capo_in_scope(_c: Capo) {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal hand-written MusicXML fixture: one measure, three
    /// guitar notes (open E low, fret 3 on the A string, open D),
    /// standard guitar tuning declared via <staff-tuning>. Hand-
    /// written rather than pulled from MuseScore so the test stays
    /// independent of any specific exporter's quirks.
    const SIMPLE_GUITAR: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<score-partwise version="3.1">
  <work>
    <work-title>Test Song</work-title>
  </work>
  <identification>
    <creator type="composer">Test Composer</creator>
  </identification>
  <part-list>
    <score-part id="P1">
      <part-name>Guitar</part-name>
    </score-part>
  </part-list>
  <part id="P1">
    <measure number="1">
      <attributes>
        <divisions>4</divisions>
        <staff-details number="1">
          <staff-lines>6</staff-lines>
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
      <note>
        <pitch><step>C</step><octave>3</octave></pitch>
        <duration>4</duration>
        <technical><string>2</string><fret>3</fret></technical>
      </note>
      <note>
        <pitch><step>D</step><octave>3</octave></pitch>
        <duration>4</duration>
        <technical><string>3</string><fret>0</fret></technical>
      </note>
    </measure>
  </part>
</score-partwise>"#;

    #[test]
    fn parses_simple_guitar_score() {
        let out = parse(SIMPLE_GUITAR).expect("parse");
        assert_eq!(out.tab.tempo, 100);
        assert_eq!(out.tab.title.as_deref(), Some("Test Song"));
        // Composer wins for subtitle.
        assert!(
            out.tab
                .subtitle
                .as_deref()
                .unwrap()
                .contains("Test Composer")
        );
        // 6 strings, MusicXML order inverted (string 1 in TWANGA = highest = E4).
        assert_eq!(
            out.tab.tuning_names,
            vec!["E4", "B3", "G3", "D3", "A2", "E2"]
        );
        // 3 columns, each a single note.
        assert_eq!(out.tab.columns.len(), 3);
        for col in &out.tab.columns {
            assert_eq!(col.duration_denom, 4); // quarter notes (divisions=4, duration=4)
            assert_eq!(col.hits.len(), 1);
        }
        // String numbering inverted: MusicXML string=1 (lowest E) →
        // TWANGA string=6.
        assert_eq!(out.tab.columns[0].hits[0], (6, 0));
        assert_eq!(out.tab.columns[1].hits[0], (5, 3));
        assert_eq!(out.tab.columns[2].hits[0], (4, 0));
        assert!(out.warnings.is_empty());
    }

    #[test]
    fn rejects_non_partwise_root() {
        let xml = r#"<?xml version="1.0"?><something-else/>"#;
        let err = parse(xml).expect_err("should reject");
        assert!(matches!(err, MusicXmlError::NotPartwise));
    }

    #[test]
    fn rejects_timewise_score() {
        let xml = r#"<?xml version="1.0"?><score-timewise version="3.1"/>"#;
        let err = parse(xml).expect_err("should reject");
        assert!(matches!(err, MusicXmlError::NotPartwise));
    }

    #[test]
    fn empty_score_returns_error() {
        let xml =
            r#"<?xml version="1.0"?><score-partwise version="3.1"><part-list/></score-partwise>"#;
        let err = parse(xml).expect_err("should reject");
        assert!(matches!(err, MusicXmlError::EmptyScore));
    }

    #[test]
    fn chord_member_appends_to_previous_column() {
        let xml = r#"<?xml version="1.0"?>
<score-partwise version="3.1">
  <part id="P1">
    <measure number="1">
      <attributes><divisions>4</divisions></attributes>
      <note>
        <pitch><step>E</step><octave>2</octave></pitch>
        <duration>4</duration>
        <technical><string>1</string><fret>0</fret></technical>
      </note>
      <note>
        <chord/>
        <pitch><step>A</step><octave>2</octave></pitch>
        <duration>4</duration>
        <technical><string>2</string><fret>0</fret></technical>
      </note>
    </measure>
  </part>
</score-partwise>"#;
        let out = parse(xml).expect("parse");
        // Two notes, but only one column (the second is a chord
        // member of the first).
        assert_eq!(out.tab.columns.len(), 1);
        assert_eq!(out.tab.columns[0].hits.len(), 2);
    }

    #[test]
    fn rest_emits_empty_column() {
        let xml = r#"<?xml version="1.0"?>
<score-partwise version="3.1">
  <part id="P1">
    <measure number="1">
      <attributes><divisions>4</divisions></attributes>
      <note><rest/><duration>4</duration></note>
    </measure>
  </part>
</score-partwise>"#;
        let out = parse(xml).expect("parse");
        assert_eq!(out.tab.columns.len(), 1);
        assert!(out.tab.columns[0].hits.is_empty());
    }

    #[test]
    fn capo_round_trips_through_subtitle() {
        let xml = r#"<?xml version="1.0"?>
<score-partwise version="3.1">
  <identification><creator type="composer">Author</creator></identification>
  <part id="P1">
    <measure number="1">
      <attributes>
        <divisions>4</divisions>
        <staff-details><capo>3</capo></staff-details>
      </attributes>
      <note><rest/><duration>4</duration></note>
    </measure>
  </part>
</score-partwise>"#;
        let out = parse(xml).expect("parse");
        let subtitle = out.tab.subtitle.as_deref().expect("subtitle present");
        assert!(
            subtitle.contains("Author"),
            "subtitle keeps composer: {subtitle}"
        );
        assert!(
            subtitle.contains("capo=3"),
            "subtitle carries machine annotation: {subtitle}"
        );
    }

    #[test]
    fn irregular_duration_emits_warning() {
        let xml = r#"<?xml version="1.0"?>
<score-partwise version="3.1">
  <part id="P1">
    <measure number="1">
      <attributes><divisions>4</divisions></attributes>
      <note>
        <pitch><step>E</step><octave>2</octave></pitch>
        <duration>3</duration>
        <technical><string>1</string><fret>0</fret></technical>
      </note>
    </measure>
  </part>
</score-partwise>"#;
        // divisions=4 → quarter=4. duration=3 is a dotted-eighth
        // analogue (3/4 of a quarter). Should warn.
        //
        // Note: this fixture also lacks `<staff-tuning>` while
        // having a `<string>1</string>` reference, so a
        // `MissingStringTuning` warning fires too — we check for
        // the IrregularDuration specifically rather than asserting
        // a total count.
        let out = parse(xml).expect("parse");
        assert!(
            out.warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::IrregularDuration { .. })),
            "expected an IrregularDuration warning, got {:?}",
            out.warnings
        );
    }

    #[test]
    fn missing_divisions_falls_back_safely() {
        // No <divisions> declared — the parser defaults divisions=1
        // (one division per quarter). Duration=1 → denom=4 (quarter),
        // duration=2 → denom=2 (half).
        let xml = r#"<?xml version="1.0"?>
<score-partwise version="3.1">
  <part id="P1">
    <measure number="1">
      <note>
        <pitch><step>E</step><octave>2</octave></pitch>
        <duration>1</duration>
        <technical><string>1</string><fret>0</fret></technical>
      </note>
    </measure>
  </part>
</score-partwise>"#;
        let out = parse(xml).expect("parse");
        assert_eq!(out.tab.columns.len(), 1);
        assert_eq!(out.tab.columns[0].duration_denom, 4);
    }

    #[test]
    fn pitch_inferred_from_tuning_when_no_explicit_fret() {
        // No <technical> on the note — should match against the
        // staff tuning. E2 on the low E string of a guitar tuning
        // should land at string 6 (TWANGA convention), fret 0.
        let xml = r#"<?xml version="1.0"?>
<score-partwise version="3.1">
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
      <note>
        <pitch><step>E</step><octave>2</octave></pitch>
        <duration>4</duration>
      </note>
    </measure>
  </part>
</score-partwise>"#;
        let out = parse(xml).expect("parse");
        assert_eq!(out.tab.columns.len(), 1);
        assert_eq!(out.tab.columns[0].hits.len(), 1);
        // E2 → string 6 (TWANGA's low E), fret 0.
        assert_eq!(out.tab.columns[0].hits[0], (6, 0));
    }

    #[test]
    fn unreachable_pitch_emits_warning() {
        // High pitch beyond the fretboard — should warn and drop
        // the note (no hit recorded) rather than fail the parse.
        let xml = r#"<?xml version="1.0"?>
<score-partwise version="3.1">
  <part id="P1">
    <measure number="1">
      <attributes>
        <divisions>4</divisions>
        <staff-details>
          <staff-tuning line="1"><tuning-step>E</tuning-step><tuning-octave>2</tuning-octave></staff-tuning>
          <staff-tuning line="6"><tuning-step>E</tuning-step><tuning-octave>4</tuning-octave></staff-tuning>
        </staff-details>
      </attributes>
      <note>
        <pitch><step>C</step><octave>9</octave></pitch>
        <duration>4</duration>
      </note>
    </measure>
  </part>
</score-partwise>"#;
        let out = parse(xml).expect("parse");
        assert_eq!(out.tab.columns.len(), 1);
        // The note IS in the score but couldn't be placed — column
        // has no hits (effectively a rest) and a warning was emitted.
        assert!(out.tab.columns[0].hits.is_empty());
        assert!(
            out.warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::UnreachableNote { .. })),
            "expected an UnreachableNote warning, got {:?}",
            out.warnings
        );
    }

    #[test]
    fn step_alter_octave_to_midi_matches_known_pitches() {
        // Reference values from a MIDI chart.
        assert_eq!(step_alter_octave_to_midi('C', 0, 4), Some(60)); // middle C
        assert_eq!(step_alter_octave_to_midi('A', 0, 4), Some(69)); // concert A
        assert_eq!(step_alter_octave_to_midi('E', 0, 2), Some(40)); // guitar low E
        assert_eq!(step_alter_octave_to_midi('C', 1, 4), Some(61)); // C#4
        assert_eq!(step_alter_octave_to_midi('B', -1, 4), Some(70)); // Bb4
        // Out of MIDI range.
        assert_eq!(step_alter_octave_to_midi('C', 0, -2), None);
        assert_eq!(step_alter_octave_to_midi('C', 0, 10), None);
        // Bad step.
        assert_eq!(step_alter_octave_to_midi('H', 0, 4), None);
    }

    #[test]
    fn snap_to_power_of_two_handles_clean_values() {
        assert_eq!(snap_to_power_of_two(1.0), (1, false));
        assert_eq!(snap_to_power_of_two(4.0), (4, false));
        assert_eq!(snap_to_power_of_two(8.0), (8, false));
        // Dotted quarter (1.5x quarter, denom=8/3 ≈ 2.67) → rounds.
        let (denom, rounded) = snap_to_power_of_two(2.67);
        assert!(rounded);
        assert!(denom == 2 || denom == 4);
        // NaN / negative → defensive default.
        assert_eq!(snap_to_power_of_two(f64::NAN), (4, true));
        assert_eq!(snap_to_power_of_two(-1.0), (4, true));
    }
}
