//! Filesystem-discovery layer for the alphaTex content the GUI's library
//! views surface: bundled examples, bundled patterns, and the user's
//! recordings directory. Used by `twanga play` (no path) and the
//! `twanga patterns` subcommand to offer interactive pickers — same
//! behaviour as the GUI's Playback library + Patterns screen.
//!
//! Source-of-truth invariants mirror the GUI:
//!   - Examples live at `assets/examples/manifest.json` + matching
//!     `.alphatex` files in the same dir.
//!   - Patterns live at `assets/patterns/manifest.json` (with
//!     groups + per-pattern difficulty) + matching `.alphatex` files.
//!   - User recordings live at `./recordings/*.alphatex` next to wherever
//!     the user ran the command — same convention `twanga record` uses
//!     when writing files out.
//!
//! All discovery is filesystem-based against the current working
//! directory. The GUI does the same thing via `fetch()` against the
//! deployed site root. A future Tauri / installed-binary path could
//! fall back to a baked-in copy via `include_str!` if the relative
//! manifests aren't found; not bothering yet because the documented
//! usage runs from the repo root.

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

/// One entry in `assets/examples/manifest.json`. Shape is dictated by
/// the GUI's reader, which we mirror here for parity.
#[derive(Debug, Clone, Deserialize)]
pub struct ExampleEntry {
    pub id: String,
    pub title: String,
    /// File path relative to the manifest dir (e.g. `"foo.alphatex"`).
    pub path: String,
    /// Slug of the source tuning (optional — older entries may lack it).
    #[serde(default)]
    pub tuning: Option<String>,
}

/// Top-level shape of `assets/examples/manifest.json`. The `//`
/// comment in the actual JSON is dropped on parse (serde_json ignores
/// `//`-prefixed keys by default — they don't deserialize to anything
/// in our struct).
#[derive(Debug, Deserialize)]
struct ExamplesManifest {
    examples: Vec<ExampleEntry>,
}

/// One pattern in `assets/patterns/manifest.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct PatternEntry {
    pub id: String,
    pub title: String,
    pub path: String,
    #[serde(default)]
    pub tuning: Option<String>,
    /// Difficulty marker for the tree-of-difficulty UI; missing /
    /// non-integer values sort to the bottom of their group.
    #[serde(default)]
    pub difficulty: Option<u32>,
}

/// One group in `assets/patterns/manifest.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct PatternGroup {
    // Manifest schema includes `id`; not consumed by any current
    // call site (we identify groups by title/order in the CLI),
    // but it's load-bearing for the GUI's keyed renders so keep
    // the field for parity with the file shape.
    #[allow(dead_code)]
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub patterns: Vec<PatternEntry>,
}

#[derive(Debug, Deserialize)]
struct PatternsManifest {
    #[serde(default)]
    groups: Vec<PatternGroup>,
}

/// One `.alphatex` file under `./recordings/`. Title is best-effort:
/// we parse the `\title` directive out of the file if present, otherwise
/// fall back to the filename stem.
#[derive(Debug, Clone)]
pub struct RecordingEntry {
    /// Filesystem path (relative or absolute — whatever discovery used).
    pub path: PathBuf,
    /// Human-readable label for the picker. Either the file's `\title`
    /// or the filename stem.
    pub title: String,
}

/// Default location of the examples manifest (relative to cwd).
pub fn examples_manifest_path() -> PathBuf {
    PathBuf::from("assets/examples/manifest.json")
}

/// Default location of the patterns manifest (relative to cwd).
pub fn patterns_manifest_path() -> PathBuf {
    PathBuf::from("assets/patterns/manifest.json")
}

/// Default location of the user recordings dir (relative to cwd).
pub fn recordings_dir_path() -> PathBuf {
    PathBuf::from("recordings")
}

/// Load + parse the bundled-examples manifest. Returns an empty list
/// (not an error) if the manifest file doesn't exist — the user
/// running from a non-TWANGA cwd just doesn't see examples in their
/// picker, which is correct behaviour.
pub fn load_examples_at(manifest_path: &Path) -> Result<Vec<ExampleEntry>> {
    if !manifest_path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let parsed: ExamplesManifest = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {} as JSON", manifest_path.display()))?;
    Ok(parsed.examples)
}

/// Load + parse the bundled-patterns manifest. Returns an empty
/// `Vec<PatternGroup>` if the file is missing — same rationale as
/// `load_examples_at`.
pub fn load_patterns_at(manifest_path: &Path) -> Result<Vec<PatternGroup>> {
    if !manifest_path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(manifest_path)
        .with_context(|| format!("failed to read {}", manifest_path.display()))?;
    let parsed: PatternsManifest = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {} as JSON", manifest_path.display()))?;
    Ok(parsed.groups)
}

