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
//!       recordings/           ← `twanga record` output + GUI recorder
//!           <slug>-<ts>.alphatex
//!       library/              ← imported tabs (GUI Importer + `twanga import`)
//!           <slug>-<ts>.alphatex
//!
//! Portable mode (sentinel `twanga.portable` next to the binary)
//! swaps the prefix for `<binary-dir>/twanga-data/` but the rest of
//! the layout is identical.
//!
//! Tab ids on the Tauri side are filenames (e.g.
//! `my-take-1779133041.alphatex`). The JS shim prefixes them with
//! `recording:` or `library:` before exposing them to the rest of
//! the frontend so subsequent load / update / delete calls know
//! which dir to look in — see `library-tauri.js`.

use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// One row in the library list — filesystem-backed mirror of the
/// browser IDB row shape. Used by `list_recordings` and
/// `list_library_tabs`; the `source` field is what tells the
/// frontend whether this came from `recordings/` or `library/`.
#[derive(Debug, Clone, Serialize)]
pub struct LocalTabRow {
    pub id: String,
    pub title: String,
    pub source: String,
    /// Unix millis (matches the IDB backend's `createdAt`).
    pub created_at: Option<u64>,
    pub last_backed_up_at: Option<u64>,
}

/// Full content + metadata for a single tab. Returned by
/// `load_recording` and `load_library_tab`. Same shape as
/// `library.load()`'s return in `library.js`.
#[derive(Debug, Clone, Serialize)]
pub struct LocalTabFull {
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

/// `<data-root>/library/`. Created on demand. Distinct from
/// `recordings_dir` so the file-system mirrors the data model:
/// recordings are live captures from this machine, library entries
/// are tabs imported from external files.
fn library_dir() -> Result<PathBuf> {
    let dir = data_root()?.library_dir();
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
/// every file in the dir.
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
        .unwrap_or("tab")
        .to_string()
}

// ──────────────────────────── Generic per-dir engine ────────────────────────

/// List every `.alphatex` file in `dir`, newest first (mtime DESC,
/// matching the GUI's IDB ordering). `source` is the tag attached to
/// each row — `"user"` for recordings, `"imported"` for library tabs.
fn list_in_dir(dir: &Path, source: &str) -> Result<Vec<LocalTabRow>> {
    let mut rows: Vec<(LocalTabRow, SystemTime)> = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
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
            LocalTabRow {
                id,
                title,
                source: source.to_string(),
                created_at,
                last_backed_up_at: None,
            },
            modified,
        ));
    }
    rows.sort_by_key(|r| std::cmp::Reverse(r.1));
    Ok(rows.into_iter().map(|(r, _)| r).collect())
}

/// Read a single tab file from `dir`. `id` is a bare filename; path-
/// traversal protection runs first via [`resolve_in_dir`].
fn load_from_dir(dir: &Path, source: &str, id: &str) -> Result<LocalTabFull> {
    let path = resolve_in_dir(dir, id)?;
    if !path.exists() {
        return Err(anyhow!("no tab with id '{id}' in {}", dir.display()));
    }
    let alphatex =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let title = title_or_stem(&path);
    let created_at = mtime_millis(&path);
    Ok(LocalTabFull {
        id: id.to_string(),
        title,
        source: source.to_string(),
        alphatex,
        created_at,
        last_backed_up_at: None,
    })
}

/// Write a new `.alphatex` file into `dir`. Filename is derived
/// from `title` (slugified) with a unix-seconds suffix; when the
/// title is blank or slugifies to an empty string we fall back to
/// `<default_stem>-<ts>.alphatex` (e.g. `recording-` / `imported-`).
/// Returns the bare filename.
fn save_to_dir(dir: &Path, default_stem: &str, title: &str, alphatex: &str) -> Result<String> {
    if alphatex.is_empty() {
        return Err(anyhow!("alphatex is empty"));
    }
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let slug = slugify(title);
    let filename = if slug.is_empty() {
        format!("{default_stem}-{ts}.alphatex")
    } else {
        format!("{slug}-{ts}.alphatex")
    };
    let path = dir.join(&filename);
    fs::write(&path, alphatex).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(filename)
}

