//! Page-cache preloading to speed up game startup.
//!
//! On slow storage (spinning disks, encrypted/CoW filesystems) the Wine prefix
//! and the game's startup files are read on demand, which shows up as multi
//! second stalls while the loader maps a single DLL. Warming the page cache
//! ahead of the launch lets the kernel read those files early, so the loader
//! finds them in RAM.
//!
//! Warming is split in two:
//! * **Critical** ([`warm_critical`]): the Wine prefix, the game executable and
//!   the mod loaders (`.asi/.dll/.cleo`). It runs **synchronously at normal I/O
//!   priority before spawning `umu`**, so Wine never hits the loader stall.
//! * **Extra** ([`warm_extra`]): the mod startup configs (or the whole game in
//!   `full` mode). It runs on a background thread and stops once the game
//!   process appears.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// How much to preload before launching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreloadMode {
    /// Do not preload anything.
    Off,
    /// Warm the Wine prefix, the game executable, the mod loaders and the
    /// mods' startup files (configs and the `modloader/` tree).
    Prefix,
    /// Also warm the whole game base and every enabled mod file.
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

/// Startup files bigger than this are skipped: it keeps the warmer fast by
/// leaving the game's big data blobs (`gta3.img`, TXD streams…) to the on-demand
/// loader, which needs them later anyway.
const STARTUP_MAX_FILE: u64 = 64 * 1024 * 1024;

#[derive(Debug, Default, Clone, Copy)]
pub struct WarmStats {
    pub files: u64,
    pub bytes: u64,
    /// True when the warmer was interrupted because the game already started.
    pub stopped: bool,
}

/// Which files a tree walk should keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    /// Every regular file.
    All,
    /// Mod loaders (`.asi/.dll/.cleo`).
    Loaders,
    /// Startup configs/scripts that are not loaders.
    Config,
}

impl Filter {
    fn keeps(self, path: &Path) -> bool {
        match self {
            Filter::All => true,
            Filter::Loaders => is_loader_file(path),
            Filter::Config => is_startup_file(path) && !is_loader_file(path),
        }
    }

    fn size_capped(self) -> bool {
        !matches!(self, Filter::All)
    }
}

/// Handle to a running background preload of the *extra* files.
pub struct Preload {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<WarmStats>>,
}

