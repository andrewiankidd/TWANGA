//! Pure pitch detection over `&[f32]`. No IO, no async.
//!
//! `Yin` is the underlying detector (allocation-free after first call).
//! `Tuner` layers buffer management + a configurable "what does this
//! frequency mean?" lookup on top — see [`TunerMode`]. Same code path for
//! the CLI's single-line chromatic tuner and the multi-string display.

use twanga_core::{Frequency, MidiNote, Tuning};

pub mod calibration;
pub mod onset;

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
            Self::Strings(tuning) => tuning
                .nearest_string(freq)
                .map(|(s, cents)| (s.name.clone(), s.open.to_frequency(), cents)),
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
    /// True iff this reading came from a YIN window that started at
    /// a fresh onset — i.e., this is the first window's reading
    /// since the most recent note attack. Wait-mode uses this to
    /// distinguish "user just played a new note" from "previous
    /// note is sustaining and YIN keeps reading the same pitch."
    /// False on the default detection path (chromatic / strings
    /// without an onset reset). Subsequent readings from the same
    /// onset session also come through as false — each onset
    /// produces exactly one fresh-window reading.
    pub from_onset_window: bool,
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
    /// Runtime-configurable silence threshold. Default is
    /// [`DEFAULT_SILENCE_RMS`]; can be raised to suppress more
    /// ambient noise or lowered to catch quieter plucks. Set via
    /// [`set_silence_rms`](Self::set_silence_rms).
    silence_rms: f32,
    /// In-progress noise-floor calibration, if any. See
    /// [`Self::start_noise_calibration`]. Calibration runs as a tap on
    /// the incoming sample stream — pitch detection continues in
    /// parallel during the window, so the user isn't gated out while
    /// the floor is being measured.
    calibration: Option<NoiseCalibration>,
    /// Most recently completed calibration result, for UI display.
    /// Cleared by a fresh [`Self::start_noise_calibration`] call so the
    /// "result" surface only ever shows the latest run's numbers.
    last_calibration: Option<CalibrationResult>,
    /// Energy-derivative onset detector running alongside YIN.
    /// When it fires, [`Self::feed`] clears the sample buffer so the
    /// next YIN window starts at fresh post-attack samples — and
    /// the first reading produced after the onset is tagged
    /// [`TunerReading::from_onset_window`] so wait-mode consumers
    /// can distinguish "the user just played" from "the previous
    /// note is still sustaining."
    onset_detector: onset::OnsetDetector,
    /// True after an onset fires, cleared when the next reading is
    /// produced. Implements the "each onset = one fresh-window
    /// reading" semantic that wait-mode relies on.
    onset_pending: bool,
}

/// Internal state for an in-progress noise-floor calibration. The Tuner
/// taps incoming samples into fixed-size chunks (~100 ms each), computes
/// the RMS of each filled chunk, and once we have `chunks_needed` of them
/// takes the 10th-percentile as the noise floor — robust to the user
/// briefly playing during the calibration window because only the quiet
/// gaps contribute to the low-percentile bucket.
struct NoiseCalibration {
    chunk_size: usize,
    chunks_needed: usize,
    chunk_buffer: Vec<f32>,
    chunk_rms: Vec<f32>,
}

/// Progress snapshot of an in-progress calibration. The UI surfaces this
/// so the user can see a countdown / progress bar during the silent
/// listening window.
#[derive(Debug, Clone, Copy)]
pub struct CalibrationProgress {
    /// Whole-sample count consumed so far.
    pub samples_collected: usize,
    /// Total samples the calibration window needs.
    pub samples_total: usize,
}

impl CalibrationProgress {
    /// Fraction of the window completed, in `[0.0, 1.0]`.
    pub fn fraction(&self) -> f32 {
        if self.samples_total == 0 {
            1.0
        } else {
            (self.samples_collected as f32 / self.samples_total as f32).clamp(0.0, 1.0)
        }
    }
}

/// Outcome of a completed noise-floor calibration. `floor_rms` is the
/// measured noise level (10th-percentile of the chunk RMSes); `threshold_rms`
/// is what we set the silence gate to — the floor multiplied by a safety
/// factor and clamped into a sane range.
#[derive(Debug, Clone, Copy)]
pub struct CalibrationResult {
    pub floor_rms: f32,
    pub threshold_rms: f32,
}

impl Tuner {
    pub const DEFAULT_WINDOW: usize = 8192;
    pub const DEFAULT_SLIDE_BY: usize = 4096;

