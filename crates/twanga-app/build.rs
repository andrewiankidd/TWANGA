// Tauri build hook — emits the platform-specific glue (Windows app manifest,
// macOS Info.plist, Linux .desktop entries) that lets the produced binary
// behave as a real OS-integrated app rather than a bare exe. Reads from
// `tauri.conf.json` next to this file.
fn main() {
    tauri_build::build()
}
