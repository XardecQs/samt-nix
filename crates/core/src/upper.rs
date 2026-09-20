//! Management of a profile's overlay upperdir — the "overwrite" layer.
//!
//! Every write made by the game (or by an installer run against `run/merged`)
//! lands in `run/profiles/<slug>/upper`. Unlike [`crate::userdata`] (which only
//! looks at the PortableGTA `userfiles/` subtree), this module scans the whole
//! upper and supports turning it into mod content:
//!
//! - move a single file into a mod's mounted layer,
//! - discard (delete) a file so the lower layers show through again,
//! - capture the whole upper as a new mod (`mods/<name>/content/` + `mod.toml`).
//!
//! Overlayfs whiteout markers (`.wh.<name>`, `.wh..wh..opq`) are never treated
//! as files: they are reported as deletions, because a regular lower-layer mod
//! cannot reproduce a deletion anyway.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Maximum directory nesting walked while scanning, to bound pathological trees.
const MAX_DEPTH: usize = 64;

/// What kind of write an upper file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// PortableGTA user files (saves/settings/tracks/screenshots).
    UserData,
    /// The launcher-generated `modloader/modloader.ini`.
    ModLoaderIni,
    /// Anything else written over the game tree.
    Game,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::UserData => "Datos de usuario",
            Kind::ModLoaderIni => "modloader.ini",
            Kind::Game => "Juego",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Kind::UserData => "userdata",
            Kind::ModLoaderIni => "modloader-ini",
            Kind::Game => "game",
        }
    }
}

/// One file inside the upper.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Path relative to the upper root, with `/` separators (game-relative).
    pub rel: String,
    pub name: String,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub kind: Kind,
}

/// Result of scanning an upper directory.
#[derive(Debug, Clone, Default)]
pub struct Scan {
    pub files: Vec<Entry>,
    /// Game-relative paths the upper deletes (overlayfs whiteouts), sorted and
    /// deduplicated.
    pub deleted: Vec<String>,
}

/// Classifies an upper-relative path.
pub fn classify(rel: &str, user_data_dir: Option<&str>) -> Kind {
    let lower = rel.replace('\\', "/").to_lowercase();
    if let Some(ud) = user_data_dir {
        let ud = ud.trim_matches('/').to_lowercase();
        if !ud.is_empty() && (lower == ud || lower.starts_with(&format!("{ud}/"))) {
            return Kind::UserData;
        }
    }
    if lower == "modloader/modloader.ini" {
        Kind::ModLoaderIni
    } else {
        Kind::Game
    }
}

/// Scans the whole `upper` directory, skipping whiteouts into `deleted`.
pub fn scan(upper: &Path, user_data_dir: Option<&str>) -> Scan {
    let mut out = Scan::default();
    walk(upper, upper, user_data_dir, &mut out, 0);
    out.files.sort_by(|a, b| a.rel.cmp(&b.rel));
    out.deleted.sort();
    out.deleted.dedup();
    out
}

fn walk(root: &Path, dir: &Path, user_data_dir: Option<&str>, out: &mut Scan, depth: usize) {
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
            walk(root, &path, user_data_dir, out, depth + 1);
            continue;
        }
        if !ft.is_file() {
            continue;
        }
        let file_name = e.file_name();
        let name = file_name.to_string_lossy().to_string();
        let Ok(rel_path) = path.strip_prefix(root) else {
            continue;
        };
        let rel = rel_path.to_string_lossy().replace('\\', "/");
        if name.starts_with(".wh.") {
            if let Some(target) = whiteout_target(&rel) {
                out.deleted.push(target);
            }
            continue;
        }
        let meta = e.metadata().ok();
        out.files.push(Entry {
            name,
            rel: rel.clone(),
            size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
            modified: meta.and_then(|m| m.modified().ok()),
            kind: classify(&rel, user_data_dir),
        });
    }
}

