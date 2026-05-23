//! Output→input round-trip latency measurement + persistence.
//!
//! Plays a series of short metronome clicks through the default
//! audio output, captures the mic input throughout, and measures
//! the offset between when each click was scheduled and when its
//! peak shows up in the recording. The median of those offsets is
//! the round-trip latency (output driver, speaker, air gap, mic,
//! input driver, buffering — all rolled into one number) that the
//! playback scorer can subtract from captured onset timestamps so
//! a pluck the user makes "on time" actually scores as Hit instead
//! of being systematically Late by the user's hardware latency.
//!
//! Persisted to `$DATA_ROOT/latency.toml` keyed by input-device
//! name. Reading the value back compares the stored device name
//! against the live one; mismatch invalidates the value (we don't
//! want to score against a measurement taken on a different mic).
//!
//! Separate from the `Tuner::window_latency_ms()` value — that's a
//! fixed DSP-pipeline latency (YIN window length); this one is the
//! user's *hardware-specific* round-trip. Both get subtracted from
//! the captured timestamp; the sum is the total apparent latency
//! between attack and timestamp.

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, Instant};
use twanga_audio::{InputStream, OutputStream};

use crate::metronome_click;

/// How many clicks the calibration plays. 5 gives enough samples
/// to reject one-off scheduling jitter via median while keeping
/// the whole procedure under ~4 seconds.
pub const CLICK_COUNT: usize = 5;

/// Spacing between successive clicks. Needs to be > the longest
/// plausible round-trip latency so the previous click's echo
/// has fully arrived before the next one fires; 600 ms is
/// generous for everything short of a Bluetooth chain.
pub const CLICK_INTERVAL_MS: u64 = 600;

/// Per-click capture window. The mic recording for each click
/// extends from its scheduled play time to this many ms after.
/// Latencies past this are treated as "didn't detect" — likely
/// because the user has the mic muted or the click is below the
/// silence floor.
pub const CLICK_CAPTURE_MS: u64 = 500;

/// Persisted calibration result. Tied to the device that produced
/// the measurement; reading back asserts the live device matches
/// before honouring the value.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyCalibration {
    /// Input-device name as reported by `InputStream::device_name`
    /// at the time the measurement was taken. Used as the
    /// invalidation key — if the live device changes, the
    /// measurement no longer applies and the user is prompted to
    /// recalibrate.
    pub device_name: String,
    /// Measured round-trip latency in milliseconds. Median of
    /// `CLICK_COUNT` individual measurements.
    pub latency_ms: u32,
    /// Wall-clock timestamp of the measurement, RFC 3339. Not
    /// load-bearing for scoring; used by the GUI's
    /// "calibrated <when>" affordance.
    pub measured_at: String,
}

impl LatencyCalibration {
    /// Read a saved calibration from disk. Returns `Ok(None)` when
    /// the file doesn't exist (first-run case, never calibrated)
    /// rather than erroring — callers treat "no calibration" as
    /// "use zero, recommend the user calibrate."
    pub fn load(path: &Path) -> Result<Option<Self>> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s)
                .map(Some)
                .map_err(|e| anyhow!("parse {}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(anyhow!("read {}: {e}", path.display())),
        }
    }

    /// Persist this calibration to disk. Creates the parent
    /// directory if missing — matches the pattern used by
    /// `tunings.toml` / `play-resume.toml`.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow!("create {}: {e}", parent.display()))?;
        }
        let s = toml::to_string_pretty(self).map_err(|e| anyhow!("serialize: {e}"))?;
        std::fs::write(path, s).map_err(|e| anyhow!("write {}: {e}", path.display()))?;
        Ok(())
    }

    /// True iff this calibration is for the given device. Used by
    /// the playback loop to decide whether to subtract the stored
    /// latency or treat it as stale.
    pub fn applies_to(&self, device_name: &str) -> bool {
        self.device_name == device_name
    }
}

