//! Inspect and maintain the `umu-launcher` runtime (steamrt) install.
//!
//! `gta-mo` runs the game through `umu-run`, which on every launch may try to
//! update its `steamrt` runtime under `$XDG_DATA_HOME/umu`. When a cached or
//! mirrored archive has a bad digest this fails noisily and wastes time, even
//! though the already-installed runtime works. The launcher sets
//! `UMU_RUNTIME_UPDATE=0` by default (config `umu_runtime_update`) and this
//! module backs the `gta-mo ctl runtime` command used to inspect, clean the
//! download cache and force a reinstall.

use std::path::{Path, PathBuf};

/// Directory holding the umu runtimes (`$UMU_FOLDERS_PATH/umu`,
/// `$XDG_DATA_HOME/umu` or `~/.local/share/umu`).
pub fn data_dir() -> PathBuf {
    if let Some(base) = std::env::var_os("UMU_FOLDERS_PATH") {
        return PathBuf::from(base).join("umu");
    }
    dirs::data_dir()
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("umu")
}

/// Directory where umu caches partial downloads (`$XDG_CACHE_HOME/umu`).
pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("umu")
}

/// Path of the steamrt4 runtime inside the data dir.
pub fn steamrt4_dir() -> PathBuf {
    data_dir().join("steamrt4")
}

/// A snapshot of the current runtime install, for `ctl runtime status`.
#[derive(Debug, Clone)]
pub struct RuntimeStatus {
    pub runtime_dir: PathBuf,
    /// `.installed.ok` marker present: umu considers the runtime installed.
    pub installed: bool,
    /// Installed platform directory name (e.g. `steamrt4_platform_4.0.…`).
    pub version: Option<String>,
    pub cache_dir: PathBuf,
    pub cache_files: u64,
    pub cache_bytes: u64,
}

/// Reads the current runtime state without modifying anything.
pub fn status() -> RuntimeStatus {
    let runtime_dir = steamrt4_dir();
    let installed = runtime_dir.join(".installed.ok").is_file();
    let version = newest_platform_dir(&runtime_dir)
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let cache_dir = cache_dir();
    let (cache_files, cache_bytes) = dir_stats(&cache_dir);
    RuntimeStatus {
        runtime_dir,
        installed,
        version,
        cache_dir,
        cache_files,
        cache_bytes,
    }
}

/// Newest `steamrt4_platform_*` directory, if any.
fn newest_platform_dir(runtime_dir: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(runtime_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("steamrt4_platform_"))
                    .unwrap_or(false)
        })
        .collect();
    dirs.sort();
    dirs.pop()
}

/// Deletes umu's partial-download cache. Returns the number of bytes freed.
/// This is safe: only in-progress/partial archives live there.
pub fn clean_cache() -> anyhow::Result<u64> {
    let dir = cache_dir();
    let (_, bytes) = dir_stats(&dir);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(bytes)
}

/// Removes the installed steamrt4 runtime so umu reinstalls it on next launch.
pub fn reinstall_runtime() -> anyhow::Result<()> {
    let dir = steamrt4_dir();
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    // A partial download of the runtime may also be sitting in the cache.
    let _ = clean_cache();
    Ok(())
}

/// Recursively counts `(files, bytes)` under `dir`, ignoring errors.
fn dir_stats(dir: &Path) -> (u64, u64) {
    let mut files = 0u64;
    let mut bytes = 0u64;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (0, 0);
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            let (f, b) = dir_stats(&entry.path());
            files += f;
            bytes += b;
        } else {
            files += 1;
            bytes += meta.len();
        }
    }
    (files, bytes)
}
