use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::thread;
use std::time::Duration;

/// Resolves the fusermount binary, preferring fuse3's `fusermount3` (which
/// reads /proc/self/mounts) over the legacy fuse2 `fusermount` (which consults
/// /etc/mtab and prints noisy warnings for fuse-overlayfs mounts).
fn fusermount_bin() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        which::which("fusermount3")
            .or_else(|_| which::which("fusermount"))
            .unwrap_or_else(|_| PathBuf::from("fusermount"))
    })
    .as_path()
}

fn unmount(merged: &Path, lazy: bool) -> bool {
    let mut cmd = Command::new(fusermount_bin());
    cmd.arg(if lazy { "-uz" } else { "-u" })
        .arg(merged)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.status().map(|s| s.success()).unwrap_or(false)
}

pub struct OverlayMount {
    merged: PathBuf,
}

impl OverlayMount {
    pub fn mount(lowerdir: &str, upper: &Path, work: &Path, merged: &Path) -> anyhow::Result<Self> {
        if merged.exists() && Self::is_mounted(merged) {
            let _ = std::env::set_current_dir("/");
            crate::log::info("Intentando desmontar overlay anterior...");
            if !Self::unmount_retry(merged, 10, 2000) {
                crate::log::warn("Desmontaje normal fallido, intentando lazy unmount...");
                unmount(merged, true);
                thread::sleep(Duration::from_millis(1000));
            }
        }

        std::fs::create_dir_all(upper)?;
        std::fs::create_dir_all(work)?;
        std::fs::create_dir_all(merged)?;

        let opt = format!(
            "lowerdir={},upperdir={},workdir={}",
            lowerdir,
            upper.display(),
            work.display()
        );

        crate::log::info("Montando capas...");

        let output = Command::new("fuse-overlayfs")
            .arg("-o")
            .arg(&opt)
            .arg(merged)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Error al montar overlay: {stderr}");
        }

        crate::log::info("Overlay montado correctamente.");

        Ok(OverlayMount {
            merged: merged.to_path_buf(),
        })
    }

    /// Spawns a detached helper that unmounts `merged` shortly after this
    /// process dies, so a killed or crashed gta-mo does not leave the overlay
    /// mounted. Done with fork() instead of a `sh -c` wrapper; the child just
    /// polls for the parent's death and then runs fusermount.
    pub fn start_guard(&mut self) {
        let merged = self.merged.clone();
        let self_pid = std::process::id() as libc::pid_t;
        let _fusermount = fusermount_bin().to_owned();

        let pid = unsafe { libc::fork() };
        if pid < 0 {
            crate::log::warn(
                "No se pudo crear el proceso guardián; un cierre forzado puede dejar el overlay montado.",
            );
            return;
        }
        if pid == 0 {
            unsafe { libc::setsid() };
            loop {
                if unsafe { libc::kill(self_pid, 0) } != 0 {
                    break;
                }
                thread::sleep(Duration::from_secs(1));
            }
            thread::sleep(Duration::from_secs(2));
            if !unmount(&merged, false) {
                unmount(&merged, true);
            }
            std::process::exit(0);
        }
    }

    pub fn merged_path(&self) -> &Path {
        &self.merged
    }

    fn is_mounted(merged: &Path) -> bool {
        let Ok(contents) = std::fs::read_to_string("/proc/self/mountinfo") else {
            return false;
        };
        let escaped = merged
            .display()
            .to_string()
            .replace('\\', "\\134")
            .replace(' ', "\\040")
            .replace('\t', "\\011")
            .replace('\n', "\\012");
        contents.lines().any(|line| {
            line.split_whitespace()
                .nth(4)
                .map(|p| p == escaped)
                .unwrap_or(false)
        })
    }

    fn unmount_retry(merged: &Path, retries: u32, delay_ms: u64) -> bool {
        for i in 0..retries {
            if unmount(merged, false) {
                return true;
            }
            if i < retries - 1 {
                thread::sleep(Duration::from_millis(delay_ms));
            }
        }
        false
    }
}

impl Drop for OverlayMount {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir("/");
        if Self::is_mounted(&self.merged) && !Self::unmount_retry(&self.merged, 15, 2000) {
            crate::log::warn("Desmontaje bloqueado, intentando lazy unmount...");
            unmount(&self.merged, true);
        }
    }
}