/// Translates a whiteout file's upper-relative path into the game-relative path
/// it deletes. `.wh.<name>` deletes `<dir>/<name>`; `.wh..wh..opq` marks `<dir>`
/// as opaque, so every lower file under `<dir>` is hidden.
fn whiteout_target(rel: &str) -> Option<String> {
    let (dir, file) = match rel.rsplit_once('/') {
        Some((d, f)) => (d, f),
        None => ("", rel),
    };
    let name = file.strip_prefix(".wh.")?;
    if name == ".wh..opq" {
        if dir.is_empty() {
            None
        } else {
            Some(dir.to_string())
        }
    } else if dir.is_empty() {
        Some(name.to_string())
    } else {
        Some(format!("{dir}/{name}"))
    }
}

/// Resolves an upper-relative entry to an absolute path, refusing absolute
/// paths, `..` and escapes through symlinks. The entry must exist.
pub fn resolve_entry(upper: &Path, rel: &str) -> anyhow::Result<PathBuf> {
    if !crate::meta::valid_relative_path(rel) {
        anyhow::bail!("Ruta de upper inválida: '{rel}'");
    }
    let target = upper.join(rel);
    let canon_upper = upper.canonicalize().unwrap_or_else(|_| upper.to_path_buf());
    let canon_target = target
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("No existe '{}': {e}", target.display()))?;
    if !canon_target.starts_with(&canon_upper) {
        anyhow::bail!("Ruta fuera del upper: '{rel}'");
    }
    Ok(canon_target)
}

/// The directory a mod contributes to the game: its first valid `mount` entry,
/// or the mod folder itself when there is no manifest/mount.
pub fn layer_root_for(mods_dir: &Path, folder: &str) -> PathBuf {
    let base = mods_dir.join(folder);
    if let Ok(Some(meta)) = crate::meta::read_mod_meta(mods_dir, folder) {
        if let Some(mount) = meta.mount {
            if let Some(first) = mount.iter().find(|e| crate::meta::valid_mount_entry(e)) {
                return base.join(first);
            }
        }
    }
    base
}

/// Deletes an upper file (never a directory, never outside the root) and prunes
/// the now-empty directories it leaves behind.
pub fn discard(upper: &Path, rel: &str) -> anyhow::Result<()> {
    let target = resolve_entry(upper, rel)?;
    if target.is_dir() {
        anyhow::bail!("'{rel}' es un directorio; no se elimina.");
    }
    std::fs::remove_file(&target)
        .map_err(|e| anyhow::anyhow!("No se pudo eliminar '{}': {e}", target.display()))?;
    prune_empty_dirs(upper, target.parent());
    Ok(())
}

/// Moves an upper file into `layer_root` (the game-relative `rel` is preserved).
/// Refuses to overwrite an existing file. Falls back to copy+remove across
/// filesystems.
pub fn move_to_layer(upper: &Path, rel: &str, layer_root: &Path) -> anyhow::Result<PathBuf> {
    let src = resolve_entry(upper, rel)?;
    if src.is_dir() {
        anyhow::bail!("'{rel}' es un directorio; mueve archivos sueltos.");
    }
    let dest = layer_root.join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        anyhow::bail!("El destino ya existe: {}", dest.display());
    }
    match std::fs::rename(&src, &dest) {
        Ok(()) => {}
        Err(_) => {
            std::fs::copy(&src, &dest)
                .map_err(|e| anyhow::anyhow!("No se pudo copiar a '{}': {e}", dest.display()))?;
            std::fs::remove_file(&src).ok();
        }
    }
    prune_empty_dirs(upper, src.parent());
    Ok(dest)
}

/// Report of a full upper capture.
#[derive(Debug, Clone, Default)]
pub struct CaptureReport {
    pub moved: usize,
    /// Whiteouts skipped: deletions cannot be replayed from a lower layer.
    pub deleted_skipped: usize,
    /// `userfiles/` entries skipped.
    pub userdata_skipped: usize,
    /// Launcher-generated files skipped (e.g. `modloader/modloader.ini`).
    pub generated_skipped: usize,
}

