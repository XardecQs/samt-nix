//! Layout of the mods directory.
//!
//! The historical layout is flat: every immediate subdirectory of `mods/` is a
//! mod. A repository can opt into categories with a `.samt.toml` marker:
//!
//! ```toml
//! layout_version = 1
//! categories = ["external", "own", "generated"]
//! default_category = "external"
//! generated_category = "generated"
//! legacy_root = true
//! ignore = ["tools", "reports"]
//! ```
//!
//! With a marker, `mods/external/foo` is a mod whose `folder_name` is `foo` and
//! whose `rel_path` is `external/foo`. Direct children of `mods/` remain valid
//! mods while `legacy_root` is enabled, which keeps existing repositories
//! working during the transition. Without a marker the directory is flat and
//! behaves exactly as before.

use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Name of the optional layout marker at the root of the mods directory.
pub const LAYOUT_FILE: &str = ".samt.toml";

/// One mod found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModLocation {
    /// Path relative to the mods directory, with `/` separators (`external/foo`).
    pub rel_path: String,
    /// Last path component; the logical key used by the database and profiles.
    pub folder_name: String,
    /// Category container (`external`, `own`, `generated`), or `None` for legacy
    /// mods directly under the mods directory.
    pub category: Option<String>,
}

impl ModLocation {
    /// Stable origin label (`legacy`, `external`, `own`, `generated`, ...).
    pub fn origin(&self) -> &str {
        self.category.as_deref().unwrap_or("legacy")
    }
}

/// Parsed (and validated) layout of the mods directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModsLayout {
    /// No marker: every immediate child is a mod.
    pub flat: bool,
    pub categories: Vec<String>,
    pub default_category: Option<String>,
    pub generated_category: Option<String>,
    /// Treat immediate children that are not categories/ignored as legacy mods.
    pub legacy_root: bool,
    /// Root-level directories that are not mods (reports, tools…).
    pub ignore: Vec<String>,
}

impl Default for ModsLayout {
    fn default() -> Self {
        Self::flat()
    }
}

/// Result of scanning the mods directory.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    pub mods: Vec<ModLocation>,
    /// Non-fatal problems (invalid names, ignored entries…).
    pub warnings: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct LayoutToml {
    #[serde(default = "default_layout_version")]
    layout_version: u32,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    default_category: Option<String>,
    #[serde(default)]
    generated_category: Option<String>,
    #[serde(default = "default_true")]
    legacy_root: bool,
    #[serde(default)]
    ignore: Vec<String>,
}

fn default_layout_version() -> u32 {
    1
}

fn default_true() -> bool {
    true
}

impl ModsLayout {
    /// Flat layout used when the mods directory has no `.samt.toml`.
    pub fn flat() -> Self {
        Self {
            flat: true,
            categories: Vec::new(),
            default_category: None,
            generated_category: None,
            legacy_root: true,
            ignore: Vec::new(),
        }
    }

    pub fn is_flat(&self) -> bool {
        self.flat
    }

