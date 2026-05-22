//! Energy-derivative onset detector. Watches the incoming sample
//! stream in short fixed-size chunks (~5 ms each) and fires when
//! the current chunk's RMS rises sharply above a slowly-following
//! baseline. Tuned for plucked-string attacks — sharp transients
//! against an exponential-decay sustain.
//!
//! # Why this exists
//!
//! The [`Tuner`](crate::Tuner)'s pitch detection runs on 8192-sample
//! windows (~170 ms at 48 kHz). On fast passages the next pluck
//! lands inside the previous note's analysis window, YIN sees a
//! mixture of decay + new attack, and the resulting pitch is
//! ambiguous — often octave-jumps, sometimes still reads the
//! previous note. The fast-passage failure of [wait-mode
//! playback](../../docs/plans/onset-detection.md) traces back to
//! this.
//!
//! This module runs *alongside* YIN, not instead of it. The onset
//! detector says "a new note just attacked, here." YIN runs on a
//! fresh post-attack window and says "and the pitch is X." The two
//! detections decouple "when did a note happen" from "what pitch
//! was it" — the architectural fix for the fast-passage problem
//! that pitch-only detection can't handle.
//!
//! # Algorithm
//!
//! Per chunk (256 samples = ~5.3 ms at 48 kHz):
//!
//! 1. Compute the chunk's RMS.
//! 2. Compare against an exponentially-smoothed baseline. If the
//!    current RMS exceeds `baseline * ratio_threshold` AND the
//!    absolute jump (`current - baseline`) clears `min_delta_rms`,
//!    fire an onset. The ratio test catches transients regardless
//!    of overall volume; the absolute-delta floor stops the
//!    detector firing on noise-floor wobble in a quiet room.
//! 3. Update the baseline: `baseline = baseline*(1-α) + rms*α`.
//!    Slow follow (α = 0.05) so a real attack doesn't immediately
//!    get absorbed into the baseline and stop firing on subsequent
//!    chunks of the same attack — but the baseline DOES gradually
//!    rise during sustained signal, which is what we want (the
//!    sustain shouldn't keep triggering onsets).
//! 4. Refractory: after an onset fires, ignore the threshold test
//!    for `refractory_samples` samples (~50 ms). Prevents a single
//!    multi-chunk attack ramp from registering as several onsets.
//!
//! The detector is sample-rate-aware: refractory is specified in
//! milliseconds and converted to samples at construction. Chunk
//! size stays fixed at 256 samples regardless of rate (negligible
//! difference between 5.3 ms at 48 kHz and 5.8 ms at 44.1 kHz; not
//! worth the complexity of varying it).

/// Fixed-size analysis chunk. 256 samples is small enough that the
/// detector reports onsets within ~5 ms of the attack (well below
/// any reasonable wait-mode tolerance) and large enough that the
/// per-chunk RMS is a stable energy estimate.
const CHUNK_SIZE: usize = 256;

/// Default exponential-smoothing rate for the baseline RMS. 0.05
/// gives ~20-chunk (100 ms) effective averaging — slow enough that
/// a fresh attack isn't absorbed in a single chunk, fast enough
/// that the baseline catches up during a 1-2 second sustained note
/// (otherwise a long sustain would keep tripping the threshold).
const BASELINE_ALPHA: f32 = 0.05;

/// Default ratio threshold. The current chunk's RMS must exceed
/// `baseline * 2.5` to count as an attack candidate. Tuned against
/// plucked-string envelopes — banjo / uke / guitar attacks are
/// typically 5-10× the steady-state sustain, so 2.5× has comfortable
/// margin without being so loose it fires on amplitude wobble.
const RATIO_THRESHOLD: f32 = 2.5;

