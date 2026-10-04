//! Page-cache preloading to speed up game startup.
//!
//! On slow storage (spinning disks, encrypted/CoW filesystems) the Wine prefix
//! and the game's startup files are read on demand, which shows up as multi
//! second stalls while the loader maps a single DLL. Warming the page cache
//! before (or while) `umu`/Proton boots lets the kernel read those files ahead
//! of time, so the loader finds them in RAM.
//!
//! Warming runs on a background thread at *idle* I/O priority so it never
//! starves the actual launch, and it stops as soon as the game process appears
//! (the caller flips the shared stop flag).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// How much to preload before launching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreloadMode {
    /// Do not preload anything.
    Off,
    /// Warm the Wine prefix, the game executable and the mods' startup files
    /// (ASI/DLL/CLEO/CLEO scripts, configs and the `modloader/` tree).
    Prefix,
    /// Warm the Wine prefix, the whole game base and every enabled mod file.
    Full,
}

impl PreloadMode {
    /// Parses the `preload` config value. Unknown values fall back to
    /// [`PreloadMode::Prefix`].
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("off") | Some("none") | Some("false") | Some("0") => Self::Off,
            Some("full") => Self::Full,
            Some("prefix") | Some("true") | Some("1") | Some("on") | Some("") | None => {
                Self::Prefix
            }
            Some(_) => Self::Prefix,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Prefix => "prefix",
            Self::Full => "full",
        }
    }

    pub fn is_off(self) -> bool {
        matches!(self, Self::Off)
    }
}

/// General warning shown whenever `full` preloading is about to run. It is
/// intentionally generic (no machine-specific numbers).
pub const FULL_WARNING: &str = "\
ADVERTENCIA: preload = \"full\" lee TODO el juego base y TODOS los mods activos
(puede sumar decenas de GB) para dejarlos en la caché de página del sistema (RAM).
En discos lentos puede tardar bastante y competir por el I/O con el arranque del
juego; si el conjunto no cabe en RAM puede causar presión de memoria y swap.
Para reducir el tiempo hasta que aparece la ventana basta con preload = \"prefix\".
Usa \"full\" solo de forma puntual o previa con `gta-mo preload`.";

/// Files smaller than this are read fully (which definitely caches them); bigger
/// ones only get a non-blocking `WILLNEED` hint.
const READ_FULLY_UP_TO: u64 = 4 * 1024 * 1024;

/// In `Prefix` mode, skip startup files bigger than this: it keeps the warmer
/// fast by leaving the game's big data blobs (`gta3.img`, TXD streams…) to the
/// on-demand loader, which needs them later anyway.
const PREFIX_MAX_FILE: u64 = 64 * 1024 * 1024;

#[derive(Debug, Default, Clone, Copy)]
pub struct WarmStats {
    pub files: u64,
    pub bytes: u64,
    /// True when the warmer was interrupted because the game already started.
    pub stopped: bool,
}

/// Handle to a running background preload.
pub struct Preload {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<WarmStats>>,
}

impl Preload {
    /// Spawns the warmer. Returns `None` for [`PreloadMode::Off`] or if the
    /// thread cannot be created.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        mode: PreloadMode,
        wine_prefix: &Path,
        base_game: &Path,
        game_exe: &str,
        layer_dirs: &[PathBuf],
        stop: Arc<AtomicBool>,
    ) -> Option<Self> {
        if mode.is_off() {
            return None;
        }
        let wine_prefix = wine_prefix.to_path_buf();
        let base_game = base_game.to_path_buf();
        let game_exe = game_exe.to_string();
        let dirs = layer_dirs.to_vec();
        let stop_thread = stop.clone();
        let handle = std::thread::Builder::new()
            .name("gta-mo-preload".into())
            .spawn(move || {
                set_idle_io();
                warm(
                    mode,
                    &wine_prefix,
                    &base_game,
                    &game_exe,
                    &dirs,
                    &stop_thread,
                )
            })
            .ok()?;
        Some(Self {
            stop,
            handle: Some(handle),
        })
    }

    /// Shared flag the caller can flip to stop the warmer early (e.g. once the
    /// game process is detected).
    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }

    /// Signals the warmer to stop and waits for it, returning its stats.
    pub fn stop_and_join(mut self) -> WarmStats {
        self.stop.store(true, Ordering::Relaxed);
        match self.handle.take() {
            Some(h) => h.join().unwrap_or_default(),
            None => WarmStats::default(),
        }
    }
}

/// Warms the page cache synchronously (used by the `gta-mo preload` command and
/// by the background thread).
pub fn warm(
    mode: PreloadMode,
    wine_prefix: &Path,
    base_game: &Path,
    game_exe: &str,
    layer_dirs: &[PathBuf],
    stop: &AtomicBool,
) -> WarmStats {
    let mut stats = WarmStats::default();
    if mode.is_off() {
        return stats;
    }

    // 1. The Wine prefix is the critical path for the game window: its system
    //    DLLs are what the loader stalls on.
    warm_tree(wine_prefix, false, stop, &mut stats);
    if stop.load(Ordering::Relaxed) {
        stats.stopped = true;
        return stats;
    }

    // 2. Base game: only the executable for `Prefix`, everything for `Full`.
    if mode == PreloadMode::Full {
        warm_tree(base_game, false, stop, &mut stats);
    } else {
        warm_file(&base_game.join(game_exe), &mut stats);
    }
    if stop.load(Ordering::Relaxed) {
        stats.stopped = true;
        return stats;
    }

    // 3. Enabled mod layers: startup files for `Prefix`, everything for `Full`.
    for dir in layer_dirs {
        let startup_only = mode == PreloadMode::Prefix;
        warm_tree(dir, startup_only, stop, &mut stats);
        if stop.load(Ordering::Relaxed) {
            stats.stopped = true;
            return stats;
        }
    }

    stats
}

