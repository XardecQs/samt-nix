//! Run the Wine prefix from a tmpfs copy so a long game session cannot evict it
//! from RAM.
//!
//! The prefix on disk (`<game_root>/pfx`) stays the source of truth. When RAM
//! mode is enabled we `rsync` it to `/dev/shm/gta-mo/pfx` and point `WINEPREFIX`
//! there; after the game exits we `rsync` it back and drop the copy. A small
//! marker records the disk prefix's *signature* so an early `gta-mo preload`
//! (see the GUI prewarm) can be reused by the next launch instead of copying
//! again.
//!
//! This is deliberately user-space only: `/dev/shm` is an ordinary writable
//! tmpfs, no `mlock`/`RLIMIT_MEMLOCK` and no system configuration required.

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::log;

/// Environment override for the tmpfs root (mainly for tests).
pub const RAM_DIR_ENV: &str = "GTA_MO_RAM_DIR";

/// Default tmpfs root.
const DEFAULT_RAM_DIR: &str = "/dev/shm/gta-mo";

/// Kept free margin on the tmpfs (in addition to the prefix size).
const SPACE_MARGIN: u64 = 32 * 1024 * 1024;

#[derive(Debug, Default, Clone, Copy)]
pub struct PrepareReport {
    /// True when the prefix was actually copied to tmpfs this time.
    pub copied: bool,
    /// True when an already-prepared tmpfs copy was reused.
    pub reused: bool,
    /// Bytes materialized from Proton symlinks (only when copied).
    pub materialized: u64,
    pub elapsed: Duration,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SyncReport {
    pub synced: bool,
    pub elapsed: Duration,
}

/// A prefix ready to be handed to `umu`/Proton. Dropping it does nothing;
/// call [`PreparedPrefix::finalize`] to sync back and clean the tmpfs.
pub struct PreparedPrefix {
    path: PathBuf,
    disk: PathBuf,
    tmpfs: Option<PathBuf>,
    proton: Option<PathBuf>,
    report: PrepareReport,
}

impl PreparedPrefix {
    /// Path to pass as `WINEPREFIX`.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// True when the game runs against the tmpfs copy.
    pub fn using_tmpfs(&self) -> bool {
        self.tmpfs.is_some()
    }

    pub fn report(&self) -> PrepareReport {
        self.report
    }

    /// Prepares the prefix: copies it to tmpfs (or reuses an already-prepared
    /// copy) and, when `materialize_proton` is set, replaces the prefix's file
    /// symlinks into `proton` with real copies so Wine reads them from RAM.
    /// Falls back to the disk prefix when RAM mode is off/unavailable.
    pub fn prepare(
        disk: &Path,
        enabled: bool,
        proton: Option<&Path>,
        materialize_proton: bool,
    ) -> Self {
        let disk_only = |report: PrepareReport| Self {
            path: disk.to_path_buf(),
            disk: disk.to_path_buf(),
            tmpfs: None,
            proton: None,
            report,
        };

        if !enabled || !disk.is_dir() {
            return disk_only(PrepareReport::default());
        }
        let Some(root) = ram_root() else {
            log::warn("prefix_ram: no hay tmpfs disponible; usando el prefijo del disco.");
            return disk_only(PrepareReport::default());
        };
        let tmpfs = root.join("pfx");
        let marker = root.join(".prepared");

        let signature = prefix_signature(disk, proton);
        if tmpfs.is_dir()
            && std::fs::read_to_string(&marker)
                .map(|m| m.trim() == signature)
                .unwrap_or(false)
        {
            return Self {
                path: tmpfs.clone(),
                disk: disk.to_path_buf(),
                tmpfs: Some(tmpfs),
                proton: proton.map(Path::to_path_buf),
                report: PrepareReport {
                    copied: false,
                    reused: true,
                    materialized: 0,
                    elapsed: Duration::ZERO,
                },
            };
        }

        let needed = signature_bytes(disk) + SPACE_MARGIN;
        if free_bytes(&root).map(|f| f < needed).unwrap_or(false) {
            log::warn(format!(
                "prefix_ram: poco espacio en {}; usando el prefijo del disco.",
                root.display()
            ));
            return disk_only(PrepareReport::default());
        }

        if let Err(e) = std::fs::create_dir_all(&root) {
            log::warn(format!(
                "prefix_ram: no se pudo crear {}: {e}",
                root.display()
            ));
            return disk_only(PrepareReport::default());
        }

        let started = Instant::now();
        if let Err(e) = rsync(disk, &tmpfs, true) {
            log::warn(format!(
                "prefix_ram: copia a tmpfs falló ({e}); usando disco."
            ));
            let _ = std::fs::remove_dir_all(&tmpfs);
            return disk_only(PrepareReport::default());
        }
        let materialized = if materialize_proton {
            match proton {
                Some(proton) => materialize_proton_symlinks(&tmpfs, proton),
                None => 0,
            }
        } else {
            0
        };
        let _ = std::fs::write(&marker, &signature);

        Self {
            path: tmpfs.clone(),
            disk: disk.to_path_buf(),
            tmpfs: Some(tmpfs),
            proton: proton.map(Path::to_path_buf),
            report: PrepareReport {
                copied: true,
                reused: false,
                materialized,
                elapsed: started.elapsed(),
            },
        }
    }

