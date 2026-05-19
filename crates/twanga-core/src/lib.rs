//! Domain types and a small amount of shared static content. No IO, no async, no algorithms.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::LazyLock;

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

/// On-disk schema for a single tuning entry. The same shape is used for the
/// built-in `presets.toml` (compiled into the binary) and the user-config file
/// (`$CONFIG/twanga/tunings.toml`). This lets the CLI's "define a custom
/// tuning" flow round-trip through the exact same format the maintainer would
/// hand-edit in source — so promoting a user creation to a built-in is just
/// copying the TOML block over.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PresetEntry {
    pub slug: String,
    pub name: String,
    pub strings: Vec<PresetString>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PresetString {
    pub name: String,
    pub midi: u8,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PresetFile {
    #[serde(default, rename = "tuning")]
    pub tunings: Vec<PresetEntry>,
}

impl PresetEntry {
    pub fn to_tuning(&self) -> Tuning {
        Tuning {
            name: self.name.clone(),
            strings: self
                .strings
                .iter()
                .map(|s| TunedString {
                    name: s.name.clone(),
                    open: MidiNote(s.midi),
                })
                .collect(),
        }
    }

    pub fn from_tuning(slug: impl Into<String>, t: &Tuning) -> Self {
        Self {
            slug: slug.into(),
            name: t.name.clone(),
            strings: t
                .strings
                .iter()
                .map(|s| PresetString {
                    name: s.name.clone(),
                    midi: s.open.0,
                })
                .collect(),
        }
    }
}

const BUILTIN_PRESETS_TOML: &str = include_str!("presets.toml");

static BUILTIN_PRESETS: LazyLock<Vec<PresetEntry>> = LazyLock::new(|| {
    let file: PresetFile = toml::from_str(BUILTIN_PRESETS_TOML)
        .expect("crates/twanga-core/src/presets.toml is malformed (TWANGA bug)");
    file.tunings
});

impl Tuning {
    pub fn standard_guitar() -> Self {
        Self::from_preset("standard-guitar")
            .expect("standard-guitar preset missing from built-in registry")
    }

    pub fn standard_banjo() -> Self {
        Self::from_preset("standard-banjo")
            .expect("standard-banjo preset missing from built-in registry")
    }

    pub fn standard_ukulele() -> Self {
        Self::from_preset("standard-ukulele")
            .expect("standard-ukulele preset missing from built-in registry")
    }

    /// All built-in preset entries in the order they appear in `presets.toml`.
    /// User-defined tunings live in `$CONFIG/twanga/tunings.toml` and are
    /// merged with these by the CLI — twanga-core stays IO-free.
    pub fn builtin_presets() -> &'static [PresetEntry] {
        &BUILTIN_PRESETS
    }

    /// Slugs of the built-in presets, in registry order. Use this when
    /// rendering a chooser that only includes shipped tunings; use the CLI's
    /// merged loader if you want user-defined tunings too.
    pub fn builtin_slugs() -> Vec<&'static str> {
        BUILTIN_PRESETS.iter().map(|p| p.slug.as_str()).collect()
    }

    /// Build a tuning from a built-in preset slug. Returns `None` if the slug
    /// doesn't match anything in `presets.toml`. Does NOT consult the user
    /// config file — that's the CLI's job.
    pub fn from_preset(slug: &str) -> Option<Tuning> {
        BUILTIN_PRESETS
            .iter()
            .find(|p| p.slug == slug)
            .map(|p| p.to_tuning())
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
                Some(b) => fret < b.fret || (fret == b.fret && cents_off.abs() < b.cents_off.abs()),
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
        let empty = Tuning {
            name: "".into(),
            strings: vec![],
        };
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
        assert!(
            (cents - 20.0).abs() < 0.01,
            "expected +20 cents, got {cents}"
        );
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
        let m = uke
            .match_to_fret(slightly_flat_a4, 20)
            .expect("should match");
        assert_eq!(m.fret, 0);
        assert!((m.cents_off + 30.0).abs() < 0.01);
    }

    #[test]
    fn builtin_registry_contains_three_canonical_slugs() {
        let slugs = Tuning::builtin_slugs();
        assert!(slugs.contains(&"standard-guitar"));
        assert!(slugs.contains(&"standard-banjo"));
        assert!(slugs.contains(&"standard-ukulele"));
    }

    #[test]
    fn registry_backed_guitar_matches_canonical_open_pitches() {
        let g = Tuning::standard_guitar();
        assert_eq!(g.strings.len(), 6);
        assert_eq!(g.strings[0].open, MidiNote(64)); // E4
        assert_eq!(g.strings[5].open, MidiNote(40)); // E2
    }

    #[test]
    fn registry_backed_banjo_keeps_drone_as_string_5() {
        let b = Tuning::standard_banjo();
        assert_eq!(b.strings.len(), 5);
        // 5th-string drone is higher in pitch than the 4th string — the whole
        // point of the registry round-trip preserving string-number order.
        assert!(b.strings[4].open > b.strings[3].open);
        assert_eq!(b.strings[4].open, MidiNote(67));
    }

    #[test]
    fn registry_backed_ukulele_keeps_reentrant_g_as_string_4() {
        let u = Tuning::standard_ukulele();
        assert_eq!(u.strings.len(), 4);
        assert_eq!(u.strings[3].open, MidiNote(67)); // reentrant g4
        assert!(u.strings[3].open > u.strings[2].open); // higher than C4 string
    }

    #[test]
    fn from_preset_returns_none_for_unknown_slug() {
        assert!(Tuning::from_preset("standard-mandolin").is_none());
        assert!(Tuning::from_preset("").is_none());
    }

    #[test]
    fn preset_entry_round_trips_through_toml() {
        let original = PresetEntry {
            slug: "custom-test".into(),
            name: "Custom Test Tuning".into(),
            strings: vec![
                PresetString {
                    name: "A4".into(),
                    midi: 69,
                },
                PresetString {
                    name: "D4".into(),
                    midi: 62,
                },
            ],
        };
        let file = PresetFile {
            tunings: vec![original.clone()],
        };
        let serialised = toml::to_string(&file).expect("serialise");
        let parsed: PresetFile = toml::from_str(&serialised).expect("parse");
        assert_eq!(parsed.tunings, vec![original]);
    }

    #[test]
    fn preset_entry_round_trips_through_tuning() {
        let entry = PresetEntry {
            slug: "tenor-banjo".into(),
            name: "Tenor Banjo (CGDA)".into(),
            strings: vec![
                PresetString {
                    name: "A4".into(),
                    midi: 69,
                },
                PresetString {
                    name: "D4".into(),
                    midi: 62,
                },
                PresetString {
                    name: "G3".into(),
                    midi: 55,
                },
                PresetString {
                    name: "C3".into(),
                    midi: 48,
                },
            ],
        };
        let tuning = entry.to_tuning();
        let recovered = PresetEntry::from_tuning(&entry.slug, &tuning);
        assert_eq!(recovered, entry);
    }

    #[test]
    fn drop_d_guitar_only_differs_from_standard_on_string_six() {
        let std = Tuning::standard_guitar();
        let drop = Tuning::from_preset("drop-d-guitar").expect("drop-d-guitar preset");
        assert_eq!(drop.strings.len(), 6);
        // String 1-5 (indices 0-4) must match standard guitar exactly.
        for i in 0..5 {
            assert_eq!(
                drop.strings[i].open,
                std.strings[i].open,
                "string {} should match standard guitar",
                i + 1
            );
        }
        // String 6 is dropped from E2 (40) to D2 (38).
        assert_eq!(std.strings[5].open, MidiNote(40));
        assert_eq!(drop.strings[5].open, MidiNote(38));
    }

    #[test]
    fn tenor_banjo_is_fifths_cgda() {
        let t = Tuning::from_preset("tenor-banjo").expect("tenor-banjo preset");
        assert_eq!(t.strings.len(), 4);
        // String-1-first: A3 D3 G2 C2 — each string a fifth (7 semitones) lower.
        assert_eq!(t.strings[0].open, MidiNote(57)); // A3
        assert_eq!(t.strings[1].open, MidiNote(50)); // D3
        assert_eq!(t.strings[2].open, MidiNote(43)); // G2
        assert_eq!(t.strings[3].open, MidiNote(36)); // C2
        for i in 0..3 {
            assert_eq!(
                t.strings[i].open.0 - t.strings[i + 1].open.0,
                7,
                "consecutive strings should be a fifth apart"
            );
        }
    }

    #[test]
    fn tenor_ukulele_uses_low_g_not_reentrant() {
        let t = Tuning::from_preset("tenor-ukulele").expect("tenor-ukulele preset");
        assert_eq!(t.strings.len(), 4);
        let g3 = t.strings[3].open;
        let c4 = t.strings[2].open;
        // String 4 (G) is BELOW the C string — low-G, not the reentrant high-g
        // that standard ukulele uses. Whole point of the separate preset.
        assert!(g3 < c4, "tenor-ukulele G should be below C (low-G)");
        assert_eq!(g3, MidiNote(55));
    }

    #[test]
    fn builtin_presets_parse_with_all_slugs_unique() {
        let presets = Tuning::builtin_presets();
        let mut seen = std::collections::HashSet::new();
        for p in presets {
            assert!(
                seen.insert(&p.slug),
                "duplicate slug in built-in registry: {}",
                p.slug
            );
            assert!(
                !p.name.is_empty(),
                "preset {} has empty display name",
                p.slug
            );
            assert!(!p.strings.is_empty(), "preset {} has zero strings", p.slug);
        }
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
