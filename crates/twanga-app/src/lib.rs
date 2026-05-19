//! TWANGA's Tauri shell. Hosts the shared `frontend/web/` bundle (alphaTab
//! renderer, Web Audio capture, twanga-web WASM bindings) inside a native
//! webview on Windows / macOS / Linux today, iOS / Android once Tauri Mobile
//! lands.
//!
//! The web build at `frontend/web/` is the same bundle — Tauri just serves
//! it locally from the bundled assets and wraps it in an OS window. Same UI
//! code, same WASM artifacts, same DSP. The desktop story is "one frontend,
//! shipped two ways," not a separate native UI.

/// Tauri 2 mobile entry points (`mobile_entry_point` on iOS, `cdylib` JNI on
/// Android) want a `run()` function they can call into. Keeping the work in
/// `lib.rs` means we can `pub use` the same fn for the desktop bin and for
/// mobile glue without a refactor when we wire Tauri Mobile.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running TWANGA");
}
