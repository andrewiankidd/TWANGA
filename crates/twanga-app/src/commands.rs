//! Tauri commands exposed to the webview frontend. The frontend
//! (`frontend/web/`) imports `library-tauri.js` and `user-tunings-tauri.js`
//! shims that route through these commands when `window.__TAURI__` is
//! defined; otherwise it falls back to IndexedDB + localStorage (the
//! web build's behaviour).
//!
//! Storage layout — resolved by `twanga-paths`, so the desktop shell
//! and the CLI read + write the same files. Default home mode:
//!
//!   ~/twanga/
//!       tunings.toml          ← user-defined tunings (CLI + Tauri share this)
//!       recordings/
//!           <slug>-<ts>.alphatex
//!
//! Portable mode (sentinel `twanga.portable` next to the binary)
//! swaps the prefix for `<binary-dir>/twanga-data/` but the rest of
//! the layout is identical.
//!
//! Recording ids are filenames (e.g. `my-take-1779133041.alphatex`).
//! Stable across runs, unique-by-construction (the recorder adds a
//! unix-seconds suffix), and don't collide with the GUI's auto-increment
//! IDB ids because those are integers, not strings.

use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

/// One library row, mirror of the GUI's IDB row shape so the JS
/// dispatcher can swap backends without touching the consumer code.
#[derive(Debug, Clone, Serialize)]
pub struct RecordingRow {
    pub id: String,
    pub title: String,
    pub source: String,
    /// Unix millis (matches the IDB backend's `createdAt`).
    pub created_at: Option<u64>,
    pub last_backed_up_at: Option<u64>,
}

/// Full content for a single recording. Matches `library.load()`'s
/// return shape in `library.js`.
#[derive(Debug, Clone, Serialize)]
pub struct RecordingFull {
    pub id: String,
    pub title: String,
    pub source: String,
    pub alphatex: String,
    pub created_at: Option<u64>,
    pub last_backed_up_at: Option<u64>,
}

/// Resolve the user data root (home / portable). Errors only when
/// `twanga-paths` can't resolve EITHER the binary dir or the home
/// dir — a configuration so broken there's no recoverable place to
/// write to.
fn data_root() -> Result<twanga_paths::DataRoot> {
    twanga_paths::data_root().ok_or_else(|| {
        anyhow!("could not resolve TWANGA data root (no home dir, no portable sentinel)")
    })
}

/// `<data-root>/recordings/`. Created on demand.
fn recordings_dir() -> Result<PathBuf> {
    let dir = data_root()?.recordings_dir();
    fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
    Ok(dir)
}

/// `<data-root>/tunings.toml` — same file twanga-cli reads + writes
/// via the `tunings::user_tunings_path()` helper.
fn tunings_path() -> Result<PathBuf> {
    Ok(data_root()?.tunings_path())
}

/// Unix millis from a file's modification time. Returns `None` if
/// the platform doesn't expose mtime (vanishingly rare).
fn mtime_millis(path: &std::path::Path) -> Option<u64> {
    let meta = fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let duration = modified.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    Some(duration.as_millis() as u64)
}

/// Cheap title extraction. Read the first ~20 lines looking for a
/// `\title "..."` directive; fall back to the filename stem.
/// Avoids a full alphaTex parse for the common case of listing
/// every file in the recordings dir.
fn title_or_stem(path: &std::path::Path) -> String {
    if let Ok(text) = fs::read_to_string(path) {
        for line in text.lines().take(20) {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("\\title ") {
                let rest = rest.trim();
                let stripped = rest
                    .strip_prefix('"')
                    .and_then(|s| s.strip_suffix('"'))
                    .unwrap_or(rest);
                if !stripped.is_empty() {
                    return stripped.to_string();
                }
            }
        }
    }
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("recording")
        .to_string()
}

/// `list_recordings` — return every `.alphatex` file in
/// `$CONFIG/twanga/recordings/`. Newest first (mirrors the GUI's
/// `createdAt DESC` IDB ordering).
#[tauri::command]
pub fn list_recordings() -> std::result::Result<Vec<RecordingRow>, String> {
    list_recordings_impl().map_err(|e| e.to_string())
}

fn list_recordings_impl() -> Result<Vec<RecordingRow>> {
    let dir = recordings_dir()?;
    let mut rows: Vec<(RecordingRow, SystemTime)> = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("alphatex") {
            continue;
        }
        let id = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() {
            continue;
        }
        let title = title_or_stem(&path);
        let created_at = mtime_millis(&path);
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        rows.push((
            RecordingRow {
                id,
                title,
                source: "user".to_string(),
                created_at,
                last_backed_up_at: None,
            },
            modified,
        ));
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.1));
    Ok(rows.into_iter().map(|(r, _)| r).collect())
}

/// `load_recording(id)` — id is the filename, e.g. `my-take-...alphatex`.
/// Refuses path traversal (no `..`, no path separators) so a hostile
/// frontend can't read arbitrary files outside the recordings dir.
#[tauri::command]
pub fn load_recording(id: String) -> std::result::Result<RecordingFull, String> {
    load_recording_impl(&id).map_err(|e| e.to_string())
}

fn load_recording_impl(id: &str) -> Result<RecordingFull> {
    let path = resolve_recording_path(id)?;
    if !path.exists() {
        return Err(anyhow!("no recording with id '{id}'"));
    }
    let alphatex =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let title = title_or_stem(&path);
    let created_at = mtime_millis(&path);
    Ok(RecordingFull {
        id: id.to_string(),
        title,
        source: "user".to_string(),
        alphatex,
        created_at,
        last_backed_up_at: None,
    })
}

