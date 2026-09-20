//! Persistent state of a manual overlay mount (`gta-mo mount`).
//!
//! A manual mount is kept alive by a detached daemon (see
//! [`crate::launcher::mount`]); this record is how `unmount`, `status` and the
//! GUI find that daemon and the mount it owns. It lives at `run/mount.json`
//! and is removed when the daemon tears the mount down.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A manually mounted overlay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MountInfo {
    /// Profile slug whose mod set is mounted.
    pub slug: String,
    /// Human-readable profile name.
    pub profile_name: String,
    /// True for an ephemeral profile: its upper is captured/discarded on unmount.
    pub temp: bool,
    pub upper: PathBuf,
    pub merged: PathBuf,
    /// Kernel mount id, so `unmount` never tears down a newer, unrelated mount.
    pub mount_id: u64,
    /// Pid of the daemon holding the mount.
    pub pid: u32,
}

/// `game_root/run/mount.json`.
pub fn state_path(game_root: &Path) -> PathBuf {
    game_root.join("run/mount.json")
}

/// Atomically writes the state file.
pub fn write(path: &Path, info: &MountInfo) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    let json = serde_json::to_string_pretty(info)?;
    std::fs::write(&tmp, json)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// Reads the state file, if present and well-formed.
pub fn read(path: &Path) -> Option<MountInfo> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Removes the state file (ignoring a missing one).
pub fn remove(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// True when the recorded daemon still exists.
pub fn is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

/// Reads the state and drops it when the daemon is no longer alive, returning
/// `None` in that case so callers never act on a stale record.
pub fn read_live(path: &Path) -> Option<MountInfo> {
    let info = read(path)?;
    if is_alive(info.pid) {
        Some(info)
    } else {
        remove(path);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_stale_cleanup() {
        let dir = std::env::temp_dir().join(format!("gta-mo-mount-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = state_path(&dir);
        let info = MountInfo {
            slug: "default".into(),
            profile_name: "Default".into(),
            temp: false,
            upper: dir.join("upper"),
            merged: dir.join("merged"),
            mount_id: 42,
            pid: std::process::id(),
        };
        write(&path, &info).unwrap();
        assert_eq!(read(&path), Some(info.clone()));
        // Our own pid is alive.
        assert!(read_live(&path).is_some());

        // A dead pid drops the record.
        let dead = MountInfo { pid: 0, ..info };
        write(&path, &dead).unwrap();
        assert!(read_live(&path).is_none());
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
