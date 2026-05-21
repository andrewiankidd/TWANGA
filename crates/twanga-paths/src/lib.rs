//! TWANGA's user-data path resolution.
//!
//! Every TWANGA surface (CLI, Tauri desktop shell) that persists user
//! data routes through this crate so the location is consistent and
//! the portable-install workflow has a single source of truth.
//!
//! # Layout
//!
//! ```text
//! <root>/
//!     tunings.toml        ← user-defined tunings (CLI + Tauri share this)
//!     play-resume.toml    ← per-file resume bookmarks
//!     recordings/         ← output of `twanga record` + GUI recorder
//!         <slug>-<ts>.alphatex
//!     library/            ← imported tabs (GUI Importer + CLI `twanga import`)
//!         <slug>.alphatex
//! ```
//!
//! `<root>` resolves in one of two modes, decided once at startup:
//!
//! - **Home mode (default)** — `~/twanga/` (or the platform-equivalent
//!   home dir). Visible, lower-case, no platform-specific config-dir
//!   indirection — same string on Windows / macOS / Linux.
//! - **Portable mode** — `<binary-dir>/twanga-data/`. Activated when a
//!   `twanga.portable` sentinel file sits next to the binary. Keeps the
//!   install self-contained on a USB stick or a `Program Files` mirror
//!   without polluting the user's home dir.
//!
//! Mobile (iOS / Android) and web don't use this crate — they have their
//! own sandboxed storage (Tauri Mobile filesystem APIs, IndexedDB +
//! localStorage on the web).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Filename of the portable-mode sentinel. Present in `<binary-dir>`
/// (or alongside the .AppImage file on Linux) → activates portable
/// mode. Shipped INSIDE the portable distribution artefacts; absent
/// from the installer / DMG / MSI artefacts.
pub const SENTINEL_FILE: &str = "twanga.portable";

/// Directory name appended to `<binary-dir>` in portable mode.
pub const PORTABLE_DATA_DIR: &str = "twanga-data";

/// Directory name appended to the user's home dir in default mode.
/// Visible (no leading dot) so Windows-Explorer users can find it
/// without enabling hidden-file display.
pub const HOME_DATA_DIR: &str = "twanga";

/// Resolved location of the user-data root. See module docs for the
/// two modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataRoot {
    /// `~/<HOME_DATA_DIR>/` — the default install case.
    Home(PathBuf),
    /// `<binary-dir>/<PORTABLE_DATA_DIR>/` — sentinel detected.
    Portable(PathBuf),
}

impl DataRoot {
    /// The root directory itself. Both variants own a fully-resolved
    /// absolute path.
    pub fn root(&self) -> &Path {
        match self {
            DataRoot::Home(p) | DataRoot::Portable(p) => p,
        }
    }

    /// `<root>/tunings.toml` — the user tunings file. Same TOML schema
    /// as the built-in `presets.toml` baked into twanga-core.
    pub fn tunings_path(&self) -> PathBuf {
        self.root().join("tunings.toml")
    }

    /// `<root>/play-resume.toml` — per-file resume bookmarks.
    pub fn play_resume_path(&self) -> PathBuf {
        self.root().join("play-resume.toml")
    }

    /// `<root>/recordings/` — output of `twanga record` and the GUI
    /// recorder. Created on demand by callers.
    pub fn recordings_dir(&self) -> PathBuf {
        self.root().join("recordings")
    }

    /// `<root>/library/` — destination for the importer (GUI Importer
    /// screen + future `twanga import` CLI command). Imports are
    /// distinct from recordings because they didn't originate on this
    /// machine.
    pub fn library_dir(&self) -> PathBuf {
        self.root().join("library")
    }

    /// True iff the sentinel was detected at startup.
    pub fn is_portable(&self) -> bool {
        matches!(self, DataRoot::Portable(_))
    }
}

/// Resolve and cache the data root. Resolution looks at the running
/// binary's directory (or `$APPIMAGE` on Linux AppImage installs) for
/// a `twanga.portable` sentinel; if present, portable mode wins. If
/// neither portable detection nor the home-dir lookup succeed (which
/// would require both a broken `current_exe()` AND a missing home env
/// var — vanishingly rare), returns `None`.
///
/// The result is cached after the first call so subsequent lookups
/// don't redo the env-var / filesystem checks. The cache is process-
/// global; tests that want to override it should use the `*_at`
/// helpers below directly.
pub fn data_root() -> Option<DataRoot> {
    static CACHE: OnceLock<Option<DataRoot>> = OnceLock::new();
    CACHE.get_or_init(resolve_data_root).clone()
}

fn resolve_data_root() -> Option<DataRoot> {
    if let Some(portable_dir) = current_portable_dir() {
        return Some(DataRoot::Portable(portable_dir.join(PORTABLE_DATA_DIR)));
    }
    let home = directories::UserDirs::new()?.home_dir().to_path_buf();
    Some(DataRoot::Home(home.join(HOME_DATA_DIR)))
}

