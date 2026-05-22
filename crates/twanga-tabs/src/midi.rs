//! Standard MIDI File (.mid / .midi) parser — converts SMF format 0
//! and format 1 files into the same [`ParsedTab`] shape the other
//! parsers produce.
//!
//! # Scope
//!
//! MIDI carries pitch + timing + tempo + track names, but **not**
//! string / fret assignments — every note arrives as a raw MIDI
//! number with no instrument-physical context. The parser places
//! each pitch on a default tuning (standard 6-string guitar EADGBe)
//! via [`Tuning::match_to_fret`], surfacing an
//! [`ParseWarning::InferredTuning`] so the user knows the target
//! tuning was guessed rather than declared.
//!
//! Element coverage:
//!
//! - `MetaMessage::TrackName` → [`ParsedTab::title`] (first non-
//!   empty name across all tracks wins)
//! - `MetaMessage::Tempo` → [`ParsedTab::tempo`] (first occurrence;
//!   tempo changes mid-file aren't represented in TWANGA's column
//!   model, so we use the initial tempo and warn-future-self about
//!   the rest only implicitly)
//! - `MidiMessage::NoteOn` with velocity > 0 → a hit on a column.
//!   Simultaneous note-ons (same absolute tick) collapse into a
//!   single column (chord)
//! - `MidiMessage::NoteOff` (and zero-velocity note-on) → not used
//!   directly; column duration comes from the inter-onset interval
//!   between consecutive note-on ticks, snapped to the nearest
//!   power-of-2 denominator
//!
//! # Limitations
//!
//! - **Multi-track files** — only the first note-bearing track is
//!   read; any other note-bearing tracks surface as
//!   [`ParseWarning::SkippedTrack`]. Matches the MusicXML parser's
//!   "first part wins" posture; documented in the importer UI.
//! - **SMPTE timing** — files using SMPTE timecode (frame-based)
//!   are rejected with [`MidiError::SmpteTiming`]. PPQ (metrical)
//!   timing covers ~every MIDI file in the wild; SMPTE is mostly
//!   broadcast / film-scoring territory.
//! - **No string / fret data** — every pitch is placed on the
//!   default tuning. Users can retune via the regular
//!   `--tuning <slug>` path on `twanga play` after importing.
//! - **Tempo changes** — only the first tempo event is honoured.
//!   TWANGA's tab model has no per-column tempo so a changing-
//!   tempo MIDI ends up at its initial rate.

use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};
use twanga_core::{MidiNote, Tuning};

use crate::{ParseOutput, ParseWarning, ParsedTab, TabColumn, snap_to_power_of_two};

/// Errors the MIDI parser can return. Distinct variants per failure
/// mode so the importer UI can surface targeted messages rather than
/// a generic "import failed".
#[derive(Debug)]
pub enum MidiError {
    /// The bytes weren't a valid SMF file (bad header, truncated
    /// track, malformed event, etc.). Wraps midly's error string.
    BadFile(String),
    /// The file uses SMPTE timecode timing instead of metrical (PPQ).
    /// We don't translate frame-based timing into TWANGA's
    /// denominator model in v1; user can re-export the file with
    /// metrical timing (every DAW supports this).
    SmpteTiming,
    /// No note-on events with positive velocity anywhere in the
    /// file — nothing to render.
    EmptyScore,
}

impl std::fmt::Display for MidiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadFile(s) => write!(f, "MIDI parse error: {s}"),
            Self::SmpteTiming => write!(
                f,
                "MIDI uses SMPTE (frame-based) timing — re-export with metrical (PPQ) timing"
            ),
            Self::EmptyScore => write!(f, "MIDI has no playable notes"),
        }
    }
}
impl std::error::Error for MidiError {}

/// Maximum fret position the pitch-to-fret matcher will reach for.
/// Matches the MusicXML parser's constant.
const MAX_FRET: u8 = 20;

/// Default tempo applied when the file has no tempo meta event. SMF
/// spec says "120 BPM if absent"; matches the alphaTex parser's
/// default for consistency across the import surface.
const DEFAULT_TEMPO: u32 = 120;

