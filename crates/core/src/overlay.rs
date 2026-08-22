use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

pub struct OverlayMount {
    merged: PathBuf,
    guard_child: Option<std::process::Child>,
}

impl OverlayMount {
    pub fn mount(lowerdir: &str, upper: &Path, work: &Path, merged: &Path) -> anyhow::Result<Self> {
        if merged.exists() {
            let _ = std::env::set_current_dir("/");
            crate::db::log::info("Intentando desmontar overlay anterior...");
            if !Self::unmount_retry(merged, 10, 2000) {
                crate::db::log::warn("Desmontaje normal fallido, intentando lazy unmount...");
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

        crate::db::log::info("Montando capas...");

        let output = Command::new("fuse-overlayfs")
            .arg("-o")
            .arg(&opt)
            .arg(merged)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Error al montar overlay: {stderr}");
        }

        crate::db::log::info("Overlay montado correctamente.");

        Ok(OverlayMount {
            merged: merged.to_path_buf(),
            guard_child: None,
        })
    }

    pub fn start_guard(&mut self) {
        let merged = self.merged.display().to_string();
        let pid = std::process::id().to_string();

        let child = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "while kill -0 {pid} 2>/dev/null; do sleep 1; done; sleep 2; fusermount -u \"{merged}\" 2>/dev/null || fusermount -uz \"{merged}\" 2>/dev/null || true"
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok();

        self.guard_child = child;
    }

    pub fn merged_path(&self) -> &Path {
        &self.merged
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
        if !Self::unmount_retry(&self.merged, 15, 2000) {
            crate::db::log::warn("Desmontaje bloqueado, intentando lazy unmount...");
            let _ = Command::new("fusermount")
                .arg("-uz")
                .arg(&self.merged)
                .status();
        }
        if let Some(ref mut child) = self.guard_child {
            let _ = child.kill();
        }
    }
}
