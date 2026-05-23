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

/// How a saved `LatencyCalibration` was produced.
///
/// - **PluckAlong** is the primary method: TWANGA plays a metronome
///   and the user plucks a single note on each beat. The median
///   offset between scheduled-beat-time and detected-onset-time
///   becomes the latency. Works for any input (mic, line-in, USB
///   instrument cable) because it captures *what the user actually
///   plays*. Includes the user's reaction time, which is the
///   correct thing to subtract for scoring (the user wants to
///   score Hit when they play on the beat *as they perceive it*).
/// - **RoundTrip** is a hardware-only measurement: TWANGA plays
///   clicks through speakers and captures them via mic. Measures
///   system delay only (no human reaction time). Useful for users
///   who specifically want hardware compensation and not their
///   own reaction time absorbed. Only works with mic + speakers
///   in the same room.
/// - **Manual** is what the user typed in. Always available as a
///   fallback when neither measurement method is practical.
///
/// Surfaced in the UI as "via pluck-along" / "via round-trip" /
/// "via manual" so users can tell at a glance how the number was
/// produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CalibrationMethod {
    PluckAlong,
    RoundTrip,
    Manual,
}

impl CalibrationMethod {
    pub fn label(self) -> &'static str {
        match self {
            Self::PluckAlong => "pluck-along",
            Self::RoundTrip => "round-trip",
            Self::Manual => "manual",
        }
    }
}

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
    /// `CLICK_COUNT` individual measurements (round-trip method),
    /// or whatever the user typed in (manual method).
    pub latency_ms: u32,
    /// Wall-clock timestamp of the measurement, RFC 3339. Not
    /// load-bearing for scoring; used by the GUI's
    /// "calibrated <when>" affordance.
    pub measured_at: String,
    /// How the value was produced. Defaults to `RoundTrip` for
    /// backwards compatibility with calibrations saved before this
    /// field existed (anything on disk without it came from the
    /// original round-trip-only `twanga calibrate` flow).
    #[serde(default = "default_method")]
    pub method: CalibrationMethod,
}

fn default_method() -> CalibrationMethod {
    CalibrationMethod::RoundTrip
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
        method: CalibrationMethod::RoundTrip,
    })
}

/// Build a manual-entry calibration record. The wizard's
/// "headphones / line-in / no-mic" branches construct one of these
/// when the user types in a known value rather than running a
/// physical measurement. `device_name` is captured the same way as
/// in `run_calibration` so the per-device invalidation logic in
/// the playback loop still works.
pub fn manual_calibration(device_name: String, latency_ms: u32) -> LatencyCalibration {
    LatencyCalibration {
        device_name,
        latency_ms,
        measured_at: now_rfc3339(),
        method: CalibrationMethod::Manual,
    }
}

// ─────────────────────────── Pluck-along measurement ───────────────────────────

/// Metronome tempo for the pluck-along procedure. 80 BPM (750 ms
/// per beat) is slow enough for a beginner to land notes cleanly
/// but fast enough that 8 beats finishes in ~6 seconds.
pub const PLUCK_ALONG_BPM: u32 = 80;
/// How many beats the user is asked to pluck along to. Median over
/// this many measurements absorbs one or two miss-timed plucks
/// without throwing off the result.
pub const PLUCK_ALONG_BEATS: usize = 8;
/// Pre-roll beat count before the measurement begins, giving the
/// user time to lock onto the tempo. Plays clicks at the same BPM
/// but no plucks are recorded.
pub const PLUCK_ALONG_PRE_ROLL: usize = 4;
/// Minimum fraction of beats that must produce a detected pluck
/// for the measurement to be trusted. Below this threshold the
/// caller errors out with "play louder / clearer" guidance rather
/// than saving a garbage value derived from one or two plucks.
pub const PLUCK_ALONG_MIN_MATCH_RATIO: f32 = 0.5;