/// Production wrapper around [`load_examples_at`] using the default
/// path.
pub fn load_examples() -> Result<Vec<ExampleEntry>> {
    load_examples_at(&examples_manifest_path())
}

/// Production wrapper around [`load_patterns_at`].
pub fn load_patterns() -> Result<Vec<PatternGroup>> {
    load_patterns_at(&patterns_manifest_path())
}

/// Scan a directory for `*.alphatex` files. Skips on missing dir
/// (returns empty). Sort order: most-recently-modified first — matches
/// the GUI's IDB list, which is `createdAt DESC`.
pub fn scan_recordings_at(dir: &Path) -> Result<Vec<RecordingEntry>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();
    for ent in fs::read_dir(dir).with_context(|| format!("failed to read dir {}", dir.display()))? {
        let ent = ent?;
        let path = ent.path();
        if path.extension().and_then(|s| s.to_str()) != Some("alphatex") {
            continue;
        }
        let mtime = ent
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        entries.push((path, mtime));
    }
    entries.sort_by_key(|e| std::cmp::Reverse(e.1));
    Ok(entries
        .into_iter()
        .map(|(path, _)| {
            let title = title_from_alphatex(&path).unwrap_or_else(|_| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("recording")
                    .to_string()
            });
            RecordingEntry { path, title }
        })
        .collect())
}

/// Production wrapper around [`scan_recordings_at`].
pub fn scan_recordings() -> Result<Vec<RecordingEntry>> {
    scan_recordings_at(&recordings_dir_path())
}

/// Cheap title extraction — read the first ~200 bytes of the file and
/// look for `\title "..."`. Avoids a full alphaTex parse for the
/// common case of picking a file out of a list. Returns the inner
/// quoted string if found, or `Err` if not (caller falls back to the
/// filename stem).
fn title_from_alphatex(path: &Path) -> Result<String> {
    let text = fs::read_to_string(path)?;
    for line in text.lines().take(20) {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("\\title ") {
            // Expected shape: `\title "Some Title"`.
            let rest = rest.trim();
            let stripped = rest
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(rest);
            if !stripped.is_empty() {
                return Ok(stripped.to_string());
            }
        }
    }
    Err(anyhow!("no \\title directive in {}", path.display()))
}

/// Resolve the absolute filesystem path for a pattern. The manifest's
/// `path` is relative to its own directory, so we join against
/// `assets/patterns/`. Errors if the resulting file doesn't exist (a
/// manifest entry without a matching file is a packaging bug worth
/// surfacing loudly).
pub fn resolve_pattern_path(entry: &PatternEntry) -> Result<PathBuf> {
    let full = patterns_manifest_path()
        .parent()
        .unwrap_or(Path::new("."))
        .join(&entry.path);
    if !full.exists() {
        return Err(anyhow!(
            "pattern '{}' refers to '{}' but that file doesn't exist",
            entry.id,
            full.display()
        ));
    }
    Ok(full)
}

/// Resolve the absolute filesystem path for a bundled example.
/// Mirror of [`resolve_pattern_path`].
pub fn resolve_example_path(entry: &ExampleEntry) -> Result<PathBuf> {
    let full = examples_manifest_path()
        .parent()
        .unwrap_or(Path::new("."))
        .join(&entry.path);
    if !full.exists() {
        return Err(anyhow!(
            "example '{}' refers to '{}' but that file doesn't exist",
            entry.id,
            full.display()
        ));
    }
    Ok(full)
}

/// Flatten the patterns manifest into a list of `(group, pattern)`
/// pairs sorted by group order then `difficulty` ascending. Used by
/// the picker so the user sees the same tree-of-difficulty ordering
/// the GUI renders.
pub fn flat_pattern_list(groups: &[PatternGroup]) -> Vec<(PatternGroup, PatternEntry)> {
    let mut out: Vec<(PatternGroup, PatternEntry)> = Vec::new();
    for group in groups {
        let mut sorted = group.patterns.clone();
        sorted.sort_by_key(|p| p.difficulty.unwrap_or(u32::MAX));
        for p in sorted {
            out.push((group.clone(), p));
        }
    }
    out
}