    /// Default silence threshold. Window RMS below this is treated
    /// as silence — no detection attempted. 0.005 catches a quiet
    /// room while staying well below any plucked-string note. Now a
    /// runtime-tunable field (see [`Self::silence_rms`] /
    /// [`Self::set_silence_rms`]); the constant stays as the
    /// out-of-box default so callers that don't care don't need to
    /// touch it.
    pub const DEFAULT_SILENCE_RMS: f32 = 0.005;

    /// Backwards-compatible alias. New code should use
    /// [`Self::DEFAULT_SILENCE_RMS`] or the instance method
    /// [`Self::silence_rms`].
    pub const SILENCE_RMS: f32 = Self::DEFAULT_SILENCE_RMS;

    /// In `Strings` mode, detections this far from the nearest open string are
    /// treated as noise (mains hum, cable EMI, room sounds) and dropped. A
    /// tritone is enough headroom that a string can be wildly mis-tuned and
    /// still register against its intended target — anything past it is not a
    /// plausible attempt at any string in the tuning.
    pub const MAX_STRING_DISTANCE_CENTS: f32 = 700.0;

    /// Calibration chunk duration. 100 ms is short enough that the user
    /// briefly playing through a chunk doesn't poison the floor measurement
    /// (other chunks land in the quiet gaps and dominate the 10th percentile),
    /// and long enough that one chunk's RMS is a stable noise estimate.
    pub const CALIBRATION_CHUNK_MS: u32 = 100;

    /// Safety multiplier applied to the measured floor when picking the
    /// silence threshold. 3× ≈ +10 dB headroom — enough that a noisy chunk
    /// doesn't trip the gate but tight enough to catch quiet plucks on a
    /// clean DI input. Clamps still apply in [`Self::DEFAULT_FLOOR_MIN_RMS`,
    /// [`Self::DEFAULT_FLOOR_MAX_RMS`].
    pub const CALIBRATION_FLOOR_MULTIPLIER: f32 = 3.0;

    /// Floor of the clamp applied to a calibrated threshold. A pristine
    /// direct-in can measure a floor near 0; without this clamp the gate
    /// would be effectively disabled and stray subsonic transients would
    /// fire YIN unnecessarily.
    pub const DEFAULT_FLOOR_MIN_RMS: f32 = 0.0005;

    /// Ceiling of the clamp applied to a calibrated threshold. A loud
    /// environment (live mic, fan, etc.) could measure a high floor;
    /// capping it here keeps the gate from becoming so insensitive that
    /// quiet but legitimate plucks get dropped.
    pub const DEFAULT_FLOOR_MAX_RMS: f32 = 0.02;

    pub fn new(mode: TunerMode, sample_rate: u32) -> Self {
        Self {
            yin: Yin::new(0.15),
            mode,
            sample_rate,
            window_size: Self::DEFAULT_WINDOW,
            slide_by: Self::DEFAULT_SLIDE_BY,
            buffer: Vec::with_capacity(Self::DEFAULT_WINDOW * 2),
            readings: Vec::new(),
            silence_rms: Self::DEFAULT_SILENCE_RMS,
            calibration: None,
            last_calibration: None,
            onset_detector: onset::OnsetDetector::new(sample_rate),
            onset_pending: false,
        }
    }

    /// Read the current silence threshold (linear amplitude RMS).
    pub fn silence_rms(&self) -> f32 {
        self.silence_rms
    }

    /// Update the silence threshold at runtime. Clamped to a sane
    /// range — 0 disables the gate entirely (detection on every
    /// window, including pure silence) and 1.0 is the maximum
    /// possible RMS, so values outside that range have no useful
    /// meaning. Cancels any in-progress noise calibration: when the
    /// user reaches for the threshold control they're taking manual
    /// authority, and finishing a calibration that's about to overwrite
    /// their value would be a confusing "slider jumped under my finger"
    /// experience.
    pub fn set_silence_rms(&mut self, rms: f32) {
        self.silence_rms = rms.clamp(0.0, 1.0);
        // Manual override clears both the in-progress calibration AND the
        // most recent result — the UI uses `last_calibration().is_some()`
        // to decide whether to show the "auto" badge near the slider, so
        // leaving it populated after the user overrode would mislabel
        // their manual value as auto-calibrated.
        self.calibration = None;
        self.last_calibration = None;
    }

