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
    /// Write end of the guard pipe. Kept open for the life of this process; the
    /// guard child unmounts when it closes (including on a crash/SIGKILL).
    guard_fd: Option<std::os::fd::RawFd>,
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
            guard_fd: None,
        })
    }

    /// Spawns a detached helper that unmounts `merged` shortly after this
    /// process dies, so a killed or crashed gta-mo does not leave the overlay
    /// mounted. Done with fork() instead of a `sh -c` wrapper; the child blocks
    /// on the read end of a pipe until the parent closes the write end (on exit
    /// or death), so there is no PID-reuse race.
    pub fn start_guard(&mut self) {
        let merged = self.merged.clone();

        let mut fds = [0 as libc::c_int; 2];
        // O_CLOEXEC: children that exec (umu-run) must not keep the write end
        // alive, or the guard would never see the parent's death.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            crate::log::warn(
                "No se pudo crear el proceso guardián; un cierre forzado puede dejar el overlay montado.",
            );
            return;
        }
        let (read_fd, write_fd) = (fds[0], fds[1]);

        let pid = unsafe { libc::fork() };
        if pid < 0 {
            unsafe {
                libc::close(read_fd);
                libc::close(write_fd);
            }
            crate::log::warn(
                "No se pudo crear el proceso guardián; un cierre forzado puede dejar el overlay montado.",
            );
            return;
        }
        if pid == 0 {
            // Child: close the write end and block until the parent's write end
            // is closed (normal exit, crash or SIGKILL), then unmount.
            unsafe {
                libc::close(write_fd);
                libc::setsid();
            }
            let mut buf = [0u8; 1];
            loop {
                let n = unsafe { libc::read(read_fd, buf.as_mut_ptr() as *mut libc::c_void, 1) };
                if n <= 0 {
                    break;
                }
            }
            unsafe { libc::close(read_fd) };
            thread::sleep(Duration::from_secs(2));
            if !unmount(&merged, false) {
                unmount(&merged, true);
            }
            std::process::exit(0);
        }

        // Parent: keep only the write end, open for the rest of this process.
        unsafe { libc::close(read_fd) };
        self.guard_fd = Some(write_fd);
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
        // Release the guard: its read() sees EOF and finishes (harmless
        // second unmount attempt if this one already succeeded).
        if let Some(fd) = self.guard_fd.take() {
            unsafe { libc::close(fd) };
        }
    }
}