/// Progress events emitted during a pluck-along run so the caller
/// (CLI / GUI / tests) can render feedback without coupling to the
/// measurement loop's internals.
pub enum PluckProgress<'a> {
    PreRoll {
        i: usize,
        total: usize,
    },
    Beat {
        i: usize,
        total: usize,
    },
    Done {
        matched: usize,
        total: usize,
        label: &'a str,
    },
}

/// Run a pluck-along calibration: TWANGA plays a metronome and
/// the user plucks a single note on each beat. The median offset
/// between scheduled beat time and detected onset becomes the
/// latency.
///
/// Works for any input that produces a detectable energy spike
/// per pluck — mic, line-in, USB instrument cable. The procedure
/// includes the user's reaction time in the measured offset,
/// which is the correct thing to subtract for scoring (a user
/// playing "on the beat as they perceive it" should score Hit,
/// not Late by their reaction time).
pub fn run_pluck_along_calibration(
    progress: &mut dyn FnMut(PluckProgress<'_>),
) -> Result<LatencyCalibration> {
    let mut input = InputStream::open()?;
    let mut output = OutputStream::open()?;
    let device_name = input.device_name.clone();
    let click = metronome_click(output.sample_rate);
    let input_sr = input.sample_rate;
    let beat_ms: u64 = (60_000 / PLUCK_ALONG_BPM as u64).max(50);

    // Tuner is reused for its onset detector — we don't care about
    // pitch here, only timing. `from_onset_window` readings tell us
    // when a fresh attack arrived.
    let mut tuner = twanga_dsp::Tuner::new(twanga_dsp::TunerMode::Chromatic, input_sr);
    let window_latency_ms = tuner.window_latency_ms() as u64;
    let mut buf = vec![0.0_f32; 4096];

    // Pre-roll: clicks without recording, so the user can lock
    // onto the tempo before the measurement starts.
    for i in 0..PLUCK_ALONG_PRE_ROLL {
        progress(PluckProgress::PreRoll {
            i: i + 1,
            total: PLUCK_ALONG_PRE_ROLL,
        });
        let beat_at = Instant::now();
        output.write(&click);
        let elapsed = beat_at.elapsed();
        let target = Duration::from_millis(beat_ms);
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }
    }
    // Drain any mic samples accumulated during pre-roll so they
    // don't pollute the first beat's window.
    while input.read(&mut buf) > 0 {}

    // Measurement phase. clock_origin = "beat 1 starts now."
    let clock_origin = Instant::now();
    let mut scheduled_beats_ms: Vec<u64> = Vec::with_capacity(PLUCK_ALONG_BEATS);
    let mut onsets_ms: Vec<u64> = Vec::new();

    for beat_idx in 0..PLUCK_ALONG_BEATS {
        progress(PluckProgress::Beat {
            i: beat_idx + 1,
            total: PLUCK_ALONG_BEATS,
        });
        let beat_at = clock_origin + Duration::from_millis(beat_idx as u64 * beat_ms);
        let now = Instant::now();
        if now < beat_at {
            std::thread::sleep(beat_at - now);
        }
        let actual_beat_ms = clock_origin.elapsed().as_millis() as u64;
        scheduled_beats_ms.push(actual_beat_ms);
        output.write(&click);

        // Capture mic for the duration of this beat (until the next
        // one is due). Collect any onset-tagged readings.
        let until = Instant::now() + Duration::from_millis(beat_ms);
        while Instant::now() < until {
            let n = input.read(&mut buf);
            if n > 0 {
                tuner.feed(&buf[..n]);
                let now_ms = clock_origin.elapsed().as_millis() as u64;
                for r in tuner.take_readings() {
                    if r.from_onset_window {
                        onsets_ms.push(now_ms.saturating_sub(window_latency_ms));
                    }
                }
            } else {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    // Trailing capture: an onset arriving in the last beat's window
    // might land in the YIN buffer just after the loop above exits.
    // Capture for one more beat-length so we don't drop it.
    let trailing_until = Instant::now() + Duration::from_millis(beat_ms);
    while Instant::now() < trailing_until {
        let n = input.read(&mut buf);
        if n > 0 {
            tuner.feed(&buf[..n]);
            let now_ms = clock_origin.elapsed().as_millis() as u64;
            for r in tuner.take_readings() {
                if r.from_onset_window {
                    onsets_ms.push(now_ms.saturating_sub(window_latency_ms));
                }
            }
        } else {
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    // Pair each scheduled beat with its nearest unused onset within
    // ±half-a-beat. Compute the signed offset (positive = late,
    // negative = early) and median over the matches.
    let mut used = vec![false; onsets_ms.len()];
    let half_beat = (beat_ms / 2) as i64;
    let mut offsets: Vec<i64> = Vec::new();
    for &beat in &scheduled_beats_ms {
        let mut best: Option<(usize, i64)> = None;
        for (i, &onset) in onsets_ms.iter().enumerate() {
            if used[i] {
                continue;
            }
            let signed = onset as i64 - beat as i64;
            if signed.abs() > half_beat {
                continue;
            }
            if best.is_none_or(|(_, d)| signed.abs() < d.abs()) {
                best = Some((i, signed));
            }
        }
        if let Some((i, signed)) = best {
            used[i] = true;
            offsets.push(signed);
        }
    }

    let matched = offsets.len();
    let total = scheduled_beats_ms.len();
    let min_matches = (total as f32 * PLUCK_ALONG_MIN_MATCH_RATIO).ceil() as usize;
    if matched < min_matches {
        return Err(anyhow!(
            "only {matched} of {total} beats had a detected pluck — try again with louder, more deliberate plucks, or check your input level (silence threshold may be too high)"
        ));
    }

    offsets.sort_unstable();
    let median = offsets[offsets.len() / 2];
    // Negative offsets (playing early) clamp to 0 — we don't want
    // to ADD latency to onset timestamps, only subtract.
    let latency_ms = median.max(0) as u32;

    let label = CalibrationMethod::PluckAlong.label();
    progress(PluckProgress::Done {
        matched,
        total,
        label,
    });
    Ok(LatencyCalibration {
        device_name,
        latency_ms,
        measured_at: now_rfc3339(),
        method: CalibrationMethod::PluckAlong,
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
            method: CalibrationMethod::RoundTrip,
        };
        cal.save(&path).expect("save");
        let loaded = LatencyCalibration::load(&path)
            .expect("load")
            .expect("present");
        assert_eq!(loaded.device_name, "Test Mic");
        assert_eq!(loaded.latency_ms, 42);
        assert_eq!(loaded.method, CalibrationMethod::RoundTrip);
        assert!(loaded.applies_to("Test Mic"));
        assert!(!loaded.applies_to("Other Mic"));
    }

    #[test]
    fn manual_calibration_serializes_method() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("latency.toml");
        let cal = manual_calibration("Headphones + Mic".to_string(), 30);
        cal.save(&path).expect("save");
        let loaded = LatencyCalibration::load(&path)
            .expect("load")
            .expect("present");
        assert_eq!(loaded.latency_ms, 30);
        assert_eq!(loaded.method, CalibrationMethod::Manual);
    }

    #[test]
    fn missing_method_field_defaults_to_round_trip() {
        // Backwards compat: calibrations saved before the `method`
        // field existed should still load and be treated as
        // round-trip (the only path the old binary supported).
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("latency.toml");
        let toml = r#"
            device_name = "Old Mic"
            latency_ms = 25
            measured_at = "epoch-seconds:999"
        "#;
        std::fs::write(&path, toml).expect("write");
        let loaded = LatencyCalibration::load(&path)
            .expect("load")
            .expect("present");
        assert_eq!(loaded.method, CalibrationMethod::RoundTrip);
    }

    #[test]
    fn load_returns_none_when_file_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("does-not-exist.toml");
        let loaded = LatencyCalibration::load(&path).expect("load");
        assert!(loaded.is_none());
    }
}
