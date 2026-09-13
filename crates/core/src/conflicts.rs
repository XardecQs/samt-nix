use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Maximum directory nesting walked when scanning a mod, to avoid a stack
/// overflow on a pathological (or malicious) tree.
const MAX_WALK_DEPTH: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    High,
    Medium,
    Info,
}

#[derive(Debug, Clone)]
pub struct Conflict {
    /// Game-root-relative path provided by more than one enabled mod.
    pub path: String,
    /// Providing mod folders, in overlay priority order (first one wins).
    pub providers: Vec<String>,
    /// True when every provider has identical content (harmless duplicate).
    pub duplicate: bool,
    pub severity: Severity,
}

/// Scans the files contributed by `resolved` mods (in overlay priority order)
/// and reports paths provided by more than one mod.
pub fn scan_conflicts(mods_dir: &Path, resolved: &[String]) -> anyhow::Result<Vec<Conflict>> {
    let mut files: HashMap<String, Vec<(String, PathBuf, u64)>> = HashMap::new();
    for folder in resolved {
        for layer in crate::meta::mod_layers(mods_dir, folder) {
            walk(&layer, &layer, folder, &mut files, 0)?;
        }
    }

    let mut conflicts = Vec::new();
    for (path, providers) in files {
        if providers.len() < 2 {
            continue;
        }
        let duplicate = providers_all_equal(&providers);
        let severity = severity_for(&path);
        conflicts.push(Conflict {
            path,
            providers: providers.iter().map(|(f, _, _)| f.clone()).collect(),
            duplicate,
            severity,
        });
    }
    conflicts.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(conflicts)
}

/// Result of asking "who provides this game-relative path".
pub struct PathProviders {
    /// Folders in overlay priority order (first one wins).
    pub providers: Vec<String>,
    /// File sizes, aligned with `providers`.
    pub sizes: Vec<u64>,
    pub duplicate: bool,
    pub severity: Severity,
}

/// Finds every enabled mod that provides `rel` (a game-root-relative path).
/// Returns `None` when no mod provides it (the file comes from the base game).
pub fn providers_for_path(
    mods_dir: &Path,
    resolved: &[String],
    rel: &str,
) -> anyhow::Result<Option<PathProviders>> {
    let rel = rel.trim_start_matches("./").replace('\\', "/");
    let mut found: Vec<(String, PathBuf, u64)> = Vec::new();
    for folder in resolved {
        for layer in crate::meta::mod_layers(mods_dir, folder) {
            walk_path(&layer, &layer, folder, &rel, &mut found, 0)?;
        }
    }
    if found.is_empty() {
        return Ok(None);
    }
    let duplicate = providers_all_equal(&found);
    Ok(Some(PathProviders {
        providers: found.iter().map(|(f, _, _)| f.clone()).collect(),
        sizes: found.iter().map(|(_, _, s)| *s).collect(),
        duplicate,
        severity: severity_for(&rel),
    }))
}

fn walk_path(
    root: &Path,
    dir: &Path,
    folder: &str,
    rel: &str,
    found: &mut Vec<(String, PathBuf, u64)>,
    depth: usize,
) -> anyhow::Result<()> {
    if depth > MAX_WALK_DEPTH {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            walk_path(root, &path, folder, rel, found, depth + 1)?;
        } else {
            let r = path
                .strip_prefix(root)
                .map_err(|_| anyhow::anyhow!("strip_prefix falló"))?
                .to_string_lossy()
                .replace('\\', "/");
            if r == rel {
                found.push((folder.to_string(), path, entry.metadata()?.len()));
            }
        }
    }
    Ok(())
}

fn walk(
    root: &Path,
    dir: &Path,
    folder: &str,
    files: &mut HashMap<String, Vec<(String, PathBuf, u64)>>,
    depth: usize,
) -> anyhow::Result<()> {
    if depth > MAX_WALK_DEPTH {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            walk(root, &path, folder, files, depth + 1)?;
        } else {
            let rel = path
                .strip_prefix(root)
                .map_err(|_| anyhow::anyhow!("strip_prefix falló"))?
                .to_string_lossy()
                .replace('\\', "/");
            let size = entry.metadata()?.len();
            files
                .entry(rel)
                .or_default()
                .push((folder.to_string(), path, size));
        }
    }
    Ok(())
}

/// All providers with the same size are byte-compared by streaming chunks, so a
/// conflict between very large files never loads them fully into memory. Any
/// difference means the files are not identical duplicates.
fn providers_all_equal(providers: &[(String, PathBuf, u64)]) -> bool {
    let first_size = providers[0].2;
    if providers.iter().any(|(_, _, s)| *s != first_size) {
        return false;
    }
    for (_, path, _) in &providers[1..] {
        let (Ok(mut a), Ok(mut b)) = (
            std::fs::File::open(&providers[0].1),
            std::fs::File::open(path),
        ) else {
            return false;
        };
        if !streams_equal(&mut a, &mut b) {
            return false;
        }
    }
    true
}

fn streams_equal(a: &mut impl Read, b: &mut impl Read) -> bool {
    let mut ba = [0u8; 64 * 1024];
    let mut bb = [0u8; 64 * 1024];
    loop {
        let na = match a.read(&mut ba) {
            Ok(n) => n,
            Err(_) => return false,
        };
        let nb = match b.read(&mut bb) {
            Ok(n) => n,
            Err(_) => return false,
        };
        if na != nb || ba[..na] != bb[..nb] {
            return false;
        }
        if na == 0 {
            return true;
        }
    }
}

fn severity_for(path: &str) -> Severity {
    let p = path.to_lowercase();
    if p == "gta_sa.exe" || p == "gta_sa.pdb" || p.ends_with(".exe") {
        Severity::High
    } else if p.starts_with("modloader/") {
        // Mod Loader manages its own priorities.
        Severity::Info
    } else {
        Severity::Medium
    }
}
