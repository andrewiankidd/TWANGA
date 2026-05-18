//! Pure audio synthesis. Deterministic, IO-free, no allocations beyond the returned buffer.

use std::f32::consts::TAU;
use twanga_core::Frequency;

/// Pure sine wave at `freq` for `n_samples` samples at `sample_rate` Hz. Peak amplitude 1.0.
pub fn sine(freq: Frequency, sample_rate: u32, n_samples: usize) -> Vec<f32> {
    let step = TAU * freq.hz() / sample_rate as f32;
    (0..n_samples).map(|i| (step * i as f32).sin()).collect()
}

/// Sum of harmonics. `partial_amplitudes[i]` scales the (i+1)th partial:
/// index 0 is the fundamental, index 1 is the 2nd partial (2× freq), etc.
/// Result is not normalised — caller is responsible for keeping the sum below clipping.
pub fn harmonic_stack(
    freq: Frequency,
    sample_rate: u32,
    n_samples: usize,
    partial_amplitudes: &[f32],
) -> Vec<f32> {
    let mut buf = vec![0.0_f32; n_samples];
    for (n, &amp) in partial_amplitudes.iter().enumerate() {
        let partial_hz = freq.hz() * (n as f32 + 1.0);
        let step = TAU * partial_hz / sample_rate as f32;
        for (i, sample) in buf.iter_mut().enumerate() {
            *sample += amp * (step * i as f32).sin();
        }
    }
    buf
}

/// Apply an exponential decay envelope in place. After `half_life_secs` the amplitude is halved.
pub fn exp_decay(buf: &mut [f32], sample_rate: u32, half_life_secs: f32) {
    let n = (half_life_secs * sample_rate as f32).max(1.0);
    let decay_per_sample = (-2f32.ln() / n).exp();
    let mut gain = 1.0_f32;
    for s in buf.iter_mut() {
        *s *= gain;
        gain *= decay_per_sample;
    }
}

/// Deterministic white noise via xorshift64. Output in `[-amplitude, amplitude]`.
pub fn white_noise(amplitude: f32, n_samples: usize, seed: u64) -> Vec<f32> {
    let mut state: u64 = if seed == 0 { 1 } else { seed };
    let mut out = Vec::with_capacity(n_samples);
    for _ in 0..n_samples {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let u = (state >> 32) as u32;
        let f = (u as f32 / u32::MAX as f32) * 2.0 - 1.0;
        out.push(amplitude * f);
    }
    out
}

