//! Domain types and a small amount of shared static content. No IO, no async, no algorithms.

use std::fmt;

/// MOTD splash strings — one per line. Shared between the CLIs (banner on
/// startup) and the future Tauri main menu (random splash per visit, à la
/// Minecraft). One source of truth so the two surfaces can't drift.
pub const SPLASHES: &str = include_str!("splashes.txt");

/// Iterator over non-empty splash lines.
pub fn splashes() -> impl Iterator<Item = &'static str> + Clone {
    SPLASHES.lines().filter(|l| !l.trim().is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Frequency(pub f32);

impl Frequency {
    pub const A4: Frequency = Frequency(440.0);

    pub fn hz(self) -> f32 {
        self.0
    }

    pub fn to_midi(self) -> f32 {
        69.0 + 12.0 * (self.0 / Self::A4.hz()).log2()
    }
}

impl fmt::Display for Frequency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.2} Hz", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MidiNote(pub u8);

impl MidiNote {
    pub fn to_frequency(self) -> Frequency {
        Frequency(Frequency::A4.hz() * 2f32.powf((self.0 as f32 - 69.0) / 12.0))
    }

    pub fn name(self) -> String {
        const NAMES: [&str; 12] = [
            "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
        ];
        let octave = (self.0 as i32 / 12) - 1;
        let n = NAMES[(self.0 % 12) as usize];
        format!("{n}{octave}")
    }

    /// Nearest MIDI note (12-TET) to the given frequency, plus the signed cents
    /// difference (positive = `freq` is sharp of the returned note).
    pub fn nearest_to(freq: Frequency) -> (MidiNote, f32) {
        let midi_float = freq.to_midi();
        let clamped = midi_float.round().clamp(0.0, 127.0) as u8;
        let note = MidiNote(clamped);
        let target = note.to_frequency();
        let cents = 1200.0 * (freq.hz() / target.hz()).log2();
        (note, cents)
    }

    /// Parse a note name like `"A4"` or `"C#3"` into the matching `MidiNote`.
    /// Inverse of [`Self::name`]. Returns `None` for empty input, flats
    /// (`Cb4` etc. — only sharps), pitch letters outside `A..=G`, or octave
    /// values that put the result outside `0..=127`.
    pub fn from_name(name: &str) -> Option<MidiNote> {
        if name.is_empty() {
            return None;
        }
        let (letter, octave_str) = if name.len() >= 2 && &name[1..2] == "#" {
            (&name[..2], &name[2..])
        } else {
            (&name[..1], &name[1..])
        };
        let pitch_class: i32 = match letter {
            "C" => 0,
            "C#" => 1,
            "D" => 2,
            "D#" => 3,
            "E" => 4,
            "F" => 5,
            "F#" => 6,
            "G" => 7,
            "G#" => 8,
            "A" => 9,
            "A#" => 10,
            "B" => 11,
            _ => return None,
        };
        let octave: i32 = octave_str.parse().ok()?;
        let midi = (octave + 1) * 12 + pitch_class;
        if (0..=127).contains(&midi) {
            Some(MidiNote(midi as u8))
        } else {
            None
        }
    }
}

#[derive(Debug, Clone)]
pub struct TunedString {
    pub name: String,
    pub open: MidiNote,
}

/// Strings are listed in string-number order (string 1 first), NOT pitch order.
/// This matters for the banjo 5th-string drone and reentrant uke high-G.
#[derive(Debug, Clone)]
pub struct Tuning {
    pub name: String,
    pub strings: Vec<TunedString>,
}

impl Tuning {
    pub fn standard_guitar() -> Self {
        Self {
            name: "Standard Guitar".into(),
            strings: vec![
                TunedString { name: "E4".into(), open: MidiNote(64) },
                TunedString { name: "B3".into(), open: MidiNote(59) },
                TunedString { name: "G3".into(), open: MidiNote(55) },
                TunedString { name: "D3".into(), open: MidiNote(50) },
                TunedString { name: "A2".into(), open: MidiNote(45) },
                TunedString { name: "E2".into(), open: MidiNote(40) },
            ],
        }
    }

