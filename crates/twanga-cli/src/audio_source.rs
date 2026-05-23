//! Abstract over "where do audio samples come from" so the playback
//! loop can be driven either by a live mic (the normal case) or a
//! pre-synthesised WAV (test fixtures, the `--from-file` flag).
//!
//! The trait is intentionally narrow — just `sample_rate()` +
//! `read()` — because that's all
//! [`wait_for_expected_note`](crate::wait_for_expected_note) and
//! [`capture_onsets_for_duration`](crate::capture_onsets_for_duration)
//! actually need. Anything richer would couple the loop to mic
//! lifecycle concerns the WAV source doesn't have.

use anyhow::Result;
use std::path::Path;
use std::time::Instant;
use twanga_audio::InputStream;

/// Source of mono `f32` audio samples for the playback loop.
///
/// The contract mirrors [`InputStream`]: `read` is non-blocking, may
/// return fewer samples than requested (including zero when no
/// samples are ready yet), and never blocks the calling thread.
/// `sample_rate` is constant for the lifetime of the source.
pub trait SampleSource {
    fn sample_rate(&self) -> u32;
    fn read(&mut self, out: &mut [f32]) -> usize;
}

impl SampleSource for InputStream {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn read(&mut self, out: &mut [f32]) -> usize {
        InputStream::read(self, out)
    }
}

/// A `SampleSource` that replays a pre-loaded WAV file at the
/// file's own sample rate, paced to wall-clock time.
///
/// The wall-clock pacing is the load-bearing detail: the playback
/// loop sleeps between column ticks and reads whatever's "ready"
/// during the sleep. A live mic naturally produces ~sample_rate
/// samples per second; a naive WAV reader would dump the whole
/// file on the first `read`, breaking the scoring math (every
/// onset would land at timestamp 0). This source returns only as
/// many samples as wall-clock time has elapsed since the first
/// `read` call, so it behaves like a live stream from the loop's
/// perspective.
///
/// Used by `twanga play --from-file <wav>` for deterministic
/// end-to-end tests of wait-mode + proximity-score behaviour
/// without needing mic hardware.
pub struct WavSampleSource {
    samples: Vec<f32>,
    sample_rate: u32,
    /// Samples already handed out. Acts as a read cursor into
    /// `samples`.
    position: usize,
    /// `None` until the first `read()` call; subsequent reads pace
    /// against this so the source's "now" is "first-read = t0,"
    /// matching how a live mic feels.
    start: Option<Instant>,
}

impl WavSampleSource {
    /// Load a WAV file from disk and prepare it for paced replay.
    pub fn from_file(path: &Path) -> Result<Self> {
        let wav = crate::wav::read(path)?;
        Ok(Self {
            samples: wav.samples,
            sample_rate: wav.sample_rate,
            position: 0,
            start: None,
        })
    }

    /// Construct from an in-memory sample buffer + rate. Used by
    /// integration tests that synth fixtures programmatically
    /// rather than writing a temp WAV.
    #[cfg(test)]
    pub fn from_samples(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            samples,
            sample_rate,
            position: 0,
            start: None,
        }
    }
}

impl SampleSource for WavSampleSource {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn read(&mut self, out: &mut [f32]) -> usize {
        let start = *self.start.get_or_insert_with(Instant::now);
        // How many samples "should have" been produced by now if we
        // were a real-time stream at this sample rate.
        let elapsed_ms = start.elapsed().as_millis() as u64;
        let want = (elapsed_ms * self.sample_rate as u64 / 1000) as usize;
        // How many of those haven't been handed out yet (capped at
        // the buffer size + the file's remaining length).
        let ready = want.saturating_sub(self.position);
        let n = ready.min(out.len()).min(self.samples.len() - self.position);
        if n == 0 {
            return 0;
        }
        out[..n].copy_from_slice(&self.samples[self.position..self.position + n]);
        self.position += n;
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn wav_source_paces_to_wall_clock() {
        // 1-second of samples at 48 kHz = 48000 samples. After
        // ~20 ms of wall-clock, we should get back roughly
        // 48000 * 0.020 = 960 samples (give or take scheduling
        // jitter — assertion is loose).
        let samples: Vec<f32> = (0..48_000).map(|i| (i as f32 / 48_000.0).sin()).collect();
        let mut src = WavSampleSource::from_samples(samples, 48_000);
        let mut buf = vec![0.0_f32; 4096];

        // First read — establishes t0, may return 0.
        let _ = src.read(&mut buf);
        sleep(Duration::from_millis(20));
        let n = src.read(&mut buf);

        // Loose bounds — scheduling can deliver anywhere in the
        // range. Floor at 100 (would take 2.1 ms of wall-clock,
        // we slept 20 ms — should comfortably exceed); ceiling at
        // buf.len() (capped by buffer regardless of how much
        // wall-clock fired).
        assert!(
            (100..=buf.len()).contains(&n),
            "expected 100..={} samples after 20 ms wall-clock, got {n}",
            buf.len()
        );
    }

    #[test]
    fn wav_source_returns_zero_once_file_drained() {
        let mut src = WavSampleSource::from_samples(vec![0.0; 100], 48_000);
        let mut buf = vec![0.0_f32; 256];
        // First read establishes t0 (returns 0). Then sleep long
        // enough that wall-clock has "produced" the whole 100-sample
        // file and a second read drains it.
        let _ = src.read(&mut buf);
        sleep(Duration::from_millis(20));
        let drained = src.read(&mut buf);
        assert_eq!(
            drained, 100,
            "second read after 20 ms should drain the whole 100-sample file"
        );
        // Subsequent reads return 0 forever — the source is
        // exhausted, the loop's column ticks just keep advancing
        // through silence (scored as Missed).
        let after = src.read(&mut buf);
        assert_eq!(after, 0);
    }

    #[test]
    fn wav_source_reports_its_sample_rate() {
        let src = WavSampleSource::from_samples(vec![0.0; 0], 44_100);
        assert_eq!(src.sample_rate(), 44_100);
    }
}