/// Add `src` into `dest` element-wise. Both slices must be the same length.
pub fn mix_into(dest: &mut [f32], src: &[f32]) {
    debug_assert_eq!(dest.len(), src.len());
    for (d, s) in dest.iter_mut().zip(src.iter()) {
        *d += *s;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 44100;

    fn rms(buf: &[f32]) -> f32 {
        let sum_sq: f32 = buf.iter().map(|x| x * x).sum();
        (sum_sq / buf.len() as f32).sqrt()
    }

    fn zero_crossings(buf: &[f32]) -> usize {
        buf.windows(2)
            .filter(|w| w[0].signum() != w[1].signum() && w[0] != 0.0)
            .count()
    }

    #[test]
    fn sine_has_correct_length() {
        let buf = sine(Frequency::A4, SR, 2048);
        assert_eq!(buf.len(), 2048);
    }

    #[test]
    fn sine_peak_is_approximately_one() {
        let buf = sine(Frequency(440.0), SR, 10_000);
        let peak = buf.iter().copied().fold(0.0_f32, f32::max);
        assert!(peak > 0.999, "peak should be near 1.0, got {peak}");
        assert!(peak <= 1.0);
    }

    #[test]
    fn sine_rms_is_approximately_one_over_sqrt_two() {
        let buf = sine(Frequency::A4, SR, 10_000);
        let r = rms(&buf);
        let expected = 1.0 / 2_f32.sqrt();
        assert!(
            (r - expected).abs() < 0.01,
            "RMS should be ≈1/√2 ({expected}), got {r}"
        );
    }

    #[test]
    fn sine_zero_crossings_match_frequency() {
        let freq = 440.0;
        let n = SR as usize; // exactly 1 second
        let buf = sine(Frequency(freq), SR, n);
        let crossings = zero_crossings(&buf);
        let expected = (2.0 * freq) as usize;
        assert!(
            crossings.abs_diff(expected) <= 2,
            "expected ~{expected} zero crossings, got {crossings}"
        );
    }

    #[test]
    fn harmonic_stack_with_single_fundamental_equals_sine() {
        let pure = sine(Frequency::A4, SR, 1024);
        let stacked = harmonic_stack(Frequency::A4, SR, 1024, &[1.0]);
        for (i, (a, b)) in pure.iter().zip(stacked.iter()).enumerate() {
            assert!((a - b).abs() < 1e-6, "diff at sample {i}: {a} vs {b}");
        }
    }

    #[test]
    fn harmonic_stack_adds_correctly() {
        let buf = harmonic_stack(Frequency::A4, SR, 10_000, &[1.0, 1.0]);
        let peak = buf.iter().copied().fold(0.0_f32, f32::max);
        assert!(
            peak > 1.5 && peak <= 2.0,
            "two unit-amplitude harmonics should peak near 2.0, got {peak}"
        );
    }

    #[test]
    fn exp_decay_halves_at_half_life() {
        let mut buf = vec![1.0_f32; SR as usize];
        exp_decay(&mut buf, SR, 0.5);
        let mid = buf[SR as usize / 2];
        assert!(
            (mid - 0.5).abs() < 0.01,
            "amplitude at the half-life sample should be ≈0.5, got {mid}"
        );
    }

    #[test]
    fn white_noise_is_deterministic() {
        let a = white_noise(1.0, 100, 42);
        let b = white_noise(1.0, 100, 42);
        assert_eq!(a, b);
    }

    #[test]
    fn white_noise_amplitude_is_bounded() {
        let buf = white_noise(0.5, 10_000, 1);
        for &s in &buf {
            assert!(s.abs() <= 0.5, "sample out of range: {s}");
        }
    }

    #[test]
    fn white_noise_zero_seed_does_not_lock_up() {
        let buf = white_noise(1.0, 10, 0);
        assert_eq!(buf.len(), 10);
        assert!(buf.iter().any(|&s| s != 0.0));
    }

    #[test]
    fn mix_into_adds_buffers_elementwise() {
        let mut a = vec![1.0, 2.0, 3.0];
        let b = vec![10.0, 20.0, 30.0];
        mix_into(&mut a, &b);
        assert_eq!(a, vec![11.0, 22.0, 33.0]);
    }

    // ---- Layer 1: math anchors ----
    // These tests assert the synth against pure arithmetic at frequencies where
    // the sample pattern is closed-form. If they pass, twanga-synth is not silently
    // miscalibrated against itself — the values match what math says they should be.

    #[test]
    fn sine_at_quarter_sample_rate_matches_exact_pattern() {
        // sin(2π · fs/4 · n / fs) = sin(π/2 · n) → [0, 1, 0, -1] repeating.
        let fs = 44100_u32;
        let f = Frequency(fs as f32 / 4.0);
        let buf = sine(f, fs, 16);
        let expected = [0.0, 1.0, 0.0, -1.0];
        for (i, &sample) in buf.iter().enumerate() {
            let want = expected[i % 4];
            assert!(
                (sample - want).abs() < 1e-5,
                "sample {i}: got {sample}, want {want}"
            );
        }
    }

    #[test]
    fn sine_at_sixth_sample_rate_matches_exact_pattern() {
        // sin(2π · fs/6 · n / fs) = sin(π/3 · n) → [0, √3/2, √3/2, 0, -√3/2, -√3/2].
        let fs = 44100_u32;
        let f = Frequency(fs as f32 / 6.0);
        let buf = sine(f, fs, 12);
        let s = 3_f32.sqrt() / 2.0;
        let expected = [0.0, s, s, 0.0, -s, -s];
        for (i, &sample) in buf.iter().enumerate() {
            let want = expected[i % 6];
            assert!(
                (sample - want).abs() < 1e-5,
                "sample {i}: got {sample}, want {want}"
            );
        }
    }

    #[test]
    fn frequency_doubling_property_holds() {
        // sin(2π · 2f · n / fs) == sin(2π · f · 2n / fs) — same mathematical value
        // reached through different indexing paths. If the synth's frequency→step
        // mapping is consistent, these match across the buffer.
        let fs = 44100_u32;
        let f = Frequency(440.0);
        let f2 = Frequency(880.0);
        let len = 2048;
        let buf_f = sine(f, fs, len);
        let buf_2f = sine(f2, fs, len / 2);
        for n in 0..len / 2 {
            let a = buf_f[2 * n];
            let b = buf_2f[n];
            assert!((a - b).abs() < 1e-5, "n={n}: sine(f)[2n]={a}, sine(2f)[n]={b}");
        }
    }
}