    /// Reads and validates `mods_dir/.samt.toml`. A missing marker means the
    /// historical flat layout; a malformed marker is a hard error so we never
    /// silently discover the wrong set of mods.
    pub fn load(mods_dir: &Path) -> anyhow::Result<Self> {
        let path = mods_dir.join(LAYOUT_FILE);
        if !path.exists() {
            return Ok(Self::flat());
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| anyhow::anyhow!("No se pudo leer '{}': {e}", path.display()))?;
        let raw: LayoutToml = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("'{}' inválido: {e}", path.display()))?;
        Self::from_toml(raw).map_err(|e| anyhow::anyhow!("'{}' inválido: {e}", path.display()))
    }

    fn from_toml(raw: LayoutToml) -> anyhow::Result<Self> {
        if raw.layout_version != 1 {
            anyhow::bail!(
                "layout_version {} no soportada (se esperaba 1)",
                raw.layout_version
            );
        }

        let layout = Self {
            flat: false,
            categories: raw.categories,
            default_category: raw.default_category,
            generated_category: raw.generated_category,
            legacy_root: raw.legacy_root,
            ignore: raw.ignore,
        };
        layout.validate()?;
        Ok(layout)
    }

    /// Validates category names, defaults and ignored entries.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.flat {
            return Ok(());
        }

        let mut seen = HashSet::new();
        for category in &self.categories {
            if !valid_segment(category) {
                anyhow::bail!("categoría inválida '{category}'");
            }
            if category.eq_ignore_ascii_case("legacy") {
                anyhow::bail!("'legacy' está reservado y no puede ser una categoría");
            }
            if !seen.insert(category.as_str()) {
                anyhow::bail!("categoría duplicada '{category}'");
            }
        }

        if let Some(default) = &self.default_category {
            if !self.categories.iter().any(|c| c == default) {
                anyhow::bail!("default_category '{default}' no está en categories");
            }
        }
        if let Some(generated) = &self.generated_category {
            if !self.categories.iter().any(|c| c == generated) {
                anyhow::bail!("generated_category '{generated}' no está en categories");
            }
        }

        let mut ignored = HashSet::new();
        for entry in &self.ignore {
            if !valid_segment(entry) {
                anyhow::bail!("entrada inválida en ignore: '{entry}'");
            }
            if !ignored.insert(entry.as_str()) {
                anyhow::bail!("entrada duplicada en ignore: '{entry}'");
            }
            if self.categories.iter().any(|c| c == entry) {
                anyhow::bail!("'{entry}' no puede ser categoría e ignore a la vez");
            }
        }

        if self.categories.is_empty() && !self.legacy_root {
            anyhow::bail!("categories está vacío y legacy_root = false: no habría mods");
        }

        Ok(())
    }

    /// Scans the mods directory according to this layout.
    ///
    /// Duplicate folder names are a hard error: the database keys mods by
    /// `folder_name`, and silently choosing one would risk disabling the other
    /// mod's per-profile state.
    pub fn scan(&self, mods_dir: &Path) -> anyhow::Result<Scan> {
        let mut out = Scan::default();

        if self.flat {
            self.scan_dir(mods_dir, None, &mut out)?;
        } else {
            let categories: HashSet<&str> = self.categories.iter().map(String::as_str).collect();
            let ignored: HashSet<&str> = self.ignore.iter().map(String::as_str).collect();

            for category in &self.categories {
                let dir = mods_dir.join(category);
                self.scan_dir(&dir, Some(category), &mut out)?;
            }

            if self.legacy_root {
                for name in read_dirs(mods_dir)? {
                    if categories.contains(name.as_str()) || ignored.contains(name.as_str()) {
                        continue;
                    }
                    self.push_location(mods_dir, None, &name, &mut out);
                }
            }
        }

        self.check_duplicates(&out.mods)?;
        Ok(out)
    }

    fn scan_dir(&self, dir: &Path, category: Option<&str>, out: &mut Scan) -> anyhow::Result<()> {
        let root = dir.to_path_buf();
        for name in read_dirs(&root)? {
            self.push_location(&root, category, &name, out);
        }
        Ok(())
    }

    fn push_location(&self, dir: &Path, category: Option<&str>, name: &str, out: &mut Scan) {
        if name.starts_with('.') {
            return;
        }
        if !valid_segment(name) {
            out.warnings.push(format!(
                "carpeta ignorada '{}': nombre no válido",
                dir.join(name).display()
            ));
            return;
        }
        let rel_path = match category {
            Some(category) => format!("{category}/{name}"),
            None => name.to_string(),
        };
        out.mods.push(ModLocation {
            rel_path,
            folder_name: name.to_string(),
            category: category.map(str::to_string),
        });
    }

    fn check_duplicates(&self, mods: &[ModLocation]) -> anyhow::Result<()> {
        let mut by_name: HashMap<&str, Vec<&str>> = HashMap::new();
        for m in mods {
            by_name
                .entry(m.folder_name.as_str())
                .or_default()
                .push(m.rel_path.as_str());
        }
        let mut duplicates: Vec<String> = by_name
            .into_iter()
            .filter(|(_, paths)| paths.len() > 1)
            .map(|(name, paths)| format!("{name}: {}", paths.join(", ")))
            .collect();
        duplicates.sort();
        if !duplicates.is_empty() {
            anyhow::bail!(
                "nombres de carpeta duplicados entre categorías (renombra uno): {}",
                duplicates.join("; ")
            );
        }
        Ok(())
    }

    /// Destination for a newly created mod. With a marker this uses
    /// `default_category`; without one it falls back to the mods root.
    pub fn default_location(&self, name: &str) -> anyhow::Result<ModLocation> {
        if !valid_segment(name) {
            anyhow::bail!("nombre de mod inválido: '{name}'");
        }
        if self.flat {
            return Ok(ModLocation {
                rel_path: name.to_string(),
                folder_name: name.to_string(),
                category: None,
            });
        }
        let category = self
            .default_category
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("define default_category en {LAYOUT_FILE}"))?;
        Ok(ModLocation {
            rel_path: format!("{category}/{name}"),
            folder_name: name.to_string(),
            category: Some(category.clone()),
        })
    }

    /// Destination for a captured/imported mod. With a marker this requires
    /// `generated_category`; without one it uses the legacy root.
    pub fn generated_location(&self, name: &str) -> anyhow::Result<ModLocation> {
        if !valid_segment(name) {
            anyhow::bail!("nombre de mod inválido: '{name}'");
        }
        if self.flat {
            return Ok(ModLocation {
                rel_path: name.to_string(),
                folder_name: name.to_string(),
                category: None,
            });
        }
        let category = self
            .generated_category
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("define generated_category en {LAYOUT_FILE}"))?;
        Ok(ModLocation {
            rel_path: format!("{category}/{name}"),
            folder_name: name.to_string(),
            category: Some(category.clone()),
        })
    }
}

