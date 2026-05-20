//! TWANGA's Tauri shell. Hosts the shared `frontend/web/` bundle (alphaTab
//! renderer, Web Audio capture, twanga-web WASM bindings) inside a native
//! webview on Windows / macOS / Linux today, iOS / Android once Tauri Mobile
//! lands.
//!
//! The web build at `frontend/web/` is the same bundle — Tauri just serves
//! it locally from the bundled assets and wraps it in an OS window. Same UI
//! code, same WASM artifacts, same DSP. The desktop story is "one frontend,
//! shipped two ways," not a separate native UI.
//!
//! What the shell adds beyond the browser:
//!   - **Filesystem-backed library** at `$CONFIG/twanga/recordings/`, so
//!     recordings made in the GUI show up alongside those made via
//!     `twanga record` on the CLI. Replaces IndexedDB.
//!   - **Filesystem-backed user tunings** at `$CONFIG/twanga/tunings.toml`
//!     (the same file `twanga tunings add` writes), so a custom tuning
//!     defined in the GUI is immediately visible to the CLI and vice
//!     versa. Replaces `localStorage`.
//!   - **`@tauri-apps/plugin-shell`** so external link clicks (e.g. the
//!     personal-site footer) open in the OS browser instead of
//!     navigating away from the app.
//!
//! See `src/commands.rs` for the filesystem commands' Rust side and
//! `frontend/web/lib/library-tauri.js` / `frontend/web/lib/user-tunings-tauri.js`
//! for the JS wrappers.

mod commands;

/// Tauri 2 mobile entry points (`mobile_entry_point` on iOS, `cdylib` JNI on
/// Android) want a `run()` function they can call into. Keeping the work in
/// `lib.rs` means we can `pub use` the same fn for the desktop bin and for
/// mobile glue without a refactor when we wire Tauri Mobile.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Shell plugin — used by the frontend's external-link
        // interceptor to open URLs in the OS browser. Capabilities
        // are declared in `capabilities/default.json`.
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![
            commands::list_recordings,
            commands::load_recording,
            commands::save_recording,
            commands::update_recording,
            commands::delete_recording,
            commands::read_tunings_toml,
            commands::write_tunings_toml,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TWANGA");
}
