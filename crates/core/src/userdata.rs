//! Per-profile user data written by the game (with PortableGTA, saves,
//! settings, user tracks and screenshots land in `<game root>/userfiles`,
//! which the overlay turns into `<profile>/upper/userfiles`).
//!
//! This module only reads/classifies and deletes files inside that directory;
//! it never walks outside it.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Maximum directory nesting walked while scanning, to bound pathological trees.
const MAX_DEPTH: usize = 64;

/// Kind of user-data file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Saves,
    Settings,
    UserTracks,
    Screenshots,
    Other,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Saves => "Partidas",
            Category::Settings => "Ajustes",
            Category::UserTracks => "User Tracks",
            Category::Screenshots => "Capturas",
            Category::Other => "Otros",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Category::Saves => "saves",
            Category::Settings => "settings",
            Category::UserTracks => "usertracks",
            Category::Screenshots => "screenshots",
            Category::Other => "other",
        }
    }
}

/// One file inside the user-data directory.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Path relative to the user-data root, with `/` separators.
    pub rel: String,
    pub name: String,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub category: Category,
}

/// Classifies a user-data-relative path.
pub fn classify(rel: &str) -> Category {
    let lower = rel.replace('\\', "/").to_lowercase();
    if lower.starts_with("user tracks/") || lower.starts_with("usertracks/") {
        Category::UserTracks
    } else if lower.ends_with(".b") {
        Category::Saves
    } else if lower.ends_with(".set") {
        Category::Settings
    } else if crate::meta::is_image_file(Path::new(&lower)) {
        Category::Screenshots
    } else {
        Category::Other
    }
}

/// Scans `<upper>/<subdir>` and returns its files, sorted by category then path.
/// A missing directory yields an empty list.
pub fn scan(upper: &Path, subdir: &str) -> Vec<Entry> {
    let root = upper.join(subdir);
    let mut out = Vec::new();
    walk(&root, &root, &mut out, 0);
    out.sort_by(|a, b| {
        a.category
            .key()
            .cmp(b.category.key())
            .then_with(|| a.rel.cmp(&b.rel))
    });
    out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<Entry>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let path = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            walk(root, &path, out, depth + 1);
        } else if ft.is_file() {
            let Ok(rel_path) = path.strip_prefix(root) else {
                continue;
            };
            let rel = rel_path.to_string_lossy().replace('\\', "/");
            if rel.is_empty() {
                continue;
            }
            let meta = e.metadata().ok();
            out.push(Entry {
                name: e.file_name().to_string_lossy().to_string(),
                rel: rel.clone(),
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                modified: meta.and_then(|m| m.modified().ok()),
                category: classify(&rel),
            });
        }
    }
}

/// Resolves a relative entry to an absolute path inside `<upper>/<subdir>`,
/// refusing absolute paths, `..` and escapes through symlinks.
pub fn resolve_entry(upper: &Path, subdir: &str, rel: &str) -> anyhow::Result<PathBuf> {
    if !crate::meta::valid_relative_path(rel) {
        anyhow::bail!("Ruta de datos inválida: '{rel}'");
    }
    let base = upper.join(subdir);
    let target = base.join(rel);
    let canon_base = base.canonicalize().unwrap_or(base);
    let canon_target = target
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("No existe '{}': {e}", target.display()))?;
    if !canon_target.starts_with(&canon_base) {
        anyhow::bail!("Ruta fuera de la carpeta de datos: '{rel}'");
    }
    Ok(canon_target)
}

/// Deletes a user-data file (never a directory, never outside the root).
pub fn remove(upper: &Path, subdir: &str, rel: &str) -> anyhow::Result<()> {
    let target = resolve_entry(upper, subdir, rel)?;
    if target.is_dir() {
        anyhow::bail!("'{rel}' es un directorio; no se elimina.");
    }
    std::fs::remove_file(&target)
        .map_err(|e| anyhow::anyhow!("No se pudo eliminar '{}': {e}", target.display()))
}

/// True when an enabled mod provides `portablegta.asi` (so the game writes its
/// user files inside the game directory, per profile).
pub fn portablegta_installed(mods_dir: &Path, resolved: &[String]) -> bool {
    resolved.iter().any(|folder| {
        crate::meta::mod_layers(mods_dir, folder)
            .iter()
            .any(|layer| contains_file(layer, "portablegta.asi", 0))
    })
}

fn contains_file(dir: &Path, name: &str, depth: usize) -> bool {
    if depth > 8 {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for e in entries.flatten() {
        let file_name = e.file_name();
        if file_name.to_string_lossy().eq_ignore_ascii_case(name) {
            return true;
        }
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false)
            && contains_file(&e.path(), name, depth + 1)
        {
            return true;
        }
    }
    false
}

/// Human-readable size (e.g. `1.2 MiB`).
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_paths() {
        assert_eq!(classify("GTASAsf1.b"), Category::Saves);
        assert_eq!(classify("gta_sa.set"), Category::Settings);
        assert_eq!(classify("User Tracks/song.mp3"), Category::UserTracks);
        assert_eq!(classify("usertracks/x.wav"), Category::UserTracks);
        assert_eq!(classify("Gallery/shot.png"), Category::Screenshots);
        assert_eq!(classify("notes.txt"), Category::Other);
    }

    #[test]
    fn scan_and_remove_stay_inside() {
        let dir = std::env::temp_dir().join(format!("gta-mo-userdata-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let upper = dir.join("upper");
        std::fs::create_dir_all(upper.join("userfiles/User Tracks")).unwrap();
        std::fs::create_dir_all(upper.join("userfiles/Gallery")).unwrap();
        std::fs::write(upper.join("userfiles/GTASAsf1.b"), "save").unwrap();
        std::fs::write(upper.join("userfiles/gta_sa.set"), "cfg").unwrap();
        std::fs::write(upper.join("userfiles/User Tracks/a.mp3"), "audio").unwrap();
        std::fs::write(upper.join("userfiles/Gallery/shot.png"), "img").unwrap();

        let entries = scan(&upper, "userfiles");
        assert_eq!(entries.len(), 4);
        assert!(entries.iter().any(|e| e.category == Category::Saves));
        assert!(entries.iter().any(|e| e.category == Category::Screenshots));

        // Valid removal.
        remove(&upper, "userfiles", "GTASAsf1.b").unwrap();
        assert!(!upper.join("userfiles/GTASAsf1.b").exists());

        // Escapes and non-existent paths are refused.
        assert!(remove(&upper, "userfiles", "../escape").is_err());
        assert!(remove(&upper, "userfiles", "/etc/passwd").is_err());
        assert!(remove(&upper, "userfiles", "nope.b").is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_portablegta_in_a_mod() {
        let dir = std::env::temp_dir().join(format!("gta-mo-pgta-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("SomeMod/modloader/PortableGTA")).unwrap();
        std::fs::write(
            dir.join("SomeMod/modloader/PortableGTA/portablegta.asi"),
            "x",
        )
        .unwrap();
        assert!(portablegta_installed(&dir, &["SomeMod".to_string()]));
        assert!(!portablegta_installed(&dir, &[]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