    /// Syncs the tmpfs copy back to disk. On success the copy is **kept** (and
    /// its marker refreshed) so the next launch reuses it without copying; on
    /// failure it is dropped so the next launch starts from a clean disk state.
    pub fn finalize(self) -> SyncReport {
        let Some(tmpfs) = self.tmpfs else {
            return SyncReport::default();
        };
        let started = Instant::now();
        if tmpfs.is_dir() {
            match rsync(&tmpfs, &self.disk, false) {
                Ok(()) => {
                    if let Some(parent) = tmpfs.parent() {
                        let _ = std::fs::write(
                            parent.join(".prepared"),
                            prefix_signature(&self.disk, self.proton.as_deref()),
                        );
                    }
                    return SyncReport {
                        synced: true,
                        elapsed: started.elapsed(),
                    };
                }
                Err(e) => {
                    log::warn(format!("prefix_ram: sincronización a disco falló: {e}"));
                    let _ = std::fs::remove_dir_all(&tmpfs);
                    if let Some(parent) = tmpfs.parent() {
                        let _ = std::fs::remove_file(parent.join(".prepared"));
                    }
                }
            }
        } else if let Some(parent) = tmpfs.parent() {
            let _ = std::fs::remove_file(parent.join(".prepared"));
        }
        SyncReport {
            synced: false,
            elapsed: started.elapsed(),
        }
    }
}

/// Result of [`clean`].
#[derive(Debug, Default, Clone, Copy)]
pub struct CleanReport {
    pub existed: bool,
    pub bytes: u64,
}

/// Removes the tmpfs prefix copy (and its marker). The disk prefix is untouched.
pub fn clean() -> CleanReport {
    let Some(root) = ram_root() else {
        return CleanReport::default();
    };
    if !root.exists() {
        return CleanReport::default();
    }
    let bytes = dir_size(&root);
    let _ = std::fs::remove_dir_all(&root);
    CleanReport {
        existed: true,
        bytes,
    }
}

/// Path of the prepared tmpfs prefix, if there is one.
pub fn status() -> Option<PathBuf> {
    let pfx = ram_root()?.join("pfx");
    pfx.is_dir().then_some(pfx)
}

/// Size in bytes of the prepared tmpfs prefix copy, if any.
pub fn prepared_bytes() -> u64 {
    match status() {
        Some(pfx) => dir_size(&pfx),
        None => 0,
    }
}

fn dir_size(root: &Path) -> u64 {
    let mut stack = vec![root.to_path_buf()];
    let mut total = 0u64;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(md) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if md.is_dir() {
                stack.push(path);
            } else if md.is_file() {
                total += md.len();
            }
        }
    }
    total
}

fn ram_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(RAM_DIR_ENV) {
        let p = PathBuf::from(dir);
        if p.is_dir() || p.parent().map(|d| d.is_dir()).unwrap_or(false) {
            return Some(p);
        }
    }
    let shm = PathBuf::from(DEFAULT_RAM_DIR);
    if shm.parent().map(|p| p.is_dir()).unwrap_or(false) {
        Some(shm)
    } else {
        None
    }
}

