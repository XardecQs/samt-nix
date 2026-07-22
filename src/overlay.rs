use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct OverlayMount {
    merged: PathBuf,
    guard_child: Option<std::process::Child>,
}

impl OverlayMount {
    pub fn mount(lowerdir: &str, upper: &Path, work: &Path, merged: &Path) -> anyhow::Result<Self> {
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
                "while kill -0 {pid} 2>/dev/null; do sleep 1; done; fusermount -u \"{merged}\" 2>/dev/null || true"
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
}

impl Drop for OverlayMount {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.guard_child {
            let _ = child.kill();
        }
        let _ = Command::new("fusermount")
            .arg("-u")
            .arg(&self.merged)
            .status();
    }
}

#[allow(dead_code)]
pub fn unmount(merged: &Path) {
    let _ = Command::new("fusermount")
        .arg("-u")
        .arg(merged)
        .status();
}
