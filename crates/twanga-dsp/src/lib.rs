//! Pure pitch detection over `&[f32]`. No IO, no async.
//!
//! `Yin` is the underlying detector (allocation-free after first call).
//! `Tuner` layers buffer management + a configurable "what does this
//! frequency mean?" lookup on top — see [`TunerMode`]. Same code path for
//! the CLI's single-line chromatic tuner and the multi-string display.

use twanga_core::{Frequency, MidiNote, Tuning};

pub trait PitchDetector {
    fn detect(&mut self, samples: &[f32], sample_rate: u32) -> Option<Frequency>;
}

pub struct Yin {
    pub threshold: f32,
    d: Vec<f32>,
    cmnd: Vec<f32>,
}

impl Yin {
    pub fn new(threshold: f32) -> Self {
        Self {
            threshold,
            d: Vec::new(),
            cmnd: Vec::new(),
        }
    }
}

impl PitchDetector for Yin {
    fn detect(&mut self, samples: &[f32], sample_rate: u32) -> Option<Frequency> {
        let half = samples.len() / 2;
        if half < 2 {
            return None;
        }

        self.d.clear();
        self.d.resize(half, 0.0);
        self.cmnd.clear();
        self.cmnd.resize(half, 1.0);

        for tau in 1..half {
            let mut sum = 0.0_f32;
            for j in 0..half {
                let diff = samples[j] - samples[j + tau];
                sum += diff * diff;
            }
            self.d[tau] = sum;
        }

        let mut running_sum = 0.0_f32;
        for tau in 1..half {
            running_sum += self.d[tau];
            self.cmnd[tau] = if running_sum > 0.0 {
                self.d[tau] * tau as f32 / running_sum
            } else {
                1.0
            };
        }

        let mut tau = None;
        let mut t = 2_usize;
        while t < half {
            if self.cmnd[t] < self.threshold {
                while t + 1 < half && self.cmnd[t + 1] < self.cmnd[t] {
                    t += 1;
                }
                tau = Some(t);
                break;
            }
            t += 1;
        }
        let tau = tau?;

        let tau_refined = if tau > 0 && tau < half - 1 {
            let x0 = self.cmnd[tau - 1];
            let x1 = self.cmnd[tau];
            let x2 = self.cmnd[tau + 1];
            let denom = x0 - 2.0 * x1 + x2;
            if denom.abs() > f32::EPSILON {
                tau as f32 + (x0 - x2) / (2.0 * denom)
            } else {
                tau as f32
            }
        } else {
            tau as f32
        };

        if tau_refined > 0.0 {
            Some(Frequency(sample_rate as f32 / tau_refined))
        } else {
            None
        }
    }
}

/// How the tuner maps a detected frequency to a target.
///
/// - `Chromatic` snaps to the nearest 12-TET note; the label becomes that
///   note's name (`A4`, `C#3`, etc.). Use when you don't want to pre-select
///   an instrument — works like a clip-on chromatic tuner.
/// - `Strings(t)` snaps to the nearest open string in the given tuning;
///   the label becomes that string's name. Use when you want per-string
///   targets and a multi-string display.
pub enum TunerMode {
    Chromatic,
    Strings(Tuning),
}

impl TunerMode {
    fn lookup(&self, freq: Frequency) -> Option<(String, Frequency, f32)> {
        match self {
            Self::Chromatic => {
                let (note, cents) = MidiNote::nearest_to(freq);
                Some((note.name(), note.to_frequency(), cents))
            }
            Self::Strings(tuning) => tuning.nearest_string(freq).map(|(s, cents)| {
                (s.name.clone(), s.open.to_frequency(), cents)
            }),
        }
    }
}

/// One detection event from the live tuner.
#[derive(Debug, Clone)]
pub struct TunerReading {
    /// Frequency YIN detected on the most recent analysis window.
    pub detected: Frequency,
    /// Label of the matched target: the open-string name in `Strings` mode,
    /// the chromatic note name in `Chromatic` mode.
    pub label: String,
    /// The target's exact pitch.
    pub target: Frequency,
    /// Signed cents difference (positive = detected is sharp of target).
    pub cents: f32,
}

/// Streaming tuner. Accumulates mono samples, runs YIN on each completed
/// analysis window, and runs the configured [`TunerMode`] lookup.
pub struct Tuner {
    yin: Yin,
    mode: TunerMode,
    sample_rate: u32,
    window_size: usize,
    slide_by: usize,
    buffer: Vec<f32>,
    readings: Vec<TunerReading>,
}

impl Tuner {
    pub const DEFAULT_WINDOW: usize = 8192;
    pub const DEFAULT_SLIDE_BY: usize = 4096;

