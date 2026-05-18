# twanga-audio

Realtime audio capture (`InputStream`) and playback (`OutputStream`), wrapping CPAL.

`InputStream::open()` picks an instrument-input device by keyword (`guitar` / `uke` / `banjo` / `mandolin` / `instrument`) when one is available; otherwise falls back to the host's default input device. `OutputStream::open()` goes to the default output device. Samples flow between the audio-thread callbacks and the consumer/producer through lock-free SPSC ring buffers (`ringbuf`). Both sides are wait-free and allocation-free in steady state — the architecture invariant for the realtime path.

The input callback downmixes to mono on the way in (channel-0 extraction), so consumers always get a mono `read()` regardless of the device's native channel count. The output callback duplicates a single mono stream to every output channel, so `OutputStream::write(samples)` writes the same waveform to all speakers (appropriate for sparse signals like the metronome click).

- **Check**: `cargo check -p twanga-audio`
- **Test**: `cargo test -p twanga-audio`
- **Features**: `asio` — enable ASIO support on Windows via `cpal/asio` for lower-latency capture from USB instrument cables.
- **Depends on**: `twanga-core`, `cpal`, `ringbuf`, `anyhow`
- **Used by**: `twanga-cli`, `twanga-app`

## Notes

Sample formats: `F32` and `I16` only, both directions. Other formats error with a clear message at stream-open time.

On Windows, freshly-built test binaries occasionally trip Smart App Control / Defender because the test opens audio devices. If `cargo test -p twanga-audio` fails with error 4551 (`STATUS_TRUSTED_INSTALLER_REQUIRED`), add the workspace `target/` directory to Defender exclusions. The actual `twanga.exe` binary is unaffected.

See [the workspace README](../../README.md) for project context.