impl Preload {
    /// Spawns the background warmer for the extra files. Returns `None` for
    /// [`PreloadMode::Off`] or if the thread cannot be created.
    pub fn spawn_extra(
        mode: PreloadMode,
        base_game: &Path,
        layer_dirs: &[PathBuf],
        stop: Arc<AtomicBool>,
    ) -> Option<Self> {
        if mode.is_off() {
            return None;
        }
        let base_game = base_game.to_path_buf();
        let dirs = layer_dirs.to_vec();
        let stop_thread = stop.clone();
        let handle = std::thread::Builder::new()
            .name("gta-mo-preload".into())
            .spawn(move || warm_extra(mode, &base_game, &dirs, &stop_thread))
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

    /// Spawns a background warmer for **every** file under `root` (used for the
    /// optional Proton/steamrt prefetches). Abortable via `stop`.
    pub fn spawn_tree(root: &Path, stop: Arc<AtomicBool>) -> Option<Self> {
        let root = root.to_path_buf();
        let stop_thread = stop.clone();
        let handle = std::thread::Builder::new()
            .name("gta-mo-prefetch".into())
            .spawn(move || {
                let mut stats = WarmStats::default();
                warm_tree(&root, Filter::All, Some(&stop_thread), &mut stats);
                stats
            })
            .ok()?;
        Some(Self {
            stop,
            handle: Some(handle),
        })
    }
}

/// Warms every regular file under `root` synchronously (used by `gta-mo
/// preload` for the Proton/steamrt prefetches).
pub fn warm_root(root: &Path, stop: &AtomicBool) -> WarmStats {
    let mut stats = WarmStats::default();
    warm_tree(root, Filter::All, Some(stop), &mut stats);
    stats
}

/// umu's Python environment, resolved from the `umu-run` wrapper.
struct UmuEnv {
    /// `/nix/store/<hash>-umu-launcher-*` package dir, if the wrapper lives in
    /// the store (the wrapper is usually a symlink to the `-bwrap` script).
    pkg_dir: Option<PathBuf>,
    /// `.../umu-launcher-*-fhsenv-rootfs` FHS environment.
    rootfs: PathBuf,
    /// `.../python3-*` interpreter + stdlib dir.
    python_dir: PathBuf,
}

/// Warms umu's Python/umu environment (interpreter, stdlib and site-packages) so
/// the next launch does not read it cold. That cold read is the `⏱ umu responde`
/// gap (measured ~20-47 s cold vs ~0.3 s warm; it is I/O, not CPU). Best-effort:
/// returns `None` when umu cannot be located.
pub fn warm_umu(stop: &AtomicBool) -> Option<WarmStats> {
    let env = umu_environment()?;
    let mut stats = WarmStats::default();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    // Python interpreter + stdlib (the bulk, ~125 MiB).
    warm_store_dir(&env.python_dir, stop, &mut seen, &mut stats);
    // umu's Python dependencies (site-packages: real dirs + symlinks to store).
    for sp in [
        env.rootfs.join("usr/lib64/python3.13/site-packages"),
        env.rootfs.join("usr/lib/python3.13/site-packages"),
    ] {
        warm_site_packages(&sp, stop, &mut seen, &mut stats);
    }
    // The umu-launcher package (the wrapper itself), when it is a store dir.
    if let Some(pkg) = &env.pkg_dir {
        warm_store_dir(pkg, stop, &mut seen, &mut stats);
    }
    Some(stats)
}

/// Resolves umu's environment from `umu-run` (or `$GTA_MO_UMU_RUN`).
fn umu_environment() -> Option<UmuEnv> {
    let wrapper = std::env::var_os("GTA_MO_UMU_RUN")
        .map(PathBuf::from)
        .or_else(|| which::which("umu-run").ok())?;
    // NB: do not canonicalize the wrapper: `bin/umu-run` is often a symlink to
    // `...-umu-launcher-*-bwrap` (a single file directly under `/nix/store`),
    // which would make the "package dir" resolve to `/nix`.
    let pkg_dir = store_dir_of(&wrapper);
    let rootfs = std::env::var_os("GTA_MO_UMU_ROOTFS")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| fhsenv_rootfs_from(&wrapper))?;
    let python = rootfs.join("usr/bin/python3.13").canonicalize().ok()?;
    let python_dir = python.parent()?.parent()?.to_path_buf();
    Some(UmuEnv {
        pkg_dir,
        rootfs,
        python_dir,
    })
}

/// Finds the `...-umu-launcher-*-fhsenv-rootfs` store path referenced by the
/// `umu-run` wrapper script.
fn fhsenv_rootfs_from(wrapper: &Path) -> Option<PathBuf> {
    let script = std::fs::read_to_string(wrapper).ok()?;
    const MARK: &str = "-fhsenv-rootfs";
    for tok in script.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | ';')) {
        // The path may be a prefix of a longer token (e.g. `...-fhsenv-rootfs/etc`).
        let Some(rel) = tok.find(MARK) else { continue };
        let end = rel + MARK.len();
        let Some(start) = tok[..end].rfind("/nix/store/") else {
            continue;
        };
        let p = PathBuf::from(&tok[start..end]);
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

/// `warm_tree` over a store dir, once (deduped via `seen`).
fn warm_store_dir(
    dir: &Path,
    stop: &AtomicBool,
    seen: &mut HashSet<PathBuf>,
    stats: &mut WarmStats,
) {
    if !dir.is_dir() {
        return;
    }
    let dir = dir.to_path_buf();
    if !seen.insert(dir.clone()) {
        return;
    }
    warm_tree(&dir, Filter::All, Some(stop), stats);
}

/// Warms a `site-packages` dir: its real files plus the store dirs its
/// top-level symlinks (the compiled `.so`s) point into.
fn warm_site_packages(
    dir: &Path,
    stop: &AtomicBool,
    seen: &mut HashSet<PathBuf>,
    stats: &mut WarmStats,
) {
    if !dir.is_dir() {
        return;
    }
    warm_tree(dir, Filter::All, Some(stop), stats);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if let Ok(target) = std::fs::read_link(entry.path()) {
            if let Some(store) = store_dir_of(&target) {
                warm_store_dir(&store, stop, seen, stats);
            }
        }
    }
}

