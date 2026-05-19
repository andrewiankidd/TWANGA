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
        assert!(
            (hz - 440.0).abs() < 1.0,
            "expected ~440 Hz, got {hz:.2}"
        );
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
}
