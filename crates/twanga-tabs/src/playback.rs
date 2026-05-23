//! Playback scoring policy + per-column outcome scoring.
//!
//! Ship 2 of [docs/plans/onset-detection.md] (local plan, not in repo):
//! the architectural move from "pause until pitch confirmed"
//! ([`PlaybackPolicy::WaitOnPitch`]) to "run at tempo and score by
//! proximity to expected onsets" ([`PlaybackPolicy::ProximityScore`]).
//! Wait-mode survives as a slow-down practice preset; proximity
//! scoring becomes the default for real practice.
//!
//! Module is **pure** — no IO, no async, no tuner integration.
//! Consumers (CLI play loop, web playback engine) own the scheduling
//! and audio capture; this module just takes a precomputed schedule
//! plus a stream of detected onsets and tells you which columns
//! were hit, late, missed, or wrong-pitch.
//!
//! # Why decouple scheduling from scoring
//!
//! The CLI and web play loops both compute when each column "should"
//! fire from `bpm * column.duration_denom`, but the exact math
//! differs subtly between them (CLI uses a sleep-based tick, web
//! uses `performance.now()` deltas with a paused-time accumulator).
//! Having scoring take precomputed `(expected_ms, expected_hits)`
//! pairs lets both surfaces share this code without dragging in
//! their respective scheduling subsystems. The downside is each
//! surface duplicates the "compute timestamps" step — small price
//! for the testability win.

use twanga_core::Tuning;

use crate::TabColumn;

/// What the playhead does at each note. Selected per-session;
/// scoring + visualisation behaviour follow.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlaybackPolicy {
    /// Pause the playhead at each non-rest column until the user
    /// plays a matching pitch from a fresh onset. The previous
    /// default — slow but bulletproof for note-by-note drill.
    /// Matches today's `--wait` behaviour exactly.
    WaitOnPitch { cents_tolerance: f32 },

    /// Run at tempo; for each expected onset, pair with the nearest
    /// detected onset within `[expected_ms - early_ms,
    /// expected_ms + late_ms]`. Hit / Late / Missed / WrongPitch
    /// per column. The "real practice" mode — you keep up or you
    /// score lower; the song doesn't wait for you.
    ProximityScore {
        early_ms: u32,
        late_ms: u32,
        cents_tolerance: f32,
    },

    /// No verification, no scoring — playhead just scrolls at
    /// tempo. The "I want to read along" mode; useful for getting
    /// familiar with a tab before drilling it.
    FreePlay,
}

impl PlaybackPolicy {
    /// Same cents tolerance the existing wait-mode uses. Wide enough
    /// to forgive amateur intonation; tight enough to catch a clearly
    /// wrong note.
    pub const DEFAULT_CENTS_TOLERANCE: f32 = 50.0;

    /// Tight scoring preset — ±50 ms around the expected onset. Real
    /// instrument with a real audio interface; the user is asking
    /// the app to hold them to a strict tempo.
    pub fn tight() -> Self {
        Self::ProximityScore {
            early_ms: 50,
            late_ms: 50,
            cents_tolerance: Self::DEFAULT_CENTS_TOLERANCE,
        }
    }

    /// Casual scoring preset — ±150 ms around the expected onset.
    /// Mic / web Audio / cheap USB cable; absorbs typical playback
    /// + capture latency without calibration.
    pub fn casual() -> Self {
        Self::ProximityScore {
            early_ms: 150,
            late_ms: 150,
            cents_tolerance: Self::DEFAULT_CENTS_TOLERANCE,
        }
    }

    /// Wait-mode preset — same as the existing `--wait` behaviour
    /// (pause until pitch confirmed, no timing tolerance).
    pub fn wait() -> Self {
        Self::WaitOnPitch {
            cents_tolerance: Self::DEFAULT_CENTS_TOLERANCE,
        }
    }
}

impl Default for PlaybackPolicy {
    /// The right default for "real practice" — proximity scoring at
    /// casual tolerance. Tightens to `tight()` once the latency
    /// calibration wizard is wired in (see BACKLOG.md).
    fn default() -> Self {
        Self::casual()
    }
}