/// In-place overwrite of an existing file at `dir/<id>`.
fn update_in_dir(dir: &Path, id: &str, alphatex: &str) -> Result<()> {
    if alphatex.is_empty() {
        return Err(anyhow!("alphatex is empty"));
    }
    let path = resolve_in_dir(dir, id)?;
    if !path.exists() {
        return Err(anyhow!("no tab with id '{id}' in {}", dir.display()));
    }
    fs::write(&path, alphatex).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Delete the file at `dir/<id>`. No-op (not error) when the file
/// doesn't exist — matches the IDB backend's behaviour.
fn delete_in_dir(dir: &Path, id: &str) -> Result<()> {
    let path = resolve_in_dir(dir, id)?;
    if !path.exists() {
        return Ok(());
    }
    fs::remove_file(&path).with_context(|| format!("failed to delete {}", path.display()))?;
    Ok(())
}

/// Resolve `id` (a bare filename) to an absolute path under `dir`.
/// Refuses path traversal — the frontend should only ever pass
/// filenames that came out of a `list_*` call, but a hostile or
/// buggy caller shouldn't be able to escape the dir.
fn resolve_in_dir(dir: &Path, id: &str) -> Result<PathBuf> {
    if id.contains('/') || id.contains('\\') || id.contains("..") || id.is_empty() {
        return Err(anyhow!(
            "invalid tab id '{id}' (must be a bare filename, no path separators)"
        ));
    }
    Ok(dir.join(id))
}

// ──────────────────────────── Recordings commands ───────────────────────────
//
// Thin wrappers around the generic engine, fixed to the recordings
// dir + `"user"` source + `"recording"` fallback slug.

/// `list_recordings` — return every `.alphatex` file in
/// `<data-root>/recordings/`. Newest first.
#[tauri::command]
pub fn list_recordings() -> std::result::Result<Vec<LocalTabRow>, String> {
    let dir = recordings_dir().map_err(|e| e.to_string())?;
    list_in_dir(&dir, "user").map_err(|e| e.to_string())
}

/// `load_recording(id)` — id is the bare filename, e.g.
/// `my-take-1779133041.alphatex`. Refuses path traversal.
#[tauri::command]
pub fn load_recording(id: String) -> std::result::Result<LocalTabFull, String> {
    let dir = recordings_dir().map_err(|e| e.to_string())?;
    load_from_dir(&dir, "user", &id).map_err(|e| e.to_string())
}

/// `save_recording(title, alphatex, source?)` → returns the new
/// filename. `source` is accepted for API parity but unused — every
/// file written here is by definition a recording.
#[tauri::command]
pub fn save_recording(
    title: String,
    alphatex: String,
    _source: Option<String>,
) -> std::result::Result<String, String> {
    let dir = recordings_dir().map_err(|e| e.to_string())?;
    save_to_dir(&dir, "recording", &title, &alphatex).map_err(|e| e.to_string())
}

/// `update_recording(id, title?, alphatex)` — overwrites the file
/// at `recordings/<id>`. `title` is accepted but unused on the
/// Rust side (alphaTex's `\title` IS the title; the Editor inlines
/// it into the body before calling update).
#[tauri::command]
pub fn update_recording(
    id: String,
    _title: Option<String>,
    alphatex: String,
) -> std::result::Result<(), String> {
    let dir = recordings_dir().map_err(|e| e.to_string())?;
    update_in_dir(&dir, &id, &alphatex).map_err(|e| e.to_string())
}

/// `delete_recording(id)` — removes the file. No-op if it doesn't
/// exist.
#[tauri::command]
pub fn delete_recording(id: String) -> std::result::Result<(), String> {
    let dir = recordings_dir().map_err(|e| e.to_string())?;
    delete_in_dir(&dir, &id).map_err(|e| e.to_string())
}

// ──────────────────────────── Library commands ──────────────────────────────
//
// Same surface as the recordings commands, against `<data-root>/library/`
// with `"imported"` source + `"imported"` fallback slug. The JS shim
// (library-tauri.js) routes by id prefix: `library:` → these
// commands, `recording:` → the ones above.

/// `list_library_tabs` — every `.alphatex` file in
/// `<data-root>/library/`, newest first.
#[tauri::command]
pub fn list_library_tabs() -> std::result::Result<Vec<LocalTabRow>, String> {
    let dir = library_dir().map_err(|e| e.to_string())?;
    list_in_dir(&dir, "imported").map_err(|e| e.to_string())
}

/// `load_library_tab(id)` — id is the bare filename. Refuses path
/// traversal.
#[tauri::command]
pub fn load_library_tab(id: String) -> std::result::Result<LocalTabFull, String> {
    let dir = library_dir().map_err(|e| e.to_string())?;
    load_from_dir(&dir, "imported", &id).map_err(|e| e.to_string())
}

/// `save_library_tab(title, alphatex)` → returns the new filename.
/// Used by the Importer (GUI screen + `twanga import` parity layer).
#[tauri::command]
pub fn save_library_tab(title: String, alphatex: String) -> std::result::Result<String, String> {
    let dir = library_dir().map_err(|e| e.to_string())?;
    save_to_dir(&dir, "imported", &title, &alphatex).map_err(|e| e.to_string())
}

/// `update_library_tab(id, title?, alphatex)` — overwrites
/// `library/<id>`. Imported tabs are user-owned, so the Editor's
/// "Save" lands here just like it does for recordings.
#[tauri::command]
pub fn update_library_tab(
    id: String,
    _title: Option<String>,
    alphatex: String,
) -> std::result::Result<(), String> {
    let dir = library_dir().map_err(|e| e.to_string())?;
    update_in_dir(&dir, &id, &alphatex).map_err(|e| e.to_string())
}

/// `delete_library_tab(id)` — removes the file. No-op if absent.
#[tauri::command]
pub fn delete_library_tab(id: String) -> std::result::Result<(), String> {
    let dir = library_dir().map_err(|e| e.to_string())?;
    delete_in_dir(&dir, &id).map_err(|e| e.to_string())
}

// ──────────────────────────── Tunings commands ──────────────────────────────

/// `read_tunings_toml` — return the contents of `<data-root>/tunings.toml`
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

/// `write_tunings_toml(contents)` — overwrite `<data-root>/tunings.toml`
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
    fn resolve_in_dir_rejects_traversal() {
        let dir = PathBuf::from("/tmp/twanga-test");
        assert!(resolve_in_dir(&dir, "../etc/passwd").is_err());
        assert!(resolve_in_dir(&dir, "foo/bar.alphatex").is_err());
        assert!(resolve_in_dir(&dir, "foo\\bar.alphatex").is_err());
        assert!(resolve_in_dir(&dir, "..").is_err());
        assert!(resolve_in_dir(&dir, "").is_err());
        // Bare filenames are fine.
        assert!(resolve_in_dir(&dir, "my-tab.alphatex").is_ok());
    }

    #[test]
    fn save_and_list_round_trip_in_temp_dir() {
        // Generic helpers should work against any dir, so we test
        // them against a scratch path rather than the real data-root
        // (which would touch the user's home dir during `cargo test`).
        let tmp = std::env::temp_dir().join(format!(
            "twanga-cmds-test-{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&tmp).expect("create scratch");

        let filename = save_to_dir(&tmp, "imported", "My Import", "\\title \"My Import\"\n")
            .expect("save_to_dir");
        assert!(filename.starts_with("my-import-"));
        assert!(filename.ends_with(".alphatex"));

        let rows = list_in_dir(&tmp, "imported").expect("list_in_dir");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source, "imported");
        assert_eq!(rows[0].title, "My Import");

        let full = load_from_dir(&tmp, "imported", &filename).expect("load_from_dir");
        assert_eq!(full.source, "imported");
        assert_eq!(full.alphatex, "\\title \"My Import\"\n");

        update_in_dir(&tmp, &filename, "\\title \"Updated\"\n").expect("update_in_dir");
        let after = load_from_dir(&tmp, "imported", &filename).expect("load after update");
        assert_eq!(after.alphatex, "\\title \"Updated\"\n");

        delete_in_dir(&tmp, &filename).expect("delete_in_dir");
        let rows_after = list_in_dir(&tmp, "imported").expect("list after delete");
        assert!(rows_after.is_empty());

        // Cleanup
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn save_falls_back_to_default_stem_on_empty_title() {
        let tmp = std::env::temp_dir().join(format!(
            "twanga-cmds-test-empty-{}",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        fs::create_dir_all(&tmp).expect("create scratch");

        let recording_name =
            save_to_dir(&tmp, "recording", "", "\\title \"\"\n").expect("save with blank title");
        assert!(recording_name.starts_with("recording-"));

        let imported_name =
            save_to_dir(&tmp, "imported", "   ", "\\title \"\"\n").expect("save with blank title");
        assert!(imported_name.starts_with("imported-"));

        let _ = fs::remove_dir_all(&tmp);
    }
}