/// Difficulty pips for a pattern's level. 4-star scale matching the
/// GUI; `None` / 0 → empty string so the picker line isn't cluttered.
pub fn difficulty_pips(level: Option<u32>) -> String {
    match level {
        Some(n) if n > 0 => {
            let solid = n.min(4) as usize;
            let hollow = 4usize.saturating_sub(solid);
            format!("{}{}", "★".repeat(solid), "☆".repeat(hollow))
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir(test_name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("twanga-bundled-{test_name}-{nanos}"));
        fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn load_examples_missing_file_returns_empty() {
        let dir = scratch_dir("examples_missing");
        let manifest = dir.join("manifest.json");
        assert!(!manifest.exists());
        let entries = load_examples_at(&manifest).expect("load");
        assert!(entries.is_empty());
    }

    #[test]
    fn load_examples_parses_real_shape() {
        let dir = scratch_dir("examples_parse");
        let manifest = dir.join("manifest.json");
        fs::write(
            &manifest,
            r#"{
              "//": "comment is fine — serde_json ignores unknown fields",
              "examples": [
                { "id": "twinkle", "title": "Twinkle", "path": "twinkle.alphatex", "tuning": "standard-ukulele" }
              ]
            }"#,
        )
        .unwrap();
        let entries = load_examples_at(&manifest).expect("load");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "twinkle");
        assert_eq!(entries[0].title, "Twinkle");
        assert_eq!(entries[0].path, "twinkle.alphatex");
        assert_eq!(entries[0].tuning.as_deref(), Some("standard-ukulele"));
    }

    #[test]
    fn load_patterns_parses_groups_and_difficulty() {
        let dir = scratch_dir("patterns_parse");
        let manifest = dir.join("manifest.json");
        fs::write(
            &manifest,
            r#"{
              "groups": [
                {
                  "id": "clawhammer",
                  "title": "Clawhammer (banjo)",
                  "description": "drone-based bum-diddy",
                  "patterns": [
                    { "id": "a", "title": "A", "path": "a.alphatex", "tuning": "standard-banjo", "difficulty": 2 },
                    { "id": "b", "title": "B", "path": "b.alphatex", "tuning": "standard-banjo", "difficulty": 1 }
                  ]
                }
              ]
            }"#,
        )
        .unwrap();
        let groups = load_patterns_at(&manifest).expect("load");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].patterns.len(), 2);
    }

    #[test]
    fn flat_pattern_list_sorts_by_difficulty_within_group() {
        let groups = vec![PatternGroup {
            id: "g".into(),
            title: "G".into(),
            description: None,
            patterns: vec![
                PatternEntry {
                    id: "a".into(),
                    title: "A".into(),
                    path: "a.alphatex".into(),
                    tuning: None,
                    difficulty: Some(2),
                },
                PatternEntry {
                    id: "b".into(),
                    title: "B".into(),
                    path: "b.alphatex".into(),
                    tuning: None,
                    difficulty: Some(1),
                },
                PatternEntry {
                    id: "c".into(),
                    title: "C".into(),
                    path: "c.alphatex".into(),
                    tuning: None,
                    difficulty: None,
                },
            ],
        }];
        let flat = flat_pattern_list(&groups);
        assert_eq!(flat.len(), 3);
        assert_eq!(flat[0].1.id, "b");
        assert_eq!(flat[1].1.id, "a");
        // None difficulty sorts last via u32::MAX sentinel.
        assert_eq!(flat[2].1.id, "c");
    }

    #[test]
    fn difficulty_pips_match_gui_scale() {
        assert_eq!(difficulty_pips(None), "");
        assert_eq!(difficulty_pips(Some(0)), "");
        assert_eq!(difficulty_pips(Some(1)), "★☆☆☆");
        assert_eq!(difficulty_pips(Some(2)), "★★☆☆");
        assert_eq!(difficulty_pips(Some(4)), "★★★★");
        // Beyond 4 caps at 4 solid — matches the GUI's render.
        assert_eq!(difficulty_pips(Some(99)), "★★★★");
    }

    #[test]
    fn scan_recordings_skips_non_alphatex_and_handles_missing_dir() {
        let dir = scratch_dir("recordings_scan");
        let recordings = dir.join("recordings");
        fs::create_dir_all(&recordings).unwrap();
        fs::write(recordings.join("a.alphatex"), "\\title \"A\"\n").unwrap();
        fs::write(recordings.join("b.alphatex"), "no title here\n").unwrap();
        fs::write(recordings.join("notes.txt"), "not a tab").unwrap();
        let entries = scan_recordings_at(&recordings).expect("scan");
        assert_eq!(entries.len(), 2, "should ignore the .txt file");
        // Title parsing picks up the \title directive when present.
        let titles: Vec<String> = entries.iter().map(|e| e.title.clone()).collect();
        assert!(titles.contains(&"A".to_string()));
        // Fallback to stem when no title:
        assert!(titles.contains(&"b".to_string()));

        let absent = dir.join("does-not-exist");
        let empty = scan_recordings_at(&absent).expect("missing dir → empty");
        assert!(empty.is_empty());
    }
}