/// One detected note attack from the audio stream. Consumers
/// produce these from their tuner / onset-detector pipelines.
///
/// `timestamp_ms` is measured against the same clock the expected
/// onset schedule was built from — typically "ms since playback
/// start," modulo any pause/resume accumulators the consumer
/// maintains. The scoring function doesn't care about the origin,
/// only that the two timestamp series share a clock.
#[derive(Debug, Clone, Copy)]
pub struct OnsetEvent {
    pub timestamp_ms: u32,
    pub detected_hz: f32,
}

/// The "what should happen, and when" snapshot for one column.
/// Built by the consumer from `(tab, bpm)` and handed to the
/// scoring function alongside the actual onset events.
///
/// Rest columns (`expected_hits.is_empty()`) are silently dropped
/// during scoring — they don't produce a `ColumnOutcome` because
/// there's nothing to evaluate (the user isn't expected to play
/// anything, so "Missed" would be wrong).
#[derive(Debug, Clone)]
pub struct ColumnExpected {
    pub expected_ms: u32,
    pub expected_hits: Vec<(u8, u8)>,
}

/// Per-column scoring outcome. Aggregated for the end-of-song
/// summary; also surfaced per-column in real time for the visual
/// feedback layer (cell-flash on hit, cell-red on miss).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColumnOutcome {
    /// Onset arrived within the policy's tolerance window with a
    /// matching pitch. `offset_ms` is `detected - expected`
    /// (signed; negative = early, positive = late, 0 = on the beat).
    Hit { offset_ms: i32 },

    /// Onset arrived AFTER the late cutoff but is still the closest
    /// onset for this column. Counted as a partial credit in the
    /// summary — the user played it, just out of time. `offset_ms`
    /// is the unsigned ms past the late cutoff.
    Late { offset_ms: u32 },

    /// No onset within `expected_ms ± (early_ms + late_ms)` —
    /// the user didn't play anything for this column.
    Missed,

    /// An onset arrived within timing tolerance, but its pitch was
    /// outside the cents tolerance for any of the expected hits.
    /// The user played SOMETHING on time, just the wrong note.
    WrongPitch { detected_hz: f32 },
}

impl ColumnOutcome {
    /// True for hits + late hits (the "you played it" cases).
    /// Used by the summary screen for "N/M notes played" headlines.
    pub fn was_played(&self) -> bool {
        matches!(self, Self::Hit { .. } | Self::Late { .. })
    }
}

/// Aggregate counts across a whole playback session. Built from a
/// `Vec<ColumnOutcome>` for the end-of-song summary surface.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PlaybackSummary {
    pub hit: usize,
    pub late: usize,
    pub missed: usize,
    pub wrong_pitch: usize,
}

impl PlaybackSummary {
    pub fn total(&self) -> usize {
        self.hit + self.late + self.missed + self.wrong_pitch
    }

    /// Build from a slice of outcomes — convenience for the
    /// end-of-song / per-section views.
    pub fn from_outcomes(outcomes: &[ColumnOutcome]) -> Self {
        let mut s = Self::default();
        for o in outcomes {
            match o {
                ColumnOutcome::Hit { .. } => s.hit += 1,
                ColumnOutcome::Late { .. } => s.late += 1,
                ColumnOutcome::Missed => s.missed += 1,
                ColumnOutcome::WrongPitch { .. } => s.wrong_pitch += 1,
            }
        }
        s
    }
}

/// Compute the expected onset timestamps for each column in a tab.
/// `ms_per_quarter_note = 60_000 / bpm`; column duration in ms is
/// `4.0 / column.duration_denom * ms_per_quarter`.
///
/// Result includes ALL columns (rests too) so the index aligns with
/// `tab.columns`; the scoring function filters out rests separately.
pub fn build_schedule(columns: &[TabColumn], bpm: u32) -> Vec<ColumnExpected> {
    let ms_per_quarter = 60_000.0 / bpm.max(1) as f32;
    let mut out = Vec::with_capacity(columns.len());
    let mut now_ms = 0.0_f32;
    for col in columns {
        out.push(ColumnExpected {
            expected_ms: now_ms.round() as u32,
            expected_hits: col.hits.clone(),
        });
        let denom = col.duration_denom.max(1) as f32;
        now_ms += (4.0 / denom) * ms_per_quarter;
    }
    out
}

