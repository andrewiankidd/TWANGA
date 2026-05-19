//! User-tunings layer for the CLI. The same TOML schema lives in
//! `crates/twanga-core/src/presets.toml` (compiled into the binary) and in
//! the user-config file under the OS-conventional config dir; this module
//! merges the two for menus and lookup.
//!
//! The low-level `*_at` functions take an explicit path and are unit-tested.
//! The high-level functions wrap them around `directories::ProjectDirs` so
//! production code uses the platform's standard config location.

use anyhow::{Context, Result, anyhow};
use directories::ProjectDirs;
use std::fs;
use std::path::{Path, PathBuf};
use twanga_core::{PresetEntry, PresetFile, Tuning};

/// Where a given tuning came from. Used for menu labels and to keep built-in
/// presets unshadowable by the user config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Builtin,
    User,
}

/// One entry in the merged registry, with provenance attached.
#[derive(Debug, Clone)]
pub struct KnownTuning {
    pub slug: String,
    pub origin: Origin,
    pub entry: PresetEntry,
}

impl KnownTuning {
    pub fn to_tuning(&self) -> Tuning {
        self.entry.to_tuning()
    }
}

/// Path to the user tunings file. Returns `None` only on exotic platforms
/// where `directories` can't resolve a config dir at all.
pub fn user_tunings_path() -> Option<PathBuf> {
    let dirs = ProjectDirs::from("", "", "twanga")?;
    Some(dirs.config_dir().join("tunings.toml"))
}

/// Read the user-tunings file at `path`. Treats a missing file as "no user
/// tunings yet" (returns empty), since first-run users won't have one and
/// that's not an error.
pub fn load_user_tunings_at(path: &Path) -> Result<Vec<PresetEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let file: PresetFile = toml::from_str(&text)
        .with_context(|| format!("failed to parse {} as TOML", path.display()))?;
    Ok(file.tunings)
}

/// Write `entries` to `path`, creating parent directories as needed.
pub fn save_user_tunings_at(path: &Path, entries: &[PresetEntry]) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let file = PresetFile {
        tunings: entries.to_vec(),
    };
    let body = toml::to_string_pretty(&file).context("failed to serialise user tunings to TOML")?;
    fs::write(path, body).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Append `entry` to the file at `path`. Refuses if a user-defined entry with
/// the same slug already exists, or if the slug shadows a built-in preset.
/// Returns the path on success.
pub fn add_user_tuning_at(path: &Path, entry: PresetEntry) -> Result<PathBuf> {
    if Tuning::builtin_slugs().contains(&entry.slug.as_str()) {
        return Err(anyhow!(
            "'{}' is a built-in preset slug — pick a different slug for your custom tuning",
            entry.slug
        ));
    }
    let mut entries = load_user_tunings_at(path)?;
    if entries.iter().any(|e| e.slug == entry.slug) {
        return Err(anyhow!(
            "a user tuning with slug '{}' already exists at {}",
            entry.slug,
            path.display()
        ));
    }
    entries.push(entry);
    save_user_tunings_at(path, &entries)?;
    Ok(path.to_path_buf())
}

/// Production wrapper: load user tunings from the platform's config dir.
pub fn load_user_tunings() -> Result<Vec<PresetEntry>> {
    let Some(path) = user_tunings_path() else {
        return Ok(Vec::new());
    };
    load_user_tunings_at(&path)
}

/// Production wrapper: save a new user tuning to the platform's config dir.
pub fn add_user_tuning(entry: PresetEntry) -> Result<PathBuf> {
    let path = user_tunings_path()
        .ok_or_else(|| anyhow!("no user config directory available on this platform"))?;
    add_user_tuning_at(&path, entry)
}

