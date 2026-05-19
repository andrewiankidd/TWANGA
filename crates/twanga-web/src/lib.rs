//! WASM bindings layer for TWANGA's browser frontend.
//!
//! Thin re-exports of `twanga-core` / `twanga-dsp` / `twanga-tabs` shaped for
//! JavaScript callers via `wasm-bindgen`. No audio I/O here — that lives in
//! `twanga-audio`'s WebAudio backend (still TBD) and the JS-side glue.
//!
//! Public surface is deliberately small for now: the goal is to verify the
//! Rust → WASM → JS pipeline works end-to-end before piping real audio through it.

use wasm_bindgen::prelude::*;

/// Pick a random splash line from the shared `twanga_core::SPLASHES` list.
/// Browser-visible proof-of-life that the WASM bridge is working at all.
#[wasm_bindgen]
pub fn pick_splash(seed: u32) -> String {
    let splashes: Vec<&str> = twanga_core::splashes().collect();
    if splashes.is_empty() {
        return String::new();
    }
    splashes[seed as usize % splashes.len()].to_string()
}

/// Names of every built-in tuning preset slug, JSON-ready as a `string[]`.
/// The frontend's tuning picker will eventually consume this directly.
#[wasm_bindgen]
pub fn builtin_tuning_slugs() -> Vec<String> {
    twanga_core::Tuning::builtin_slugs()
        .into_iter()
        .map(|s| s.to_string())
        .collect()
}

/// Parse a note name like `"A4"` / `"C#3"` into a MIDI number (or `None` if
/// invalid). Useful for the frontend to validate user-entered string pitches
/// when they're defining custom tunings, without re-implementing the parser.
#[wasm_bindgen]
pub fn midi_from_name(name: &str) -> Option<u8> {
    twanga_core::MidiNote::from_name(name).map(|n| n.0)
}

/// Inverse of [`midi_from_name`]. `0..=127` → name string.
#[wasm_bindgen]
pub fn midi_to_name(midi: u8) -> String {
    twanga_core::MidiNote(midi).name()
}

/// Human-readable display name for a built-in preset slug — e.g.
/// `"Standard Ukulele (Reentrant GCEA)"` for `"standard-ukulele"`. Returns
/// `None` for unknown slugs (matches `MidiNote::from_name`'s shape). Used
/// by the tuning picker so buttons read "Standard Ukulele" rather than
/// the developer-facing `standard-ukulele` slug.
#[wasm_bindgen]
pub fn preset_display_name(slug: &str) -> Option<String> {
    twanga_core::Tuning::from_preset(slug).map(|t| t.name)
}

/// Run YIN pitch detection over a buffer of mono f32 samples at `sample_rate`.
/// Returns the detected frequency in Hz, or `None` if the buffer is too quiet
/// or doesn't contain a confident pitch. This is the actual pitch engine the
/// browser tuner will end up calling once we wire microphone capture.
///
/// The YIN threshold of 0.15 matches what the native `Tuner` uses — keeps the
/// browser pitch detection behaviourally identical to the CLI tuner.
#[wasm_bindgen]
pub fn detect_pitch(samples: &[f32], sample_rate: u32) -> Option<f32> {
    use twanga_dsp::PitchDetector;
    let mut yin = twanga_dsp::Yin::new(0.15);
    yin.detect(samples, sample_rate).map(|f| f.hz())
}