/// Return the directory that should host the sentinel — either the
/// running binary's directory, or `$APPIMAGE`'s parent on Linux
/// AppImage installs. Only returns `Some` if the sentinel actually
/// exists; that way the caller can fall through to home mode.
fn current_portable_dir() -> Option<PathBuf> {
    let candidate_dir = if let Some(appimage) = std::env::var_os("APPIMAGE") {
        // AppImage runtime exports the path to the .AppImage file on
        // the user's filesystem. Its parent dir is "where the user
        // put the AppImage" — that's where they'd drop the sentinel.
        // current_exe() inside an AppImage points into the squashfs
        // mount, which is the wrong dir to look in.
        PathBuf::from(appimage).parent()?.to_path_buf()
    } else {
        std::env::current_exe().ok()?.parent()?.to_path_buf()
    };
    portable_dir_at(&candidate_dir, SENTINEL_FILE)
}

/// Test-friendly portable-dir check. Returns `Some(dir)` iff
/// `<dir>/<sentinel>` exists. The unit tests exercise this directly
/// against a `tempfile::tempdir()` so they don't depend on
/// `current_exe()` or `$APPIMAGE`.
pub fn portable_dir_at(dir: &Path, sentinel: &str) -> Option<PathBuf> {
    if dir.join(sentinel).exists() {
        Some(dir.to_path_buf())
    } else {
        None
    }
}

/// Build a `DataRoot` from explicit inputs — for code that needs to
/// reason about specific paths without going through the cached
/// global. Mirrors `resolve_data_root` but takes the binary-dir +
/// home-dir as parameters so it's unit-testable.
pub fn data_root_from(
    binary_dir: Option<&Path>,
    home_dir: Option<&Path>,
    sentinel: &str,
) -> Option<DataRoot> {
    if let Some(dir) = binary_dir
        && let Some(portable) = portable_dir_at(dir, sentinel)
    {
        return Some(DataRoot::Portable(portable.join(PORTABLE_DATA_DIR)));
    }
    let home = home_dir?;
    Some(DataRoot::Home(home.join(HOME_DATA_DIR)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn home_mode_default_when_no_sentinel() {
        let bin = tempdir().unwrap();
        let home = tempdir().unwrap();
        let root = data_root_from(Some(bin.path()), Some(home.path()), SENTINEL_FILE).unwrap();
        assert!(matches!(root, DataRoot::Home(_)));
        assert_eq!(root.root(), home.path().join(HOME_DATA_DIR));
        assert!(!root.is_portable());
    }

    #[test]
    fn portable_mode_when_sentinel_present() {
        let bin = tempdir().unwrap();
        let home = tempdir().unwrap();
        fs::write(bin.path().join(SENTINEL_FILE), b"").unwrap();
        let root = data_root_from(Some(bin.path()), Some(home.path()), SENTINEL_FILE).unwrap();
        assert!(root.is_portable());
        assert_eq!(root.root(), bin.path().join(PORTABLE_DATA_DIR));
    }

    #[test]
    fn portable_wins_over_home_when_both_resolvable() {
        // Sentinel + home both resolvable → portable. Captures the
        // "USB-stick install while also having a home dir" case.
        let bin = tempdir().unwrap();
        let home = tempdir().unwrap();
        fs::write(bin.path().join(SENTINEL_FILE), b"").unwrap();
        let root = data_root_from(Some(bin.path()), Some(home.path()), SENTINEL_FILE).unwrap();
        assert!(root.is_portable());
    }

    #[test]
    fn falls_through_to_home_when_binary_dir_missing() {
        // current_exe() failed → no binary_dir → must use home.
        let home = tempdir().unwrap();
        let root = data_root_from(None, Some(home.path()), SENTINEL_FILE).unwrap();
        assert!(matches!(root, DataRoot::Home(_)));
    }

    #[test]
    fn returns_none_when_neither_resolvable() {
        assert!(data_root_from(None, None, SENTINEL_FILE).is_none());
    }

    #[test]
    fn portable_dir_at_returns_none_without_sentinel() {
        let dir = tempdir().unwrap();
        assert_eq!(portable_dir_at(dir.path(), SENTINEL_FILE), None);
    }

    #[test]
    fn portable_dir_at_returns_dir_when_sentinel_present() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(SENTINEL_FILE), b"").unwrap();
        assert_eq!(
            portable_dir_at(dir.path(), SENTINEL_FILE),
            Some(dir.path().to_path_buf())
        );
    }

    #[test]
    fn subdir_paths_descend_from_root() {
        // Verify the four per-feature paths really live under the
        // resolved root and aren't accidentally absolute somewhere.
        let bin = tempdir().unwrap();
        let home = tempdir().unwrap();
        let root = data_root_from(Some(bin.path()), Some(home.path()), SENTINEL_FILE).unwrap();
        let r = root.root().to_path_buf();
        assert_eq!(root.tunings_path(), r.join("tunings.toml"));
        assert_eq!(root.play_resume_path(), r.join("play-resume.toml"));
        assert_eq!(root.recordings_dir(), r.join("recordings"));
        assert_eq!(root.library_dir(), r.join("library"));
    }

    #[test]
    fn home_root_name_is_unprefixed() {
        // The visible-by-default constraint: no leading dot. Windows
        // Explorer doesn't hide dot-prefixed dirs by default but
        // many third-party file managers do, and the Unix
        // convention of `~/.app/` was specifically rejected.
        assert_eq!(HOME_DATA_DIR, "twanga");
        assert!(!HOME_DATA_DIR.starts_with('.'));
    }
}