/// Recursively warms every regular file under `root`.
fn warm_tree(root: &Path, startup_only: bool, stop: &AtomicBool, stats: &mut WarmStats) {
    let mut stack = vec![root.to_path_buf()];
    let mut buf = vec![0u8; 256 * 1024];
    while let Some(dir) = stack.pop() {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            let Ok(ft) = entry.file_type() else { continue };
            let path = entry.path();
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() && (!startup_only || is_startup_file(&path)) {
                if startup_only
                    && entry
                        .metadata()
                        .map(|m| m.len() > PREFIX_MAX_FILE)
                        .unwrap_or(false)
                {
                    continue;
                }
                warm_file_with(&path, &mut buf, stats);
            }
        }
    }
}

/// Startup-relevant extensions for [`PreloadMode::Prefix`] mod layers.
const STARTUP_EXTS: &[&str] = &[
    "asi", "dll", "cleo", "cs", "ini", "cfg", "dat", "json", "xml", "txt",
];

fn is_startup_file(path: &Path) -> bool {
    if path.components().any(|c| {
        c.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("modloader")
    }) {
        return true;
    }
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| STARTUP_EXTS.iter().any(|x| e.eq_ignore_ascii_case(x)))
        .unwrap_or(false)
}

fn warm_file(path: &Path, stats: &mut WarmStats) {
    let mut buf = vec![0u8; 256 * 1024];
    warm_file_with(path, &mut buf, stats);
}

fn warm_file_with(path: &Path, buf: &mut [u8], stats: &mut WarmStats) {
    let Ok(mut f) = std::fs::File::open(path) else {
        return;
    };
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    stats.files += 1;
    stats.bytes += size;

    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        unsafe {
            libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_WILLNEED);
        }
    }

    // Small files are read fully: reading is what actually pulls the pages in,
    // and the prefix is full of small DLLs/configs where that matters most.
    if size <= READ_FULLY_UP_TO {
        while matches!(f.read(buf), Ok(n) if n > 0) {}
    }
}

/// Best-effort: drop this thread to the idle I/O class so the warmer only uses
/// disk time the launcher is not using.
#[cfg(target_os = "linux")]
fn set_idle_io() {
    // IOPRIO_WHO_PROCESS = 1, IOPRIO_CLASS_IDLE = 3, data = 0.
    const IOPRIO_WHO_PROCESS: libc::c_long = 1;
    const IOPRIO_CLASS_IDLE: libc::c_long = 3;
    let prio = IOPRIO_CLASS_IDLE << 13; // IOPRIO_PRIO_VALUE(class, data)
    unsafe {
        libc::syscall(libc::SYS_ioprio_set, IOPRIO_WHO_PROCESS, 0, prio);
    }
}

#[cfg(not(target_os = "linux"))]
fn set_idle_io() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modes() {
        assert_eq!(PreloadMode::parse(None), PreloadMode::Prefix);
        assert_eq!(PreloadMode::parse(Some("off")), PreloadMode::Off);
        assert_eq!(PreloadMode::parse(Some("OFF")), PreloadMode::Off);
        assert_eq!(PreloadMode::parse(Some("full")), PreloadMode::Full);
        assert_eq!(PreloadMode::parse(Some("prefix")), PreloadMode::Prefix);
        assert_eq!(PreloadMode::parse(Some("true")), PreloadMode::Prefix);
        assert_eq!(PreloadMode::parse(Some("nonsense")), PreloadMode::Prefix);
    }

    #[test]
    fn startup_filter_matches_extensions_and_modloader() {
        assert!(is_startup_file(Path::new("/x/foo.asi")));
        assert!(is_startup_file(Path::new("/x/foo.DLL")));
        assert!(is_startup_file(Path::new("/x/modloader/data/thing.bin")));
        assert!(!is_startup_file(Path::new("/x/models/gta3.img")));
    }

    #[test]
    fn prefix_warms_prefix_exe_and_startup_files_only() {
        let root = std::env::temp_dir().join(format!("gta-mo-preload-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let prefix = root.join("pfx");
        let base = root.join("base");
        let layer = root.join("layer");
        std::fs::create_dir_all(prefix.join("drive_c")).unwrap();
        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(&layer).unwrap();
        std::fs::write(prefix.join("drive_c/sys.dll"), vec![7u8; 4096]).unwrap();
        std::fs::write(base.join("gta_sa.exe"), vec![1u8; 4096]).unwrap();
        std::fs::write(layer.join("loader.asi"), vec![2u8; 4096]).unwrap();
        std::fs::write(layer.join("gta3.img"), vec![3u8; 4096]).unwrap();

        let stop = AtomicBool::new(false);
        let stats = warm(
            PreloadMode::Prefix,
            &prefix,
            &base,
            "gta_sa.exe",
            std::slice::from_ref(&layer),
            &stop,
        );
        assert_eq!(stats.files, 3, "prefix + exe + .asi, sin contar el .img");

        let full = warm(
            PreloadMode::Full,
            &prefix,
            &base,
            "gta_sa.exe",
            &[layer],
            &stop,
        );
        assert_eq!(full.files, 4, "full incluye también el .img");
        let _ = std::fs::remove_dir_all(&root);
    }
}