/// Stateful tuner — the WASM mirror of `twanga_dsp::Tuner`. Buffers incoming
/// samples across multiple `feed` calls (web audio worklets post 128-sample
/// chunks; the YIN window is ~8K samples), drains readings on demand.
///
/// Three factory constructors:
/// - `WebTuner.new_chromatic(sr)` — nearest-12-TET-note mode; one reading
///   per accepted detection, label is the note name (`A4`, `C#3`, …).
/// - `WebTuner.new_for_strings(slug, sr)` — per-string mode against the
///   built-in tuning identified by `slug` (`standard-guitar`, etc.). At most
///   one reading per string per analysis window, gated against detections
///   that aren't plausibly any string in the tuning.
/// - `WebTuner.new_for_strings_with_capo(slug, capo_spec, sr)` — as above,
///   but applies a `Capo` (parsed from `capo_spec` — `"3"` for uniform,
///   `"0,2,2,2,2,2"` for per-string) before constructing the underlying
///   tuner. Targets are the effective post-capo open pitches, matching
///   what `twanga tune --capo` does on the CLI.
#[wasm_bindgen]
pub struct WebTuner {
    inner: twanga_dsp::Tuner,
    string_labels: Vec<String>,
    /// The tuning's display name, possibly suffixed with `(capo N)` /
    /// `(partial capo)` if a capo was applied. Mirrors `Tuning.name` after
    /// `Capo::apply`.
    name: String,
}

#[derive(serde::Serialize)]
struct ReadingJs {
    label: String,
    detected_hz: f32,
    target_hz: f32,
    cents: f32,
}

#[derive(serde::Serialize)]
struct StringInfoJs {
    label: String,
    open_hz: f32,
    midi: u8,
}

#[wasm_bindgen]
impl WebTuner {
    /// Build a chromatic tuner. Use this when the user hasn't picked an
    /// instrument — readings carry the nearest-MIDI note name as the label.
    pub fn new_chromatic(sample_rate: u32) -> WebTuner {
        WebTuner {
            inner: twanga_dsp::Tuner::new(twanga_dsp::TunerMode::Chromatic, sample_rate),
            string_labels: Vec::new(),
            name: "Chromatic".to_string(),
        }
    }

    /// Build a per-string tuner from a built-in preset slug
    /// (`standard-guitar` / `standard-banjo` / `standard-ukulele` / `drop-d-guitar`
    /// / `tenor-banjo` / `tenor-ukulele`). Errors if the slug is unknown so
    /// the JS side surfaces a clear message rather than silently falling back.
    /// Returns `Result<_, String>` rather than `Result<_, JsError>` so the
    /// same code paths can run in `cargo test` on native targets — the
    /// `JsError::new` import panics outside wasm.
    pub fn new_for_strings(slug: &str, sample_rate: u32) -> Result<WebTuner, String> {
        let tuning = twanga_core::Tuning::from_preset(slug)
            .ok_or_else(|| format!("unknown tuning slug: {slug}"))?;
        let labels = tuning.strings.iter().map(|s| s.name.clone()).collect();
        let name = tuning.name.clone();
        Ok(WebTuner {
            inner: twanga_dsp::Tuner::new(twanga_dsp::TunerMode::Strings(tuning), sample_rate),
            string_labels: labels,
            name,
        })
    }

    /// Like [`Self::new_for_strings`] but applies a `Capo` first, so the
    /// targets are the post-capo open pitches. `capo_spec` accepts either a
    /// uniform integer (`"3"` — capo on fret 3) or a per-string comma list
    /// (`"0,2,2,2,2,2"` — drop-D-style partial capo). Empty / `"0"` produces
    /// a no-op capo and behaves identically to `new_for_strings`. Mirrors
    /// `twanga tune --capo` on the CLI exactly.
    pub fn new_for_strings_with_capo(
        slug: &str,
        capo_spec: &str,
        sample_rate: u32,
    ) -> Result<WebTuner, String> {
        let tuning = twanga_core::Tuning::from_preset(slug)
            .ok_or_else(|| format!("unknown tuning slug: {slug}"))?;
        let capo = twanga_core::Capo::parse(capo_spec, tuning.strings.len())?;
        let effective = capo.apply(&tuning)?;
        let labels = effective.strings.iter().map(|s| s.name.clone()).collect();
        let name = effective.name.clone();
        Ok(WebTuner {
            inner: twanga_dsp::Tuner::new(twanga_dsp::TunerMode::Strings(effective), sample_rate),
            string_labels: labels,
            name,
        })
    }

