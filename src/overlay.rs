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
        let _ = Command::new("fusermount")
            .arg("-u")
            .arg(merged)
            .status();

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
                "while kill -0 {pid} 2>/dev/null; do sleep 1; done; sleep 2; fusermount -u \"{merged}\" 2>/dev/null || true"
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
            let output = Command::new("fusermount")
                .arg("-u")
                .arg(merged)
                .output();

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
        if let Some(ref mut child) = self.guard_child {
            let _ = child.kill();
        }
        Self::unmount_retry(&self.merged, 5, 1000);
    }
}

#[allow(dead_code)]
pub fn unmount(merged: &Path) {
    let _ = Command::new("fusermount")
        .arg("-u")
        .arg(merged)
        .status();
}
