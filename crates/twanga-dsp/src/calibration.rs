//! Pure DSP for output→input round-trip latency calibration.
//!
//! Stateless, allocation-light, mirrors the rest of the crate.
//! The actual playback + capture (the part that touches audio
//! devices) lives in the consumer — `twanga-cli`'s
//! `calibration` module on native, `frontend/web/app.html`'s
//! Calibrate screen via the WASM bridge on web. Shared logic
//! lives here so both surfaces agree on what counts as a click
//! peak and what the median across N measurements is.

/// Linear amplitude below which we don't trust that a click
/// actually arrived. 0.02 ≈ -34 dB — generous enough to accept
/// a click captured at low monitor volume but firm enough that a
/// noise-floor blip can't masquerade as a peak.
pub const PEAK_THRESHOLD: f32 = 0.02;

/// Find the loudest absolute-value peak in `samples` and convert
/// its index to milliseconds at `sample_rate`. Returns `None` if
/// the peak is below [`PEAK_THRESHOLD`] — interpreted by callers
/// as "no audible click reached the mic" (muted, too quiet, or
/// wrong device wiring).
///
/// Caller invariant: `samples` should begin at the moment the
/// click was scheduled to play — the returned millisecond value
/// is the offset from sample 0 to the peak.
pub fn locate_click_peak_ms(samples: &[f32], sample_rate: u32) -> Option<u32> {
    let mut peak_idx = 0usize;
    let mut peak_val = 0.0_f32;
    for (i, &s) in samples.iter().enumerate() {
        let a = s.abs();
        if a > peak_val {
            peak_val = a;
            peak_idx = i;
        }
    }
    if peak_val < PEAK_THRESHOLD {
        return None;
    }
    Some((peak_idx as u64 * 1000 / sample_rate as u64) as u32)
}

/// Sort + take the median of a measurement list. `None` on empty
/// input; otherwise the value at index `len / 2` after sorting
/// (lower-of-middle-two on even lengths — for `CLICK_COUNT = 5`
/// in the calibration UI it's the unambiguous middle value).
pub fn median(measurements: &mut [u32]) -> Option<u32> {
    if measurements.is_empty() {
        return None;
    }
    measurements.sort_unstable();
    Some(measurements[measurements.len() / 2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locate_peak_finds_loud_sample_position() {
        // 48 kHz buffer with one loud spike at sample 480 → 10 ms.
        let mut samples = vec![0.0_f32; 48_000];
        samples[480] = 0.8;
        assert_eq!(locate_click_peak_ms(&samples, 48_000), Some(10));
    }

    #[test]
    fn locate_peak_returns_none_when_below_threshold() {
        // Max amplitude 0.005 — well under PEAK_THRESHOLD.
        let samples = vec![0.005_f32; 48_000];
        assert_eq!(locate_click_peak_ms(&samples, 48_000), None);
    }

    #[test]
    fn locate_peak_handles_negative_peaks() {
        // The click's loudest sample might be negative; the
        // peak-finder uses absolute value, so a -0.8 peak is
        // located the same as a +0.8 peak.
        let mut samples = vec![0.0_f32; 48_000];
        samples[960] = -0.8;
        assert_eq!(locate_click_peak_ms(&samples, 48_000), Some(20));
    }

    #[test]
    fn locate_peak_at_threshold_boundary() {
        // Just under the threshold = rejected; just over = accepted.
        // The strict `<` in the function means exactly-at-threshold
        // passes; this test pins values on either side of that to
        // catch a regression that flipped the comparator.
        let mut samples = vec![0.0_f32; 48_000];
        samples[100] = PEAK_THRESHOLD - 0.001;
        assert_eq!(locate_click_peak_ms(&samples, 48_000), None);
        samples[100] = PEAK_THRESHOLD + 0.001;
        assert!(locate_click_peak_ms(&samples, 48_000).is_some());
    }

    #[test]
    fn median_of_empty_is_none() {
        let mut m: Vec<u32> = vec![];
        assert_eq!(median(&mut m), None);
    }

    #[test]
    fn median_of_odd_length_picks_middle() {
        let mut m = vec![50, 10, 30, 40, 20];
        assert_eq!(median(&mut m), Some(30));
    }

    #[test]
    fn median_of_single_value() {
        let mut m = vec![42];
        assert_eq!(median(&mut m), Some(42));
    }
}