/// Merged registry view: built-in presets first (in registry order), then any
/// user-defined tunings whose slugs don't collide with a built-in. Used by
/// the tune/record/play menus so the menu surface is "everything I can play."
pub fn all_known_tunings() -> Vec<KnownTuning> {
    let mut out: Vec<KnownTuning> = Tuning::builtin_presets()
        .iter()
        .map(|p| KnownTuning {
            slug: p.slug.clone(),
            origin: Origin::Builtin,
            entry: p.clone(),
        })
        .collect();
    let builtin_slugs: std::collections::HashSet<String> =
        out.iter().map(|k| k.slug.clone()).collect();
    let user = load_user_tunings().unwrap_or_default();
    for e in user {
        if !builtin_slugs.contains(&e.slug) {
            out.push(KnownTuning {
                slug: e.slug.clone(),
                origin: Origin::User,
                entry: e,
            });
        }
    }
    out
}

/// Lookup by slug in the merged registry. Built-in slugs take priority.
pub fn lookup(slug: &str) -> Option<KnownTuning> {
    all_known_tunings().into_iter().find(|k| k.slug == slug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use twanga_core::PresetString;

    /// Per-test scratch dir under `std::env::temp_dir()` — keeps tests
    /// independent without pulling in the `tempfile` crate.
    fn scratch_path(test_name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = env::temp_dir().join(format!("twanga-test-{test_name}-{nanos}"));
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir.join("tunings.toml")
    }

    /// Test fixture: a fake "open D" guitar tuning. Deliberately uses a slug
    /// that's not in `presets.toml`, so these tests stay valid as the built-in
    /// registry grows.
    fn sample_user_entry() -> PresetEntry {
        PresetEntry {
            slug: "open-d-guitar-test".into(),
            name: "Open D Guitar (test fixture)".into(),
            strings: vec![
                PresetString {
                    name: "D4".into(),
                    midi: 62,
                },
                PresetString {
                    name: "A3".into(),
                    midi: 57,
                },
                PresetString {
                    name: "F#3".into(),
                    midi: 54,
                },
                PresetString {
                    name: "D3".into(),
                    midi: 50,
                },
                PresetString {
                    name: "A2".into(),
                    midi: 45,
                },
                PresetString {
                    name: "D2".into(),
                    midi: 38,
                },
            ],
        }
    }

    #[test]
    fn load_missing_file_returns_empty_list() {
        let path = scratch_path("load_missing");
        assert!(!path.exists());
        let entries = load_user_tunings_at(&path).expect("load");
        assert!(entries.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = scratch_path("round_trip");
        let entry = sample_user_entry();
        save_user_tunings_at(&path, std::slice::from_ref(&entry)).expect("save");
        let recovered = load_user_tunings_at(&path).expect("load");
        assert_eq!(recovered, vec![entry]);
    }

    #[test]
    fn add_user_tuning_appends() {
        let path = scratch_path("append");
        let first = sample_user_entry();
        let second = PresetEntry {
            slug: "open-c-guitar-test".into(),
            name: "Open C Guitar (test fixture)".into(),
            strings: vec![PresetString {
                name: "C2".into(),
                midi: 36,
            }],
        };
        add_user_tuning_at(&path, first.clone()).expect("first add");
        add_user_tuning_at(&path, second.clone()).expect("second add");
        let recovered = load_user_tunings_at(&path).expect("load");
        assert_eq!(recovered, vec![first, second]);
    }

    #[test]
    fn add_user_tuning_rejects_duplicate_slug() {
        let path = scratch_path("dup_slug");
        let entry = sample_user_entry();
        add_user_tuning_at(&path, entry.clone()).expect("first add");
        let err = add_user_tuning_at(&path, entry).expect_err("second should fail");
        assert!(
            err.to_string().contains("already exists"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn add_user_tuning_rejects_builtin_slug() {
        let path = scratch_path("builtin_clash");
        let entry = PresetEntry {
            slug: "standard-ukulele".into(),
            name: "My Override".into(),
            strings: vec![PresetString {
                name: "A4".into(),
                midi: 69,
            }],
        };
        let err = add_user_tuning_at(&path, entry).expect_err("should refuse to shadow built-in");
        assert!(
            err.to_string().contains("built-in"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn all_known_tunings_lists_builtins_even_without_user_file() {
        let known = all_known_tunings();
        assert!(known.iter().any(|k| k.slug == "standard-guitar"));
        assert!(known.iter().filter(|k| k.origin == Origin::Builtin).count() >= 3);
    }
}
