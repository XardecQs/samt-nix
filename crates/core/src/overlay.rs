use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
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
    /// Mount ID of the overlay we created, so we never unmount a *newer* mount
    /// that replaced ours on the same path (e.g. a quick relaunch while this
    /// process' guard is still pending).
    mount_id: Option<u64>,
    /// Write end of the guard pipe. Kept open for the life of this process; the
    /// guard child unmounts when it closes (including on a crash/SIGKILL).
    guard_fd: Option<std::os::fd::RawFd>,
}

impl OverlayMount {
    pub fn mount(lowerdir: &str, upper: &Path, work: &Path, merged: &Path) -> anyhow::Result<Self> {
        if merged.exists() && Self::is_mounted(merged) {
            let _ = std::env::set_current_dir("/");
            crate::log::info("Intentando desmontar overlay anterior...");
            if !Self::unmount_retry(merged, 10, 1000) {
                crate::log::warn("Desmontaje normal fallido, intentando lazy unmount...");
                unmount(merged, true);
            }
            if !wait_unmounted(merged, 50, 100) {
                anyhow::bail!(
                    "No se pudo desmontar el overlay anterior en '{}'. Cierra cualquier \
                     programa que esté dentro de esa carpeta (explorador de archivos, \
                     terminal) y vuelve a intentarlo.",
                    merged.display()
                );
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

        let mount_id = mount_id_at(merged);
        if mount_id.is_none() {
            crate::log::warn(
                "No se pudo identificar el montaje recién creado; el guardián no lo desmontará.",
            );
        }

        Ok(OverlayMount {
            merged: merged.to_path_buf(),
            mount_id,
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
        // Only this mount may be unmounted by the guard: a newer overlay could
        // have replaced ours on the same path in the meantime.
        let our_mount_id = self.mount_id;

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
            // Do not keep unrelated inherited descriptors (notably the launcher
            // lock) open: this helper may outlive the process that created the
            // mount by a couple of seconds, and holding the lock would block an
            // immediate relaunch.
            close_inherited_fds(read_fd);
            let mut buf = [0u8; 1];
            loop {
                let n = unsafe { libc::read(read_fd, buf.as_mut_ptr() as *mut libc::c_void, 1) };
                if n <= 0 {
                    break;
                }
            }
            unsafe { libc::close(read_fd) };
            thread::sleep(Duration::from_secs(2));
            // Strict identity check: never tear down a newer overlay that took
            // this path after a quick relaunch.
            if our_mount_id.is_some()
                && mount_id_at(&merged) == our_mount_id
                && !unmount(&merged, false)
            {
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

    /// Mount ID of this overlay (see [`mount_id_at`]).
    pub fn mount_id(&self) -> Option<u64> {
        self.mount_id
    }

    fn is_mounted(merged: &Path) -> bool {
        mount_id_at(merged).is_some()
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

/// Closes every open descriptor above stderr except `keep`, so a forked helper
/// does not inherit (and pin) things like the launcher lock file.
fn close_inherited_fds(keep: libc::c_int) {
    let Ok(entries) = std::fs::read_dir("/proc/self/fd") else {
        return;
    };
    let mut fds: Vec<libc::c_int> = entries
        .flatten()
        .filter_map(|e| e.file_name().to_string_lossy().parse::<libc::c_int>().ok())
        .collect();
    fds.sort_unstable();
    for fd in fds {
        if fd > 2 && fd != keep {
            unsafe {
                libc::close(fd);
            }
        }
    }
}

/// Escapes a path the way `/proc/self/mountinfo` does for the mount point field.
fn escape_mount_path(path: &Path) -> String {
    path.display()
        .to_string()
        .replace('\\', "\\134")
        .replace(' ', "\\040")
        .replace('\t', "\\011")
        .replace('\n', "\\012")
}

/// Mount ID of the filesystem mounted exactly at `path`, if any. Mount IDs are
/// unique and monotonic, so they identify a specific mount even if the path is
/// later reused by another mount.
fn mount_id_at(path: &Path) -> Option<u64> {
    let escaped = escape_mount_path(path);
    let contents = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    contents.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let id = fields.next()?.parse::<u64>().ok()?;
        if line.split_whitespace().nth(4)? == escaped {
            Some(id)
        } else {
            None
        }
    })
}

/// Mount ID of the filesystem mounted exactly at `path`, if any (public
/// wrapper used to validate a manual mount before tearing it down).
pub fn current_mount_id(path: &Path) -> Option<u64> {
    mount_id_at(path)
}

/// Unmounts `merged` only if it is still the mount identified by `expected`,
/// so a newer, unrelated mount on the same path is never touched. Tries a
/// clean unmount first, then a lazy one.
pub fn unmount_matching(merged: &Path, expected: u64) -> bool {
    if mount_id_at(merged) != Some(expected) {
        return false;
    }
    if unmount(merged, false) {
        return true;
    }
    unmount(merged, true)
}

/// Whether `path` is currently a mount point.
pub fn is_mounted(path: &Path) -> bool {
    mount_id_at(path).is_some()
}

/// Set by the signal handler in [`wait_for_shutdown`].
static SHUTDOWN: AtomicBool = AtomicBool::new(false);
/// Write end of the self-pipe woken by the signal handler.
static WAKE_FD: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_shutdown_signal(_sig: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
    let fd = WAKE_FD.load(Ordering::SeqCst);
    if fd >= 0 {
        let byte = [1u8];
        // write() is async-signal-safe; a full pipe is harmless.
        unsafe {
            libc::write(fd, byte.as_ptr() as *const libc::c_void, 1);
        }
    }
}

/// Blocks until the process receives SIGTERM or SIGINT, used by the manual
/// mount daemon. A self-pipe avoids the check-then-`pause()` race; if the pipe
/// cannot be created it falls back to a `pause()` loop with the atomic flag.
pub fn wait_for_shutdown() {
    let mut fds = [0 as libc::c_int; 2];
    let have_pipe = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } == 0;
    if have_pipe {
        WAKE_FD.store(fds[1], Ordering::SeqCst);
    }

    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = on_shutdown_signal as *const () as usize;
    action.sa_flags = 0;
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(libc::SIGTERM, &action, std::ptr::null_mut());
        libc::sigaction(libc::SIGINT, &action, std::ptr::null_mut());
    }

    if have_pipe {
        let mut buf = [0u8; 1];
        loop {
            if SHUTDOWN.load(Ordering::SeqCst) {
                break;
            }
            let n = unsafe { libc::read(fds[0], buf.as_mut_ptr() as *mut libc::c_void, 1) };
            // A byte (n > 0) or a closed pipe (n == 0) means: tear down. On
            // EINTR from another signal, retry unless we are shutting down.
            if n != -1 || SHUTDOWN.load(Ordering::SeqCst) {
                break;
            }
        }
        unsafe {
            libc::close(fds[0]);
            libc::close(fds[1]);
        }
    } else {
        while !SHUTDOWN.load(Ordering::SeqCst) {
            unsafe { libc::pause() };
        }
    }
}

/// Polls until `path` is no longer a mount point (or `tries` are exhausted).
fn wait_unmounted(path: &Path, tries: u32, delay_ms: u64) -> bool {
    for i in 0..tries {
        if !OverlayMount::is_mounted(path) {
            return true;
        }
        if i < tries - 1 {
            thread::sleep(Duration::from_millis(delay_ms));
        }
    }
    !OverlayMount::is_mounted(path)
}

impl Drop for OverlayMount {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir("/");
        // Only unmount if this is still our mount (or we could not identify it,
        // in which case this owner is the only candidate).
        let ours = match self.mount_id {
            Some(id) => mount_id_at(&self.merged) == Some(id),
            None => true,
        };
        if ours && Self::is_mounted(&self.merged) && !Self::unmount_retry(&self.merged, 15, 2000) {
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