/// Reads the immediate subdirectories of `dir`, sorted by name. Missing or
/// unreadable directories yield an empty list (the caller decides whether that
/// is an error).
fn read_dirs(dir: &Path) -> std::io::Result<Vec<String>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(dir)?;
    let mut out: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    out.sort();
    Ok(out)
}

/// One path component of a mod folder or category. Mirrors the database's
/// `folder_name` restrictions and the characters that would corrupt an overlay
/// option string.
pub fn valid_segment(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.starts_with('.')
        && name.trim() == name
        && !name.contains(['/', '\\', ':', ',', '|'])
        && !name.chars().any(char::is_control)
}

/// A safe path relative to the mods directory (`external/foo`).
pub fn valid_rel_path(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.starts_with('/')
        && !rel.contains('\\')
        && rel.split('/').all(valid_segment)
}

/// Origin label for a stored `rel_path` (`legacy` when it has no category).
pub fn origin_of(rel_path: &str) -> &str {
    rel_path
        .split_once('/')
        .map(|(category, _)| category)
        .unwrap_or("legacy")
}

/// Joins a mod's relative path to the mods directory.
pub fn mod_dir(mods_dir: &Path, rel_path: &str) -> PathBuf {
    mods_dir.join(rel_path)
}

/// Maps the logical `folder_name` used by profiles/options to the on-disk
/// `rel_path`. Missing entries fall back to the folder name, which keeps legacy
/// flat layouts working without an explicit map.
#[derive(Debug, Clone, Default)]
pub struct ModPaths {
    map: HashMap<String, String>,
}

