//! Realtime audio capture. Wraps CPAL.
//!
//! Samples flow from the CPAL audio-thread callback to the consumer through a
//! lock-free SPSC ring buffer (`ringbuf`). Both sides are wait-free and
//! allocation-free in steady state. The callback also downmixes to mono on the
//! way in by extracting channel 0 from each frame, so the consumer never has
//! to think about the device's native channel layout — `read()` always emits mono.

use anyhow::{Context, Result, anyhow};
use cpal::SampleFormat;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::traits::{Consumer, Producer, Split};
use ringbuf::{HeapCons, HeapProd, HeapRb};

/// Substrings (case-insensitive) that suggest a device is an instrument input
/// rather than a headset / built-in mic. First matching device wins.
const INSTRUMENT_KEYWORDS: &[&str] = &[
    "guitar",
    "ukulele",
    "uke",
    "banjo",
    "mandolin",
    "instrument",
];

pub fn list_input_devices() -> Result<Vec<String>> {
    let host = cpal::default_host();
    let mut names = Vec::new();
    for device in host.input_devices()? {
        names.push(device.name().unwrap_or_else(|_| "(unnamed)".into()));
    }
    Ok(names)
}

/// `true` if `name` (case-insensitive) contains any instrument keyword. Pure
/// function shared between the device-picking code in `open()` and the unit
/// tests below.
fn is_instrument_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    INSTRUMENT_KEYWORDS.iter().any(|kw| lower.contains(kw))
}

/// Returns the first name from `names` that matches an instrument keyword, or
/// `None` if no name matches.
#[cfg(test)]
fn pick_instrument_name<I, S>(names: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    names
        .into_iter()
        .find(|n| is_instrument_name(n.as_ref()))
        .map(|n| n.as_ref().to_string())
}

/// A live mono capture stream. Picks an instrument-input device by name if one
/// is present, otherwise falls back to the host's default input device.
///
/// The underlying CPAL stream is held in this struct and stops on drop.
pub struct InputStream {
    consumer: HeapCons<f32>,
    pub sample_rate: u32,
    /// Native channel count of the device. Informational — samples emitted by
    /// `read()` are already downmixed to mono inside the audio callback.
    pub channels: u16,
    pub device_name: String,
    _stream: cpal::Stream,
}

impl InputStream {
    pub fn open() -> Result<Self> {
        let host = cpal::default_host();

        let mut chosen: Option<cpal::Device> = None;
        if let Ok(devices) = host.input_devices() {
            for device in devices {
                if let Ok(name) = device.name() {
                    if is_instrument_name(&name) {
                        chosen = Some(device);
                        break;
                    }
                }
            }
        }
        let device = match chosen {
            Some(d) => d,
            None => host
                .default_input_device()
                .ok_or_else(|| anyhow!("no input device available"))?,
        };
        let device_name = device.name().unwrap_or_else(|_| "(unnamed)".into());

        let config = device
            .default_input_config()
            .context("failed to query default input config")?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        let channels_usize = channels as usize;
        let format = config.sample_format();
        let stream_config: cpal::StreamConfig = config.into();

        // 1 second of mono buffering at the device's native rate.
        let capacity = sample_rate as usize;
        let rb = HeapRb::<f32>::new(capacity);
        let (mut prod, cons) = rb.split();

        let err_fn = |err| eprintln!("audio input stream error: {err}");

        let stream = match format {
            SampleFormat::F32 => device.build_input_stream(
                &stream_config,
                move |samples: &[f32], _: &cpal::InputCallbackInfo| {
                    // Downmix to mono by taking channel 0 from each frame.
                    // Stack buffer flushed in 1024-sample chunks; no heap alloc.
                    let mut mono = [0.0_f32; 1024];
                    let mut idx = 0;
                    for frame in samples.chunks(channels_usize) {
                        if let Some(&s) = frame.first() {
                            mono[idx] = s;
                            idx += 1;
                            if idx == mono.len() {
                                let _ = prod.push_slice(&mono);
                                idx = 0;
                            }
                        }
                    }
                    if idx > 0 {
                        let _ = prod.push_slice(&mono[..idx]);
                    }
                },
                err_fn,
                None,
            )?,
            SampleFormat::I16 => device.build_input_stream(
                &stream_config,
                move |samples: &[i16], _: &cpal::InputCallbackInfo| {
                    let mut mono = [0.0_f32; 1024];
                    let mut idx = 0;
                    for frame in samples.chunks(channels_usize) {
                        if let Some(&s) = frame.first() {
                            mono[idx] = s as f32 / i16::MAX as f32;
                            idx += 1;
                            if idx == mono.len() {
                                let _ = prod.push_slice(&mono);
                                idx = 0;
                            }
                        }
                    }
                    if idx > 0 {
                        let _ = prod.push_slice(&mono[..idx]);
                    }
                },
                err_fn,
                None,
            )?,
            other => {
                return Err(anyhow!(
                    "unsupported sample format: {other:?}. v1 supports F32 and I16."
                ));
            }
        };

        stream.play()?;

        Ok(Self {
            consumer: cons,
            sample_rate,
            channels,
            device_name,
            _stream: stream,
        })
    }