    /// Effective display name for the active tuning. After capo application
    /// this is e.g. `"Standard Banjo (Open G) (capo 3)"`; without a capo it's
    /// the raw preset name (`"Standard Ukulele (Reentrant GCEA)"`). For
    /// chromatic-mode tuners it's `"Chromatic"`.
    pub fn name(&self) -> String {
        self.name.clone()
    }

    /// The string labels in string-number order (string 1 first). Lets the
    /// frontend render the per-string rows up-front, before any audio has
    /// been fed, so the layout doesn't shift around when the first reading
    /// arrives. Returns an empty array for chromatic-mode tuners.
    pub fn string_labels(&self) -> Vec<String> {
        self.string_labels.clone()
    }

    /// Open-string info (label, open Hz, MIDI) for each string in the
    /// per-string tuner. Returns an empty array for chromatic tuners.
    /// Useful for the UI to show "A4 — 440.00 Hz" target headers.
    pub fn strings_info(&self) -> JsValue {
        let info: Vec<StringInfoJs> = match self.inner.mode() {
            twanga_dsp::TunerMode::Strings(t) => t
                .strings
                .iter()
                .map(|s| StringInfoJs {
                    label: s.name.clone(),
                    open_hz: s.open.to_frequency().hz(),
                    midi: s.open.0,
                })
                .collect(),
            twanga_dsp::TunerMode::Chromatic => Vec::new(),
        };
        serde_wasm_bindgen::to_value(&info).unwrap()
    }

    /// Push mono f32 samples into the analysis buffer. Same call pattern as
    /// the native `Tuner::feed`. Worklet chunks (128 samples on most
    /// browsers) accumulate across calls until a YIN window fills.
    pub fn feed(&mut self, samples: &[f32]) {
        self.inner.feed(samples);
    }

    /// Drain accumulated readings. Each entry is `{ label, detected_hz,
    /// target_hz, cents }`. In `Strings` mode multiple strings can produce
    /// readings per call; the UI should treat the result as "updates for
    /// these strings" and leave others unchanged.
    pub fn take_readings(&mut self) -> JsValue {
        let readings: Vec<ReadingJs> = self
            .inner
            .take_readings()
            .map(|r| ReadingJs {
                label: r.label,
                detected_hz: r.detected.hz(),
                target_hz: r.target.hz(),
                cents: r.cents,
            })
            .collect();
        serde_wasm_bindgen::to_value(&readings).unwrap()
    }
}

#[cfg(test)]
mod tests {
    //! `#[wasm_bindgen]` macros expand to regular fn bodies when compiling for
    //! a non-wasm target, so we can `cargo test -p twanga-web` directly. These
    //! tests cover the wrapper logic — translation from Rust types to the
    //! shapes JS sees — not the underlying crates (those have their own
    //! coverage already in twanga-core / twanga-dsp).
    use super::*;

    #[test]
    fn pick_splash_returns_nonempty_and_is_stable_per_seed() {
        let a = pick_splash(0);
        let b = pick_splash(0);
        assert!(!a.is_empty(), "splash should not be empty");
        assert_eq!(a, b, "same seed must yield same splash");
    }

    #[test]
    fn pick_splash_wraps_around_modulo_list_length() {
        // Doesn't panic on huge seeds — modulo over the splash count keeps it
        // in range. Brittle assertion: just confirm it succeeds and returns
        // a known-good string. Equality with seed=0 is the proof of wrap.
        let s = pick_splash(u32::MAX);
        assert!(!s.is_empty());
    }