/// Default duration denominator for the final column (where we
/// don't have a "next note" to measure the inter-onset interval
/// against). Quarter note = 4.
const DEFAULT_TAIL_DENOM: u32 = 4;

/// Parse a MIDI file from its raw bytes.
pub fn parse(bytes: &[u8]) -> Result<ParseOutput, MidiError> {
    let smf = Smf::parse(bytes).map_err(|e| MidiError::BadFile(e.to_string()))?;

    let ppq: u32 = match smf.header.timing {
        Timing::Metrical(n) => n.as_int() as u32,
        Timing::Timecode(..) => return Err(MidiError::SmpteTiming),
    };
    if ppq == 0 {
        return Err(MidiError::BadFile("PPQ is zero".into()));
    }

    // ── Tempo + title — scanned across ALL tracks because format-1
    //    MIDI puts tempo on track 0 (the conductor) and notes on
    //    later tracks. First non-zero value wins to keep the result
    //    deterministic on multi-tempo files.
    let mut tempo_bpm: Option<u32> = None;
    let mut title: Option<String> = None;
    for track in &smf.tracks {
        for ev in track.iter() {
            match ev.kind {
                TrackEventKind::Meta(MetaMessage::Tempo(usec_per_quarter))
                    if tempo_bpm.is_none() =>
                {
                    let usec = usec_per_quarter.as_int();
                    if let Some(bpm) = 60_000_000u32.checked_div(usec) {
                        tempo_bpm = Some(bpm);
                    }
                }
                TrackEventKind::Meta(MetaMessage::TrackName(name)) if title.is_none() => {
                    let s = std::str::from_utf8(name).unwrap_or("").trim();
                    if !s.is_empty() {
                        title = Some(s.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    // ── Track selection — first track that contains any note-on
    //    with positive velocity. Skipped notes on other tracks
    //    surface as warnings so the user knows what was dropped.
    let mut chosen: Option<usize> = None;
    let mut skipped: Vec<(usize, String)> = Vec::new();
    for (i, track) in smf.tracks.iter().enumerate() {
        if track_has_note(track) {
            if chosen.is_none() {
                chosen = Some(i);
            } else {
                skipped.push((i, track_name(track)));
            }
        }
    }
    let chosen = chosen.ok_or(MidiError::EmptyScore)?;

    // ── Walk the chosen track, accumulating one column per distinct
    //    note-on absolute-tick value. Chord members (same tick)
    //    collapse into the same column.
    let target_tuning = Tuning::standard_guitar();
    let mut columns: Vec<TabColumn> = Vec::new();
    let mut warnings: Vec<ParseWarning> = Vec::new();

    let mut current_tick: u64 = 0;
    let mut col_start_tick: Option<u64> = None;
    let mut col_pitches: Vec<u8> = Vec::new();

    for ev in smf.tracks[chosen].iter() {
        current_tick += ev.delta.as_int() as u64;
        if let TrackEventKind::Midi {
            message: MidiMessage::NoteOn { key, vel },
            ..
        } = ev.kind
        {
            // Zero-velocity note-on = note-off per SMF spec; ignore.
            if vel.as_int() == 0 {
                continue;
            }
            let pitch = key.as_int();
            match col_start_tick {
                None => {
                    col_start_tick = Some(current_tick);
                    col_pitches.push(pitch);
                }
                Some(t) if t == current_tick => {
                    col_pitches.push(pitch);
                }
                Some(t) => {
                    // New column starting — flush the previous one
                    // with the inter-onset interval as its duration.
                    let dt = current_tick.saturating_sub(t);
                    let (denom, rounded) = tick_delta_to_denom(dt, ppq);
                    push_column(
                        &mut columns,
                        &mut warnings,
                        denom,
                        rounded,
                        &col_pitches,
                        &target_tuning,
                    );
                    col_pitches.clear();
                    col_start_tick = Some(current_tick);
                    col_pitches.push(pitch);
                }
            }
        }
    }
    // Final column gets a default tail duration since there's no
    // following onset to measure against.
    if !col_pitches.is_empty() {
        push_column(
            &mut columns,
            &mut warnings,
            DEFAULT_TAIL_DENOM,
            false,
            &col_pitches,
            &target_tuning,
        );
    }

    if columns.is_empty() {
        return Err(MidiError::EmptyScore);
    }

    // Skipped-track warnings come last so the importer UI lists
    // per-column issues first then "and also, N tracks were dropped".
    for (idx, name) in skipped {
        warnings.push(ParseWarning::SkippedTrack {
            index: idx,
            name: if name.is_empty() {
                format!("track {idx}")
            } else {
                name
            },
        });
    }
    // MIDI carries no tuning info so the target was inferred. Always
    // surface this so the user knows to pick a different tuning at
    // playback time if standard guitar isn't what they wanted.
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
            tempo: tempo_bpm.unwrap_or(DEFAULT_TEMPO),
            title,
            subtitle: Some(target_tuning.name.clone()),
            tuning_names,
            columns,
        },
        warnings,
    })
}

/// Pack a monophonic note sequence into a Standard MIDI File
/// (format-0, single track, 480 PPQ). The symmetric inverse of
/// [`parse`] for the simple-melody subset — bytes written here
/// round-trip back through it.
///
/// Used by tests that need a real `.mid` on disk to feed the CLI
/// without committing a binary blob, and available for any caller
/// that needs to emit SMF from in-memory pitch data. Each `(pitch,
/// denom)` is one note; `denom` is the standard TWANGA denominator
/// (4 = quarter, 8 = eighth, etc).
///
/// Optional `title` is embedded as a `TrackName` meta event and
/// optional `tempo_bpm` as a `Tempo` meta event, so the round-trip
/// preserves the metadata the parser surfaces.
pub fn write_smf_bytes(
    notes: &[(u8, u32)],
    title: Option<&str>,
    tempo_bpm: Option<u32>,
) -> Vec<u8> {
    const PPQ: u16 = 480;
    let mut track: Vec<u8> = Vec::new();

    // Title (TrackName meta event, type 0x03)
    if let Some(t) = title {
        write_vlq(&mut track, 0);
        track.extend_from_slice(&[0xFF, 0x03]);
        write_vlq(&mut track, t.len() as u32);
        track.extend_from_slice(t.as_bytes());
    }
    // Tempo (Set Tempo meta event, type 0x51, 3-byte microseconds-per-quarter)
    if let Some(bpm) = tempo_bpm
        && bpm > 0
    {
        let usec = 60_000_000 / bpm;
        write_vlq(&mut track, 0);
        track.extend_from_slice(&[0xFF, 0x51, 0x03]);
        track.push(((usec >> 16) & 0xFF) as u8);
        track.push(((usec >> 8) & 0xFF) as u8);
        track.push((usec & 0xFF) as u8);
    }

    // Notes: each `(pitch, denom)` → note-on at delta=0 from prev
    // note-off, note-off after (4 * PPQ / denom) ticks.
    for &(pitch, denom) in notes {
        let ticks = (4 * PPQ as u32) / denom.max(1);
        write_vlq(&mut track, 0);
        track.extend_from_slice(&[0x90, pitch, 64]); // note-on chan 0, vel 64
        write_vlq(&mut track, ticks);
        track.extend_from_slice(&[0x80, pitch, 0]); // note-off
    }

    // End-of-track meta event (required by SMF spec)
    write_vlq(&mut track, 0);
    track.extend_from_slice(&[0xFF, 0x2F, 0x00]);

    // Assemble the file header + track chunk
    let mut bytes: Vec<u8> = Vec::new();
    bytes.extend_from_slice(b"MThd");
    bytes.extend_from_slice(&[0, 0, 0, 6]);
    bytes.extend_from_slice(&[0, 0]); // format 0
    bytes.extend_from_slice(&[0, 1]); // 1 track
    bytes.extend_from_slice(&PPQ.to_be_bytes());
    bytes.extend_from_slice(b"MTrk");
    bytes.extend_from_slice(&(track.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&track);
    bytes
}

/// MIDI variable-length-quantity writer — the SMF spec's encoding
/// for delta times and meta-event lengths.
fn write_vlq(out: &mut Vec<u8>, mut value: u32) {
    let mut bytes = [0u8; 5];
    let mut len = 0;
    loop {
        bytes[len] = (value & 0x7F) as u8;
        len += 1;
        value >>= 7;
        if value == 0 {
            break;
        }
    }
    for i in (0..len).rev() {
        let last = i == 0;
        out.push(bytes[i] | if last { 0 } else { 0x80 });
    }
}

/// True if any event in the track is a note-on with positive
/// velocity. Used to skip empty / metadata-only tracks (track 0 in
/// format-1 files is typically conductor-only).
fn track_has_note(track: &midly::Track<'_>) -> bool {
    track.iter().any(|ev| {
        matches!(
            ev.kind,
            TrackEventKind::Midi {
                message: MidiMessage::NoteOn { vel, .. },
                ..
            } if vel.as_int() > 0
        )
    })
}

/// First `TrackName` meta event in the track, as a String. Empty
/// string if the track has no name.
fn track_name(track: &midly::Track<'_>) -> String {
    track
        .iter()
        .find_map(|ev| match ev.kind {
            TrackEventKind::Meta(MetaMessage::TrackName(name)) => {
                Some(std::str::from_utf8(name).unwrap_or("").trim().to_string())
            }
            _ => None,
        })
        .unwrap_or_default()
}

/// Convert a tick delta between two consecutive note-onsets into a
/// power-of-2 denominator. PPQ = ticks per quarter, so quarter-note
/// onset spacing = PPQ ticks → denom 4; eighth = PPQ/2 → denom 8;
/// half = 2*PPQ → denom 2.
///
/// Returns `(denom, was_rounded)`. `was_rounded` true when the
/// source duration wasn't a clean power-of-2 multiple of the quarter
/// (dotted, triplet, swung).
fn tick_delta_to_denom(ticks: u64, ppq: u32) -> (u32, bool) {
    if ticks == 0 {
        // Defensive — caller shouldn't pass 0, but if it happens we
        // pick a sensible default rather than divide-by-zero.
        return (DEFAULT_TAIL_DENOM, true);
    }
    let raw = (4.0 * ppq as f64) / ticks as f64;
    snap_to_power_of_two(raw)
}

/// Place a set of simultaneous MIDI pitches on the target tuning and
/// push the resulting column. Surfaces `IrregularDuration` if the
/// caller flagged rounding, and `UnreachableNote` for any pitch the
/// tuning can't place within `MAX_FRET`.
fn push_column(
    columns: &mut Vec<TabColumn>,
    warnings: &mut Vec<ParseWarning>,
    denom: u32,
    rounded: bool,
    pitches: &[u8],
    tuning: &Tuning,
) {
    let column_index = columns.len();
    if rounded {
        warnings.push(ParseWarning::IrregularDuration {
            column_index,
            raw_duration: format!("denom~{denom}"),
        });
    }
    let mut hits: Vec<(u8, u8)> = Vec::new();
    for &pitch in pitches {
        match crate::place_pitch(tuning, pitch, MAX_FRET) {
            Some(hit) => hits.push(hit),
            None => {
                warnings.push(ParseWarning::UnreachableNote {
                    column_index,
                    note: MidiNote(pitch).name(),
                });
            }
        }
    }
    columns.push(TabColumn {
        duration_denom: denom,
        hits,
        articulation: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal SMF format-0 byte stream from raw `(delta,
    /// key, velocity)` events. The unit-test counterpart of
    /// [`write_smf_bytes`] — kept inline because we want explicit
    /// control over individual deltas for chord-tick and timing
    /// tests, whereas `write_smf_bytes` always emits one
    /// `note-on → note-off` pair per note.
    fn minimal_smf(events: &[(u32, u8, u8)]) -> Vec<u8> {
        let mut track: Vec<u8> = Vec::new();
        for &(dt, key, vel) in events {
            super::write_vlq(&mut track, dt);
            track.push(if vel > 0 { 0x90 } else { 0x80 });
            track.push(key);
            track.push(vel.max(1));
        }
        super::write_vlq(&mut track, 0);
        track.extend_from_slice(&[0xFF, 0x2F, 0x00]);

        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&[0, 0, 0, 6]);
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&[0, 1]);
        bytes.extend_from_slice(&[0x01, 0xE0]); // 480 PPQ
        bytes.extend_from_slice(b"MTrk");
        bytes.extend_from_slice(&(track.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&track);
        bytes
    }

    /// Build a format-1 SMF with multiple note-bearing tracks. Each
    /// track is given by `(name, [(pitch, denom)])`. Used to exercise
    /// the multi-track / `SkippedTrack` code path the format-0
    /// `minimal_smf` helper can't reach (format-0 is single-track by
    /// definition).
    fn multitrack_smf(tracks: &[(&str, &[(u8, u32)])]) -> Vec<u8> {
        const PPQ: u16 = 480;
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&[0, 0, 0, 6]);
        bytes.extend_from_slice(&[0, 1]); // format 1
        bytes.extend_from_slice(&(tracks.len() as u16).to_be_bytes());
        bytes.extend_from_slice(&PPQ.to_be_bytes());

        for &(name, notes) in tracks {
            let mut track: Vec<u8> = Vec::new();
            if !name.is_empty() {
                super::write_vlq(&mut track, 0);
                track.extend_from_slice(&[0xFF, 0x03]);
                super::write_vlq(&mut track, name.len() as u32);
                track.extend_from_slice(name.as_bytes());
            }
            for &(pitch, denom) in notes {
                let ticks = (4 * PPQ as u32) / denom.max(1);
                super::write_vlq(&mut track, 0);
                track.extend_from_slice(&[0x90, pitch, 64]);
                super::write_vlq(&mut track, ticks);
                track.extend_from_slice(&[0x80, pitch, 0]);
            }
            super::write_vlq(&mut track, 0);
            track.extend_from_slice(&[0xFF, 0x2F, 0x00]);
            bytes.extend_from_slice(b"MTrk");
            bytes.extend_from_slice(&(track.len() as u32).to_be_bytes());
            bytes.extend_from_slice(&track);
        }
        bytes
    }

    #[test]
    fn multitrack_first_note_bearing_wins_and_others_warn() {
        // Format-1 SMF: track 0 is conductor (metadata only — title +
        // tempo via track-name on the first note-bearing track since
        // our test helper attaches names per track); tracks 1, 2, 3
        // each have notes. Parser must use track 1 and emit two
        // SkippedTrack warnings for tracks 2 and 3.
        let bytes = multitrack_smf(&[
            ("Conductor", &[]),
            ("Lead Guitar", &[(60, 4), (62, 4), (64, 4)]),
            ("Rhythm Guitar", &[(48, 4), (50, 4)]),
            ("Bass", &[(36, 4)]),
        ]);
        let out = parse(&bytes).expect("parse");
        // Track 1's three quarters → 3 columns. Tracks 2 and 3
        // contributed nothing to the column list.
        assert_eq!(out.tab.columns.len(), 3);

        // Two SkippedTrack warnings — one each for tracks 2 and 3 —
        // and the warning carries the track name so the importer UI
        // can show "Rhythm Guitar skipped" rather than "track 2".
        let skipped: Vec<(usize, String)> = out
            .warnings
            .iter()
            .filter_map(|w| match w {
                ParseWarning::SkippedTrack { index, name } => Some((*index, name.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            skipped.len(),
            2,
            "expected 2 SkippedTrack warnings, got {skipped:?}"
        );
        assert_eq!(skipped[0].0, 2);
        assert_eq!(skipped[0].1, "Rhythm Guitar");
        assert_eq!(skipped[1].0, 3);
        assert_eq!(skipped[1].1, "Bass");
    }

    #[test]
    fn multitrack_skipped_track_falls_back_to_index_when_unnamed() {
        // A note-bearing track without a TrackName meta event should
        // surface as "track N" in the warning, not an empty string.
        let bytes = multitrack_smf(&[("Conductor", &[]), ("Lead", &[(60, 4)]), ("", &[(50, 4)])]);
        let out = parse(&bytes).expect("parse");
        let skipped: Vec<String> = out
            .warnings
            .iter()
            .filter_map(|w| match w {
                ParseWarning::SkippedTrack { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(skipped, vec!["track 2".to_string()]);
    }

    #[test]
    fn write_smf_bytes_round_trips_through_parse() {
        // Build → parse → check the metadata survives. Same shape as
        // the musicxml::write_mxl_bytes / parse_mxl round-trip test.
        let notes: Vec<(u8, u32)> = vec![
            (60, 4), // C4 quarter
            (62, 4), // D4 quarter
            (64, 4), // E4 quarter
        ];
        let bytes = write_smf_bytes(&notes, Some("Round Trip"), Some(120));
        let out = parse(&bytes).expect("parse round-trip");
        assert_eq!(out.tab.title.as_deref(), Some("Round Trip"));
        assert_eq!(out.tab.tempo, 120);
        assert_eq!(out.tab.columns.len(), 3);
    }

    #[test]
    fn parses_single_quarter_note() {
        // C4 (MIDI 60), then a note-off after one quarter (480 ticks).
        let bytes = minimal_smf(&[(0, 60, 64), (480, 60, 0)]);
        let out = parse(&bytes).expect("parse");
        assert_eq!(out.tab.columns.len(), 1);
        assert_eq!(out.tab.columns[0].duration_denom, 4);
        // C4 on standard guitar → string 2 (B3), fret 1 (lowest
        // fret across all valid placements: B3+1, G3+5, D3+10, etc).
        assert_eq!(out.tab.columns[0].hits, vec![(2, 1)]);
        // Tuning was inferred (no MIDI-side declaration).
        assert!(
            out.warnings
                .iter()
                .any(|w| matches!(w, ParseWarning::InferredTuning { .. }))
        );
    }

    #[test]
    fn empty_score_returns_error() {
        // Only an end-of-track event, no note-ons.
        let bytes = minimal_smf(&[]);
        assert!(matches!(parse(&bytes), Err(MidiError::EmptyScore)));
    }

    #[test]
    fn chord_collapses_into_one_column() {
        // Three notes at tick 0 (delta 0), then move forward.
        let bytes = minimal_smf(&[
            (0, 60, 64),  // C4 onset
            (0, 64, 64),  // E4 onset (same tick)
            (0, 67, 64),  // G4 onset (same tick)
            (480, 60, 0), // C4 off
        ]);
        let out = parse(&bytes).expect("parse");
        assert_eq!(
            out.tab.columns.len(),
            1,
            "three simultaneous note-ons should be one column"
        );
        assert_eq!(
            out.tab.columns[0].hits.len(),
            3,
            "column should hold three hits"
        );
    }

    #[test]
    fn back_to_back_quarters_produce_two_columns() {
        // C then D, each held for one quarter (480 ticks).
        let bytes = minimal_smf(&[(0, 60, 64), (480, 60, 0), (0, 62, 64), (480, 62, 0)]);
        let out = parse(&bytes).expect("parse");
        assert_eq!(out.tab.columns.len(), 2);
    }

    #[test]
    fn snap_to_power_of_two_path_handles_eighth_notes() {
        // C onset, D onset 240 ticks later (an eighth at 480 PPQ).
        let bytes = minimal_smf(&[(0, 60, 64), (240, 62, 64), (240, 62, 0)]);
        let out = parse(&bytes).expect("parse");
        // First column is the eighth (between C-onset and D-onset).
        assert_eq!(out.tab.columns[0].duration_denom, 8);
    }
}