impl ModPaths {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, folder_name: &str, rel_path: &str) {
        self.map
            .insert(folder_name.to_string(), rel_path.to_string());
    }

    pub fn from_locations(locations: &[ModLocation]) -> Self {
        let mut paths = Self::new();
        for location in locations {
            paths.insert(&location.folder_name, &location.rel_path);
        }
        paths
    }

    pub fn rel<'a>(&'a self, folder_name: &'a str) -> &'a str {
        self.map
            .get(folder_name)
            .map(String::as_str)
            .unwrap_or(folder_name)
    }

    pub fn get(&self, folder_name: &str) -> Option<&str> {
        self.map.get(folder_name).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("gta-mo-layout-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_layout(dir: &Path, content: &str) {
        std::fs::write(dir.join(LAYOUT_FILE), content).unwrap();
    }

    fn rel_paths(scan: &Scan) -> Vec<&str> {
        scan.mods.iter().map(|m| m.rel_path.as_str()).collect()
    }

    #[test]
    fn without_marker_the_layout_is_flat() {
        let dir = tmp("flat");
        std::fs::create_dir_all(dir.join("foo")).unwrap();
        std::fs::create_dir_all(dir.join("bar")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join("README.md"), "docs").unwrap();

        let layout = ModsLayout::load(&dir).unwrap();
        assert!(layout.is_flat());
        let scan = layout.scan(&dir).unwrap();
        assert_eq!(rel_paths(&scan), vec!["bar", "foo"]);
        assert!(scan.mods.iter().all(|m| m.origin() == "legacy"));
    }

    #[test]
    fn marker_scan_categories_legacy_and_ignore() {
        let dir = tmp("categories");
        write_layout(
            &dir,
            r#"
layout_version = 1
categories = ["external", "own", "generated"]
default_category = "external"
generated_category = "generated"
legacy_root = true
ignore = ["tools", "reports"]
"#,
        );
        for path in [
            "external/foo",
            "own/bar",
            "generated/cap",
            "legacy-mod",
            "tools",
            "reports",
        ] {
            std::fs::create_dir_all(dir.join(path)).unwrap();
        }

        let layout = ModsLayout::load(&dir).unwrap();
        assert!(!layout.is_flat());
        let scan = layout.scan(&dir).unwrap();
        let mut paths = rel_paths(&scan);
        paths.sort();
        assert_eq!(
            paths,
            vec!["external/foo", "generated/cap", "legacy-mod", "own/bar"]
        );
        let owners: HashMap<&str, &str> = scan
            .mods
            .iter()
            .map(|m| (m.rel_path.as_str(), m.origin()))
            .collect();
        assert_eq!(owners["external/foo"], "external");
        assert_eq!(owners["own/bar"], "own");
        assert_eq!(owners["generated/cap"], "generated");
        assert_eq!(owners["legacy-mod"], "legacy");
    }

    #[test]
    fn duplicate_folder_names_are_an_error() {
        let dir = tmp("duplicates");
        write_layout(
            &dir,
            r#"
layout_version = 1
categories = ["external", "own"]
default_category = "external"
legacy_root = true
"#,
        );
        std::fs::create_dir_all(dir.join("external/foo")).unwrap();
        std::fs::create_dir_all(dir.join("own/foo")).unwrap();

        let layout = ModsLayout::load(&dir).unwrap();
        let err = layout.scan(&dir).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("duplicados"), "{msg}");
        assert!(msg.contains("external/foo"), "{msg}");
        assert!(msg.contains("own/foo"), "{msg}");
    }

    #[test]
    fn invalid_marker_is_rejected() {
        let dir = tmp("invalid");
        write_layout(&dir, "layout_version = 2\n");
        assert!(ModsLayout::load(&dir).is_err());

        write_layout(
            &dir,
            r#"
layout_version = 1
categories = ["external"]
default_category = "own"
"#,
        );
        assert!(ModsLayout::load(&dir).is_err());

        write_layout(
            &dir,
            r#"
layout_version = 1
categories = ["external"]
generated_category = "generated"
"#,
        );
        assert!(ModsLayout::load(&dir).is_err());
    }

    #[test]
    fn default_and_generated_locations() {
        let dir = tmp("locations");
        write_layout(
            &dir,
            r#"
layout_version = 1
categories = ["external", "own", "generated"]
default_category = "external"
generated_category = "generated"
"#,
        );
        let layout = ModsLayout::load(&dir).unwrap();
        assert_eq!(
            layout.default_location("foo").unwrap().rel_path,
            "external/foo"
        );
        assert_eq!(
            layout.generated_location("cap").unwrap().rel_path,
            "generated/cap"
        );
        assert!(layout.default_location("..").is_err());

        let flat = ModsLayout::flat();
        assert_eq!(flat.default_location("foo").unwrap().rel_path, "foo");
        assert_eq!(flat.generated_location("cap").unwrap().rel_path, "cap");
    }

    #[test]
    fn mod_paths_fall_back_to_folder_names() {
        let mut paths = ModPaths::new();
        paths.insert("foo", "external/foo");
        assert_eq!(paths.rel("foo"), "external/foo");
        assert_eq!(paths.rel("legacy"), "legacy");
        assert_eq!(paths.get("missing"), None);
    }

    #[test]
    fn relative_path_validation() {
        assert!(valid_rel_path("external/foo"));
        assert!(valid_rel_path("foo"));
        assert!(!valid_rel_path(""));
        assert!(!valid_rel_path("/foo"));
        assert!(!valid_rel_path("external//foo"));
        assert!(!valid_rel_path("external/../foo"));
        assert!(!valid_rel_path("external/foo:bar"));
        assert!(!valid_rel_path("external/foo\\bar"));
    }
}
