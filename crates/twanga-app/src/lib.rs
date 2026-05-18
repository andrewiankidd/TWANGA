//! Tauri shell — placeholder until `cargo tauri init` is run inside this crate.
//!
//! Re-exports the workspace-shared MOTD splash list from `twanga-core` so the
//! eventual Tauri main menu and the CLI banner draw from the same one source.

pub use twanga_core::{SPLASHES, splashes};