    #[test]
    fn builtin_tuning_slugs_matches_core_registry() {
        let from_web = builtin_tuning_slugs();
        let from_core: Vec<String> = twanga_core::Tuning::builtin_slugs()
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            from_web, from_core,
            "wasm wrapper must not reorder or drop slugs"
        );
    }

    #[test]
    fn midi_from_name_handles_canonical_notes() {
        assert_eq!(midi_from_name("A4"), Some(69));
        assert_eq!(midi_from_name("C4"), Some(60));
        assert_eq!(midi_from_name("C#3"), Some(49));
        assert_eq!(midi_from_name("E2"), Some(40));
    }

    #[test]
    fn midi_from_name_rejects_garbage() {
        assert_eq!(midi_from_name(""), None);
        assert_eq!(midi_from_name("garbage"), None);
        assert_eq!(midi_from_name("H4"), None); // no H note in 12-TET
        assert_eq!(midi_from_name("Cb4"), None); // no flats accepted
    }

    #[test]
    fn midi_to_name_is_inverse_of_from_name_across_full_range() {
        for midi in 21..=108_u8 {
            let name = midi_to_name(midi);
            assert_eq!(
                midi_from_name(&name),
                Some(midi),
                "round-trip failed for MIDI {midi} (name {name})"
            );
        }
    }

    #[test]
    fn detect_pitch_identifies_440hz_sine() {
        let sr = 48_000;
        let n = 4096;
        let buf: Vec<f32> = (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin())
            .collect();
        let hz = detect_pitch(&buf, sr).expect("should detect a pitch");
        assert!((hz - 440.0).abs() < 1.0, "expected ~440 Hz, got {hz:.2}");
    }

    #[test]
    fn detect_pitch_rejects_silence() {
        // All-zero buffer has no pitch — YIN's threshold gate should reject
        // it and return None. This confirms the wrapper propagates the
        // detector's "no confident pitch" signal without translating to e.g.
        // 0.0 Hz, which would mislead the frontend.
        let buf = vec![0.0_f32; 4096];
        assert_eq!(detect_pitch(&buf, 48_000), None);
    }

    #[test]
    fn detect_pitch_handles_buffer_too_short_to_analyse() {
        // YIN needs at least a couple of samples; tiny buffers should return
        // None rather than panic.
        let buf = vec![0.5_f32; 4];
        let _ = detect_pitch(&buf, 48_000); // just confirm no panic
    }

    /// Helper: synthetic sine wave of `freq` Hz, length samples, normalised
    /// to ±1. Mirrors the test fixture pattern from twanga-dsp.
    fn sine(freq: f32, sample_rate: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate as f32).sin())
            .collect()
    }

    #[test]
    fn web_tuner_strings_labels_match_preset() {
        // Per-string-tuner labels must come back in preset order — the
        // browser UI relies on this to lay out rows top-to-bottom in
        // string-number order (string 1 first).
        let t = WebTuner::new_for_strings("standard-ukulele", 48_000).unwrap();
        assert_eq!(t.string_labels(), vec!["A4", "E4", "C4", "g4 (reentrant)"]);
    }

    #[test]
    fn web_tuner_rejects_unknown_slug() {
        // `WebTuner` doesn't implement Debug (no value in deriving on a
        // wasm-bindgen exported struct), so use `is_err()` rather than
        // `unwrap_err()`, which would need Debug on the success type.
        assert!(WebTuner::new_for_strings("not-a-tuning", 48_000).is_err());
    }

    #[test]
    fn web_tuner_chromatic_has_no_string_labels() {
        let t = WebTuner::new_chromatic(48_000);
        assert!(t.string_labels().is_empty());
    }

    #[test]
    fn web_tuner_strings_detects_a4_against_uke_a_string() {
        // 440 Hz fed in should produce a reading labelled "A4" (the uke's
        // first string) within a few cents of zero.
        let mut t = WebTuner::new_for_strings("standard-ukulele", 48_000).unwrap();
        // Native Tuner needs enough samples to fill a window (8192). Feed
        // a generous run so we definitely get a reading out.
        let samples = sine(440.0, 48_000, 16_384);
        t.inner.feed(&samples);
        let readings: Vec<_> = t.inner.take_readings().collect();
        assert!(
            !readings.is_empty(),
            "expected at least one reading for 440 Hz"
        );
        let first = &readings[0];
        assert_eq!(first.label, "A4");
        assert!(
            first.cents.abs() < 5.0,
            "expected near-zero cents on the A string, got {:.2}",
            first.cents
        );
    }

    #[test]
    fn web_tuner_chromatic_identifies_440hz_as_a4() {
        let mut t = WebTuner::new_chromatic(48_000);
        let samples = sine(440.0, 48_000, 16_384);
        t.inner.feed(&samples);
        let readings: Vec<_> = t.inner.take_readings().collect();
        assert!(!readings.is_empty());
        assert_eq!(readings[0].label, "A4");
    }

    #[test]
    fn preset_display_name_returns_human_label() {
        assert_eq!(
            preset_display_name("standard-ukulele").as_deref(),
            Some("Standard Ukulele (Reentrant GCEA)"),
        );
        assert_eq!(
            preset_display_name("drop-d-guitar").as_deref(),
            Some("Drop D Guitar (DADGBE)"),
        );
        assert!(preset_display_name("not-a-tuning").is_none());
    }

    #[test]
    fn web_tuner_name_reflects_active_mode() {
        let chromatic = WebTuner::new_chromatic(48_000);
        assert_eq!(chromatic.name(), "Chromatic");

        let uke = WebTuner::new_for_strings("standard-ukulele", 48_000).unwrap();
        assert_eq!(uke.name(), "Standard Ukulele (Reentrant GCEA)");
    }

    #[test]
    fn web_tuner_with_capo_shifts_targets_and_relabels_strings() {
        // Uniform capo 3 on uke: every string +3 semitones. A4 → C5,
        // E4 → G4, C4 → D#4 (Eb4), reentrant g4 → A#4 (Bb4). The labels
        // come from `MidiNote::name()` of the shifted MIDI value, which
        // is `Capo::apply`'s convention when the offset is non-zero.
        let t =
            WebTuner::new_for_strings_with_capo("standard-ukulele", "3", 48_000).unwrap();
        let labels = t.string_labels();
        assert_eq!(labels, vec!["C5", "G4", "D#4", "A#4"]);

        // Effective name carries the capo annotation.
        assert!(
            t.name().contains("capo 3"),
            "expected '(capo 3)' in name, got {:?}",
            t.name(),
        );
    }

    #[test]
    fn web_tuner_with_zero_capo_matches_no_capo_constructor() {
        // `"0"` is the canonical no-op capo. Should produce the same labels
        // as the no-capo constructor, with the reentrant label preserved
        // (since the offset on that string is also 0).
        let with = WebTuner::new_for_strings_with_capo("standard-ukulele", "0", 48_000).unwrap();
        let without = WebTuner::new_for_strings("standard-ukulele", 48_000).unwrap();
        assert_eq!(with.string_labels(), without.string_labels());
        assert_eq!(with.string_labels()[3], "g4 (reentrant)");
    }

    #[test]
    fn web_tuner_with_partial_capo_preserves_unchanged_string_labels() {
        // Banjo body capo on fret 3, 5th-string drone left open. The drone
        // label `g4 (drone)` should survive because its offset is 0; the
        // body strings get relabelled to their new MIDI names.
        let t = WebTuner::new_for_strings_with_capo("standard-banjo", "3,3,3,3,0", 48_000)
            .unwrap();
        let labels = t.string_labels();
        // Body strings: D4+3=F4, B3+3=D4, G3+3=A#3, D3+3=F3.
        assert_eq!(&labels[..4], &["F4", "D4", "A#3", "F3"]);
        // 5th-string drone label is preserved (offset 0).
        assert_eq!(labels[4], "g4 (drone)");
        assert!(t.name().contains("partial capo"));
    }

    #[test]
    fn web_tuner_with_invalid_capo_returns_err() {
        // Capo spec with wrong length should propagate the Capo::parse
        // error message up to the JS side, not panic.
        assert!(
            WebTuner::new_for_strings_with_capo("standard-ukulele", "1,2", 48_000).is_err()
        );
    }
}