/// The `/nix/store/<hash>-<name>` dir a path lives in, if it is under the store.
fn store_dir_of(path: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut comps = path.components();
    match (comps.next(), comps.next(), comps.next()) {
        (
            Some(Component::RootDir),
            Some(Component::Normal(nix)),
            Some(Component::Normal(store)),
        ) if nix == "nix" && store == "store" => {
            let pkg = comps.next()?;
            let mut out = PathBuf::from("/nix/store");
            out.push(pkg);
            Some(out)
        }
        _ => None,
    }
}

/// Warms the **critical** path synchronously: the Wine prefix (unless
/// `prefix` is `None`, e.g. when running from a tmpfs copy), the game executable
/// and the mod loaders. It never early-stops.
pub fn warm_critical(
    prefix: Option<&Path>,
    base_game: &Path,
    game_exe: &str,
    layer_dirs: &[PathBuf],
) -> WarmStats {
    let mut stats = WarmStats::default();
    if let Some(prefix) = prefix {
        warm_tree(prefix, Filter::All, None, &mut stats);
    }
    warm_file(&base_game.join(game_exe), &mut stats);
    for dir in layer_dirs {
        warm_tree(dir, Filter::Loaders, None, &mut stats);
    }
    stats
}

/// Warms the **extra** files synchronously, honouring `stop`.
pub fn warm_extra(
    mode: PreloadMode,
    base_game: &Path,
    layer_dirs: &[PathBuf],
    stop: &AtomicBool,
) -> WarmStats {
    let mut stats = WarmStats::default();
    match mode {
        PreloadMode::Off => {}
        PreloadMode::Prefix => {
            for dir in layer_dirs {
                warm_tree(dir, Filter::Config, Some(stop), &mut stats);
                if stop.load(Ordering::Relaxed) {
                    stats.stopped = true;
                    return stats;
                }
            }
        }
        PreloadMode::Full => {
            warm_tree(base_game, Filter::All, Some(stop), &mut stats);
            if stop.load(Ordering::Relaxed) {
                stats.stopped = true;
                return stats;
            }
            for dir in layer_dirs {
                warm_tree(dir, Filter::All, Some(stop), &mut stats);
                if stop.load(Ordering::Relaxed) {
                    stats.stopped = true;
                    return stats;
                }
            }
        }
    }
    stats
}

/// Warms everything synchronously (used by the `gta-mo preload` command).
pub fn warm_all(
    mode: PreloadMode,
    prefix: Option<&Path>,
    base_game: &Path,
    game_exe: &str,
    layer_dirs: &[PathBuf],
    stop: &AtomicBool,
) -> WarmStats {
    if mode.is_off() {
        return WarmStats::default();
    }
    let mut stats = warm_critical(prefix, base_game, game_exe, layer_dirs);
    let extra = warm_extra(mode, base_game, layer_dirs, stop);
    stats.files += extra.files;
    stats.bytes += extra.bytes;
    stats.stopped = extra.stopped;
    stats
}

/// Recursively warms the regular files under `root` that pass `filter`.
fn warm_tree(root: &Path, filter: Filter, stop: Option<&AtomicBool>, stats: &mut WarmStats) {
    let mut stack = vec![root.to_path_buf()];
    let mut buf = vec![0u8; 256 * 1024];
    while let Some(dir) = stack.pop() {
        if stop.map(|s| s.load(Ordering::Relaxed)).unwrap_or(false) {
            return;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if stop.map(|s| s.load(Ordering::Relaxed)).unwrap_or(false) {
                return;
            }
            let Ok(ft) = entry.file_type() else { continue };
            let path = entry.path();
            if ft.is_dir() {
                stack.push(path);
            } else if ft.is_file() && filter.keeps(&path) {
                if filter.size_capped()
                    && entry
                        .metadata()
                        .map(|m| m.len() > STARTUP_MAX_FILE)
                        .unwrap_or(false)
                {
                    continue;
                }
                warm_file_with(&path, &mut buf, stats);
            }
        }
    }
}

/// Startup-relevant extensions for the mod layers.
const STARTUP_EXTS: &[&str] = &[
    "asi", "dll", "cleo", "cs", "fxt", "ini", "cfg", "dat", "json", "xml", "txt",
];

/// Mod loader files, loaded early by the ASI/CLEO loaders.
fn is_loader_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            ["asi", "dll", "cleo"]
                .iter()
                .any(|x| e.eq_ignore_ascii_case(x))
        })
        .unwrap_or(false)
}