    /// Begin a fresh noise-floor calibration. Subsequent [`Self::feed`]
    /// calls accumulate samples for `window_seconds`; once full, the
    /// 10th-percentile chunk RMS is used as the floor and
    /// [`Self::silence_rms`] is set to `floor * CALIBRATION_FLOOR_MULTIPLIER`
    /// (clamped into [`DEFAULT_FLOOR_MIN_RMS`, [`DEFAULT_FLOOR_MAX_RMS`]).
    ///
    /// Pitch detection continues normally during the calibration window —
    /// users can play through it and still see readings, the calibration
    /// just runs as a tap on the same sample stream.
    ///
    /// Calling this while a calibration is already in progress restarts
    /// the window. Calling [`Self::set_silence_rms`] cancels it.
    pub fn start_noise_calibration(&mut self, window_seconds: f32) {
        let chunk_size =
            ((self.sample_rate as u64 * Self::CALIBRATION_CHUNK_MS as u64) / 1000) as usize;
        let chunk_size = chunk_size.max(1);
        let total_samples = (self.sample_rate as f32 * window_seconds).round() as usize;
        let chunks_needed = (total_samples / chunk_size).max(1);
        self.calibration = Some(NoiseCalibration {
            chunk_size,
            chunks_needed,
            chunk_buffer: Vec::with_capacity(chunk_size),
            chunk_rms: Vec::with_capacity(chunks_needed),
        });
        self.last_calibration = None;
    }

    /// Cancel an in-progress calibration without changing the threshold.
    /// Useful when the audio session is tearing down (device change,
    /// shutdown) and we don't want a half-finished calibration to fire
    /// its "complete" side-effects after the fact.
    pub fn cancel_noise_calibration(&mut self) {
        self.calibration = None;
    }

    /// Progress of an in-progress calibration, or `None` if no calibration
    /// is currently running. UI code uses this to render a countdown or
    /// progress bar during the listening window.
    pub fn calibration_progress(&self) -> Option<CalibrationProgress> {
        let c = self.calibration.as_ref()?;
        let collected = c.chunk_rms.len() * c.chunk_size + c.chunk_buffer.len();
        let total = c.chunks_needed * c.chunk_size;
        Some(CalibrationProgress {
            samples_collected: collected,
            samples_total: total,
        })
    }

    /// Result of the most recently completed calibration, if any. Cleared
    /// when a new calibration is started. UI code reads this to show the
    /// measured floor + applied threshold after the window completes.
    pub fn last_calibration(&self) -> Option<CalibrationResult> {
        self.last_calibration
    }

