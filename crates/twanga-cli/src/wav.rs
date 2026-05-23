//! Minimal mono PCM WAV reader + writer.
//!
//! Why in-house rather than `hound`: we only need three things — read
//! mono PCM (16-bit int or 32-bit float) into a `Vec<f32>`, write a
//! `Vec<f32>` out as 32-bit float WAV, surface the file's sample
//! rate. The full RIFF/WAVE spec has dozens of variant chunks and
//! bit depths we don't care about. ~80 LOC + zero new deps beats
//! pulling in a crate.
//!
//! Used by [`--from-file`](`crate::Command::Play`) so playback can
//! consume pre-synthesised audio instead of a live mic — closes the
//! end-to-end integration-test gap that's blocked us from covering
//! `wait_for_expected_note` + the proximity-score loop against
//! deterministic inputs.

use anyhow::{Result, anyhow};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

/// Decoded contents of a mono PCM WAV file.
pub struct WavData {
    pub sample_rate: u32,
    pub samples: Vec<f32>,
}

/// Parse a mono WAV file from disk. Accepts:
/// - PCM 16-bit integer (`fmt` audio_format = 1, bits_per_sample = 16)
/// - PCM 32-bit float (`fmt` audio_format = 3, bits_per_sample = 32)
///
/// Stereo + other formats return an error rather than silently
/// down-mixing — synth fixtures are always mono by construction,
/// and a stereo input here would mean the caller built the file
/// incorrectly.
pub fn read(path: &Path) -> Result<WavData> {
    let mut f = File::open(path).map_err(|e| anyhow!("open {}: {e}", path.display()))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)
        .map_err(|e| anyhow!("read {}: {e}", path.display()))?;
    parse(&buf).map_err(|e| anyhow!("parse {}: {e}", path.display()))
}

/// Parse WAV bytes in memory — separate from `read` so unit tests
/// can synth + verify the writer's output without touching the
/// filesystem.
pub fn parse(buf: &[u8]) -> Result<WavData> {
    if buf.len() < 44 {
        return Err(anyhow!("file too small to be a WAV ({} bytes)", buf.len()));
    }
    if &buf[0..4] != b"RIFF" || &buf[8..12] != b"WAVE" {
        return Err(anyhow!("not a RIFF/WAVE file"));
    }

    // Walk the subchunks looking for `fmt ` then `data`. The fmt
    // subchunk's payload size varies (16 for PCM int, 18 / 40 for
    // extensible formats); skipping by `subchunk_size` keeps us
    // chunk-aware rather than hardcoding offsets.
    let mut pos = 12;
    let mut audio_format: u16 = 0;
    let mut channels: u16 = 0;
    let mut sample_rate: u32 = 0;
    let mut bits_per_sample: u16 = 0;
    let mut data: &[u8] = &[];

    while pos + 8 <= buf.len() {
        let id = &buf[pos..pos + 4];
        let size =
            u32::from_le_bytes([buf[pos + 4], buf[pos + 5], buf[pos + 6], buf[pos + 7]]) as usize;
        let payload_start = pos + 8;
        let payload_end = payload_start + size;
        if payload_end > buf.len() {
            return Err(anyhow!("chunk '{}' truncated", String::from_utf8_lossy(id)));
        }
        match id {
            b"fmt " => {
                if size < 16 {
                    return Err(anyhow!("fmt chunk too small ({size} bytes)"));
                }
                audio_format = u16::from_le_bytes([buf[payload_start], buf[payload_start + 1]]);
                channels = u16::from_le_bytes([buf[payload_start + 2], buf[payload_start + 3]]);
                sample_rate = u32::from_le_bytes([
                    buf[payload_start + 4],
                    buf[payload_start + 5],
                    buf[payload_start + 6],
                    buf[payload_start + 7],
                ]);
                bits_per_sample =
                    u16::from_le_bytes([buf[payload_start + 14], buf[payload_start + 15]]);
            }
            b"data" => {
                data = &buf[payload_start..payload_end];
            }
            _ => {} // skip unknown chunks (LIST, INFO, etc)
        }
        // Chunks are padded to even-byte boundary per RIFF spec.
        pos = payload_end + (size & 1);
    }

    if channels != 1 {
        return Err(anyhow!("expected mono ({}-channel file)", channels));
    }
    let samples: Vec<f32> = match (audio_format, bits_per_sample) {
        (1, 16) => {
            // PCM signed 16-bit. Scale to [-1.0, 1.0].
            data.chunks_exact(2)
                .map(|c| {
                    let n = i16::from_le_bytes([c[0], c[1]]);
                    n as f32 / 32768.0
                })
                .collect()
        }
        (3, 32) => {
            // IEEE 32-bit float — already in our representation.
            data.chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect()
        }
        _ => {
            return Err(anyhow!(
                "unsupported format (audio_format={audio_format}, bits={bits_per_sample}); \
                 want PCM int-16 or float-32"
            ));
        }
    };

    Ok(WavData {
        sample_rate,
        samples,
    })
}