/// Startup configs/scripts, by extension. `modloader/` is deliberately **not**
/// special-cased: its assets (`.dff/.txd/.wav/.mp3`…) are read on demand by Mod
/// Loader, not at startup, so warming them only wastes I/O. The `.asi/.dll/
/// .cleo` loaders are handled separately by [`is_loader_file`].
fn is_startup_file(path: &Path) -> bool {
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
    fn store_dir_of_extracts_the_store_prefix() {
        assert_eq!(
            store_dir_of(Path::new("/nix/store/abc-pkg/lib/x.so")),
            Some(PathBuf::from("/nix/store/abc-pkg"))
        );
        assert_eq!(store_dir_of(Path::new("/home/user/x")), None);
        assert_eq!(store_dir_of(Path::new("/nix/store")), None);
    }

    #[test]
    fn umu_environment_from_env_overrides() {
        let dir = std::env::temp_dir().join(format!("gta-mo-umuenv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let pkg = dir.join("umu-launcher-1.0");
        let rootfs = dir.join("umu-launcher-1.0-fhsenv-rootfs");
        let pybin = dir.join("python3-3.13.15/bin");
        std::fs::create_dir_all(pkg.join("bin")).unwrap();
        std::fs::create_dir_all(rootfs.join("usr/bin")).unwrap();
        std::fs::create_dir_all(&pybin).unwrap();
        std::fs::write(pkg.join("bin/umu-run"), "#!/bin/sh\n").unwrap();
        std::fs::write(pybin.join("python3.13"), b"").unwrap();
        std::os::unix::fs::symlink(pybin.join("python3.13"), rootfs.join("usr/bin/python3.13"))
            .unwrap();

        std::env::set_var("GTA_MO_UMU_RUN", pkg.join("bin/umu-run"));
        std::env::set_var("GTA_MO_UMU_ROOTFS", &rootfs);
        let env = umu_environment().expect("env");
        // The wrapper is under /tmp (not /nix/store), so there is no package dir.
        assert!(env.pkg_dir.is_none());
        assert_eq!(
            env.rootfs.file_name().unwrap(),
            "umu-launcher-1.0-fhsenv-rootfs"
        );
        assert_eq!(env.python_dir.file_name().unwrap(), "python3-3.13.15");
        std::env::remove_var("GTA_MO_UMU_RUN");
        std::env::remove_var("GTA_MO_UMU_ROOTFS");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_filter_matches_extensions_and_modloader() {
        assert!(is_startup_file(Path::new("/x/foo.asi")));
        assert!(is_startup_file(Path::new("/x/foo.DLL")));
        assert!(is_startup_file(Path::new("/x/foo.fxt")));
        assert!(is_startup_file(Path::new("/x/modloader/data/thing.ini")));
        // Mod Loader reads its assets on demand, so they are not startup files.
        assert!(!is_startup_file(Path::new("/x/modloader/car.dff")));
        assert!(!is_startup_file(Path::new("/x/modloader/car.txd")));
        assert!(!is_startup_file(Path::new("/x/modloader/data/thing.bin")));
        assert!(!is_startup_file(Path::new("/x/models/gta3.img")));
        assert!(is_loader_file(Path::new("/x/foo.cleo")));
        assert!(!is_loader_file(Path::new("/x/foo.ini")));
    }

    #[test]
    fn critical_and_extra_split() {
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
        std::fs::write(layer.join("config.ini"), vec![4u8; 4096]).unwrap();
        std::fs::write(layer.join("gta3.img"), vec![3u8; 4096]).unwrap();

        let layers = std::slice::from_ref(&layer);
        // Critical: prefix file + exe + `.asi` loader.
        let crit = warm_critical(Some(&prefix), &base, "gta_sa.exe", layers);
        assert_eq!(crit.files, 3);

        // Extra `prefix`: only the non-loader startup config.
        let stop = AtomicBool::new(false);
        let extra = warm_extra(PreloadMode::Prefix, &base, layers, &stop);
        assert_eq!(extra.files, 1);

        // Extra `full`: whole base + whole layer.
        let full = warm_extra(PreloadMode::Full, &base, layers, &stop);
        assert_eq!(full.files, 4); // gta_sa.exe + asi + ini + img

        // Without a prefix (tmpfs mode) the prefix tree is skipped.
        let no_prefix = warm_critical(None, &base, "gta_sa.exe", layers);
        assert_eq!(no_prefix.files, 2);
        let _ = std::fs::remove_dir_all(&root);
    }
}