/// `save_recording(title, alphatex, source?)` → returns the id (filename)
/// the file was written to. Mirrors `library.save({ title, alphatex,
/// source })` in `library.js`. The filename is derived from `title` if
/// provided, otherwise `recording-<unix-secs>.alphatex`.
#[tauri::command]
pub fn save_recording(
    title: String,
    alphatex: String,
    _source: Option<String>,
) -> std::result::Result<String, String> {
    save_recording_impl(&title, &alphatex).map_err(|e| e.to_string())
}

fn save_recording_impl(title: &str, alphatex: &str) -> Result<String> {
    if alphatex.is_empty() {
        return Err(anyhow!("alphatex is empty"));
    }
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let slug = slugify(title);
    let filename = if slug.is_empty() {
        format!("recording-{ts}.alphatex")
    } else {
        format!("{slug}-{ts}.alphatex")
    };
    let path = recordings_dir()?.join(&filename);
    fs::write(&path, alphatex).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(filename)
}

/// `update_recording(id, title?, alphatex)` — in-place overwrite. The
/// frontend Editor calls this for "Save" on a user recording.
#[tauri::command]
pub fn update_recording(
    id: String,
    _title: Option<String>,
    alphatex: String,
) -> std::result::Result<(), String> {
    update_recording_impl(&id, &alphatex).map_err(|e| e.to_string())
}

fn update_recording_impl(id: &str, alphatex: &str) -> Result<()> {
    if alphatex.is_empty() {
        return Err(anyhow!("alphatex is empty"));
    }
    let path = resolve_recording_path(id)?;
    if !path.exists() {
        return Err(anyhow!("no recording with id '{id}'"));
    }
    fs::write(&path, alphatex).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// `delete_recording(id)` — removes the file. No-op (not error) if the
/// file doesn't exist (matches the IDB backend's behaviour).
#[tauri::command]
pub fn delete_recording(id: String) -> std::result::Result<(), String> {
    delete_recording_impl(&id).map_err(|e| e.to_string())
}

fn delete_recording_impl(id: &str) -> Result<()> {
    let path = resolve_recording_path(id)?;
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(&path).with_context(|| format!("failed to delete {}", path.display()))?;
    Ok(())
}

/// `read_tunings_toml` — return the contents of `$CONFIG/twanga/tunings.toml`
/// (or an empty string if the file doesn't exist yet). The frontend's
/// tunings-tauri.js shim parses this into the same `PresetEntry` shape
/// localStorage uses.
#[tauri::command]
pub fn read_tunings_toml() -> std::result::Result<String, String> {
    read_tunings_toml_impl().map_err(|e| e.to_string())
}

fn read_tunings_toml_impl() -> Result<String> {
    let path = tunings_path()?;
    if !path.exists() {
        return Ok(String::new());
    }
    fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))
}

/// `write_tunings_toml(contents)` — overwrite `$CONFIG/twanga/tunings.toml`
/// with `contents`. The frontend serialises its localStorage map back
/// into the CLI's TOML schema before calling this.
#[tauri::command]
pub fn write_tunings_toml(contents: String) -> std::result::Result<(), String> {
    write_tunings_toml_impl(&contents).map_err(|e| e.to_string())
}

fn write_tunings_toml_impl(contents: &str) -> Result<()> {
    let path = tunings_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(&path, contents).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Resolve a recording id (filename) to an absolute path under
/// `recordings/`. Refuses path traversal — the frontend should only
/// ever pass filenames that came out of `list_recordings`, but a
/// hostile or buggy caller shouldn't be able to escape the
/// recordings dir.
fn resolve_recording_path(id: &str) -> Result<PathBuf> {
    if id.contains('/') || id.contains('\\') || id.contains("..") || id.is_empty() {
        return Err(anyhow!(
            "invalid recording id '{id}' (must be a bare filename, no path separators)"
        ));
    }
    Ok(recordings_dir()?.join(id))
}

/// Slugify a title for use in a filename. Matches twanga-cli's
/// `slugify` in spirit — lowercase, ascii letters/digits/hyphens,
/// no leading/trailing hyphen.
fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_dash = true;
    for ch in s.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if ch.is_ascii_whitespace() || matches!(ch, '-' | '_' | '/') {
            Some('-')
        } else {
            None
        };
        if let Some(c) = mapped {
            if c == '-' {
                if !last_dash {
                    out.push('-');
                    last_dash = true;
                }
            } else {
                out.push(c);
                last_dash = false;
            }
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_drops_punctuation_collapses_whitespace() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("  Title  "), "title");
        assert_eq!(slugify("Foo_Bar Baz"), "foo-bar-baz");
        assert_eq!(slugify(""), "");
        assert_eq!(slugify("---"), "");
        assert_eq!(slugify("Twinkle Twinkle"), "twinkle-twinkle");
    }

    #[test]
    fn resolve_recording_path_rejects_traversal() {
        assert!(resolve_recording_path("../etc/passwd").is_err());
        assert!(resolve_recording_path("foo/bar.alphatex").is_err());
        assert!(resolve_recording_path("foo\\bar.alphatex").is_err());
        assert!(resolve_recording_path("..").is_err());
        assert!(resolve_recording_path("").is_err());
        // Bare filenames are fine.
        assert!(resolve_recording_path("my-tab.alphatex").is_ok());
    }
}