    pub fn mode(&self) -> &TunerMode {
        &self.mode
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Approximate delay between when audio for a pluck begins
    /// entering the buffer and when YIN can produce a pitch reading
    /// for that pluck. Equals `window_size / sample_rate` — the
    /// reading isn't available until the analysis window is full.
    ///
    /// The onset-detector chunk granularity adds a separate ~5 ms
    /// of attack-to-buffer jitter on top, which is below the
    /// resolution of the scoring tolerance window and not folded in
    /// here. Callers that timestamp `from_onset_window` readings
    /// against an external clock should subtract this value to get
    /// the moment the pluck *started*, not when its pitch was
    /// confirmed.
    pub fn window_latency_ms(&self) -> u32 {
        (self.window_size as u64 * 1000 / self.sample_rate as u64) as u32
    }

    /// Push mono samples. Runs YIN on every completed window (subject to a
    /// silence gate) and queues a `TunerReading` per accepted detection.
    /// If a noise-floor calibration is in progress, also taps the samples
    /// into 100 ms RMS chunks until the window fills — at which point the
    /// silence threshold is updated and the calibration completes.
    pub fn feed(&mut self, samples: &[f32]) {
        if self.calibration.is_some() {
            self.advance_calibration(samples);
        }

        // Run onset detection on the new samples BEFORE they enter
        // the YIN buffer. If an attack fires, clear the buffer so
        // the next YIN window starts at fresh post-attack audio —
        // this prevents the "previous note's sustain bleeds into
        // the next column's analysis window" failure mode that
        // afflicts fast passages today. See `crates/twanga-dsp/src/onset.rs`
        // and docs/plans/onset-detection.md for the why.
        if self.onset_detector.feed(samples) {
            self.buffer.clear();
            self.onset_pending = true;
        }

        self.buffer.extend_from_slice(samples);
        while self.buffer.len() >= self.window_size {
            let window = &self.buffer[..self.window_size];

            // Silence gate — don't try to find pitch in cable hum or room noise.
            if window_rms(window) < self.silence_rms {
                self.buffer.drain(..self.slide_by);
                continue;
            }

            if let Some(freq) = self.yin.detect(window, self.sample_rate) {
                if let Some((label, target, cents)) = self.mode.lookup(freq) {
                    let too_far_from_any_string = matches!(self.mode, TunerMode::Strings(_))
                        && cents.abs() > Self::MAX_STRING_DISTANCE_CENTS;
                    if !too_far_from_any_string {
                        // Tag the first reading after an onset; the
                        // pending flag clears so subsequent readings
                        // from the same onset session (overlapping
                        // YIN windows on the same sustain) come
                        // through as plain detections. Wait-mode
                        // matches only the tagged one, giving "each
                        // onset = one match opportunity."
                        let from_onset = self.onset_pending;
                        self.onset_pending = false;
                        self.readings.push(TunerReading {
                            detected: freq,
                            label,
                            target,
                            cents,
                            from_onset_window: from_onset,
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

    /// Reset transient pitch-detection state so the next [`Self::feed`]
    /// call starts from a clean slate. Specifically drops the YIN
    /// sample buffer, drains any queued readings, and clears
    /// `onset_pending` — so no stale pre-wait audio can satisfy the
    /// first column's wait-mode match. Calibration state, the silence
    /// threshold, and the onset detector's baseline are intentionally
    /// preserved (those are slow-adapting and shouldn't be discarded
    /// just because the consumer paused for a moment).
    pub fn clear_for_wait(&mut self) {
        self.buffer.clear();
        self.readings.clear();
        self.onset_pending = false;
    }

    /// Consume new samples into the in-progress calibration's chunk
    /// buffer. Each time a chunk fills, compute its RMS and push it onto
    /// the chunk-RMS list; once the list is full, finalise.
    fn advance_calibration(&mut self, samples: &[f32]) {
        // Take a mutable borrow we can finish-and-drop deterministically —
        // we can't hold `&mut self.calibration` while we also mutate other
        // fields (`silence_rms`, `last_calibration`) in `finalize`, so move
        // the state out, work on it, and put it back only if not done.
        let Some(mut cal) = self.calibration.take() else {
            return;
        };
        for &s in samples {
            cal.chunk_buffer.push(s);
            if cal.chunk_buffer.len() >= cal.chunk_size {
                let rms = window_rms(&cal.chunk_buffer);
                cal.chunk_buffer.clear();
                cal.chunk_rms.push(rms);
                if cal.chunk_rms.len() >= cal.chunks_needed {
                    self.finalize_calibration(&cal.chunk_rms);
                    return; // cal goes out of scope; calibration field stays None
                }
            }
        }
        self.calibration = Some(cal);
    }

    /// Sort the collected chunk RMSes, take the 10th percentile as the
    /// noise floor, multiply by the safety factor, clamp into the sane
    /// range, and apply as the new silence threshold. Records the
    /// `CalibrationResult` so UI code can display the numbers.
    fn finalize_calibration(&mut self, chunk_rms: &[f32]) {
        let mut sorted: Vec<f32> = chunk_rms.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = ((sorted.len() as f32 * 0.10).floor() as usize).min(sorted.len() - 1);
        let floor = sorted[idx];
        let threshold = (floor * Self::CALIBRATION_FLOOR_MULTIPLIER)
            .clamp(Self::DEFAULT_FLOOR_MIN_RMS, Self::DEFAULT_FLOOR_MAX_RMS);
        self.silence_rms = threshold;
        self.last_calibration = Some(CalibrationResult {
            floor_rms: floor,
            threshold_rms: threshold,
        });
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
    use std::f32::consts::PI;
    use twanga_core::MidiNote;
    use twanga_synth::{harmonic_stack, mix_into, sine, white_noise};

    fn cents(detected: Frequency, expected: Frequency) -> f32 {
        1200.0 * (detected.hz() / expected.hz()).log2()
    }

    /// Generate a plucked-string-shaped signal: sharp attack followed by
    /// exponential decay. Used by the onset-integration tests below to
    /// produce realistic input for the Tuner's onset-detection +
    /// fresh-window-pitch-detection pipeline.
    fn pluck_signal(sr: u32, n: usize, hz: f32, peak: f32, decay_secs: f32) -> Vec<f32> {
        let decay_samples = decay_secs * sr as f32;
        (0..n)
            .map(|i| {
                let env = peak * (-(i as f32) / decay_samples).exp();
                env * (2.0 * PI * hz * i as f32 / sr as f32).sin()
            })
            .collect()
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
        assert!(yin.detect(&[0.0_f32; 2], 44100).is_none());
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
        use rustfft::{FftPlanner, num_complex::Complex};

        let n = samples.len();
        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n);

        let mut buffer: Vec<Complex<f32>> = samples
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                let w = 0.5 * (1.0 - (std::f32::consts::TAU * i as f32 / (n - 1) as f32).cos());
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
            assert!(
                diff_cents.abs() < 10.0,
                "MIDI {midi}: {diff_cents:+.2} cents"
            );
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
    fn tuner_tags_first_post_onset_reading_then_clears_flag() {
        // Feed a pluck-shaped signal: sharp attack + decay. The
        // attack fires an onset → the FIRST reading is tagged
        // `from_onset_window = true`. Subsequent readings produced
        // from later windows of the same sustain are tagged false
        // — each onset gives exactly one fresh-window reading,
        // which is the wait-mode "one match per pluck" contract.
        const SR: u32 = 48_000;
        let mut tuner = Tuner::new(TunerMode::Chromatic, SR);
        // Long enough signal to produce multiple YIN windows.
        // DEFAULT_WINDOW + 2 * DEFAULT_SLIDE_BY = 16384 samples ⇒
        // 3 windows.
        let n = Tuner::DEFAULT_WINDOW + 2 * Tuner::DEFAULT_SLIDE_BY;
        let signal = pluck_signal(SR, n, 440.0, 0.5, 0.5);
        tuner.feed(&signal);
        let readings: Vec<TunerReading> = tuner.take_readings().collect();
        assert!(!readings.is_empty(), "expected at least one reading");
        assert!(
            readings[0].from_onset_window,
            "first reading after pluck onset should be from_onset_window=true"
        );
        for (i, r) in readings.iter().enumerate().skip(1) {
            assert!(
                !r.from_onset_window,
                "reading {i} should be from_onset_window=false (no fresh onset)"
            );
        }
    }

    #[test]
    fn tuner_clear_for_wait_drops_buffer_readings_and_pending_onset() {
        // The contract the CLI/web wait-mode entry points rely on:
        // after `clear_for_wait`, no reading queued from before the
        // call can come back, and no `from_onset_window` flag from a
        // pre-clear onset can leak into the next reading. Pin both.
        const SR: u32 = 48_000;
        let mut tuner = Tuner::new(TunerMode::Chromatic, SR);

        // Pluck → generates queued readings + sets onset_pending.
        let signal = pluck_signal(SR, Tuner::DEFAULT_WINDOW * 2, 440.0, 0.5, 0.5);
        tuner.feed(&signal);
        assert!(
            tuner.take_readings().count() > 0,
            "sanity: pluck should produce readings before the test does anything"
        );
        // Re-feed half a window so onset_pending could still be set
        // (no YIN window completed since the last take_readings to
        // consume it).
        tuner.feed(&pluck_signal(
            SR,
            Tuner::DEFAULT_WINDOW / 2,
            440.0,
            0.5,
            0.5,
        ));

        tuner.clear_for_wait();

        // After clear: silence in, no readings out (buffer empty, no
        // sustained-tail leak).
        tuner.feed(&vec![0.0_f32; Tuner::DEFAULT_WINDOW]);
        let after_clear: Vec<TunerReading> = tuner.take_readings().collect();
        assert_eq!(
            after_clear.len(),
            0,
            "silence after clear_for_wait shouldn't produce readings — got {after_clear:?}"
        );
    }

    #[test]
    fn tuner_sustained_tail_doesnt_fire_a_second_from_onset_reading() {
        // The headline wait-mode failure mode the onset gate is
        // designed to fix: a sustained note that decays slowly keeps
        // producing YIN readings as the window slides. Only the
        // FIRST should be `from_onset_window = true`; the rest are
        // sustain. Wait-mode treats them as such and won't accept
        // them for the NEXT column.
        const SR: u32 = 48_000;
        let mut tuner = Tuner::new(TunerMode::Chromatic, SR);
        // 6× window of slow-decay pluck — produces ~10 overlapping
        // YIN readings on the sustained tail.
        let signal = pluck_signal(SR, Tuner::DEFAULT_WINDOW * 6, 440.0, 0.6, 1.0);
        tuner.feed(&signal);
        let readings: Vec<TunerReading> = tuner.take_readings().collect();
        let onset_tagged = readings.iter().filter(|r| r.from_onset_window).count();
        assert_eq!(
            onset_tagged,
            1,
            "exactly one reading should be from_onset_window=true across a long sustain, \
             got {onset_tagged} of {total}",
            total = readings.len()
        );
    }

    #[test]
    fn tuner_two_separated_plucks_produce_two_from_onset_readings() {
        // Two distinct attacks separated by enough silence that
        // the second is a clean onset. Each should yield one
        // `from_onset_window = true` reading; sustains in between
        // tagged false. Verifies wait-mode can match TWO consecutive
        // columns without spurious matches in the gap.
        //
        // Fed in small chunks (mirroring worklet / cpal feed sizes)
        // so each onset is processed in its own `feed()` call —
        // matches real-world usage where two onsets never land in
        // the same buffer (refractory is ~50 ms, typical feed is
        // ~2-20 ms). A single feed containing both onsets would
        // collapse them at the Tuner layer (one buffer-clear per
        // feed call), which is a known limitation documented in
        // `docs/plans/onset-detection.md`.
        const SR: u32 = 48_000;
        let mut tuner = Tuner::new(TunerMode::Chromatic, SR);
        let mut signal = pluck_signal(SR, Tuner::DEFAULT_WINDOW * 2, 440.0, 0.5, 0.3);
        signal.extend(vec![0.0_f32; SR as usize / 4]); // 250 ms silence
        signal.extend(pluck_signal(SR, Tuner::DEFAULT_WINDOW * 2, 660.0, 0.5, 0.3));
        for chunk in signal.chunks(256) {
            tuner.feed(chunk);
        }
        let readings: Vec<TunerReading> = tuner.take_readings().collect();
        let onset_tagged = readings.iter().filter(|r| r.from_onset_window).count();
        assert_eq!(
            onset_tagged,
            2,
            "two separated plucks should produce 2 onset-tagged readings, \
             got {onset_tagged} of {total}",
            total = readings.len()
        );
    }

    #[test]
    fn tuner_legato_no_attack_produces_no_extra_from_onset_readings() {
        // Documented v1 limitation: a hammer-on (no fresh pluck,
        // just a pitch change inside a sustain envelope) produces
        // no new onset, so wait-mode can't advance on it. Pin the
        // behaviour so any future fix is a deliberate change rather
        // than an accidental regression on legato fixtures.
        const SR: u32 = 48_000;
        let mut tuner = Tuner::new(TunerMode::Chromatic, SR);
        // Single pluck envelope, but pitch shifts halfway through —
        // synthetic "hammer-on" approximation.
        let n = Tuner::DEFAULT_WINDOW * 4;
        let decay_samples = 1.0 * SR as f32;
        let signal: Vec<f32> = (0..n)
            .map(|i| {
                let env = 0.5 * (-(i as f32) / decay_samples).exp();
                let hz = if i < n / 2 { 440.0 } else { 494.0 }; // A4 -> B4
                env * (2.0 * std::f32::consts::PI * hz * i as f32 / SR as f32).sin()
            })
            .collect();
        tuner.feed(&signal);
        let readings: Vec<TunerReading> = tuner.take_readings().collect();
        let onset_tagged = readings.iter().filter(|r| r.from_onset_window).count();
        assert_eq!(
            onset_tagged, 1,
            "legato/hammer-on (single attack envelope) should produce 1 onset-tagged \
             reading, got {onset_tagged}"
        );
    }

    #[test]
    fn tuner_from_onset_default_false_on_first_reading_in_silence_session() {
        // Cold-start a Tuner and feed it pure silence-then-tiny-
        // noise that's BELOW the onset min_delta_rms floor. No
        // onset fires → no reading carries the from_onset flag.
        // This pins that the flag isn't accidentally defaulted to
        // true at construction.
        const SR: u32 = 48_000;
        let mut tuner = Tuner::new(TunerMode::Chromatic, SR);
        // Quiet sustained sine at amplitude well below min_delta_rms.
        // RMS = 0.001 * sqrt(2) / 2 ≈ 0.0007 — below the 0.005 floor.
        let n = Tuner::DEFAULT_WINDOW + Tuner::DEFAULT_SLIDE_BY;
        let signal: Vec<f32> = (0..n)
            .map(|i| 0.001 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SR as f32).sin())
            .collect();
        tuner.feed(&signal);
        let readings: Vec<TunerReading> = tuner.take_readings().collect();
        // The silence gate may suppress these readings entirely;
        // either way, none should be onset-tagged.
        let onset_tagged = readings.iter().filter(|r| r.from_onset_window).count();
        assert_eq!(
            onset_tagged, 0,
            "no onset → no reading should carry from_onset_window=true"
        );
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

    // ---- Noise-calibration tests ----

    fn quiet_noise(sample_count: usize, amplitude: f32, seed: u64) -> Vec<f32> {
        // Deterministic low-amplitude pseudo-random samples for testing the
        // calibration's noise-floor measurement. White noise from
        // `twanga_synth::white_noise` would also work; rolling our own
        // here keeps the test self-contained and the magnitude controllable.
        let mut state = seed;
        (0..sample_count)
            .map(|_| {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                let r = ((state >> 32) as i32) as f32 / i32::MAX as f32;
                r * amplitude
            })
            .collect()
    }

    #[test]
    fn calibration_status_none_when_not_started() {
        let tuner = Tuner::new(TunerMode::Chromatic, 44100);
        assert!(tuner.calibration_progress().is_none());
        assert!(tuner.last_calibration().is_none());
    }

    #[test]
    fn calibration_progresses_through_feed() {
        let mut tuner = Tuner::new(TunerMode::Chromatic, 44100);
        tuner.start_noise_calibration(0.5); // half-second window
        let p = tuner
            .calibration_progress()
            .expect("calibration should be active");
        assert_eq!(p.samples_collected, 0);
        assert!(p.samples_total > 0);
        // Feed about a third of the window — progress should reflect that.
        let third = (44100 / 6) as usize;
        tuner.feed(&quiet_noise(third, 0.001, 1));
        let p = tuner.calibration_progress().expect("still active");
        assert!(p.samples_collected >= third - tuner.sample_rate() as usize / 10);
        assert!(tuner.last_calibration().is_none());
    }

    #[test]
    fn calibration_completes_and_sets_threshold_from_percentile() {
        let mut tuner = Tuner::new(TunerMode::Chromatic, 44100);
        let window_secs = 1.0;
        tuner.start_noise_calibration(window_secs);

        // Stream a full window's worth of quiet noise (~0.002 amplitude).
        // The chunk RMS values will hover around amplitude / sqrt(3) for a
        // uniform distribution — well above the floor clamp but well below
        // the ceiling clamp.
        let samples_needed = (44100.0 * window_secs).ceil() as usize + 4410;
        tuner.feed(&quiet_noise(samples_needed, 0.002, 42));

        assert!(
            tuner.calibration_progress().is_none(),
            "calibration should have completed after a full window"
        );
        let result = tuner
            .last_calibration()
            .expect("calibration should have produced a result");
        // Threshold must be > floor (the multiplier kicks in unless clamps
        // bite) and must equal the live `silence_rms()` (the side-effect
        // of calibration is to set the gate).
        assert!(result.threshold_rms > result.floor_rms);
        assert!((tuner.silence_rms() - result.threshold_rms).abs() < 1e-7);
        // Threshold respects the configured clamp range.
        assert!(result.threshold_rms >= Tuner::DEFAULT_FLOOR_MIN_RMS);
        assert!(result.threshold_rms <= Tuner::DEFAULT_FLOOR_MAX_RMS);
    }

    #[test]
    fn calibration_clamps_threshold_to_min_for_pristine_silence() {
        // A truly silent input (all-zero samples) measures a floor of 0.
        // Without clamping, the threshold would also be 0 — effectively
        // disabling the gate, which we never want. The MIN clamp keeps a
        // sane floor even on pristine DI inputs.
        let mut tuner = Tuner::new(TunerMode::Chromatic, 44100);
        tuner.start_noise_calibration(1.0);
        let samples_needed = (44100.0_f32 * 1.0).ceil() as usize + 4410;
        tuner.feed(&vec![0.0_f32; samples_needed]);
        let result = tuner.last_calibration().expect("should complete");
        assert!((result.threshold_rms - Tuner::DEFAULT_FLOOR_MIN_RMS).abs() < 1e-7);
    }

    #[test]
    fn calibration_clamps_threshold_to_max_for_loud_environment() {
        // A loud noise floor (say a constant 0.1 amplitude) would push
        // the multiplied threshold past 0.02 — the ceiling clamp keeps
        // the gate from becoming so insensitive that real plucks are
        // dropped. Confirms the upper bound is enforced.
        let mut tuner = Tuner::new(TunerMode::Chromatic, 44100);
        tuner.start_noise_calibration(1.0);
        let samples_needed = (44100.0_f32 * 1.0).ceil() as usize + 4410;
        tuner.feed(&quiet_noise(samples_needed, 0.1, 7));
        let result = tuner.last_calibration().expect("should complete");
        assert!((result.threshold_rms - Tuner::DEFAULT_FLOOR_MAX_RMS).abs() < 1e-7);
    }

    #[test]
    fn manual_set_silence_rms_cancels_in_progress_calibration() {
        // User reaching for the threshold control mid-calibration is an
        // override — finishing the calibration after that would overwrite
        // their value and look like the slider jumped under their finger.
        let mut tuner = Tuner::new(TunerMode::Chromatic, 44100);
        tuner.start_noise_calibration(3.0);
        // Feed a fraction of the window so calibration is in progress.
        tuner.feed(&quiet_noise(4410 * 5, 0.001, 99));
        assert!(tuner.calibration_progress().is_some());

        tuner.set_silence_rms(0.01);
        assert!(tuner.calibration_progress().is_none());
        assert!(tuner.last_calibration().is_none());
        // The user's value sticks.
        assert!((tuner.silence_rms() - 0.01).abs() < 1e-7);
    }

    #[test]
    fn manual_set_silence_rms_after_complete_clears_last_calibration() {
        // After cal completes, the UI uses `last_calibration().is_some()`
        // to show an "auto" badge. If the user manually adjusts the
        // threshold, that badge needs to go away — confirm `set_silence_rms`
        // clears the post-cal result too.
        let mut tuner = Tuner::new(TunerMode::Chromatic, 44100);
        tuner.start_noise_calibration(0.5);
        tuner.feed(&quiet_noise(44100 / 2 + 4410, 0.001, 5));
        assert!(tuner.last_calibration().is_some());

        tuner.set_silence_rms(0.008);
        assert!(tuner.last_calibration().is_none());
    }

    #[test]
    fn calibration_does_not_block_pitch_detection_during_window() {
        // Calibration runs as a tap on the input stream — pitch detection
        // continues in parallel. A clean A4 fed during the calibration
        // window should still produce a reading (assuming amplitude clears
        // the current silence gate, which starts at the default 0.005).
        let mut tuner = Tuner::new(TunerMode::Strings(Tuning::standard_ukulele()), 44100);
        tuner.start_noise_calibration(2.0);
        // Strong A4 — well above the default 0.005 threshold during cal.
        let buf = sine(Frequency::A4, 44100, Tuner::DEFAULT_WINDOW);
        tuner.feed(&buf);
        let readings = tuner.take_readings().count();
        assert!(readings > 0, "pitch detection should not be paused by cal");
    }

    #[test]
    fn cancel_noise_calibration_drops_in_flight_state() {
        let mut tuner = Tuner::new(TunerMode::Chromatic, 44100);
        tuner.start_noise_calibration(3.0);
        tuner.feed(&quiet_noise(4410 * 3, 0.001, 11));
        assert!(tuner.calibration_progress().is_some());

        tuner.cancel_noise_calibration();
        assert!(tuner.calibration_progress().is_none());
        // Unlike `set_silence_rms`, cancel doesn't clear the most recent
        // result — there isn't one yet, but if there had been (sequential
        // start → complete → start → cancel scenario), we'd want the
        // earlier complete to remain visible. Confirm that semantic.
        assert!(tuner.last_calibration().is_none());
    }

    #[test]
    fn calibration_tenth_percentile_picks_quiet_samples_when_user_plays_through() {
        // The 10th-percentile choice is specifically robust to the user
        // briefly playing through the calibration window — the loud
        // chunks land in the upper percentiles, leaving the quiet gaps
        // to dominate the floor estimate. Simulate this by mixing a
        // short loud burst into otherwise-quiet noise.
        let mut tuner = Tuner::new(TunerMode::Chromatic, 44100);
        tuner.start_noise_calibration(1.0);

        // First 200 ms: quiet noise (≈ 0.001 amplitude → tiny chunk RMS).
        // Next 200 ms: a loud A4 burst (amplitude 0.3, way above default
        // threshold). Then 600 ms more quiet noise. The 30 100-ms chunks
        // break down roughly 2 loud + 28 quiet — the 10th percentile
        // (3rd from bottom when sorted) lands in the quiet bucket.
        let mut samples = quiet_noise(44100 / 5, 0.001, 17);
        samples.extend(
            sine(Frequency::A4, 44100, 44100 / 5)
                .iter()
                .map(|s| s * 0.3),
        );
        samples.extend(quiet_noise(44100 * 3 / 5 + 4410, 0.001, 23));
        tuner.feed(&samples);

        let result = tuner
            .last_calibration()
            .expect("calibration should complete");
        // The measured floor should reflect the quiet bucket, NOT the
        // loud burst — i.e. small RMS. If we'd accidentally used the
        // mean or a high percentile, this would be far higher.
        assert!(
            result.floor_rms < 0.005,
            "10th percentile should ignore the loud burst, got floor {}",
            result.floor_rms
        );
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