    /// Window RMS below this is treated as silence — no detection attempted.
    /// 0.005 catches a quiet room while staying well below any plucked-string note.
    pub const SILENCE_RMS: f32 = 0.005;

    /// In `Strings` mode, detections this far from the nearest open string are
    /// treated as noise (mains hum, cable EMI, room sounds) and dropped. A
    /// tritone is enough headroom that a string can be wildly mis-tuned and
    /// still register against its intended target — anything past it is not a
    /// plausible attempt at any string in the tuning.
    pub const MAX_STRING_DISTANCE_CENTS: f32 = 700.0;

    pub fn new(mode: TunerMode, sample_rate: u32) -> Self {
        Self {
            yin: Yin::new(0.15),
            mode,
            sample_rate,
            window_size: Self::DEFAULT_WINDOW,
            slide_by: Self::DEFAULT_SLIDE_BY,
            buffer: Vec::with_capacity(Self::DEFAULT_WINDOW * 2),
            readings: Vec::new(),
        }
    }

    pub fn mode(&self) -> &TunerMode {
        &self.mode
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Push mono samples. Runs YIN on every completed window (subject to a
    /// silence gate) and queues a `TunerReading` per accepted detection.
    pub fn feed(&mut self, samples: &[f32]) {
        self.buffer.extend_from_slice(samples);
        while self.buffer.len() >= self.window_size {
            let window = &self.buffer[..self.window_size];

            // Silence gate — don't try to find pitch in cable hum or room noise.
            if window_rms(window) < Self::SILENCE_RMS {
                self.buffer.drain(..self.slide_by);
                continue;
            }

            if let Some(freq) = self.yin.detect(window, self.sample_rate) {
                if let Some((label, target, cents)) = self.mode.lookup(freq) {
                    let too_far_from_any_string = matches!(self.mode, TunerMode::Strings(_))
                        && cents.abs() > Self::MAX_STRING_DISTANCE_CENTS;
                    if !too_far_from_any_string {
                        self.readings.push(TunerReading {
                            detected: freq,
                            label,
                            target,
                            cents,
                        });
                    }
                }
            }
            self.buffer.drain(..self.slide_by);
        }
    }

    pub fn take_readings(&mut self) -> std::vec::Drain<'_, TunerReading> {
        self.readings.drain(..)
    }
}

fn window_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|x| x * x).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use twanga_core::MidiNote;
    use twanga_synth::{harmonic_stack, mix_into, sine, white_noise};

    fn cents(detected: Frequency, expected: Frequency) -> f32 {
        1200.0 * (detected.hz() / expected.hz()).log2()
    }

    // ---- YIN tests ----

    #[test]
    fn yin_detects_a4_sine() {
        let mut yin = Yin::new(0.15);
        let buf = sine(Frequency::A4, 44100, 2048);
        let detected = yin
            .detect(&buf, 44100)
            .expect("YIN should detect on a clean A4 sine");
        let diff = cents(detected, Frequency::A4);
        assert!(
            diff.abs() < 5.0,
            "expected A4 within ±5 cents, got {detected} ({diff:+.2} cents)"
        );
    }

    #[test]
    fn yin_detects_midi_sweep() {
        let mut yin = Yin::new(0.15);
        for midi in 48..=81 {
            let expected = MidiNote(midi).to_frequency();
            let buf = sine(expected, 44100, 4096);
            let detected = yin
                .detect(&buf, 44100)
                .unwrap_or_else(|| panic!("YIN failed on MIDI {midi}"));
            let diff = cents(detected, expected);
            assert!(diff.abs() < 5.0, "MIDI {midi}: {diff:+.2} cents");
        }
    }

    #[test]
    fn yin_picks_fundamental_not_harmonic() {
        let mut yin = Yin::new(0.15);
        let buf = harmonic_stack(Frequency::A4, 44100, 4096, &[1.0, 0.7, 0.5, 0.3]);
        let detected = yin.detect(&buf, 44100).expect("should detect");
        assert!(cents(detected, Frequency::A4).abs() < 5.0);
    }

    #[test]
    fn yin_detects_a4_through_moderate_noise() {
        let mut yin = Yin::new(0.15);
        let mut buf = sine(Frequency::A4, 44100, 4096);
        let noise = white_noise(0.1, 4096, 12345);
        mix_into(&mut buf, &noise);
        let detected = yin.detect(&buf, 44100).expect("should detect");
        assert!(cents(detected, Frequency::A4).abs() < 10.0);
    }

    #[test]
    fn yin_returns_none_for_silence() {
        let mut yin = Yin::new(0.15);
        assert!(yin.detect(&vec![0.0_f32; 2048], 44100).is_none());
    }

    #[test]
    fn yin_returns_none_for_too_short_buffer() {
        let mut yin = Yin::new(0.15);
        assert!(yin.detect(&vec![0.0_f32; 2], 44100).is_none());
    }

    #[test]
    fn yin_reuses_scratch_buffers_across_calls() {
        let mut yin = Yin::new(0.15);
        let buf = sine(Frequency::A4, 44100, 4096);
        let _ = yin.detect(&buf, 44100);
        let cap_d = yin.d.capacity();
        let cap_cmnd = yin.cmnd.capacity();
        let _ = yin.detect(&buf, 44100);
        assert_eq!(yin.d.capacity(), cap_d);
        assert_eq!(yin.cmnd.capacity(), cap_cmnd);
    }

    // ---- Layer 2: FFT cross-check ----

    fn fft_peak_hz(samples: &[f32], sample_rate: u32) -> f32 {
        use rustfft::{num_complex::Complex, FftPlanner};

        let n = samples.len();
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n);

        let mut buffer: Vec<Complex<f32>> = samples
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                let w = 0.5
                    * (1.0 - (std::f32::consts::TAU * i as f32 / (n - 1) as f32).cos());
                Complex { re: s * w, im: 0.0 }
            })
            .collect();
        fft.process(&mut buffer);

        let half = n / 2;
        let magnitudes: Vec<f32> = buffer[..half]
            .iter()
            .map(|c| (c.re * c.re + c.im * c.im).sqrt())
            .collect();

        let (peak_bin, _) = magnitudes
            .iter()
            .enumerate()
            .skip(1)
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .expect("non-empty FFT");

        let refined_bin = if peak_bin > 0 && peak_bin < half - 1 {
            let y0 = magnitudes[peak_bin - 1].max(f32::EPSILON).ln();
            let y1 = magnitudes[peak_bin].max(f32::EPSILON).ln();
            let y2 = magnitudes[peak_bin + 1].max(f32::EPSILON).ln();
            let denom = y0 - 2.0 * y1 + y2;
            if denom.abs() > f32::EPSILON {
                peak_bin as f32 + (y0 - y2) / (2.0 * denom)
            } else {
                peak_bin as f32
            }
        } else {
            peak_bin as f32
        };

        refined_bin * sample_rate as f32 / n as f32
    }

    #[test]
    fn yin_agrees_with_fft_across_midi_sweep() {
        let mut yin = Yin::new(0.15);
        for midi in 48..=81 {
            let expected = MidiNote(midi).to_frequency();
            let buf = sine(expected, 44100, 8192);
            let yin_hz = yin
                .detect(&buf, 44100)
                .unwrap_or_else(|| panic!("YIN failed on MIDI {midi}"))
                .hz();
            let fft_hz = fft_peak_hz(&buf, 44100);
            let diff_cents = 1200.0 * (yin_hz / fft_hz).log2();
            assert!(diff_cents.abs() < 10.0, "MIDI {midi}: {diff_cents:+.2} cents");
        }
    }

    #[test]
    #[ignore = "TODO: source an external 440 Hz reference WAV (sox, Audacity, etc.)"]
    fn yin_detects_external_a4_reference_wav() {
        todo!("source and decode an external 440 Hz reference WAV");
    }

    // ---- Tuner tests (string mode) ----

    #[test]
    fn tuner_strings_yields_one_reading_per_completed_window() {
        let mut tuner = Tuner::new(TunerMode::Strings(Tuning::standard_ukulele()), 44100);
        let samples = sine(Frequency::A4, 44100, Tuner::DEFAULT_WINDOW);
        tuner.feed(&samples);
        assert_eq!(tuner.take_readings().count(), 1);
    }

    #[test]
    fn tuner_strings_emits_no_readings_until_window_is_full() {
        let mut tuner = Tuner::new(TunerMode::Strings(Tuning::standard_banjo()), 44100);
        let samples = sine(Frequency::A4, 44100, Tuner::DEFAULT_WINDOW / 2);
        tuner.feed(&samples);
        assert_eq!(tuner.take_readings().count(), 0);
    }

    #[test]
    fn tuner_strings_resolves_a4_to_uke_a_string() {
        let mut tuner = Tuner::new(TunerMode::Strings(Tuning::standard_ukulele()), 44100);
        let samples = sine(Frequency::A4, 44100, Tuner::DEFAULT_WINDOW);
        tuner.feed(&samples);
        let r = tuner.take_readings().next().expect("reading");
        assert_eq!(r.label, "A4");
        assert!(r.cents.abs() < 5.0);
    }

    #[test]
    fn tuner_handles_streaming_chunks_smaller_than_window() {
        let mut tuner = Tuner::new(TunerMode::Strings(Tuning::standard_ukulele()), 44100);
        let samples = sine(Frequency::A4, 44100, Tuner::DEFAULT_WINDOW);
        for chunk in samples.chunks(256) {
            tuner.feed(chunk);
        }
        assert_eq!(tuner.take_readings().count(), 1);
    }

    #[test]
    fn tuner_emits_multiple_readings_when_multiple_windows_fit() {
        let mut tuner = Tuner::new(TunerMode::Strings(Tuning::standard_ukulele()), 44100);
        let total = Tuner::DEFAULT_WINDOW + 2 * Tuner::DEFAULT_SLIDE_BY;
        let samples = sine(Frequency::A4, 44100, total);
        tuner.feed(&samples);
        assert_eq!(tuner.take_readings().count(), 3);
    }

    // ---- Tuner tests (chromatic mode) ----

    #[test]
    fn tuner_chromatic_resolves_a4_sine_to_a4_label() {
        let mut tuner = Tuner::new(TunerMode::Chromatic, 44100);
        let samples = sine(Frequency::A4, 44100, Tuner::DEFAULT_WINDOW);
        tuner.feed(&samples);
        let r = tuner.take_readings().next().expect("reading");
        assert_eq!(r.label, "A4");
        assert!(r.cents.abs() < 5.0);
    }

    // ---- Gating tests ----

    #[test]
    fn tuner_strings_ignores_silence() {
        let mut tuner = Tuner::new(TunerMode::Strings(Tuning::standard_ukulele()), 44100);
        let silence = vec![0.0_f32; Tuner::DEFAULT_WINDOW];
        tuner.feed(&silence);
        assert_eq!(tuner.take_readings().count(), 0);
    }

    #[test]
    fn tuner_chromatic_ignores_silence() {
        let mut tuner = Tuner::new(TunerMode::Chromatic, 44100);
        let silence = vec![0.0_f32; Tuner::DEFAULT_WINDOW];
        tuner.feed(&silence);
        assert_eq!(tuner.take_readings().count(), 0);
    }

    #[test]
    fn tuner_strings_rejects_mains_hum_far_from_any_string() {
        // 50 Hz mains hum is many octaves below every uke string — must not
        // pollute the C4 row (or any row) just because it's the "least bad" match.
        let mut tuner = Tuner::new(TunerMode::Strings(Tuning::standard_ukulele()), 44100);
        let hum = sine(Frequency(50.0), 44100, Tuner::DEFAULT_WINDOW);
        tuner.feed(&hum);
        assert_eq!(tuner.take_readings().count(), 0);
    }

    #[test]
    fn tuner_strings_accepts_wildly_mistuned_string_within_tritone() {
        // Uke A string sitting a whole tone flat (G4 ≈ 392 Hz). That's exactly
        // the reentrant g4 string — should match g4, not be rejected.
        let mut tuner = Tuner::new(TunerMode::Strings(Tuning::standard_ukulele()), 44100);
        let flat_a = sine(Frequency(392.0), 44100, Tuner::DEFAULT_WINDOW);
        tuner.feed(&flat_a);
        let r = tuner.take_readings().next().expect("should emit");
        assert!(r.cents.abs() < 5.0);
    }

    #[test]
    fn tuner_chromatic_accepts_low_frequency_loud_signal() {
        // Chromatic mode has no string-distance gate — a loud 50 Hz sine is
        // legitimately G1 and should be reported as such.
        let mut tuner = Tuner::new(TunerMode::Chromatic, 44100);
        let low = sine(Frequency(50.0), 44100, Tuner::DEFAULT_WINDOW * 2);
        tuner.feed(&low);
        assert!(tuner.take_readings().count() >= 1);
    }

    #[test]
    fn tuner_chromatic_labels_each_midi_note_correctly() {
        // Fresh tuner per iteration so previous MIDI's residue can't bleed into
        // the next analysis window — that's correct streaming behaviour but wrong
        // for a per-note unit test.
        for midi in 48..=72 {
            let mut tuner = Tuner::new(TunerMode::Chromatic, 44100);
            let expected = MidiNote(midi).to_frequency();
            let samples = sine(expected, 44100, Tuner::DEFAULT_WINDOW);
            tuner.feed(&samples);
            let r = tuner.take_readings().next().unwrap_or_else(|| {
                panic!("tuner failed on MIDI {midi} ({})", MidiNote(midi).name())
            });
            assert_eq!(r.label, MidiNote(midi).name(), "MIDI {midi} mislabeled");
            assert!(r.cents.abs() < 5.0);
        }
    }
}
