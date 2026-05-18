# twanga-bench

Latency and pitch-detection accuracy benchmark binary.

Distinct from `cargo bench` (which is for micro-benchmarks of pure functions) — this is for end-to-end measurement of capture → detection latency under real hardware conditions, the kind of run done once when porting to a new platform or evaluating a different pitch-detection algorithm.

Currently a placeholder; `main` prints a stub message.

- **Check**: `cargo check -p twanga-bench`
- **Run**: `cargo run -p twanga-bench --release`
- **Depends on**: `twanga-core`, `twanga-dsp`, `twanga-audio`, `anyhow`

See [the workspace README](../../README.md) for project context.