/// Score detected onsets against an expected schedule under a given
/// policy. Returns one `ColumnOutcome` per NON-REST column in the
/// schedule (rest columns are silently dropped).
///
/// Pairing algorithm:
/// 1. Walk the schedule in order.
/// 2. For each non-rest column, find the closest unused onset whose
///    `timestamp_ms` falls inside `[expected_ms - early_ms,
///    expected_ms + late_ms_max]`. `late_ms_max` is set generously
///    (`late_ms * 4`) so a very-late onset still pairs with its
///    intended column rather than spilling onto the next.
/// 3. If found: check the pitch. Within tolerance → `Hit` (or
///    `Late` if past `late_ms`); outside → `WrongPitch`. Mark the
///    onset as used either way.
/// 4. If not found: `Missed`.
///
/// `WaitOnPitch` and `FreePlay` policies don't make sense for
/// batch scoring (they're stream-driven by the playhead loop);
/// passing them returns an empty `Vec`. Use `ProximityScore` here.
pub fn score(
    schedule: &[ColumnExpected],
    onsets: &[OnsetEvent],
    policy: PlaybackPolicy,
    tuning: &Tuning,
) -> Vec<ColumnOutcome> {
    let PlaybackPolicy::ProximityScore {
        early_ms,
        late_ms,
        cents_tolerance,
    } = policy
    else {
        return Vec::new();
    };
    // Generously-extended late window so a very-late attack still
    // pairs with the column the user was AIMING at (rather than the
    // next column, which would then double-count if scored too).
    let late_ms_max = late_ms.saturating_mul(4);

    let mut used = vec![false; onsets.len()];
    let mut outcomes = Vec::with_capacity(schedule.len());

    for column in schedule {
        if column.expected_hits.is_empty() {
            continue; // rest — not scored
        }
        let lo = column.expected_ms.saturating_sub(early_ms);
        let hi = column.expected_ms.saturating_add(late_ms_max);

        // Closest unused onset within [lo, hi].
        let mut best: Option<(usize, u32)> = None;
        for (i, onset) in onsets.iter().enumerate() {
            if used[i] {
                continue;
            }
            let ts = onset.timestamp_ms;
            if ts < lo || ts > hi {
                continue;
            }
            let dist = ts.abs_diff(column.expected_ms);
            if best.is_none_or(|(_, d)| dist < d) {
                best = Some((i, dist));
            }
        }

        let outcome = match best {
            None => ColumnOutcome::Missed,
            Some((i, _)) => {
                used[i] = true;
                let onset = &onsets[i];
                if pitch_matches_any_hit(
                    onset.detected_hz,
                    &column.expected_hits,
                    tuning,
                    cents_tolerance,
                ) {
                    let offset = onset.timestamp_ms as i32 - column.expected_ms as i32;
                    if offset.unsigned_abs() <= late_ms {
                        ColumnOutcome::Hit { offset_ms: offset }
                    } else {
                        ColumnOutcome::Late {
                            offset_ms: offset.unsigned_abs(),
                        }
                    }
                } else {
                    ColumnOutcome::WrongPitch {
                        detected_hz: onset.detected_hz,
                    }
                }
            }
        };
        outcomes.push(outcome);
    }

    outcomes
}