/// Default absolute jump floor. The current RMS must exceed the
/// baseline by at least this much (linear amplitude). Prevents the
/// ratio test from firing on noise-floor variation in a quiet
/// room — `baseline = 0.0001`, `current = 0.0003` is a 3× ratio
/// but a real silence. Tuned conservatively so the detector still
/// catches quiet legato plucks.
const MIN_DELTA_RMS: f32 = 0.005;

/// Default refractory window in milliseconds. After an onset fires
/// the detector ignores the threshold test for this many ms — long
/// enough that a multi-chunk attack ramp doesn't register as
/// several onsets, short enough that 16th notes at 200 BPM
/// (~75 ms apart) still resolve as distinct.
const REFRACTORY_MS: f32 = 50.0;

/// Energy-derivative onset detector. See module docs for algorithm.
///
/// Stateful — owns a small scratch buffer for samples that don't
/// fill a complete analysis chunk (carried over to the next
/// [`Self::feed`] call), the exponentially-smoothed baseline RMS,
/// and the refractory counter.
#[derive(Debug)]
pub struct OnsetDetector {
    chunk_buf: Vec<f32>,
    baseline_rms: f32,
    baseline_alpha: f32,
    min_delta_rms: f32,
    ratio_threshold: f32,
    refractory_samples: u32,
    samples_since_last: u32,
}

impl OnsetDetector {
    /// Construct with the default thresholds tuned for plucked-string
    /// attacks. Refractory is computed from [`REFRACTORY_MS`] and the
    /// supplied sample rate.
    pub fn new(sample_rate: u32) -> Self {
        let refractory_samples = ((sample_rate as f32 * REFRACTORY_MS) / 1000.0).round() as u32;
        Self {
            chunk_buf: Vec::with_capacity(CHUNK_SIZE * 2),
            baseline_rms: 0.0,
            baseline_alpha: BASELINE_ALPHA,
            min_delta_rms: MIN_DELTA_RMS,
            ratio_threshold: RATIO_THRESHOLD,
            refractory_samples,
            // Start outside refractory so the very first attack
            // after construction can fire — important for "open the
            // mic, immediately play a note" workflows.
            samples_since_last: refractory_samples,
        }
    }

    /// Feed mono samples. Returns `true` if at least one onset
    /// fired during this batch.
    ///
    /// Samples that don't fill a complete analysis chunk carry over
    /// to the next call — call patterns with feeds of any size
    /// (worklet 128-sample chunks, cpal 1024-sample chunks, large
    /// drain-the-backlog reads) all produce identical results.
    pub fn feed(&mut self, samples: &[f32]) -> bool {
        let mut fired = false;
        for &s in samples {
            self.chunk_buf.push(s);
            if self.chunk_buf.len() >= CHUNK_SIZE {
                if self.process_chunk() {
                    fired = true;
                }
                self.chunk_buf.clear();
            }
        }
        fired
    }

    /// Process one full chunk's worth of samples (drained from
    /// `chunk_buf`). Returns true if an onset fired on THIS chunk.
    fn process_chunk(&mut self) -> bool {
        let rms = chunk_rms(&self.chunk_buf);

        // Refractory check: even if the threshold trips, suppress
        // the onset until enough samples have passed since the last
        // one. Baseline still updates inside the refractory window
        // so the detector "catches up" to a loud sustained note
        // rather than firing repeatedly on every chunk of it.
        let in_refractory = self.samples_since_last < self.refractory_samples;

        let fired = !in_refractory
            && rms > self.baseline_rms * self.ratio_threshold
            && rms - self.baseline_rms > self.min_delta_rms;

        if fired {
            self.samples_since_last = 0;
        } else {
            self.samples_since_last = self.samples_since_last.saturating_add(CHUNK_SIZE as u32);
        }

        // Baseline follows the signal regardless of whether an
        // onset fired. During sustain, this means the baseline
        // gradually rises to match the sustain level so the next
        // ratio test compares against the right reference.
        self.baseline_rms =
            self.baseline_rms * (1.0 - self.baseline_alpha) + rms * self.baseline_alpha;

        fired
    }
}