/// Run the full calibration procedure and return the measured
/// latency. Opens its own input + output streams; the caller is
/// expected to NOT have streams open against the same devices at
/// the same time (CPAL behaviour around simultaneous streams on
/// the same device is platform-dependent and not worth fighting).
///
/// On Windows + WASAPI this can take a moment to settle after
/// `OutputStream::open()` — the first `write()` call sometimes
/// gets eaten by the driver as it primes the buffer. We pad with
/// a brief warmup play before the real measurement starts.
pub fn run_calibration(progress: &mut dyn FnMut(usize, usize)) -> Result<LatencyCalibration> {
    let mut input = InputStream::open()?;
    let mut output = OutputStream::open()?;
    let device_name = input.device_name.clone();
    let click = metronome_click(output.sample_rate);
    let input_sr = input.sample_rate;

    // Warmup: play one click to prime the output buffer and let
    // the driver settle. Discard the result.
    output.write(&click);
    std::thread::sleep(Duration::from_millis(CLICK_INTERVAL_MS));
    // Drain any mic samples accumulated during warmup so the real
    // measurement starts from an empty queue.
    let mut drain_buf = vec![0.0_f32; 4096];
    while input.read(&mut drain_buf) > 0 {}

    let mut measurements: Vec<u32> = Vec::with_capacity(CLICK_COUNT);
    let mut capture_buf = vec![0.0_f32; (input_sr as u64 * CLICK_CAPTURE_MS / 1000) as usize];

    for i in 0..CLICK_COUNT {
        progress(i, CLICK_COUNT);
        let click_at = Instant::now();
        output.write(&click);
        // Capture samples for the configured window. Polled reads;
        // CPAL backpressures, so even on a slow machine we never
        // miss frames — we just collect fewer per `read()` call.
        let until = click_at + Duration::from_millis(CLICK_CAPTURE_MS);
        let mut written = 0usize;
        while Instant::now() < until && written < capture_buf.len() {
            let n = input.read(&mut capture_buf[written..]);
            if n == 0 {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            written += n;
        }
        if let Some(offset_ms) =
            twanga_dsp::calibration::locate_click_peak_ms(&capture_buf[..written], input_sr)
        {
            measurements.push(offset_ms);
        }
        // Space the clicks out.
        let elapsed = click_at.elapsed();
        let target = Duration::from_millis(CLICK_INTERVAL_MS);
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }
    }
    progress(CLICK_COUNT, CLICK_COUNT);

    let median = twanga_dsp::calibration::median(&mut measurements).ok_or_else(|| {
        anyhow!(
            "no clicks were detected in the mic input — is the mic muted, or pointed away from the speakers?"
        )
    })?;

    Ok(LatencyCalibration {
        device_name,
        latency_ms: median,
        measured_at: now_rfc3339(),
    })
}

/// Best-effort current-time stamp. Falls back to "unknown" if the
/// system clock can't be read — calibration shouldn't fail just
/// because the time crate hit a weird edge.
fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| format!("epoch-seconds:{}", d.as_secs()))
        .unwrap_or_else(|_| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_calibration_through_toml() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("latency.toml");
        let cal = LatencyCalibration {
            device_name: "Test Mic".to_string(),
            latency_ms: 42,
            measured_at: "epoch-seconds:12345".to_string(),
        };
        cal.save(&path).expect("save");
        let loaded = LatencyCalibration::load(&path)
            .expect("load")
            .expect("present");
        assert_eq!(loaded.device_name, "Test Mic");
        assert_eq!(loaded.latency_ms, 42);
        assert!(loaded.applies_to("Test Mic"));
        assert!(!loaded.applies_to("Other Mic"));
    }

    #[test]
    fn load_returns_none_when_file_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("does-not-exist.toml");
        let loaded = LatencyCalibration::load(&path).expect("load");
        assert!(loaded.is_none());
    }
}