    pub fn standard_banjo() -> Self {
        Self {
            name: "Standard 5-String Banjo (Open G)".into(),
            strings: vec![
                TunedString { name: "D4".into(), open: MidiNote(62) },
                TunedString { name: "B3".into(), open: MidiNote(59) },
                TunedString { name: "G3".into(), open: MidiNote(55) },
                TunedString { name: "D3".into(), open: MidiNote(50) },
                TunedString { name: "g4 (drone)".into(), open: MidiNote(67) },
            ],
        }
    }

    pub fn standard_ukulele() -> Self {
        Self {
            name: "Standard Ukulele (Reentrant GCEA)".into(),
            strings: vec![
                TunedString { name: "A4".into(), open: MidiNote(69) },
                TunedString { name: "E4".into(), open: MidiNote(64) },
                TunedString { name: "C4".into(), open: MidiNote(60) },
                TunedString { name: "g4 (reentrant)".into(), open: MidiNote(67) },
            ],
        }
    }

    /// Canonical preset slugs. The CLI and any future GUI enumerate options
    /// from this list — one source of truth, no string duplication elsewhere.
    pub const PRESETS: &'static [&'static str] = &[
        "standard-guitar",
        "standard-banjo",
        "standard-ukulele",
    ];

    /// Build a tuning from a preset slug. Returns `None` if the slug doesn't
    /// match a known preset (see [`Self::PRESETS`]).
    pub fn from_preset(name: &str) -> Option<Tuning> {
        match name {
            "standard-guitar" => Some(Self::standard_guitar()),
            "standard-banjo" => Some(Self::standard_banjo()),
            "standard-ukulele" => Some(Self::standard_ukulele()),
            _ => None,
        }
    }

    /// Returns the open string nearest in pitch to `freq`, with the signed cents
    /// difference (positive = detected pitch is sharp of the target).
    pub fn nearest_string(&self, freq: Frequency) -> Option<(&TunedString, f32)> {
        self.strings
            .iter()
            .map(|s| {
                let target_hz = s.open.to_frequency().hz();
                let cents = 1200.0 * (freq.hz() / target_hz).log2();
                (s, cents)
            })
            .min_by(|a, b| {
                a.1.abs()
                    .partial_cmp(&b.1.abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    /// Find the string + fret position best matching `freq`.
    ///
    /// Considers fret positions `0..=max_fret` on every string. Returns the
    /// match with the smallest absolute cents-from-exact-fret, breaking ties
    /// by preferring the lower fret (more natural fingering). Returns `None`
    /// if no string has a valid fret position for this frequency — e.g. `freq`
    /// sits below every open string, or sits past `max_fret` on every string.
    ///
    /// Detection slop tolerance: pitches up to 50 cents below a string's open
    /// pitch are still accepted as fret 0 (the open string).
    pub fn match_to_fret(&self, freq: Frequency, max_fret: u8) -> Option<FretMatch> {
        let mut best: Option<FretMatch> = None;
        for (idx, s) in self.strings.iter().enumerate() {
            let open_hz = s.open.to_frequency().hz();
            let cents_above_open = 1200.0 * (freq.hz() / open_hz).log2();
            let fret_float = cents_above_open / 100.0;
            // Accept slightly-flat-of-open as fret 0 (within 50 cents).
            if fret_float < -0.5 {
                continue;
            }
            let fret_i = fret_float.round() as i32;
            if fret_i < 0 || fret_i > max_fret as i32 {
                continue;
            }
            let fret = fret_i as u8;
            let exact_cents = fret as f32 * 100.0;
            let cents_off = cents_above_open - exact_cents;

            let candidate = FretMatch {
                string_idx: idx,
                fret,
                cents_off,
            };

            let better = match &best {
                None => true,
                Some(b) => {
                    fret < b.fret
                        || (fret == b.fret && cents_off.abs() < b.cents_off.abs())
                }
            };
            if better {
                best = Some(candidate);
            }
        }
        best
    }
}

/// Result of a fret-aware pitch match. See [`Tuning::match_to_fret`].
#[derive(Debug, Clone, Copy)]
pub struct FretMatch {
    /// Index into `Tuning::strings`.
    pub string_idx: usize,
    /// Fret number (0 = open string).
    pub fret: u8,
    /// Signed cents from the exact fret position.
    pub cents_off: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a4_round_trip() {
        assert!((Frequency::A4.to_midi() - 69.0).abs() < 1e-4);
        assert!((MidiNote(69).to_frequency().hz() - 440.0).abs() < 1e-4);
    }

    #[test]
    fn midi_note_names() {
        assert_eq!(MidiNote(60).name(), "C4");
        assert_eq!(MidiNote(69).name(), "A4");
        assert_eq!(MidiNote(21).name(), "A0");
    }

    #[test]
    fn banjo_drone_is_higher_pitch_than_string_4() {
        let b = Tuning::standard_banjo();
        assert!(b.strings[4].open > b.strings[3].open);
    }

    #[test]
    fn ukulele_g_is_reentrant() {
        let u = Tuning::standard_ukulele();
        let c4 = u.strings[2].open;
        let e4 = u.strings[1].open;
        let g4 = u.strings[3].open;
        assert!(g4 > c4 && g4 > e4);
    }

    #[test]
    fn nearest_string_picks_closest_open_pitch_with_correct_cents() {
        let banjo = Tuning::standard_banjo();
        // D3 sharpened by +5 cents.
        let d3 = MidiNote(50).to_frequency();
        let sharp = Frequency(d3.hz() * 2_f32.powf(5.0 / 1200.0));
        let (string, cents) = banjo.nearest_string(sharp).expect("non-empty tuning");
        assert_eq!(string.name, "D3");
        assert!((cents - 5.0).abs() < 0.01, "expected +5 cents, got {cents}");
    }

    #[test]
    fn nearest_string_resolves_reentrant_uke_g_correctly() {
        // 392 Hz is g4 — should match the reentrant 4th string, not E4 or A4.
        let uke = Tuning::standard_ukulele();
        let (string, cents) = uke.nearest_string(Frequency(392.0)).expect("non-empty");
        assert_eq!(string.name, "g4 (reentrant)");
        assert!(cents.abs() < 5.0);
    }

    #[test]
    fn nearest_string_returns_none_on_empty_tuning() {
        let empty = Tuning { name: "".into(), strings: vec![] };
        assert!(empty.nearest_string(Frequency::A4).is_none());
    }

    #[test]
    fn midi_nearest_to_a4_is_a4_at_zero_cents() {
        let (note, cents) = MidiNote::nearest_to(Frequency::A4);
        assert_eq!(note, MidiNote(69));
        assert!(cents.abs() < 1e-3);
    }

    #[test]
    fn midi_nearest_to_sharpened_a4_reports_offset() {
        let sharp = Frequency(Frequency::A4.hz() * 2_f32.powf(20.0 / 1200.0));
        let (note, cents) = MidiNote::nearest_to(sharp);
        assert_eq!(note, MidiNote(69));
        assert!((cents - 20.0).abs() < 0.01, "expected +20 cents, got {cents}");
    }

    #[test]
    fn match_to_fret_open_a4_on_uke_is_a_string_fret_0() {
        let uke = Tuning::standard_ukulele();
        let m = uke.match_to_fret(Frequency::A4, 20).expect("should match");
        assert_eq!(uke.strings[m.string_idx].name, "A4");
        assert_eq!(m.fret, 0);
        assert!(m.cents_off.abs() < 1.0);
    }

    #[test]
    fn match_to_fret_d4_on_uke_is_c_string_fret_2() {
        let uke = Tuning::standard_ukulele();
        let d4 = MidiNote(62).to_frequency();
        let m = uke.match_to_fret(d4, 20).expect("should match");
        assert_eq!(uke.strings[m.string_idx].name, "C4");
        assert_eq!(m.fret, 2);
    }

    #[test]
    fn match_to_fret_picks_lower_fret_when_ambiguous() {
        // D5 sits on every uke string: A fret 5, E fret 10, C fret 14, g fret 7.
        // Should pick A fret 5 — the lowest fret across all valid candidates.
        let uke = Tuning::standard_ukulele();
        let d5 = MidiNote(74).to_frequency();
        let m = uke.match_to_fret(d5, 20).expect("should match");
        assert_eq!(uke.strings[m.string_idx].name, "A4");
        assert_eq!(m.fret, 5);
    }

    #[test]
    fn match_to_fret_returns_none_for_freq_below_every_string() {
        let uke = Tuning::standard_ukulele();
        // 50 Hz mains hum — well below every open string on the uke.
        assert!(uke.match_to_fret(Frequency(50.0), 20).is_none());
    }

    #[test]
    fn match_to_fret_returns_none_when_no_string_can_reach() {
        let uke = Tuning::standard_ukulele();
        // 2000 Hz ≈ B6, far above fret 20 on every uke string.
        assert!(uke.match_to_fret(Frequency(2000.0), 20).is_none());
    }

    #[test]
    fn midi_from_name_parses_naturals_and_sharps() {
        assert_eq!(MidiNote::from_name("C4"), Some(MidiNote(60)));
        assert_eq!(MidiNote::from_name("A4"), Some(MidiNote(69)));
        assert_eq!(MidiNote::from_name("C#4"), Some(MidiNote(61)));
        assert_eq!(MidiNote::from_name("G#3"), Some(MidiNote(56)));
        assert_eq!(MidiNote::from_name("B0"), Some(MidiNote(23)));
    }

    #[test]
    fn midi_from_name_round_trips_through_name() {
        for midi in 21..=108 {
            let n = MidiNote(midi);
            let name = n.name();
            assert_eq!(
                MidiNote::from_name(&name),
                Some(n),
                "round trip failed for MIDI {midi} (name {name})"
            );
        }
    }

    #[test]
    fn midi_from_name_rejects_invalid_input() {
        assert_eq!(MidiNote::from_name(""), None);
        assert_eq!(MidiNote::from_name("H4"), None);
        assert_eq!(MidiNote::from_name("Cb4"), None); // no flats
        assert_eq!(MidiNote::from_name("foo"), None);
        assert_eq!(MidiNote::from_name("C"), None); // no octave
    }

    #[test]
    fn match_to_fret_accepts_slightly_flat_open_as_fret_0() {
        // String tuned 30 cents flat — should still register as fret 0 (the
        // user is just very slightly out of tune, not playing a negative fret).
        let uke = Tuning::standard_ukulele();
        let slightly_flat_a4 = Frequency(Frequency::A4.hz() * 2_f32.powf(-30.0 / 1200.0));
        let m = uke.match_to_fret(slightly_flat_a4, 20).expect("should match");
        assert_eq!(m.fret, 0);
        assert!((m.cents_off + 30.0).abs() < 0.01);
    }

    #[test]
    fn midi_nearest_to_clamps_to_chromatic_grid() {
        // 261.6 Hz is C4 (MIDI 60), exact.
        let (note, cents) = MidiNote::nearest_to(Frequency(261.6256));
        assert_eq!(note, MidiNote(60));
        assert!(cents.abs() < 0.01);
        // 277 Hz is between C4 and C#4 — should snap to C#4 (MIDI 61).
        let (note, _) = MidiNote::nearest_to(Frequency(277.18));
        assert_eq!(note, MidiNote(61));
    }
}
