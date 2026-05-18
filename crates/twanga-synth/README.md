# twanga-synth

Pure audio synthesis primitives — sine waves, harmonic stacks, envelopes, deterministic xorshift noise.

Used as a `dev-dependency` of `twanga-dsp` to drive pitch-detection tests with known synthetic signals. Math-anchored against arithmetic at `fs/4` and `fs/6` (closed-form sample patterns) so the synth itself can't silently drift away from what `sin()` actually computes — if the anchor tests pass, every YIN test downstream is using calibrated input.

- **Check**: `cargo check -p twanga-synth`
- **Test**: `cargo test -p twanga-synth`
- **Depends on**: `twanga-core`
- **Used by**: `twanga-dsp` (dev-dep, tests only)

See [the workspace README](../../README.md) for project context.
