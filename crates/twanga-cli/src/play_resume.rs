//! Per-file resume bookmarks for `twanga play`. Mirror of the GUI's
//! `localStorage` resume map (`twanga-playback-resume-v1`) — each
//! entry maps an absolute `.alphatex` path to the column the user
//! was on when they stopped, the file's title (for display), and a
//! `when` timestamp so we can age out / sort.
//!
//! Stored at `$CONFIG/twanga/play-resume.toml` (alongside
//! `tunings.toml`). TOML for human-readability; the file is small.
//!
//! Same shape on both sides — a future Tauri filesystem command
//! could sync this with the browser's localStorage without
//! translation.

use anyhow::{Context, Result, anyhow};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// One saved bookmark. `path` is stored as an absolute path so a
/// later `twanga play foo.alphatex` from a different working
/// directory still matches.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeBookmark {
    pub path: String,
    pub column: u64,
    /// File's `\title` directive at save time, for picker / prompt
    /// display. `None` if the file had no title.
    #[serde(default)]
    pub title: Option<String>,
    /// Unix timestamp in seconds. Used to surface the most-recent
    /// bookmark when multiple files have one.
    pub when: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct BookmarksFile {
    #[serde(default)]
    bookmarks: Vec<ResumeBookmark>,
}

/// Where the bookmarks TOML lives on disk. Same directory as the
/// user-tunings file. Returns `None` only on platforms where
/// `directories` can't resolve a config dir.
pub fn bookmarks_file_path() -> Option<PathBuf> {
    let dirs = ProjectDirs::from("", "", "twanga")?;
    Some(dirs.config_dir().join("play-resume.toml"))
}

/// Read all bookmarks from disk. Missing file → empty list (a fresh
/// install has no history yet — not an error).
pub fn load_at(path: &Path) -> Result<Vec<ResumeBookmark>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let parsed: BookmarksFile = toml::from_str(&text)
        .with_context(|| format!("failed to parse {} as TOML", path.display()))?;
    Ok(parsed.bookmarks)
}

/// Production wrapper for `load_at`. Exposed for future callers
/// (e.g. a `twanga play --list-resumes` flag); not consumed by the
/// current dispatch path, which goes through `lookup`.
#[allow(dead_code)]
pub fn load() -> Result<Vec<ResumeBookmark>> {
    let Some(path) = bookmarks_file_path() else {
        return Ok(Vec::new());
    };
    load_at(&path)
}

/// Write the full bookmark list to disk, creating parent dirs.
pub fn save_at(path: &Path, bookmarks: &[ResumeBookmark]) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let file = BookmarksFile {
        bookmarks: bookmarks.to_vec(),
    };
    let body = toml::to_string_pretty(&file)
        .context("failed to serialise bookmarks to TOML")?;
    fs::write(path, body).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Canonicalise a path the same way before lookups + writes, so
/// `twanga play foo.alphatex` and `twanga play ./foo.alphatex` map
/// to the same bookmark. Falls back to the input on canonicalize
/// failure (e.g. file doesn't exist yet — irrelevant for the
/// playback use case but safe).
fn key(file_path: &Path) -> String {
    fs::canonicalize(file_path)
        .unwrap_or_else(|_| file_path.to_path_buf())
        .display()
        .to_string()
}

/// Set or update the bookmark for `file_path` to `column`. Replaces
/// any existing entry for the same path.
pub fn record_at(
    bookmarks_path: &Path,
    file_path: &Path,
    column: u64,
    title: Option<String>,
) -> Result<()> {
    let mut bookmarks = load_at(bookmarks_path)?;
    let k = key(file_path);
    bookmarks.retain(|b| b.path != k);
    let when = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    bookmarks.push(ResumeBookmark {
        path: k,
        column,
        title,
        when,
    });
    save_at(bookmarks_path, &bookmarks)
}

/// Production wrapper for `record_at`.
pub fn record(file_path: &Path, column: u64, title: Option<String>) -> Result<()> {
    let Some(bp) = bookmarks_file_path() else {
        return Err(anyhow!(
            "no user config directory available — can't save play bookmark"
        ));
    };
    record_at(&bp, file_path, column, title)
}

/// Look up a bookmark for `file_path`. Returns `None` if no
/// bookmark exists for that file.
pub fn lookup_at(bookmarks_path: &Path, file_path: &Path) -> Result<Option<ResumeBookmark>> {
    let bookmarks = load_at(bookmarks_path)?;
    let k = key(file_path);
    Ok(bookmarks.into_iter().find(|b| b.path == k))
}

/// Production wrapper for `lookup_at`.
pub fn lookup(file_path: &Path) -> Result<Option<ResumeBookmark>> {
    let Some(bp) = bookmarks_file_path() else {
        return Ok(None);
    };
    lookup_at(&bp, file_path)
}

/// Remove the bookmark for `file_path` (if any). No-op if missing.
pub fn clear_at(bookmarks_path: &Path, file_path: &Path) -> Result<()> {
    let mut bookmarks = load_at(bookmarks_path)?;
    let k = key(file_path);
    let before = bookmarks.len();
    bookmarks.retain(|b| b.path != k);
    if bookmarks.len() != before {
        save_at(bookmarks_path, &bookmarks)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_path(test_name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("twanga-resume-{test_name}-{nanos}"));
        fs::create_dir_all(&dir).expect("scratch dir");
        dir.join("play-resume.toml")
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let path = scratch_path("load_missing");
        let entries = load_at(&path).expect("load");
        assert!(entries.is_empty());
    }

    #[test]
    fn record_then_lookup_round_trips() {
        let bp = scratch_path("round_trip");
        let file = std::env::temp_dir().join("some-tab.alphatex");
        // Create the file so canonicalize works.
        fs::write(&file, "").unwrap();

        record_at(&bp, &file, 17, Some("Some Tab".into())).expect("record");
        let found = lookup_at(&bp, &file).expect("lookup").expect("present");
        assert_eq!(found.column, 17);
        assert_eq!(found.title.as_deref(), Some("Some Tab"));
        // Cleanup
        let _ = fs::remove_file(&file);
    }

    #[test]
    fn record_replaces_existing_entry_for_same_file() {
        let bp = scratch_path("replace");
        let file = std::env::temp_dir().join("replace-test.alphatex");
        fs::write(&file, "").unwrap();
        record_at(&bp, &file, 5, None).expect("first record");
        record_at(&bp, &file, 23, Some("Updated".into())).expect("second record");
        let all = load_at(&bp).expect("load");
        assert_eq!(all.len(), 1, "should have replaced, not appended");
        assert_eq!(all[0].column, 23);
        let _ = fs::remove_file(&file);
    }

    #[test]
    fn clear_removes_entry() {
        let bp = scratch_path("clear");
        let file = std::env::temp_dir().join("clear-test.alphatex");
        fs::write(&file, "").unwrap();
        record_at(&bp, &file, 9, None).expect("record");
        assert!(lookup_at(&bp, &file).unwrap().is_some());
        clear_at(&bp, &file).expect("clear");
        assert!(lookup_at(&bp, &file).unwrap().is_none());
        let _ = fs::remove_file(&file);
    }

    #[test]
    fn lookup_returns_none_for_unknown_file() {
        let bp = scratch_path("unknown");
        let file = std::env::temp_dir().join("never-saved.alphatex");
        fs::write(&file, "").unwrap();
        assert!(lookup_at(&bp, &file).unwrap().is_none());
        let _ = fs::remove_file(&file);
    }
}