/// True iff `detected_hz` is within `cents_tolerance` of any of
/// the column's expected `(string, fret)` placements on `tuning`.
/// Mirrors the CLI's `matches_any_expected` with tolerance as a
/// parameter (so scoring respects the policy's `cents_tolerance`
/// rather than a hardcoded constant).
fn pitch_matches_any_hit(
    detected_hz: f32,
    expected_hits: &[(u8, u8)],
    tuning: &Tuning,
    cents_tolerance: f32,
) -> bool {
    for (string_num, fret) in expected_hits {
        let string_idx = (*string_num as usize).saturating_sub(1);
        let Some(s) = tuning.strings.get(string_idx) else {
            continue;
        };
        // 5-string-banjo drone: fret_offset > 0 means tab "fret 7" =
        // open + (7 - 5) semitones above the drone's open pitch.
        let semitones = fret.saturating_sub(s.fret_offset);
        let open_hz = s.open.to_frequency().hz();
        let target_hz = open_hz * 2_f32.powf(semitones as f32 / 12.0);
        let cents = 1200.0 * (detected_hz / target_hz).log2();
        if cents.abs() < cents_tolerance {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use twanga_core::MidiNote;

    fn standard_guitar() -> Tuning {
        Tuning::standard_guitar()
    }

    /// Tab schedule helper: each entry is `(expected_ms, hits)`.
    /// Rests are `(ms, vec![])`.
    fn schedule_from(items: &[(u32, Vec<(u8, u8)>)]) -> Vec<ColumnExpected> {
        items
            .iter()
            .map(|(ms, hits)| ColumnExpected {
                expected_ms: *ms,
                expected_hits: hits.clone(),
            })
            .collect()
    }

    fn onset(ts_ms: u32, hz: f32) -> OnsetEvent {
        OnsetEvent {
            timestamp_ms: ts_ms,
            detected_hz: hz,
        }
    }

    fn e4_hz() -> f32 {
        MidiNote(64).to_frequency().hz()
    }

    fn a4_hz() -> f32 {
        MidiNote(69).to_frequency().hz()
    }

    // ───────────────────── PlaybackPolicy presets ─────────────────────

    #[test]
    fn presets_carry_the_documented_tolerances() {
        match PlaybackPolicy::tight() {
            PlaybackPolicy::ProximityScore {
                early_ms,
                late_ms,
                cents_tolerance,
            } => {
                assert_eq!((early_ms, late_ms), (50, 50));
                assert_eq!(cents_tolerance, 50.0);
            }
            _ => panic!("tight() must be ProximityScore"),
        }
        match PlaybackPolicy::casual() {
            PlaybackPolicy::ProximityScore {
                early_ms, late_ms, ..
            } => assert_eq!((early_ms, late_ms), (150, 150)),
            _ => panic!("casual() must be ProximityScore"),
        }
        assert!(matches!(
            PlaybackPolicy::wait(),
            PlaybackPolicy::WaitOnPitch { .. }
        ));
        assert!(matches!(
            PlaybackPolicy::default(),
            PlaybackPolicy::ProximityScore { .. }
        ));
    }

    // ───────────────────── build_schedule ─────────────────────

    #[test]
    fn schedule_quarter_notes_at_60bpm_are_one_second_apart() {
        // BPM 60 => 1 quarter / second. Four quarters = 0/1000/2000/3000 ms.
        let cols: Vec<TabColumn> = (0..4)
            .map(|_| TabColumn {
                duration_denom: 4,
                hits: vec![(1, 0)],
                articulation: None,
            })
            .collect();
        let s = build_schedule(&cols, 60);
        assert_eq!(s.len(), 4);
        let timestamps: Vec<u32> = s.iter().map(|c| c.expected_ms).collect();
        assert_eq!(timestamps, vec![0, 1000, 2000, 3000]);
    }

    #[test]
    fn schedule_mixed_durations_advance_proportionally() {
        // Quarter, eighth, eighth, quarter at 120 BPM (quarter = 500ms).
        // Onsets: 0, 500, 750, 1000.
        let cols = vec![
            TabColumn {
                duration_denom: 4,
                hits: vec![(1, 0)],
                articulation: None,
            },
            TabColumn {
                duration_denom: 8,
                hits: vec![(1, 0)],
                articulation: None,
            },
            TabColumn {
                duration_denom: 8,
                hits: vec![(1, 0)],
                articulation: None,
            },
            TabColumn {
                duration_denom: 4,
                hits: vec![(1, 0)],
                articulation: None,
            },
        ];
        let s = build_schedule(&cols, 120);
        assert_eq!(
            s.iter().map(|c| c.expected_ms).collect::<Vec<_>>(),
            vec![0, 500, 750, 1000]
        );
    }

    // ───────────────────── score: hit / late / missed / wrong-pitch ─────────────────────

    #[test]
    fn score_returns_empty_for_non_proximity_policies() {
        let s = schedule_from(&[(0, vec![(1, 0)])]);
        let onsets = vec![onset(0, e4_hz())];
        assert!(score(&s, &onsets, PlaybackPolicy::wait(), &standard_guitar()).is_empty());
        assert!(score(&s, &onsets, PlaybackPolicy::FreePlay, &standard_guitar()).is_empty());
    }

    #[test]
    fn score_hit_on_time_with_matching_pitch() {
        let s = schedule_from(&[(1000, vec![(1, 0)])]);
        let onsets = vec![onset(1000, e4_hz())];
        let outcomes = score(&s, &onsets, PlaybackPolicy::tight(), &standard_guitar());
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0], ColumnOutcome::Hit { offset_ms: 0 });
    }

    #[test]
    fn score_hit_carries_signed_offset_when_slightly_off() {
        let s = schedule_from(&[(1000, vec![(1, 0)])]);
        // 20ms late, still within tight ±50.
        let outcomes = score(
            &s,
            &[onset(1020, e4_hz())],
            PlaybackPolicy::tight(),
            &standard_guitar(),
        );
        assert_eq!(outcomes[0], ColumnOutcome::Hit { offset_ms: 20 });
        // 30ms early, still within tight ±50.
        let outcomes = score(
            &s,
            &[onset(970, e4_hz())],
            PlaybackPolicy::tight(),
            &standard_guitar(),
        );
        assert_eq!(outcomes[0], ColumnOutcome::Hit { offset_ms: -30 });
    }

    #[test]
    fn score_late_when_past_late_cutoff_but_within_extended_window() {
        // Tight policy: late_ms = 50, extended window = 4 * 50 = 200.
        // 150ms after expected: past 50ms cutoff → Late.
        let s = schedule_from(&[(1000, vec![(1, 0)])]);
        let outcomes = score(
            &s,
            &[onset(1150, e4_hz())],
            PlaybackPolicy::tight(),
            &standard_guitar(),
        );
        assert_eq!(outcomes[0], ColumnOutcome::Late { offset_ms: 150 });
    }

    #[test]
    fn score_missed_when_no_onset_anywhere_near() {
        let s = schedule_from(&[(1000, vec![(1, 0)])]);
        // Onset 5 seconds away — well outside extended window.
        let outcomes = score(
            &s,
            &[onset(6000, e4_hz())],
            PlaybackPolicy::tight(),
            &standard_guitar(),
        );
        assert_eq!(outcomes[0], ColumnOutcome::Missed);
    }

    #[test]
    fn score_missed_when_onsets_list_empty() {
        let s = schedule_from(&[(1000, vec![(1, 0)])]);
        let outcomes = score(&s, &[], PlaybackPolicy::tight(), &standard_guitar());
        assert_eq!(outcomes[0], ColumnOutcome::Missed);
    }

    #[test]
    fn score_wrong_pitch_when_onset_on_time_but_wrong_note() {
        let s = schedule_from(&[(1000, vec![(1, 0)])]);
        // E4 expected; user played A4 (well outside 50-cent tolerance).
        let outcomes = score(
            &s,
            &[onset(1000, a4_hz())],
            PlaybackPolicy::tight(),
            &standard_guitar(),
        );
        assert!(matches!(
            outcomes[0],
            ColumnOutcome::WrongPitch { detected_hz } if (detected_hz - a4_hz()).abs() < 0.01
        ));
    }

    // ───────────────────── score: rests / multi-column / pairing ─────────────────────

    #[test]
    fn score_skips_rest_columns_silently() {
        // Quarter, rest, quarter at 60 BPM. Only 2 outcomes — the rest
        // contributes no scoring event because there's nothing to play.
        let s = schedule_from(&[(0, vec![(1, 0)]), (1000, vec![]), (2000, vec![(1, 0)])]);
        let outcomes = score(
            &s,
            &[onset(0, e4_hz()), onset(2000, e4_hz())],
            PlaybackPolicy::tight(),
            &standard_guitar(),
        );
        assert_eq!(outcomes.len(), 2);
        assert!(matches!(outcomes[0], ColumnOutcome::Hit { .. }));
        assert!(matches!(outcomes[1], ColumnOutcome::Hit { .. }));
    }

    #[test]
    fn score_each_onset_pairs_with_at_most_one_column() {
        // Two columns 1000ms apart. ONE onset between them, closer
        // to the first — paired with column 1, column 2 = Missed.
        let s = schedule_from(&[(1000, vec![(1, 0)]), (2000, vec![(1, 0)])]);
        let outcomes = score(
            &s,
            &[onset(1100, e4_hz())], // 100ms after col 1, 900ms before col 2
            PlaybackPolicy::casual(),
            &standard_guitar(),
        );
        assert!(matches!(outcomes[0], ColumnOutcome::Hit { .. }));
        assert_eq!(outcomes[1], ColumnOutcome::Missed);
    }

    #[test]
    fn score_picks_closest_onset_when_multiple_in_window() {
        // Single column at t=1000; three onsets at 950, 1010, 1080.
        // Closest = 1010 (10ms late). Others ignored.
        let s = schedule_from(&[(1000, vec![(1, 0)])]);
        let outcomes = score(
            &s,
            &[
                onset(950, e4_hz()),
                onset(1010, e4_hz()),
                onset(1080, e4_hz()),
            ],
            PlaybackPolicy::tight(),
            &standard_guitar(),
        );
        assert_eq!(outcomes[0], ColumnOutcome::Hit { offset_ms: 10 });
    }

    #[test]
    fn score_pairs_in_column_order_even_when_onsets_overlap_windows() {
        // Two columns 100ms apart at casual tolerance (±150ms). Both
        // could theoretically claim either onset. Verify the algorithm
        // is stable: column 0 takes its closer onset, column 1 takes
        // what's left.
        let s = schedule_from(&[(0, vec![(1, 0)]), (100, vec![(1, 0)])]);
        let outcomes = score(
            &s,
            &[onset(10, e4_hz()), onset(90, e4_hz())],
            PlaybackPolicy::casual(),
            &standard_guitar(),
        );
        // Column 0 (expected 0): closer onset is t=10 (dist 10).
        // Column 1 (expected 100): closer onset is t=90 (dist 10).
        assert_eq!(outcomes[0], ColumnOutcome::Hit { offset_ms: 10 });
        assert_eq!(outcomes[1], ColumnOutcome::Hit { offset_ms: -10 });
    }

    #[test]
    fn score_chord_column_matches_when_any_pitch_aligns() {
        // Two-string chord: E (1, 0) + B (2, 0). Onset at 1000ms
        // detected as B3 only — counts as a hit because B matches
        // one of the expected pitches.
        let s = schedule_from(&[(1000, vec![(1, 0), (2, 0)])]);
        let b3_hz = MidiNote(59).to_frequency().hz();
        let outcomes = score(
            &s,
            &[onset(1000, b3_hz)],
            PlaybackPolicy::tight(),
            &standard_guitar(),
        );
        assert!(matches!(outcomes[0], ColumnOutcome::Hit { .. }));
    }

    // ───────────────────── PlaybackSummary aggregation ─────────────────────

    #[test]
    fn summary_aggregates_outcomes_by_variant() {
        let outcomes = vec![
            ColumnOutcome::Hit { offset_ms: 0 },
            ColumnOutcome::Hit { offset_ms: 25 },
            ColumnOutcome::Late { offset_ms: 120 },
            ColumnOutcome::Missed,
            ColumnOutcome::Missed,
            ColumnOutcome::WrongPitch {
                detected_hz: a4_hz(),
            },
        ];
        let s = PlaybackSummary::from_outcomes(&outcomes);
        assert_eq!(s.hit, 2);
        assert_eq!(s.late, 1);
        assert_eq!(s.missed, 2);
        assert_eq!(s.wrong_pitch, 1);
        assert_eq!(s.total(), 6);
    }

    #[test]
    fn was_played_includes_hits_and_late_but_excludes_misses() {
        assert!(ColumnOutcome::Hit { offset_ms: 0 }.was_played());
        assert!(ColumnOutcome::Late { offset_ms: 100 }.was_played());
        assert!(!ColumnOutcome::Missed.was_played());
        assert!(
            !ColumnOutcome::WrongPitch {
                detected_hz: a4_hz()
            }
            .was_played()
        );
    }
}