    /// Pop up to `out.len()` mono samples into `out`. Returns the number popped.
    /// Allocation-free.
    pub fn read(&mut self, out: &mut [f32]) -> usize {
        self.consumer.pop_slice(out)
    }
}

/// A live mono audio output stream to the default output device.
///
/// Producer side: callers push mono `f32` samples via [`Self::write`]. The
/// output callback drains them into the device buffer, duplicating to every
/// output channel (so e.g. stereo speakers play the same mono signal on both
/// sides). Underrun is silent (zeros) — appropriate for sparse signals like a
/// metronome click.
pub struct OutputStream {
    producer: HeapProd<f32>,
    pub sample_rate: u32,
    pub channels: u16,
    pub device_name: String,
    _stream: cpal::Stream,
}

impl OutputStream {
    pub fn open() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| anyhow!("no default output device available"))?;
        let device_name = device.name().unwrap_or_else(|_| "(unnamed)".into());
        let config = device
            .default_output_config()
            .context("failed to query default output config")?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        let channels_usize = channels as usize;
        let format = config.sample_format();
        let stream_config: cpal::StreamConfig = config.into();

        // 1 second of mono buffering. The metronome's clicks are short and
        // sparse, so this is far more than necessary.
        let capacity = sample_rate as usize;
        let rb = HeapRb::<f32>::new(capacity);
        let (prod, mut cons) = rb.split();

        let err_fn = |err| eprintln!("audio output stream error: {err}");

        let stream = match format {
            SampleFormat::F32 => device.build_output_stream(
                &stream_config,
                move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    for frame in out.chunks_mut(channels_usize) {
                        let s = cons.try_pop().unwrap_or(0.0);
                        for ch in frame.iter_mut() {
                            *ch = s;
                        }
                    }
                },
                err_fn,
                None,
            )?,
            SampleFormat::I16 => device.build_output_stream(
                &stream_config,
                move |out: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    for frame in out.chunks_mut(channels_usize) {
                        let s = cons.try_pop().unwrap_or(0.0);
                        let s_i16 = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                        for ch in frame.iter_mut() {
                            *ch = s_i16;
                        }
                    }
                },
                err_fn,
                None,
            )?,
            other => {
                return Err(anyhow!(
                    "unsupported output sample format: {other:?}. v1 supports F32 and I16."
                ));
            }
        };

        stream.play()?;

        Ok(Self {
            producer: prod,
            sample_rate,
            channels,
            device_name,
            _stream: stream,
        })
    }

    /// Queue mono samples for playback. Returns how many samples were accepted
    /// (truncated if the ring is full). Allocation-free.
    pub fn write(&mut self, samples: &[f32]) -> usize {
        self.producer.push_slice(samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_input_devices_does_not_panic() {
        let _ = list_input_devices();
    }

    #[test]
    fn pick_instrument_matches_guitar_substring() {
        let names = ["Microphone (Headset)", "Microphone (USB Guitar Adapter)"];
        assert_eq!(
            pick_instrument_name(names),
            Some("Microphone (USB Guitar Adapter)".to_string())
        );
    }

    #[test]
    fn pick_instrument_is_case_insensitive() {
        let names = ["UKULELE Capture"];
        assert_eq!(
            pick_instrument_name(names),
            Some("UKULELE Capture".to_string())
        );
    }

    #[test]
    fn pick_instrument_returns_first_match() {
        let names = ["Builtin Mic", "Banjo Adapter", "Guitar Interface"];
        assert_eq!(
            pick_instrument_name(names),
            Some("Banjo Adapter".to_string())
        );
    }

    #[test]
    fn pick_instrument_returns_none_when_no_keyword_matches() {
        let names = ["Speakers", "Microphone (Generic USB Audio)", "HDMI"];
        assert!(pick_instrument_name(names).is_none());
    }

    #[test]
    fn pick_instrument_matches_each_known_keyword() {
        for keyword in INSTRUMENT_KEYWORDS {
            let name = format!("Some {keyword} thing");
            assert_eq!(
                pick_instrument_name([name.as_str()]),
                Some(name.clone()),
                "expected {keyword} to match"
            );
        }
    }
}