/// Moves the whole `upper` into a new mod at `dest_mod_dir`, writing a minimal
/// `mod.toml` with `mount = ["content"]`. Generated files and (optionally) the
/// PortableGTA user data are left behind; whiteouts are reported, not moved.
pub fn capture(
    upper: &Path,
    dest_mod_dir: &Path,
    name: &str,
    user_data_dir: Option<&str>,
    skip_userdata: bool,
) -> anyhow::Result<CaptureReport> {
    if name.trim().is_empty() {
        anyhow::bail!("El nombre del mod no puede estar vacío.");
    }
    if dest_mod_dir.exists() {
        anyhow::bail!(
            "El destino ya existe: {} (usa otro nombre de mod)",
            dest_mod_dir.display()
        );
    }
    let content = dest_mod_dir.join("content");
    std::fs::create_dir_all(&content)?;

    let scanned = scan(upper, user_data_dir);
    let mut report = CaptureReport {
        deleted_skipped: scanned.deleted.len(),
        ..Default::default()
    };

    for entry in &scanned.files {
        match entry.kind {
            Kind::UserData if skip_userdata => {
                report.userdata_skipped += 1;
                continue;
            }
            Kind::ModLoaderIni => {
                report.generated_skipped += 1;
                continue;
            }
            _ => {}
        }
        let src = upper.join(&entry.rel);
        let dest = content.join(&entry.rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if dest.exists() {
            report.generated_skipped += 1;
            continue;
        }
        match std::fs::rename(&src, &dest) {
            Ok(()) => {}
            Err(_) => {
                std::fs::copy(&src, &dest)
                    .map_err(|e| anyhow::anyhow!("No se pudo copiar '{}': {e}", src.display()))?;
                std::fs::remove_file(&src).ok();
            }
        }
        report.moved += 1;
    }

    #[derive(serde::Serialize)]
    struct CapturedManifest<'a> {
        name: &'a str,
        mount: [&'a str; 1],
        tags: [&'a str; 1],
    }
    let content_str = toml::to_string(&CapturedManifest {
        name: name.trim(),
        mount: ["content"],
        tags: ["captured"],
    })?;
    std::fs::write(dest_mod_dir.join("mod.toml"), content_str)?;
    Ok(report)
}

/// Removes `dir` and its now-empty ancestors, stopping at `upper`.
fn prune_empty_dirs(upper: &Path, start: Option<&Path>) {
    let boundary = upper.canonicalize().unwrap_or_else(|_| upper.to_path_buf());
    let mut dir = start.map(Path::to_path_buf);
    while let Some(d) = dir {
        let canon = match d.canonicalize() {
            Ok(c) => c,
            Err(_) => return,
        };
        if canon == boundary || !canon.starts_with(&boundary) {
            return;
        }
        match std::fs::remove_dir(&canon) {
            Ok(()) => dir = canon.parent().map(Path::to_path_buf),
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gta-mo-upper-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn classifies_upper_paths() {
        assert_eq!(
            classify("userfiles/GTASAsf1.b", Some("userfiles")),
            Kind::UserData
        );
        assert_eq!(
            classify("modloader/modloader.ini", None),
            Kind::ModLoaderIni
        );
        assert_eq!(classify("models/x.dff", None), Kind::Game);
    }

    #[test]
    fn whiteout_targets() {
        assert_eq!(
            whiteout_target("models/.wh.x.dff").as_deref(),
            Some("models/x.dff")
        );
        assert_eq!(whiteout_target(".wh.foo").as_deref(), Some("foo"));
        assert_eq!(
            whiteout_target("models/.wh..wh..opq").as_deref(),
            Some("models")
        );
        assert_eq!(whiteout_target(".wh..wh..opq"), None);
    }

    #[test]
    fn scan_reports_files_and_deletions() {
        let dir = tmp("scan");
        let upper = dir.join("upper");
        std::fs::create_dir_all(upper.join("models")).unwrap();
        std::fs::create_dir_all(upper.join("userfiles")).unwrap();
        std::fs::create_dir_all(upper.join("modloader")).unwrap();
        std::fs::write(upper.join("models/x.dff"), "x").unwrap();
        std::fs::write(upper.join("models/.wh.old.dff"), "").unwrap();
        std::fs::write(upper.join("userfiles/GTASAsf1.b"), "save").unwrap();
        std::fs::write(upper.join("modloader/modloader.ini"), "ini").unwrap();

        let scan = scan(&upper, Some("userfiles"));
        assert_eq!(scan.files.len(), 3);
        assert_eq!(scan.deleted, vec!["models/old.dff".to_string()]);
        assert!(scan.files.iter().any(|e| e.kind == Kind::UserData));
        assert!(scan.files.iter().any(|e| e.kind == Kind::ModLoaderIni));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discard_and_move() {
        let dir = tmp("move");
        let upper = dir.join("upper");
        let mods = dir.join("mods");
        std::fs::create_dir_all(upper.join("models")).unwrap();
        std::fs::create_dir_all(upper.join("data")).unwrap();
        std::fs::create_dir_all(mods.join("MyMod/content")).unwrap();
        std::fs::write(mods.join("MyMod/mod.toml"), "mount = [\"content\"]\n").unwrap();
        std::fs::write(upper.join("models/x.dff"), "x").unwrap();
        std::fs::write(upper.join("data/keep.txt"), "k").unwrap();

        let root = layer_root_for(&mods, "MyMod");
        assert_eq!(root, mods.join("MyMod/content"));
        let dest = move_to_layer(&upper, "models/x.dff", &root).unwrap();
        assert_eq!(dest, root.join("models/x.dff"));
        assert!(dest.exists());
        assert!(!upper.join("models/x.dff").exists());

        discard(&upper, "data/keep.txt").unwrap();
        assert!(!upper.join("data/keep.txt").exists());
        // Empty parent pruned.
        assert!(!upper.join("data").exists());

        // Escapes are refused.
        assert!(discard(&upper, "../escape").is_err());
        assert!(discard(&upper, "nope.txt").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn capture_skips_generated_and_userdata() {
        let dir = tmp("capture");
        let upper = dir.join("upper");
        let dest = dir.join("mods/Captured");
        std::fs::create_dir_all(upper.join("models")).unwrap();
        std::fs::create_dir_all(upper.join("userfiles")).unwrap();
        std::fs::create_dir_all(upper.join("modloader")).unwrap();
        std::fs::write(upper.join("models/x.dff"), "x").unwrap();
        std::fs::write(upper.join("models/.wh.gone.dff"), "").unwrap();
        std::fs::write(upper.join("userfiles/GTASAsf1.b"), "save").unwrap();
        std::fs::write(upper.join("modloader/modloader.ini"), "ini").unwrap();

        let report = capture(&upper, &dest, "Captured", Some("userfiles"), true).unwrap();
        assert_eq!(report.moved, 1);
        assert_eq!(report.deleted_skipped, 1);
        assert_eq!(report.userdata_skipped, 1);
        assert_eq!(report.generated_skipped, 1);
        assert!(dest.join("content/models/x.dff").exists());
        assert!(!dest.join("content/userfiles/GTASAsf1.b").exists());
        assert!(!dest.join("content/modloader/modloader.ini").exists());
        let manifest = std::fs::read_to_string(dest.join("mod.toml")).unwrap();
        assert!(manifest.contains("mount"));
        assert!(manifest.contains("content"));

        // Capturing twice into the same destination fails.
        assert!(capture(&upper, &dest, "Captured", Some("userfiles"), true).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