/// Recursively copies `src`'s contents into `dst` with `rsync -rlpt` (symlinks,
/// permissions and times preserved; no owner/group, which avoids privilege
/// issues inside the Steam user namespace).
fn rsync(src: &Path, dst: &Path, delete: bool) -> std::io::Result<()> {
    let mut cmd = Command::new("rsync");
    cmd.arg("-rlpt").arg("--no-owner").arg("--no-group");
    if delete {
        cmd.arg("--delete");
    }
    let src_contents = format!("{}/", src.display());
    let out = cmd
        .arg("--")
        .arg(&src_contents)
        .arg(dst)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

/// A cheap signature of the disk prefix: entry count, total size, newest mtime
/// and the Proton install (path + mtime). Used to decide whether an
/// already-prepared tmpfs copy is still valid.
fn prefix_signature(disk: &Path, proton: Option<&Path>) -> String {
    let (count, bytes, newest) = walk_signature(disk);
    let proton_tag = proton
        .map(|p| {
            let mtime = std::fs::metadata(p)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            format!("{}:{}", p.display(), mtime)
        })
        .unwrap_or_default();
    format!("{count}:{bytes}:{newest}:{proton_tag}")
}

/// Replaces the prefix's file symlinks that resolve inside `proton` with real
/// copies, so Wine loads its system DLLs from the tmpfs (RAM) instead of the
/// Proton install on disk. Directory symlinks (`dosdevices`, …) are left as-is.
fn materialize_proton_symlinks(prefix: &Path, proton: &Path) -> u64 {
    let proton_canon = std::fs::canonicalize(proton).ok();
    let mut stack = vec![prefix.to_path_buf()];
    let mut bytes = 0u64;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(path);
                continue;
            }
            if !ft.is_symlink() {
                continue;
            }
            // Resolve the link and only act on files that live inside Proton.
            let Ok(real) = std::fs::canonicalize(&path) else {
                continue;
            };
            if let Some(pc) = &proton_canon {
                if !real.starts_with(pc) {
                    continue;
                }
            }
            let Ok(md) = std::fs::metadata(&real) else {
                continue;
            };
            if !md.is_file() {
                continue;
            }
            let Ok(content) = std::fs::read(&real) else {
                continue;
            };
            if std::fs::remove_file(&path).is_err() {
                continue;
            }
            if std::fs::write(&path, &content).is_ok() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(
                        &path,
                        std::fs::Permissions::from_mode(md.permissions().mode()),
                    );
                }
                bytes += md.len();
            }
        }
    }
    bytes
}

fn signature_bytes(disk: &Path) -> u64 {
    walk_signature(disk).1
}

fn walk_signature(root: &Path) -> (u64, u64, i128) {
    let mut stack = vec![root.to_path_buf()];
    let mut count = 0u64;
    let mut bytes = 0u64;
    let mut newest: i128 = 0;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(md) = entry.metadata() else { continue };
            count += 1;
            if md.is_dir() {
                stack.push(entry.path());
            } else if md.is_file() {
                bytes += md.len();
            }
            if let Ok(modified) = md.modified() {
                if let Ok(d) = modified.duration_since(std::time::UNIX_EPOCH) {
                    // Seconds (not nanos): tolerates mtime rounding differences
                    // between btrfs and tmpfs after an rsync.
                    newest = newest.max(d.as_secs() as i128);
                }
            }
        }
    }
    (count, bytes, newest)
}

#[cfg(unix)]
fn free_bytes(path: &Path) -> Option<u64> {
    let c = CString::new(path.to_str()?).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

#[cfg(not(unix))]
fn free_bytes(_path: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("gta-mo-{name}-{}", std::process::id()))
    }

    #[test]
    fn signature_changes_with_content() {
        let dir = tmp("sig");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a"), b"hello").unwrap();
        let s1 = prefix_signature(&dir, None);
        std::fs::write(dir.join("b"), b"world!").unwrap();
        let s2 = prefix_signature(&dir, None);
        assert_ne!(s1, s2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn disabled_uses_disk() {
        let disk = tmp("disk-off");
        let _ = std::fs::remove_dir_all(&disk);
        std::fs::create_dir_all(&disk).unwrap();
        let p = PreparedPrefix::prepare(&disk, false, None, false);
        assert!(!p.using_tmpfs());
        assert_eq!(p.path(), disk.as_path());
        let _ = std::fs::remove_dir_all(&disk);
    }

    #[test]
    fn tmpfs_roundtrip_persist_and_clean() {
        if !Path::new("/dev/shm").is_dir() {
            return;
        }
        let root = PathBuf::from("/dev/shm/gta-mo-test");
        let _ = std::fs::remove_dir_all(&root);
        std::env::set_var(RAM_DIR_ENV, root.display().to_string());
        let disk = tmp("rt");
        let _ = std::fs::remove_dir_all(&disk);
        std::fs::create_dir_all(disk.join("drive_c")).unwrap();
        std::fs::write(disk.join("drive_c/system.reg"), b"reg").unwrap();

        let p = PreparedPrefix::prepare(&disk, true, None, false);
        assert!(p.using_tmpfs());
        // Written through the tmpfs copy...
        std::fs::write(p.path().join("drive_c/new.reg"), b"new").unwrap();
        let sync = p.finalize();
        assert!(sync.synced);
        // ...lands back on disk...
        assert!(disk.join("drive_c/new.reg").exists());
        // ...and the tmpfs copy is kept for the next launch.
        assert!(root.join("pfx").exists());
        assert!(status().is_some());

        // A second prepare reuses it instead of copying.
        let p2 = PreparedPrefix::prepare(&disk, true, None, false);
        assert!(p2.using_tmpfs());
        assert!(p2.report().reused);
        drop(p2);

        // clean() removes it.
        let report = clean();
        assert!(report.existed);
        assert!(!root.exists());

        std::env::remove_var(RAM_DIR_ENV);
        let _ = std::fs::remove_dir_all(&disk);
        let _ = std::fs::remove_dir_all(&root);
    }
}