/// Write a mono f32 sample buffer as an IEEE 32-bit float WAV.
/// Counterpart to [`read`] used by tests that synth fixture audio.
///
/// `#[allow(dead_code)]` because the bin target doesn't call it —
/// only the `tests/` integration suite does, through the lib target
/// re-export. Cargo lints the bin in isolation and doesn't see
/// those uses; rather than feature-gating, accept the dead-code
/// flag here.
#[allow(dead_code)]
pub fn write(path: &Path, sample_rate: u32, samples: &[f32]) -> Result<()> {
    let mut f = File::create(path).map_err(|e| anyhow!("create {}: {e}", path.display()))?;
    let bytes = build(sample_rate, samples);
    f.write_all(&bytes)
        .map_err(|e| anyhow!("write {}: {e}", path.display()))?;
    Ok(())
}

/// Build the WAV byte stream in memory. Pulled out of `write` so
/// tests can verify the bytes without filesystem IO.
///
/// `#[allow(dead_code)]` — same reason as [`write`]: only the
/// integration test harness reaches for it, via the lib target.
#[allow(dead_code)]
pub fn build(sample_rate: u32, samples: &[f32]) -> Vec<u8> {
    let bytes_per_sample = 4u32; // f32
    let channels = 1u16;
    let byte_rate = sample_rate * bytes_per_sample;
    let block_align = (channels as u32 * bytes_per_sample) as u16;
    let data_size = (samples.len() as u32) * bytes_per_sample;
    let fmt_size = 16u32;
    let riff_size = 4 + (8 + fmt_size) + (8 + data_size);

    let mut out = Vec::with_capacity(44 + data_size as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_size.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&fmt_size.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes()); // audio_format = IEEE float
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&((bytes_per_sample * 8) as u16).to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_size.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_float_wav_preserves_samples_and_rate() {
        // Hand-build a known sample vector, write to bytes, parse
        // back. Tests both writer and reader against each other
        // (and against the WAV spec implicitly — if either side
        // gets the chunk layout wrong, parse fails).
        let samples: Vec<f32> = (0..1000).map(|i| (i as f32 / 1000.0).sin()).collect();
        let bytes = build(48_000, &samples);
        let parsed = parse(&bytes).expect("parse round-trip");
        assert_eq!(parsed.sample_rate, 48_000);
        assert_eq!(parsed.samples.len(), samples.len());
        for (a, b) in parsed.samples.iter().zip(samples.iter()) {
            assert!((a - b).abs() < 1e-6, "sample mismatch: {a} vs {b}");
        }
    }

    #[test]
    fn parse_rejects_non_wav_data() {
        assert!(parse(b"not a wav at all").is_err());
        assert!(parse(b"RIFF\x00\x00\x00\x00MIDIfmt ").is_err());
    }

    #[test]
    fn parse_rejects_stereo() {
        // Build the same WAV but with channels=2; should be rejected
        // explicitly rather than silently down-mixing.
        let mut bytes = build(48_000, &[0.0; 100]);
        // channels field is at offset 22 (LE u16).
        bytes[22] = 2;
        assert!(parse(&bytes).is_err());
    }

    #[test]
    fn parse_accepts_int16_pcm_wav() {
        // Build a minimal int-16 WAV by hand (the writer only emits
        // float-32, so we synthesise the int-16 case manually to
        // confirm the reader handles it too — useful when consuming
        // WAVs produced by tools that default to int-16).
        let sample_rate = 48_000u32;
        let samples_i16: Vec<i16> = vec![0, 16384, -16384, 32767, -32768];
        let data_size = (samples_i16.len() * 2) as u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(4u32 + 24 + 8 + data_size).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes()); // audio_format = PCM int
        bytes.extend_from_slice(&1u16.to_le_bytes()); // channels
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte_rate
        bytes.extend_from_slice(&2u16.to_le_bytes()); // block_align
        bytes.extend_from_slice(&16u16.to_le_bytes()); // bits_per_sample
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_size.to_le_bytes());
        for s in &samples_i16 {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        let parsed = parse(&bytes).expect("parse int-16");
        assert_eq!(parsed.sample_rate, sample_rate);
        // Spot-check the int→float conversion at the extremes.
        assert!((parsed.samples[0] - 0.0).abs() < 1e-6);
        assert!((parsed.samples[3] - 32767.0 / 32768.0).abs() < 1e-6);
        assert!((parsed.samples[4] - -1.0).abs() < 1e-6);
    }

    #[test]
    fn parse_handles_unknown_chunks_in_the_middle() {
        // Hand-build a WAV with a LIST chunk between fmt and data —
        // commonly emitted by tools that store metadata in the file.
        // The reader should skip it and find the data chunk regardless.
        let mut bytes = build(48_000, &[0.0; 10]);
        // Splice a fake LIST chunk in just before the data chunk.
        let data_pos = bytes.windows(4).position(|w| w == b"data").unwrap();
        let mut spliced = Vec::with_capacity(bytes.len() + 16);
        spliced.extend_from_slice(&bytes[..data_pos]);
        spliced.extend_from_slice(b"LIST");
        spliced.extend_from_slice(&8u32.to_le_bytes());
        spliced.extend_from_slice(b"INFO");
        spliced.extend_from_slice(b"\x00\x00\x00\x00");
        spliced.extend_from_slice(&bytes[data_pos..]);
        // RIFF size needs updating, but our parser doesn't validate
        // it strictly so the test passes regardless. (If it ever
        // does start to: the recompute is `4 + total_subchunks`.)
        bytes = spliced;
        let parsed = parse(&bytes).expect("parse with LIST chunk");
        assert_eq!(parsed.samples.len(), 10);
    }
}
