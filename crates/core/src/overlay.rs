use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

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
                let _ = Command::new("fusermount").arg("-uz").arg(merged).status();
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
            let ok = Command::new("fusermount")
                .arg("-u")
                .arg(&merged)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                let _ = Command::new("fusermount").arg("-uz").arg(&merged).status();
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
            let output = Command::new("fusermount").arg("-u").arg(merged).output();

            match output {
                Ok(o) if o.status.success() => return true,
                _ if i < retries - 1 => {
                    thread::sleep(Duration::from_millis(delay_ms));
                }
                _ => {}
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
            let _ = Command::new("fusermount")
                .arg("-uz")
                .arg(&self.merged)
                .status();
        }
    }
}
