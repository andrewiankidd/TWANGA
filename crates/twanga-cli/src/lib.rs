//! Library surface for the integration test harness — exposes the
//! WAV I/O + audio-source modules so `tests/play_from_file.rs` can
//! synth fixtures and feed them through the binary without
//! duplicating the WAV writer.
//!
//! The main binary (`src/main.rs`) is the primary consumer of these
//! modules and uses them directly via their `mod` declarations. The
//! lib re-exports them for the test harness; nothing else should
//! depend on this crate.

pub mod audio_source;
pub mod wav;
