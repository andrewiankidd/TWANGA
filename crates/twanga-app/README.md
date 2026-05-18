# twanga-app

Tauri shell — the desktop / mobile application crate. Will eventually host the GUI tuner, alphaTab-based tab renderer in the webview, and the chord trainer.

Currently a placeholder. The Tauri main menu's splash list is re-exported from `twanga_core::SPLASHES` (one source shared with the CLI banner in `twanga_tui::motd`), available as `twanga_app::SPLASHES` for symmetry. To bring up the actual Tauri scaffold:

```bash
cargo install tauri-cli@^2
cd crates/twanga-app
cargo tauri init
```

Then uncomment the Tauri dependencies in `Cargo.toml` and start wiring commands. The corresponding frontend lives in [`../../frontend/`](../../frontend/) (currently a placeholder — framework choice deferred).

- **Check**: `cargo check -p twanga-app`
- **Depends on**: `twanga-core` (for `SPLASHES`), `twanga-dsp`, `twanga-audio`, `anyhow`
- **Used by**: nothing (top of the dependency graph)

See [the workspace README](../../README.md) for project context.