/// Mean-square-root over a sample window. Same definition as
/// [`crate::window_rms`] but duplicated here so the onset module
/// stays self-contained (and the tuner's RMS helper isn't pub-
/// crate-exposed).
fn chunk_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    const SR: u32 = 48_000;

    /// Generate `n` samples of silence (zeros).
    fn silence(n: usize) -> Vec<f32> {
        vec![0.0; n]
    }

    /// Generate `n` samples of a constant-amplitude sine at `hz`.
    fn sine(n: usize, hz: f32, amplitude: f32) -> Vec<f32> {
        (0..n)
            .map(|i| amplitude * (2.0 * PI * hz * i as f32 / SR as f32).sin())
            .collect()
    }

    /// Synthesise a "pluck" — sharp attack followed by exponential
    /// decay. `decay_secs` is the time over which amplitude falls to
    /// 1/e. Approximates a plucked-string envelope (banjo / uke /
    /// guitar) closely enough for onset-detection tests.
    fn pluck(n: usize, hz: f32, peak_amplitude: f32, decay_secs: f32) -> Vec<f32> {
        let decay_samples = decay_secs * SR as f32;
        (0..n)
            .map(|i| {
                let env = peak_amplitude * (-(i as f32) / decay_samples).exp();
                env * (2.0 * PI * hz * i as f32 / SR as f32).sin()
            })
            .collect()
    }

    #[test]
    fn empty_input_fires_no_onset() {
        let mut d = OnsetDetector::new(SR);
        assert!(!d.feed(&[]));
        assert!(!d.feed(&silence(100)));
    }

    #[test]
    fn pure_silence_fires_no_onset() {
        let mut d = OnsetDetector::new(SR);
        // 1 second of silence — multiple full chunks; no onset
        // possible without a signal jump.
        assert!(!d.feed(&silence(SR as usize)));
    }

    #[test]
    fn single_pluck_fires_exactly_one_onset() {
        let mut d = OnsetDetector::new(SR);
        // 0.5 s of silence, then a 0.5 s pluck. The silence
        // primes a low baseline; the pluck's sharp attack jumps
        // far above it and fires.
        let mut signal = silence(SR as usize / 2);
        signal.extend(pluck(SR as usize / 2, 440.0, 0.3, 0.3));

        // Feed in one go to keep the test deterministic — actual
        // worklet usage feeds in small chunks but the detector
        // produces identical results either way.
        let onsets = count_onsets_per_chunk(&mut d, &signal);
        assert_eq!(onsets, 1, "expected exactly one onset for one pluck");
    }

    #[test]
    fn sustained_sine_fires_at_most_one_onset() {
        // A note's initial attack should fire one onset. The
        // subsequent steady-state shouldn't keep tripping the
        // detector — the baseline must follow the signal.
        let mut d = OnsetDetector::new(SR);
        let signal = sine(SR as usize, 440.0, 0.2);
        let onsets = count_onsets_per_chunk(&mut d, &signal);
        assert!(
            onsets <= 1,
            "sustained sine should fire <= 1 onset, got {onsets}"
        );
    }

    #[test]
    fn two_plucks_far_apart_fire_two_onsets() {
        let mut d = OnsetDetector::new(SR);
        // 0.1 s silence, pluck #1 (300 ms), 0.2 s silence, pluck #2
        // (300 ms). Plucks are 500 ms apart at their onsets — well
        // beyond the 50 ms refractory.
        let mut signal = silence((SR as f32 * 0.1) as usize);
        signal.extend(pluck((SR as f32 * 0.3) as usize, 440.0, 0.3, 0.2));
        signal.extend(silence((SR as f32 * 0.2) as usize));
        signal.extend(pluck((SR as f32 * 0.3) as usize, 660.0, 0.3, 0.2));

        let onsets = count_onsets_per_chunk(&mut d, &signal);
        assert_eq!(onsets, 2, "expected 2 onsets for 2 plucks 500 ms apart");
    }

    #[test]
    fn two_plucks_within_refractory_fire_one_onset() {
        // Two plucks 10 ms apart — inside the 50 ms refractory
        // window. The first fires; the second is suppressed.
        let mut d = OnsetDetector::new(SR);
        let mut signal = silence((SR as f32 * 0.1) as usize);
        signal.extend(pluck((SR as f32 * 0.01) as usize, 440.0, 0.3, 0.05));
        signal.extend(pluck((SR as f32 * 0.3) as usize, 660.0, 0.3, 0.2));

        let onsets = count_onsets_per_chunk(&mut d, &signal);
        assert_eq!(
            onsets, 1,
            "expected refractory to suppress second pluck, got {onsets}"
        );
    }

    #[test]
    fn established_baseline_plus_slow_swell_fires_only_initial_onset() {
        // Real-world scenario: user plays a sustained note, then
        // gradually swells the dynamics (vibrato bow pressure, slow
        // crescendo on a held chord). The initial attack should
        // fire ONE onset (the pluck / bow draw); the subsequent
        // amplitude change shouldn't fire more — that's not a new
        // note.
        //
        // Cold-start-from-silence onsets *are* legitimate ("user
        // just started playing") — only the sustained-then-swell
        // case is the no-op we're pinning here.
        let mut d = OnsetDetector::new(SR);
        let mut signal = sine(SR as usize / 2, 440.0, 0.1);
        signal.extend((0..SR as usize).map(|i| {
            // Linear swell from 0.1 to 0.2 amplitude over 1 s.
            // Slow enough that the baseline (α = 0.05, ~100 ms
            // time constant) tracks the rise comfortably.
            let env = 0.1 + 0.1 * (i as f32 / SR as f32);
            env * (2.0 * PI * 440.0 * i as f32 / SR as f32).sin()
        }));
        let onsets = count_onsets_per_chunk(&mut d, &signal);
        assert_eq!(
            onsets, 1,
            "established baseline + slow swell should give 1 initial onset only, got {onsets}"
        );
    }

    #[test]
    fn feed_size_doesnt_affect_detection_count() {
        // Same signal fed in one giant call vs in tiny worklet-sized
        // chunks should produce identical onset counts. Regression
        // pin against the leftover-samples-handling code in feed().
        let mut signal = silence((SR as f32 * 0.1) as usize);
        signal.extend(pluck((SR as f32 * 0.3) as usize, 440.0, 0.3, 0.2));
        signal.extend(silence((SR as f32 * 0.2) as usize));
        signal.extend(pluck((SR as f32 * 0.3) as usize, 660.0, 0.3, 0.2));

        let mut d_big = OnsetDetector::new(SR);
        let big = count_onsets_per_chunk(&mut d_big, &signal);

        let mut d_small = OnsetDetector::new(SR);
        let mut small = 0;
        for chunk in signal.chunks(128) {
            if d_small.feed(chunk) {
                small += 1;
            }
        }
        // Big-feed counts onsets per 256-sample chunk; small-feed
        // counts feed() calls that fired. The TOTAL onset count
        // should match — the same signal, the same algorithm.
        assert_eq!(big, small);
    }

    /// Drive the detector with one big feed and count how many
    /// separate onsets were emitted, by feeding chunk-by-chunk and
    /// summing the boolean returns. (`feed()` returns a single bool
    /// per call regardless of how many chunks it processed, so to
    /// get an accurate per-chunk count we feed chunk_size at a time.)
    fn count_onsets_per_chunk(d: &mut OnsetDetector, signal: &[f32]) -> usize {
        let mut count = 0;
        for chunk in signal.chunks(CHUNK_SIZE) {
            if d.feed(chunk) {
                count += 1;
            }
        }
        count
    }
}
