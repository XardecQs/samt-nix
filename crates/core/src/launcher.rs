use std::collections::HashMap;
use std::io::IsTerminal;
use std::io::{BufRead, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::{config, db, mount_state, overlay, preload, preload::PreloadMode, resolver, upper};

pub struct LaunchEngine;

/// umu's `steamrt4` runtime directory (`$XDG_DATA_HOME/umu/steamrt4`),
/// optionally overridden with `GTA_MO_STEAMRT_DIR` (mainly for tests).
fn steamrt_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("GTA_MO_STEAMRT_DIR") {
        let p = PathBuf::from(dir);
        return p.is_dir().then_some(p);
    }
    let p = dirs::data_dir()?.join("umu/steamrt4");
    p.is_dir().then_some(p)
}

/// Per-launch log file. It holds the high-level launcher messages and, through
/// the tee in [`LaunchEngine::run_umu`], the raw `umu`/Proton output. One file
/// per launch lives in the profile's `logs/` dir; the newest 20 are kept.
pub struct LaunchLog {
    path: PathBuf,
}

impl LaunchLog {
    /// Creates `launch-<unix-seconds>.log` under `dir` and prunes old ones.
    pub fn create(dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = dir.join(format!("launch-{secs}.log"));
        {
            let mut f = std::fs::File::create(&path)?;
            writeln!(f, "=== gta-mo — registro de lanzamiento ===")?;
            writeln!(f, "inicio (unix): {secs}")?;
        }
        prune_launch_logs(dir, 20);
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Appends one line to the log file (best-effort; never fails a launch).
    pub fn write(&self, msg: &str) {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(f, "{msg}");
        }
    }
}

/// Keeps only the newest `keep` `launch-*.log` files in `dir`.
fn prune_launch_logs(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut logs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("launch-") && n.ends_with(".log"))
                .unwrap_or(false)
        })
        .collect();
    if logs.len() <= keep {
        return;
    }
    logs.sort();
    for old in &logs[..logs.len() - keep] {
        let _ = std::fs::remove_file(old);
    }
}

/// Forwards every line of `reader` to stdout (or stderr) and appends it to
/// `log_path` when given.
fn tee_output<R: std::io::Read + Send + 'static>(
    reader: R,
    log_path: &Path,
    to_stderr: bool,
) -> std::thread::JoinHandle<()> {
    let path = log_path.to_path_buf();
    std::thread::spawn(move || {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok();
        for line in std::io::BufReader::new(reader)
            .lines()
            .map_while(Result::ok)
        {
            if to_stderr {
                eprintln!("{line}");
            } else {
                println!("{line}");
            }
            if let Some(f) = file.as_mut() {
                let _ = writeln!(f, "{line}");
            }
        }
    })
}

/// Config, runtime paths, the open database and the launcher lock, held for the
/// duration of a launch or a manual mount.
struct Bootstrap {
    cfg: config::Config,
    paths: config::RuntimePaths,
    conn: rusqlite::Connection,
    /// Kept alive until the launcher/mount is done: releasing it would let a
    /// second instance start. After a `fork` the daemon shares this lock.
    _lock: std::fs::File,
}

/// Everything needed to mount the overlay for a profile, resolved from the DB.
struct Prepared {
    profile: db::Profile,
    ppaths: config::ProfilePaths,
    resolved: Vec<String>,
    modloader_entries: Vec<(String, i64)>,
    violations: Vec<crate::constraints::Violation>,
    /// Per-folder option selections for the enabled mods (folder -> {group: value}).
    options_by_folder: HashMap<String, HashMap<String, String>>,
}

/// Result of [`LaunchEngine::mount`].
pub struct MountOutcome {
    /// Set instead of `info` for `--dry-run` and for discover/clean shortcuts.
    pub dry_run: Option<String>,
    pub info: Option<mount_state::MountInfo>,
}

/// Result of [`LaunchEngine::unmount`].
pub struct UnmountOutcome {
    pub info: mount_state::MountInfo,
    pub capture: Option<upper::CaptureReport>,
    pub discarded: bool,
    /// True when a captured mod was registered in the database.
    pub registered: bool,
}

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub dry_run: bool,
    pub debug: bool,
    pub discover: bool,
    pub clean: bool,
    pub auto_discover: bool,
    pub profile: Option<String>,
    pub deps_enable: bool,
    pub deps_ignore: bool,
    /// CLI override for the page-cache preload mode (falls back to the config).
    pub preload: Option<PreloadMode>,
    /// Skip the `full` preload confirmation prompt.
    pub preload_yes: bool,
    /// CLI override for running the Wine prefix from tmpfs (falls back to the
    /// config).
    pub prefix_ram: Option<bool>,
    /// CLI override: materialize Proton symlinks into the tmpfs prefix.
    pub prefix_ram_proton: Option<bool>,
    /// CLI override: prefetch the Proton runtime.
    pub preload_proton: Option<bool>,
    /// CLI override: prefetch umu's steamrt runtime.
    pub preload_steamrt: Option<bool>,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            debug: false,
            discover: false,
            clean: false,
            auto_discover: true,
            profile: None,
            deps_enable: false,
            deps_ignore: false,
            preload: None,
            preload_yes: false,
            prefix_ram: None,
            prefix_ram_proton: None,
            preload_proton: None,
            preload_steamrt: None,
        }
    }
}

pub struct LaunchResult {
    pub log: String,
}

/// Collects the wall-clock duration of each launch phase.
struct PhaseTimer {
    enabled: bool,
    start: Instant,
    last: Instant,
    marks: Vec<(&'static str, Duration)>,
}

impl PhaseTimer {
    fn new(enabled: bool) -> Self {
        let now = Instant::now();
        Self {
            enabled,
            start: now,
            last: now,
            marks: Vec::new(),
        }
    }

    /// Records the time since the previous mark under `name`.
    fn mark(&mut self, name: &'static str) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        self.marks.push((name, now - self.last));
        self.last = now;
    }

    /// Instant the launch command started, for absolute "time to X" metrics.
    fn start_instant(&self) -> Instant {
        self.start
    }

    fn report(&self) -> String {
        if !self.enabled || self.marks.is_empty() {
            return String::new();
        }
        let parts: Vec<String> = self
            .marks
            .iter()
            .map(|(name, d)| format!("{name}={:.0}ms", d.as_secs_f64() * 1000.0))
            .collect();
        format!(
            "⏱ fases: {} | total={:.1}s",
            parts.join(" "),
            self.start.elapsed().as_secs_f64()
        )
    }
}

impl LaunchEngine {
    pub fn check_system_deps() -> anyhow::Result<()> {
        Self::check_overlay_deps()?;
        if which::which("umu-run").is_err() {
            anyhow::bail!("Falta 'umu-run'. Instala umu-launcher para lanzar el juego con Proton.");
        }
        Ok(())
    }

    /// Only the bits a manual mount needs (`umu-run` is not required).
    pub fn check_overlay_deps() -> anyhow::Result<()> {
        if which::which("fuse-overlayfs").is_err() {
            anyhow::bail!("Falta 'fuse-overlayfs'. Instálalo para montar el overlay.");
        }
        Ok(())
    }

    pub fn validate_paths(cfg: &config::Config) -> anyhow::Result<()> {
        let game_root = PathBuf::from(&cfg.game_root);
        if !game_root.is_dir() {
            anyhow::bail!("game_root no es un directorio válido: {}", cfg.game_root);
        }

        let proton_path = PathBuf::from(&cfg.proton_path);
        if !proton_path.is_dir() {
            anyhow::bail!(
                "proton_path no es un directorio válido: {}",
                cfg.proton_path
            );
        }

        let paths = config::RuntimePaths::from_config(cfg);
        if !paths.base_game.is_dir() {
            anyhow::bail!(
                "directorio base del juego no encontrado: {}",
                paths.base_game.display()
            );
        }

        Ok(())
    }

    pub fn acquire_lock(lock_path: &Path) -> anyhow::Result<std::fs::File> {
        if let Some(parent) = lock_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .read(true)
            // Never follow a symlink: a pre-planted link could otherwise be
            // truncated (e.g. when falling back to a shared temp dir).
            .custom_flags(libc::O_NOFOLLOW)
            .open(lock_path)?;

        fs2::FileExt::try_lock_exclusive(&file)
            .map_err(|e| anyhow::anyhow!("Ya hay una instancia en ejecución: {e}"))?;

        Ok(file)
    }

    /// Real directories each enabled mod contributes to the overlay (already
    /// expanded with the selected per-profile options), in load order.
    pub fn mod_layer_dirs(
        mods_dir: &Path,
        resolved: &[String],
        options_by_folder: &HashMap<String, HashMap<String, String>>,
    ) -> Vec<PathBuf> {
        let empty = HashMap::new();
        resolved
            .iter()
            .flat_map(|f| {
                let selected = options_by_folder.get(f).unwrap_or(&empty);
                crate::meta::mod_layers_with_options(mods_dir, f, selected)
            })
            .collect()
    }

    pub fn build_lowerdir(
        mods_dir: &Path,
        resolved: &[String],
        base_game: &Path,
        options_by_folder: &HashMap<String, HashMap<String, String>>,
    ) -> String {
        let mut layers = Self::mod_layer_dirs(mods_dir, resolved, options_by_folder);
        layers.push(base_game.to_path_buf());
        layers
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(":")
    }

    pub fn setup_env(
        cfg: &config::Config,
        wine_prefix: &std::path::Path,
        log_dir: &std::path::Path,
        debug: bool,
    ) {
        std::env::set_var("WINEPREFIX", wine_prefix);
        std::env::set_var("PROTONPATH", &cfg.proton_path);
        std::env::set_var("GAMEID", cfg.game_id());
        std::env::set_var(
            "PROTON_USE_WINED3D",
            if cfg.proton_use_wined3d() { "1" } else { "0" },
        );
        std::env::set_var(
            "PROTON_DISABLE_NTSYNC",
            if cfg.proton_disable_ntsync() {
                "1"
            } else {
                "0"
            },
        );
        // umu-launcher re-checks and re-downloads its steamrt runtime on every
        // launch. When the upstream digest mismatches (bad cache/mirror) that
        // fails noisily and wastes time even though the runtime already works,
        // so it is disabled by default (see Config::umu_runtime_update).
        std::env::set_var(
            "UMU_RUNTIME_UPDATE",
            if cfg.umu_runtime_update() { "1" } else { "0" },
        );

        if cfg.proton_disable_upscalers() {
            // GE/CachyOS Proton's protonfixes setup_upscalers() downloads and
            // installs upscaler DLLs (FSR3/FSR4/MLFG/DLSS/XeSS/OptiScaler) on
            // launch. Set every *UPGRADE var to 0 so it is skipped.
            std::env::set_var("PROTON_FSR3_UPGRADE", "0");
            std::env::set_var("PROTON_FSR4_UPGRADE", "0");
            std::env::set_var("PROTON_FSR4_RDNA3_UPGRADE", "0");
            std::env::set_var("PROTON_MLFG_UPGRADE", "0");
            std::env::set_var("PROTON_DLSS_UPGRADE", "0");
            std::env::set_var("PROTON_XESS_UPGRADE", "0");
            std::env::set_var("PROTON_USE_OPTISCALER", "0");
        }

        // Always keep umu/DXVK logs inside the profile's logs/ dir so every
        // launch is inspectable; Proton's verbose `steam-<appid>.log` and the
        // most verbose DXVK/Wine output are only enabled with `--debug` (or the
        // `proton_log` config), because writing them is pure overhead.
        std::fs::create_dir_all(log_dir).ok();
        if debug || cfg.proton_log() {
            std::env::set_var("PROTON_LOG", "1");
        } else {
            std::env::remove_var("PROTON_LOG");
        }
        std::env::set_var("PROTON_LOG_DIR", log_dir);
        std::env::set_var("DXVK_LOG_PATH", log_dir);
        std::env::set_var("UMU_LOG", if debug { "debug" } else { "1" });
        if debug {
            std::env::set_var("DXVK_LOG_LEVEL", "debug");
            std::env::set_var("DXVK_HUD", cfg.dxvk_hud());
            std::env::set_var("WINEDEBUG", "+loaddll");
        }
    }

    pub fn launch_game(
        exe_path: &Path,
        working_dir: &Path,
        launch_log: Option<&LaunchLog>,
    ) -> anyhow::Result<()> {
        let log_path = launch_log.map(|l| l.path().to_path_buf());
        let drop_uid = std::env::var("GTA_MO_DROP_UID")
            .ok()
            .and_then(|v| v.parse::<u32>().ok());
        let drop_gid = std::env::var("GTA_MO_DROP_GID")
            .ok()
            .and_then(|v| v.parse::<u32>().ok());

        if unsafe { libc::geteuid() } == 0 {
            if Self::zero_maps_to_zero_in_initial_ns() {
                return Err(anyhow::anyhow!(
                    "gta-mo se ejecuta como root real en el namespace inicial; no se puede \
                     'dropear' a un usuario de forma segura. Ejecútalo como tu usuario o a \
                     través del subcomando `steam` (GTA_MO_DROP_UID no aplica aquí)."
                ));
            }
            return match drop_uid {
                Some(uid) => {
                    Self::launch_game_dropped(exe_path, working_dir, uid, drop_gid, log_path.as_deref())
                }
                None => anyhow::bail!(
                    "gta-mo se ejecuta como root. Define GTA_MO_DROP_UID (y opcionalmente GTA_MO_DROP_GID) para lanzar el juego como usuario."
                ),
            };
        }

        Self::launch_game_direct(exe_path, working_dir, log_path.as_deref())
    }

    /// Runs `umu-run` for `exe`, teeing its stdout/stderr to our own streams and
    /// (when `log_path` is set) to the launch log file.
    fn run_umu(
        exe_path: &Path,
        working_dir: &Path,
        log_path: Option<&Path>,
    ) -> std::io::Result<std::process::ExitStatus> {
        let mut cmd = std::process::Command::new("umu-run");
        cmd.arg(exe_path).current_dir(working_dir);

        let Some(path) = log_path else {
            return cmd.status();
        };

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        let mut handles = Vec::new();
        if let Some(out) = child.stdout.take() {
            handles.push(tee_output(out, path, false));
        }
        if let Some(err) = child.stderr.take() {
            handles.push(tee_output(err, path, true));
        }
        let status = child.wait()?;
        for h in handles {
            let _ = h.join();
        }
        Ok(status)
    }

    /// True when this process runs with euid 0 in the *initial* (host) user
    /// namespace, i.e. genuinely as real root. The Steam wrapper runs inside a
    /// fresh user namespace where uid 0 maps to a real (non-zero) uid, so its
    /// `/proc/self/uid_map` does not start with `0 0`.
    fn zero_maps_to_zero_in_initial_ns() -> bool {
        let Ok(content) = std::fs::read_to_string("/proc/self/uid_map") else {
            return false;
        };
        for line in content.lines() {
            let mut it = line.split_whitespace();
            let inside = it.next().and_then(|v| v.parse::<u64>().ok());
            let outside = it.next().and_then(|v| v.parse::<u64>().ok());
            match (inside, outside) {
                (Some(0), Some(outside)) => return outside == 0,
                _ => continue,
            }
        }
        false
    }

    /// Primary group of `uid` via `getpwuid`, when resolvable.
    fn primary_gid_for_uid(uid: u32) -> Option<u32> {
        unsafe {
            let mut pwd: libc::passwd = std::mem::zeroed();
            let mut buf = vec![0u8; 4096];
            let mut result: *mut libc::passwd = std::ptr::null_mut();
            let rc = libc::getpwuid_r(
                uid,
                &mut pwd,
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.len(),
                &mut result,
            );
            if rc == 0 && !result.is_null() {
                Some(pwd.pw_gid)
            } else {
                None
            }
        }
    }

    fn launch_game_direct(
        exe_path: &Path,
        working_dir: &Path,
        log_path: Option<&Path>,
    ) -> anyhow::Result<()> {
        let status = Self::run_umu(exe_path, working_dir, log_path)
            .map_err(|e| anyhow::anyhow!("Error al ejecutar umu-run: {e}"))?;

        if !status.success() {
            anyhow::bail!("umu-run terminó con error: {status}");
        }

        Ok(())
    }

    // Cuando gta-mo corre como root dentro de un user namespace (lanzado por
    // el wrapper de Steam), umu-run se niega a arrancar con euid 0. Este hijo
    // entra en un user namespace anidado que mapea el uid original y luego
    // ejecuta el juego como usuario, sin perder el montaje del overlay.
    fn launch_game_dropped(
        exe_path: &Path,
        working_dir: &Path,
        uid: u32,
        gid: Option<u32>,
        log_path: Option<&Path>,
    ) -> anyhow::Result<()> {
        // uid/GID del usuario real. El gid por defecto NO es el uid: usa el
        // grupo primario del usuario cuando se puede resolver.
        let gid = gid
            .or_else(|| Self::primary_gid_for_uid(uid))
            .unwrap_or(uid);
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            anyhow::bail!("fork falló: {}", std::io::Error::last_os_error());
        }
        if pid == 0 {
            let code = Self::drop_and_run_game(exe_path, working_dir, uid, gid, log_path);
            std::process::exit(code);
        }

        let mut status: libc::c_int = 0;
        loop {
            let r = unsafe { libc::waitpid(pid, &mut status, 0) };
            if r < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                anyhow::bail!("waitpid falló: {e}");
            }
            break;
        }

        if libc::WIFEXITED(status) {
            let code = libc::WEXITSTATUS(status);
            if code == 0 {
                return Ok(());
            }
            anyhow::bail!("umu-run terminó con error: exit status: {code}");
        }
        if libc::WIFSIGNALED(status) {
            anyhow::bail!("umu-run terminó por señal: {}", libc::WTERMSIG(status));
        }
        Ok(())
    }

    fn drop_and_run_game(
        exe_path: &Path,
        working_dir: &Path,
        uid: u32,
        gid: u32,
        log_path: Option<&Path>,
    ) -> i32 {
        if unsafe { libc::unshare(libc::CLONE_NEWUSER) } != 0 {
            eprintln!(
                "gta-mo: unshare(CLONE_NEWUSER) falló: {}",
                std::io::Error::last_os_error()
            );
            return 1;
        }
        // Requerido para poder escribir gid_map sin CAP_SETGID en el namespace
        // padre. Falla con EPERM si ya está denegado: lo ignoramos.
        let _ = std::fs::write("/proc/self/setgroups", "deny");
        if let Err(e) = std::fs::write("/proc/self/uid_map", format!("{uid} 0 1")) {
            eprintln!("gta-mo: no se pudo escribir /proc/self/uid_map: {e}");
            return 1;
        }
        if let Err(e) = std::fs::write("/proc/self/gid_map", format!("{gid} 0 1")) {
            eprintln!("gta-mo: no se pudo escribir /proc/self/gid_map: {e}");
            return 1;
        }
        if unsafe { libc::setgid(gid as libc::gid_t) } != 0 {
            eprintln!("gta-mo: setgid falló: {}", std::io::Error::last_os_error());
            return 1;
        }
        if unsafe { libc::setuid(uid as libc::uid_t) } != 0 {
            eprintln!("gta-mo: setuid falló: {}", std::io::Error::last_os_error());
            return 1;
        }
        match Self::run_umu(exe_path, working_dir, log_path) {
            Ok(s) if s.success() => 0,
            Ok(s) => s.code().unwrap_or(1),
            Err(e) => {
                eprintln!("Error al ejecutar umu-run: {e}");
                1
            }
        }
    }

    fn resolve_preload_mode(cfg: &config::Config, opts: &LaunchOptions) -> PreloadMode {
        opts.preload.unwrap_or_else(|| cfg.preload_mode())
    }

    /// Prints the `full` preload warning and, on an interactive terminal,
    /// confirms it (downgrading to `prefix` when declined). On a non-TTY
    /// (GUI/desktop) it just warns: the caller's config/flag is the consent.
    fn confirm_full_preload(
        mode: &mut PreloadMode,
        opts: &LaunchOptions,
        launch_log: Option<&LaunchLog>,
    ) {
        if *mode != PreloadMode::Full {
            return;
        }
        for line in preload::FULL_WARNING.lines() {
            crate::log::warn(line);
            if let Some(l) = launch_log {
                l.write(line);
            }
        }
        if opts.preload_yes || !std::io::stdin().is_terminal() {
            return;
        }
        eprint!("¿Continuar con la precarga completa? [s/N]: ");
        let _ = std::io::stderr().flush();
        let mut input = String::new();
        let _ = std::io::stdin().read_line(&mut input);
        if !matches!(
            input.trim().to_ascii_lowercase().as_str(),
            "s" | "si" | "sí" | "y" | "yes"
        ) {
            crate::log::info("Precarga completa cancelada; se usará 'prefix'.");
            *mode = PreloadMode::Prefix;
        }
    }

    fn append_log(path: &Path, msg: &str) {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(f, "{msg}");
        }
    }

    /// True when a process whose `comm` matches the game executable exists.
    fn game_process_running(game_exe: &str) -> bool {
        let want = &game_exe[..game_exe.len().min(15)];
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return false;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            if let Ok(comm) = std::fs::read_to_string(entry.path().join("comm")) {
                if comm.trim().eq_ignore_ascii_case(want) {
                    return true;
                }
            }
        }
        false
    }

    /// True once DXVK's d3d9 log shows it created the device (a good proxy for
    /// "the game window is up").
    fn dxvk_ready(log_dir: &Path, game_exe: &str) -> bool {
        let stem = Path::new(game_exe)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(game_exe);
        let path = log_dir.join(format!("{stem}_d3d9.log"));
        std::fs::read_to_string(path)
            .map(|s| s.contains("DXVK:") && s.contains("Creating device"))
            .unwrap_or(false)
    }

    /// Background thread that reports "time to game process" and "time to DXVK
    /// device" (window proxy) while `umu`/Proton runs.
    fn spawn_watcher(
        game_exe: String,
        log_dir: PathBuf,
        started: Instant,
        report: bool,
        watcher_stop: Arc<AtomicBool>,
        preload_stop: Arc<AtomicBool>,
        launch_log: Option<PathBuf>,
    ) -> Option<std::thread::JoinHandle<()>> {
        std::thread::Builder::new()
            .name("gta-mo-timing".into())
            .spawn(move || {
                let emit = |msg: String| {
                    if report {
                        crate::log::info(&msg);
                    }
                    if let Some(p) = &launch_log {
                        Self::append_log(p, &msg);
                    }
                };
                let deadline = started + Duration::from_secs(900);
                let mut proc_done = false;
                let mut dxvk_done = false;
                loop {
                    if watcher_stop.load(Ordering::Relaxed) {
                        break;
                    }
                    if !proc_done && Self::game_process_running(&game_exe) {
                        proc_done = true;
                        preload_stop.store(true, Ordering::Relaxed);
                        emit(format!(
                            "⏱ proceso del juego: {:.2}s",
                            started.elapsed().as_secs_f64()
                        ));
                    }
                    if !dxvk_done && Self::dxvk_ready(&log_dir, &game_exe) {
                        dxvk_done = true;
                        emit(format!(
                            "⏱ DXVK listo (≈ ventana): {:.2}s",
                            started.elapsed().as_secs_f64()
                        ));
                    }
                    if (proc_done && dxvk_done) || Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            })
            .ok()
    }

    /// Warms the page cache, sets the environment, launches `umu` and reports
    /// the timing metrics. Shared by the clean and modded launch paths.
    #[allow(clippy::too_many_arguments)]
    fn launch_phase(
        boot: &Bootstrap,
        prepared: &Prepared,
        launch_log: Option<&LaunchLog>,
        opts: &LaunchOptions,
        game_exe: &Path,
        working_dir: &Path,
        layer_dirs: &[PathBuf],
        timer: &mut PhaseTimer,
        log: &mut impl FnMut(&str),
    ) -> anyhow::Result<()> {
        let mut mode = Self::resolve_preload_mode(&boot.cfg, opts);
        Self::confirm_full_preload(&mut mode, opts, launch_log);

        // B1: run the game against a per-session tmpfs copy of the Wine prefix
        // so a long game session cannot evict it from RAM.
        let prefix_ram = opts.prefix_ram.unwrap_or_else(|| boot.cfg.prefix_ram());
        let materialize = opts
            .prefix_ram_proton
            .unwrap_or_else(|| boot.cfg.prefix_ram_proton());
        let proton_path = PathBuf::from(&boot.cfg.proton_path);
        let prepared_prefix = crate::prefix_ram::PreparedPrefix::prepare(
            &boot.paths.wine_prefix,
            prefix_ram,
            Some(&proton_path),
            materialize,
        );
        let prep = prepared_prefix.report();
        if prepared_prefix.using_tmpfs() {
            let ram = crate::prefix_ram::prepared_bytes() as f64 / 1_048_576.0;
            if prep.reused {
                Self::emit(
                    launch_log,
                    log,
                    &format!("Prefijo ya en tmpfs (RAM {ram:.0} MiB); se reutiliza."),
                );
            } else if prep.copied {
                Self::emit(
                    launch_log,
                    log,
                    &format!(
                        "Prefijo → tmpfs en {:.1}s (RAM {ram:.0} MiB, materializado {:.0} MiB).",
                        prep.elapsed.as_secs_f64(),
                        prep.materialized as f64 / 1_048_576.0
                    ),
                );
            }
        }
        timer.mark("prefix-ram");

        // A: warm the critical path synchronously, at normal I/O priority, so
        // Wine never hits the loader stall. The disk prefix is skipped when the
        // game will read it from the tmpfs copy.
        let critical_prefix = if prepared_prefix.using_tmpfs() {
            None
        } else {
            Some(boot.paths.wine_prefix.as_path())
        };
        let crit = preload::warm_critical(
            critical_prefix,
            &boot.paths.base_game,
            boot.cfg.game_exe(),
            layer_dirs,
        );
        if crit.files > 0 {
            Self::emit(
                launch_log,
                log,
                &format!(
                    "Precarga crítica: {} archivo(s), {:.0} MiB.",
                    crit.files,
                    crit.bytes as f64 / 1_048_576.0
                ),
            );
        }
        timer.mark("preload");

        // Extra files in the background (abortable when the game starts).
        let preload_stop = Arc::new(AtomicBool::new(false));
        let preloader = preload::Preload::spawn_extra(
            mode,
            &boot.paths.base_game,
            layer_dirs,
            preload_stop.clone(),
        );
        if preloader.is_some() {
            Self::emit(
                launch_log,
                log,
                &format!("Precarga en caché ({}) en segundo plano.", mode.as_str()),
            );
        }
        let mut prefetchers: Vec<preload::Preload> = Vec::new();
        if opts
            .preload_proton
            .unwrap_or_else(|| boot.cfg.preload_proton())
        {
            let files = proton_path.join("files");
            let root = if files.is_dir() {
                files
            } else {
                proton_path.clone()
            };
            if let Some(p) = preload::Preload::spawn_tree(&root, preload_stop.clone()) {
                Self::emit(launch_log, log, "Prefetch de Proton en segundo plano.");
                prefetchers.push(p);
            }
        }
        if opts
            .preload_steamrt
            .unwrap_or_else(|| boot.cfg.preload_steamrt())
        {
            if let Some(root) = steamrt_dir() {
                if let Some(p) = preload::Preload::spawn_tree(&root, preload_stop.clone()) {
                    Self::emit(launch_log, log, "Prefetch de steamrt en segundo plano.");
                    prefetchers.push(p);
                }
            }
        }

        let watcher_stop = Arc::new(AtomicBool::new(false));
        let watcher_log = launch_log.map(|l| l.path().to_path_buf());
        // The watcher also stops the preloader once the game starts, so it runs
        // whenever there is something to warm even with timings disabled.
        let watcher = if boot.cfg.timings() || preloader.is_some() || !prefetchers.is_empty() {
            Self::spawn_watcher(
                boot.cfg.game_exe().to_string(),
                prepared.ppaths.log_dir.clone(),
                timer.start_instant(),
                boot.cfg.timings(),
                watcher_stop.clone(),
                preload_stop.clone(),
                watcher_log,
            )
        } else {
            None
        };

        Self::setup_env(
            &boot.cfg,
            prepared_prefix.path(),
            &prepared.ppaths.log_dir,
            opts.debug,
        );

        let summary = timer.report();
        if !summary.is_empty() {
            log(&summary);
            if let Some(l) = launch_log {
                l.write(&summary);
            }
        }
        log("Lanzando juego...");
        if let Some(l) = launch_log {
            l.write("Lanzando juego...");
        }

        let started = Instant::now();
        let result = Self::launch_game(game_exe, working_dir, launch_log);
        let session = started.elapsed();
        timer.mark("sesión");

        watcher_stop.store(true, Ordering::Relaxed);
        if let Some(h) = watcher {
            let _ = h.join();
        }
        let warmed = preloader.map(|p| p.stop_and_join()).unwrap_or_default();
        let mut extra_files = warmed.files;
        let mut extra_stopped = warmed.stopped;
        for p in prefetchers {
            let s = p.stop_and_join();
            extra_files += s.files;
            extra_stopped |= s.stopped;
        }
        let sync = prepared_prefix.finalize();
        if sync.synced {
            Self::emit(
                launch_log,
                log,
                &format!("tmpfs → disco en {:.1}s.", sync.elapsed.as_secs_f64()),
            );
        }
        timer.mark("prefix-sync");
        let done = format!(
            "⏱ sesión: {:.1}s | extra: {} archivo(s){}",
            session.as_secs_f64(),
            extra_files,
            if extra_stopped {
                " (detenida al arrancar el juego)"
            } else {
                ""
            }
        );
        log(&done);
        if let Some(l) = launch_log {
            l.write(&done);
        }
        let final_report = timer.report();
        if !final_report.is_empty() {
            log(&final_report);
            if let Some(l) = launch_log {
                l.write(&final_report);
            }
        }

        result
    }

    /// Prints `msg` to the launch log and the console.
    fn emit(launch_log: Option<&LaunchLog>, log: &mut impl FnMut(&str), msg: &str) {
        log(msg);
        if let Some(l) = launch_log {
            l.write(msg);
        }
    }

    pub fn run(opts: &LaunchOptions, mut log: impl FnMut(&str)) -> anyhow::Result<LaunchResult> {
        let boot = Self::bootstrap(opts)?;
        if let Some(result) = Self::shortcut_result(&boot, opts, &mut log)? {
            return Ok(result);
        }

        // Only an actual launch needs Proton and the overlay tools; discover and
        // clean must work without them (e.g. from the GUI's "Descubrir" button).
        Self::check_system_deps()?;
        Self::validate_paths(&boot.cfg)?;

        let mut timer = PhaseTimer::new(!opts.dry_run && boot.cfg.timings());
        timer.mark("config/db");

        let prepared = Self::plan(&boot, opts, false)?;
        timer.mark("plan");

        // One log file per real launch (not for dry-run), in the profile's dir.
        let launch_log = if opts.dry_run {
            None
        } else {
            match LaunchLog::create(&prepared.ppaths.log_dir) {
                Ok(l) => {
                    l.write(&format!(
                        "perfil: {} (slug: {})",
                        prepared.profile.name, prepared.profile.slug
                    ));
                    l.write(&format!("game_root: {}", boot.cfg.game_root));
                    l.write(&format!("proton_path: {}", boot.cfg.proton_path));
                    Some(l)
                }
                Err(e) => {
                    crate::log::warn(format!("No se pudo crear el log de lanzamiento: {e}"));
                    None
                }
            }
        };

        if prepared.resolved.is_empty() {
            log("Sin mods activos — lanzando juego limpio.");
            if let Some(l) = &launch_log {
                l.write("Sin mods activos — lanzando juego limpio.");
            }

            if opts.dry_run {
                return Ok(LaunchResult {
                    log: Self::dry_run_report(&boot, &prepared, opts.debug),
                });
            }

            timer.mark("mount");
            let exe = boot.paths.base_game.join(boot.cfg.game_exe());
            Self::launch_phase(
                &boot,
                &prepared,
                launch_log.as_ref(),
                opts,
                &exe,
                &boot.paths.base_game,
                &[],
                &mut timer,
                &mut log,
            )?;
            return Ok(LaunchResult {
                log: "Juego lanzado (limpio).".into(),
            });
        }

        let mut log_output = String::new();
        log_output.push_str("Mods en orden de prioridad:\n");
        for folder in &prepared.resolved {
            log_output.push_str(&format!("  - {}\n", folder));
        }

        if opts.dry_run {
            log_output.push_str(&Self::dry_run_report(&boot, &prepared, opts.debug));
            return Ok(LaunchResult { log: log_output });
        }

        let layer_dirs = Self::mod_layer_dirs(
            &boot.paths.mods_dir,
            &prepared.resolved,
            &prepared.options_by_folder,
        );
        let lowerdir = Self::build_lowerdir(
            &boot.paths.mods_dir,
            &prepared.resolved,
            &boot.paths.base_game,
            &prepared.options_by_folder,
        );
        timer.mark("lowerdir");

        let mut ov = overlay::OverlayMount::mount(
            &lowerdir,
            &prepared.ppaths.upper,
            &prepared.ppaths.work,
            &boot.paths.merged,
        )?;
        ov.start_guard();
        timer.mark("mount");

        let game_exe = boot.paths.merged.join(boot.cfg.game_exe());
        if !game_exe.exists() {
            anyhow::bail!("{} no encontrado tras el montaje", game_exe.display());
        }

        log(log_output.trim_end());
        if let Some(l) = &launch_log {
            l.write(log_output.trim_end());
        }
        Self::launch_phase(
            &boot,
            &prepared,
            launch_log.as_ref(),
            opts,
            &game_exe,
            &boot.paths.merged,
            &layer_dirs,
            &mut timer,
            &mut log,
        )?;

        drop(ov);
        Ok(LaunchResult { log: String::new() })
    }

    /// Warms the page cache for the active profile without launching (used by
    /// `gta-mo preload`).
    pub fn preload(opts: &LaunchOptions, mut log: impl FnMut(&str)) -> anyhow::Result<String> {
        let boot = Self::bootstrap(opts)?;
        Self::validate_paths(&boot.cfg)?;
        let prepared = Self::plan(&boot, opts, false)?;

        let mut mode = Self::resolve_preload_mode(&boot.cfg, opts);
        Self::confirm_full_preload(&mut mode, opts, None);
        let prefix_ram = opts.prefix_ram.unwrap_or_else(|| boot.cfg.prefix_ram());
        let materialize = opts
            .prefix_ram_proton
            .unwrap_or_else(|| boot.cfg.prefix_ram_proton());
        let do_proton = opts
            .preload_proton
            .unwrap_or_else(|| boot.cfg.preload_proton());
        let do_steamrt = opts
            .preload_steamrt
            .unwrap_or_else(|| boot.cfg.preload_steamrt());
        if mode.is_off() && !prefix_ram && !do_proton && !do_steamrt {
            log("Precarga desactivada (preload = \"off\" y prefix_ram = false).");
            return Ok(String::new());
        }

        let layers = Self::mod_layer_dirs(
            &boot.paths.mods_dir,
            &prepared.resolved,
            &prepared.options_by_folder,
        );

        // C: leave the tmpfs prefix ready for the next launch. Dropping the
        // guard is a no-op, so the copy stays until `launch` consumes it.
        let mut tmpfs_ready = false;
        if prefix_ram {
            let proton = PathBuf::from(&boot.cfg.proton_path);
            let pf = crate::prefix_ram::PreparedPrefix::prepare(
                &boot.paths.wine_prefix,
                true,
                Some(&proton),
                materialize,
            );
            let r = pf.report();
            let ram = crate::prefix_ram::prepared_bytes() as f64 / 1_048_576.0;
            if r.copied {
                log(&format!(
                    "Prefijo → tmpfs en {:.1}s (RAM {ram:.0} MiB, materializado {:.0} MiB).",
                    r.elapsed.as_secs_f64(),
                    r.materialized as f64 / 1_048_576.0
                ));
            } else if r.reused {
                log(&format!("Prefijo ya en tmpfs (RAM {ram:.0} MiB)."));
            }
            tmpfs_ready = pf.using_tmpfs();
        }

        let started = Instant::now();
        let stop = AtomicBool::new(false);
        if !mode.is_off() {
            log(&format!(
                "Precargando perfil '{}' (modo {}): {} mod(s).",
                prepared.profile.name,
                mode.as_str(),
                prepared.resolved.len()
            ));
            let prefix = if tmpfs_ready {
                None
            } else {
                Some(boot.paths.wine_prefix.as_path())
            };
            let stats = preload::warm_all(
                mode,
                prefix,
                &boot.paths.base_game,
                boot.cfg.game_exe(),
                &layers,
                &stop,
            );
            log(&format!(
                "Precargado(s) {} archivo(s), {:.1} MiB.",
                stats.files,
                stats.bytes as f64 / 1_048_576.0
            ));
        }

        if do_proton {
            let files = PathBuf::from(&boot.cfg.proton_path).join("files");
            let root = if files.is_dir() {
                files
            } else {
                PathBuf::from(&boot.cfg.proton_path)
            };
            let s = preload::warm_root(&root, &stop);
            log(&format!(
                "Proton prefetch: {} archivo(s), {:.0} MiB.",
                s.files,
                s.bytes as f64 / 1_048_576.0
            ));
        }
        if do_steamrt {
            if let Some(root) = steamrt_dir() {
                let s = preload::warm_root(&root, &stop);
                log(&format!(
                    "steamrt prefetch: {} archivo(s), {:.0} MiB.",
                    s.files,
                    s.bytes as f64 / 1_048_576.0
                ));
            }
        }

        let msg = format!(
            "Precarga completada en {:.1}s.",
            started.elapsed().as_secs_f64()
        );
        log(&msg);
        Ok(msg)
    }

    /// Loads config, takes the launcher lock, opens/migrates the DB, runs the
    /// legacy-upper migration and auto-discovers mods when configured.
    fn bootstrap(opts: &LaunchOptions) -> anyhow::Result<Bootstrap> {
        let cfg = config::load_config().map_err(|e| anyhow::anyhow!("Error de config: {e}"))?;
        let paths = config::RuntimePaths::from_config(&cfg);

        let lock_file = config::lockfile_path();
        let lock = Self::acquire_lock(&lock_file)?;

        let db_path = config::db_path();
        let conn = db::open_db(&db_path)?;
        db::run_migrations(&conn)?;

        Self::migrate_legacy_upper(&cfg, &paths, &conn)?;

        let do_discover = opts.discover || (opts.auto_discover && cfg.auto_discover());
        if do_discover {
            crate::log::info("Auto-descubriendo mods...");
            db::discover_mods(&conn, &paths.mods_dir)?;
        }

        Ok(Bootstrap {
            cfg,
            paths,
            conn,
            _lock: lock,
        })
    }

    /// Handles `--discover`/`--clean`: these return early and never touch the
    /// overlay. Returns `Some` when the caller must stop.
    fn shortcut_result(
        boot: &Bootstrap,
        opts: &LaunchOptions,
        log: &mut impl FnMut(&str),
    ) -> anyhow::Result<Option<LaunchResult>> {
        if opts.discover && !opts.clean {
            return Ok(Some(LaunchResult {
                log: "Descubrimiento completado.".into(),
            }));
        }
        if opts.clean {
            log("Eliminando mods huérfanos...");
            db::clean_orphans(&boot.conn, &boot.paths.mods_dir)?;
            return Ok(Some(LaunchResult {
                log: "Limpieza completada.".into(),
            }));
        }
        Ok(None)
    }

    /// Resolves the profile and its enabled mods into an overlay plan, applying
    /// dependency resolution, cross-mod constraints and Mod Loader priorities.
    /// When `temp` is set, the upper/work/logs live in a throwaway profile dir.
    fn plan(boot: &Bootstrap, opts: &LaunchOptions, temp: bool) -> anyhow::Result<Prepared> {
        let conn = &boot.conn;
        let cfg = &boot.cfg;
        let paths = &boot.paths;

        let profile = Self::resolve_profile(cfg, conn, opts.profile.as_deref())?;
        let ppaths = if temp {
            paths.profile_paths(&Self::temp_slug())
        } else {
            paths.profile_paths(&profile.slug)
        };
        let profile_id = profile.id;

        let all_mods = db::load_all_mods_for_profile(conn, profile_id)?;
        let mods_map = all_mods.into_iter().map(|m| (m.id, m)).collect();
        let deps = db::load_dependencies(conn)?;
        let enabled_ids = db::load_enabled_mod_ids_for_profile(conn, profile_id)?;
        let mod_options = db::all_profile_options(conn, profile_id)?;

        let mut graph = resolver::DepGraph::new(mods_map, deps, enabled_ids);
        if opts.deps_enable {
            graph.prompt = resolver::DepPrompt::AutoEnable;
        } else if opts.deps_ignore {
            graph.prompt = resolver::DepPrompt::Ignore;
        } else if !std::io::stdin().is_terminal() {
            // Headless context (Steam/scripts): stdin is EOF, so a Prompt would
            // block forever. Skip disabled dependencies with a warning instead.
            crate::log::warn(
                "stdin no es una terminal: dependencias requeridas desactivadas se ignorarán. \
                 Usa --deps-enable para autoactivarlas.",
            );
            graph.prompt = resolver::DepPrompt::Ignore;
        }

        if !graph.validate_dependencies() || !graph.detect_cycles() {
            anyhow::bail!("Errores en la resolución de dependencias.");
        }

        graph.enable_mods_for_deps()?;
        graph.warn_optional_deps();
        if !opts.dry_run {
            graph.sync_enabled_to_db(conn, profile_id)?;
        }

        let resolved = graph.resolve();

        // Option selections per folder (for the enabled/known mods), used both
        // by the constraint check and to build the overlay layers.
        let mut options_by_folder: HashMap<String, HashMap<String, String>> = HashMap::new();
        for m in graph.mods.values() {
            if let Some(sel) = mod_options.get(&m.id) {
                if !sel.is_empty() {
                    options_by_folder.insert(m.folder_name.clone(), sel.clone());
                }
            }
        }

        // Cross-mod constraints (mutually exclusive variants, declared
        // incompatibilities). Abort a real mount; the dry-run report lists them.
        let mut all_folders: Vec<String> =
            graph.mods.values().map(|m| m.folder_name.clone()).collect();
        all_folders.sort();
        let mut enabled_folders: Vec<String> = graph
            .mods
            .values()
            .filter(|m| m.enabled)
            .map(|m| m.folder_name.clone())
            .collect();
        enabled_folders.sort();
        let violations = crate::constraints::check(
            &paths.mods_dir,
            &all_folders,
            &enabled_folders,
            &options_by_folder,
        );
        if !violations.is_empty() && !opts.dry_run {
            let mut msg = String::from("Conflictos entre mods activos:\n");
            for v in &violations {
                msg.push_str(&format!("  [X] {}\n", v.message));
            }
            anyhow::bail!("{}", msg.trim_end());
        }

        // Mod Loader priorities for the enabled mods (applied before mounting).
        let modloader_entries =
            crate::modloader::entries_for(&paths.mods_dir, &enabled_folders, &options_by_folder);
        if !opts.dry_run && !modloader_entries.is_empty() {
            let ini = crate::modloader::ini_path(&ppaths.upper);
            if let Err(e) = crate::modloader::apply(&ini, &modloader_entries) {
                crate::log::warn(format!("No se pudo actualizar '{}': {e}", ini.display()));
            } else {
                crate::log::info(format!(
                    "modloader.ini: {} prioridad(es) aplicada(s) en '{}'.",
                    modloader_entries.len(),
                    ini.display()
                ));
            }
        }

        if !crate::meta::overlay_safe_path(&paths.base_game) {
            anyhow::bail!(
                "La ruta base del juego contiene ',' o ':' y no se puede usar como capa de overlay: {}",
                paths.base_game.display()
            );
        }

        let empty = HashMap::new();
        for folder in &resolved {
            let selected = options_by_folder.get(folder).unwrap_or(&empty);
            let layers = crate::meta::mod_layers_with_options(&paths.mods_dir, folder, selected);
            for layer in &layers {
                if !layer.exists() {
                    anyhow::bail!(
                        "La capa del mod '{}' no existe: {} (revisa 'mount' en mod.toml)",
                        folder,
                        layer.display()
                    );
                }
                if !crate::meta::overlay_safe_path(layer) {
                    anyhow::bail!(
                        "La capa del mod '{}' contiene ',' o ':' (no se puede usar en el \
                         lowerdir de fuse-overlayfs): {}",
                        folder,
                        layer.display()
                    );
                }
            }
        }

        Ok(Prepared {
            profile,
            ppaths,
            resolved,
            modloader_entries,
            violations,
            options_by_folder,
        })
    }

    /// Mounts the overlay for a profile and keeps it alive from a detached
    /// daemon, without launching the game. With `temp`, the writes go to a
    /// throwaway profile directory (for installers like MTA) that can later be
    /// captured into a mod with [`Self::unmount`].
    pub fn mount(opts: &LaunchOptions, temp: bool) -> anyhow::Result<MountOutcome> {
        let boot = Self::bootstrap(opts)?;
        let mut sink = |_: &str| {};
        if let Some(result) = Self::shortcut_result(&boot, opts, &mut sink)? {
            return Ok(MountOutcome {
                dry_run: Some(result.log),
                info: None,
            });
        }

        let state_path = mount_state::state_path(Path::new(&boot.cfg.game_root));
        if let Some(existing) = mount_state::read_live(&state_path) {
            anyhow::bail!(
                "Ya hay un overlay montado manualmente (perfil '{}', pid {}). \
                 Desmóntalo con `gta-mo unmount`.",
                existing.profile_name,
                existing.pid
            );
        }

        Self::validate_paths(&boot.cfg)?;

        let prepared = Self::plan(&boot, opts, temp)?;

        if opts.dry_run {
            let mut report = String::new();
            report.push_str("Mods en orden de prioridad:\n");
            for folder in &prepared.resolved {
                report.push_str(&format!("  - {}\n", folder));
            }
            report.push_str(&Self::dry_run_report(&boot, &prepared, opts.debug));
            return Ok(MountOutcome {
                dry_run: Some(report),
                info: None,
            });
        }

        // A real mount needs fuse-overlayfs (`umu-run` is not required).
        Self::check_overlay_deps()?;

        std::fs::create_dir_all(&prepared.ppaths.upper)?;
        std::fs::create_dir_all(&prepared.ppaths.work)?;
        std::fs::create_dir_all(&boot.paths.merged)?;

        let lowerdir = Self::build_lowerdir(
            &boot.paths.mods_dir,
            &prepared.resolved,
            &boot.paths.base_game,
            &prepared.options_by_folder,
        );
        let mut ov = overlay::OverlayMount::mount(
            &lowerdir,
            &prepared.ppaths.upper,
            &prepared.ppaths.work,
            &boot.paths.merged,
        )?;
        ov.start_guard();
        let Some(mount_id) = ov.mount_id() else {
            anyhow::bail!("No se pudo identificar el montaje recién creado.");
        };

        // Detached daemon: holds the mount (and the inherited launcher lock)
        // until it receives SIGTERM/SIGINT.
        let pid = unsafe { libc::fork() };
        if pid < 0 {
            anyhow::bail!("fork falló: {}", std::io::Error::last_os_error());
        }
        if pid == 0 {
            unsafe { libc::setsid() };
            crate::log::info(format!(
                "Overlay montado en '{}'. Desmonta con `gta-mo unmount`.",
                boot.paths.merged.display()
            ));
            overlay::wait_for_shutdown();
            drop(ov);
            mount_state::remove(&state_path);
            std::process::exit(0);
        }

        let info = mount_state::MountInfo {
            slug: prepared.profile.slug.clone(),
            profile_name: prepared.profile.name.clone(),
            temp,
            upper: prepared.ppaths.upper.clone(),
            merged: boot.paths.merged.clone(),
            mount_id,
            pid: pid as u32,
        };
        if let Err(e) = mount_state::write(&state_path, &info) {
            unsafe { libc::kill(pid, libc::SIGKILL) };
            anyhow::bail!("No se pudo escribir el estado del montaje: {e}");
        }

        // The daemon owns the mount now; do not run our destructor.
        std::mem::forget(ov);
        Ok(MountOutcome {
            dry_run: None,
            info: Some(info),
        })
    }

    /// Stops the daemon started by [`Self::mount`] and, for temporary profiles,
    /// optionally captures or discards its upper.
    pub fn unmount(capture: Option<&str>, discard: bool) -> anyhow::Result<UnmountOutcome> {
        let cfg = config::load_config().map_err(|e| anyhow::anyhow!("Error de config: {e}"))?;
        let paths = config::RuntimePaths::from_config(&cfg);
        let state_path = mount_state::state_path(Path::new(&cfg.game_root));

        let Some(info) = mount_state::read_live(&state_path) else {
            anyhow::bail!("No hay ningún overlay montado manualmente.");
        };
        if discard && !info.temp {
            anyhow::bail!("Solo se puede descartar el upper de un perfil temporal (--temp).");
        }
        if let Some(name) = capture {
            if !valid_mod_folder(name) {
                anyhow::bail!("Nombre de mod inválido: '{name}'.");
            }
        }

        unsafe { libc::kill(info.pid as libc::pid_t, libc::SIGTERM) };
        if !wait_until_unmounted(&info.merged, info.mount_id, 100, 100) {
            crate::log::warn("El daemon no desmontó a tiempo; forzando unmount...");
            if !overlay::unmount_matching(&info.merged, info.mount_id) {
                anyhow::bail!("No se pudo desmontar '{}'.", info.merged.display());
            }
        }
        // The daemon holds the launcher lock until it exits; wait briefly so a
        // following `mount`/`launch` does not race with its shutdown.
        wait_until_dead(info.pid, 200, 20);
        mount_state::remove(&state_path);

        let mut capture_report = None;
        let mut registered = false;
        if let Some(name) = capture {
            let dest = paths.mods_dir.join(name);
            let report = upper::capture(
                &info.upper,
                &dest,
                name,
                cfg.game_spec().user_data_dir,
                true,
            )?;
            capture_report = Some(report);
            let db_path = config::db_path();
            if let Ok(conn) = db::open_db(&db_path) {
                let _ = db::run_migrations(&conn);
                registered = db::discover_mods(&conn, &paths.mods_dir).is_ok();
            }
        }

        let mut discarded = false;
        if info.temp && (discard || capture.is_some()) {
            if let Some(root) = info.upper.parent() {
                let _ = std::fs::remove_dir_all(root);
                discarded = true;
            }
        }

        Ok(UnmountOutcome {
            info,
            capture: capture_report,
            discarded,
            registered,
        })
    }

    /// Current manual mount, if any (stale records are cleaned up).
    pub fn status() -> anyhow::Result<Option<mount_state::MountInfo>> {
        let cfg = config::load_config().map_err(|e| anyhow::anyhow!("Error de config: {e}"))?;
        let path = mount_state::state_path(Path::new(&cfg.game_root));
        Ok(mount_state::read_live(&path))
    }

    /// A unique throwaway profile slug (temporary mounts).
    fn temp_slug() -> String {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("tmp-{}-{}", std::process::id(), millis)
    }

    fn resolve_profile(
        cfg: &config::Config,
        conn: &rusqlite::Connection,
        ident: Option<&str>,
    ) -> anyhow::Result<db::Profile> {
        if let Some(ident) = ident {
            return db::resolve_profile(conn, ident);
        }
        if let Some(dp) = cfg.default_profile() {
            if let Ok(p) = db::resolve_profile(conn, dp) {
                return Ok(p);
            }
            crate::log::warn(format!(
                "default_profile '{}' no existe; usando el perfil activo.",
                dp
            ));
        }
        db::active_profile(conn)
    }

    /// One-time migration of the pre-profiles `run/upper` directory into the
    /// `default` profile's upperdir.
    ///
    /// This runs at launch time (not in `db::run_migrations`) on purpose: `ctl`
    /// works without a config, so it has no game_root to compute the paths from.
    /// It is idempotent and safe to defer — if the user only ever runs `ctl`,
    /// the legacy directory stays untouched until the first `launch`, when it
    /// is moved (see README, "Directory layout").
    fn migrate_legacy_upper(
        cfg: &config::Config,
        paths: &config::RuntimePaths,
        conn: &rusqlite::Connection,
    ) -> anyhow::Result<()> {
        let legacy_upper = std::path::PathBuf::from(&cfg.game_root).join("run/upper");
        if !legacy_upper.is_dir() {
            return Ok(());
        }
        let Ok(Some(profile)) = db::get_profile_by_slug(conn, "default") else {
            return Ok(());
        };
        let target = paths.profile_paths(&profile.slug).upper;
        if target.exists() {
            crate::log::warn(
                "run/upper existe pero run/profiles/default/upper también; no se migra.",
            );
            return Ok(());
        }
        std::fs::create_dir_all(target.parent().unwrap())?;
        crate::log::info("Migrando run/upper a run/profiles/default/upper...");
        std::fs::rename(&legacy_upper, &target)?;
        crate::log::info("[+] Migración completada.");
        Ok(())
    }

    fn dry_run_report(boot: &Bootstrap, prepared: &Prepared, debug: bool) -> String {
        let cfg = &boot.cfg;
        let paths = &boot.paths;
        let ppaths = &prepared.ppaths;
        let resolved = &prepared.resolved;
        let modloader = &prepared.modloader_entries;
        let violations = &prepared.violations;

        let mut out = String::new();
        out.push_str("\n=== DRY RUN: no se montara overlay ni se lanzara el juego ===\n\n");
        out.push_str(&format!(
            "Perfil: {} ({})\n",
            prepared.profile.name, prepared.profile.slug
        ));
        out.push_str("lowerdir capas (ordenadas mayor -> menor prioridad):\n");

        if resolved.is_empty() {
            out.push_str(&format!(
                "  1. {} (juego limpio)\n",
                paths.base_game.display()
            ));
        } else {
            let mut n = 0;
            let empty = HashMap::new();
            for folder in resolved {
                let selected = prepared.options_by_folder.get(folder).unwrap_or(&empty);
                let layers =
                    crate::meta::mod_layers_with_options(&paths.mods_dir, folder, selected);
                if layers.len() == 1 && layers[0] == paths.mods_dir.join(folder) {
                    n += 1;
                    out.push_str(&format!("  {}. {}\n", n, layers[0].display()));
                } else {
                    for layer in &layers {
                        n += 1;
                        out.push_str(&format!("  {}. {} (de '{folder}')\n", n, layer.display()));
                    }
                }
            }
            out.push_str(&format!(
                "  {}. {} (base)\n",
                n + 1,
                paths.base_game.display()
            ));
        }

        out.push('\n');
        out.push_str(&format!("upperdir: {}\n", ppaths.upper.display()));
        out.push_str(&format!("workdir:  {}\n", ppaths.work.display()));
        out.push_str(&format!("merged:   {}\n", paths.merged.display()));
        out.push('\n');
        out.push_str(&format!(
            "WINEPREFIX:                {}\n",
            paths.wine_prefix.display()
        ));
        out.push_str(&format!("PROTONPATH:                {}\n", cfg.proton_path));
        out.push_str(&format!("GAMEID:                    {}\n", cfg.game_id()));
        out.push_str(&format!("GAME_EXE:                  {}\n", cfg.game_exe()));
        out.push_str(&format!(
            "PROTON_USE_WINED3D:        {}\n",
            cfg.proton_use_wined3d()
        ));
        out.push_str(&format!(
            "PROTON_DISABLE_NTSYNC:     {}\n",
            cfg.proton_disable_ntsync()
        ));
        out.push_str(&format!(
            "PROTON_DISABLE_UPSCALERS:  {}\n",
            cfg.proton_disable_upscalers()
        ));
        out.push_str(&format!(
            "UMU_RUNTIME_UPDATE:        {}\n",
            cfg.umu_runtime_update()
        ));
        out.push_str(&format!(
            "AUTO_DISCOVER:             {}\n",
            cfg.auto_discover()
        ));
        out.push_str(&format!(
            "PROTON_LOG_DIR:            {}\n",
            ppaths.log_dir.display()
        ));
        out.push_str(&format!(
            "DXVK_LOG_PATH:             {}\n",
            ppaths.log_dir.display()
        ));
        out.push_str(&format!(
            "UMU_LOG:                   {}\n",
            if debug { "debug" } else { "1" }
        ));
        out.push_str(&format!(
            "Log de lanzamiento:        {}/launch-<ts>.log\n",
            ppaths.log_dir.display()
        ));

        if debug {
            out.push_str("\n[DEBUG] Modo debug activado:\n");
            out.push_str("  PROTON_LOG:              1\n");
            out.push_str("  DXVK_LOG_LEVEL:          debug\n");
            out.push_str(&format!("  DXVK_HUD:                {}\n", cfg.dxvk_hud()));
            out.push_str("  WINEDEBUG:               +loaddll\n");
        }
        out.push_str(&format!(
            "Ejecutable:                {}\n",
            paths.merged.join(cfg.game_exe()).display()
        ));

        if !violations.is_empty() {
            out.push_str("\n[!] Conflictos entre mods activos:\n");
            for v in violations {
                out.push_str(&format!("  [X] {}\n", v.message));
            }
        }

        out.push_str("\nmodloader.ini (prioridades que se aplicarían):\n");
        if modloader.is_empty() {
            out.push_str("  (ninguna)\n");
        } else {
            for (name, prio) in modloader {
                out.push_str(&format!("  {name}={prio}\n"));
            }
            out.push_str(&format!(
                "  -> {}\n",
                crate::modloader::ini_path(&ppaths.upper).display()
            ));
        }

        out
    }
}

/// Polls until `merged` is no longer the mount `expected` (or tries run out).
fn wait_until_unmounted(merged: &Path, expected: u64, tries: u32, delay_ms: u64) -> bool {
    for i in 0..tries {
        if overlay::current_mount_id(merged) != Some(expected) {
            return true;
        }
        if i < tries - 1 {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        }
    }
    overlay::current_mount_id(merged) != Some(expected)
}

/// Waits until `pid` is no longer alive (or the tries run out).
fn wait_until_dead(pid: u32, tries: u32, delay_ms: u64) {
    for i in 0..tries {
        if !mount_state::is_alive(pid) {
            return;
        }
        if i < tries - 1 {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        }
    }
}

/// A mod folder name must be a single path component (no separators).
fn valid_mod_folder(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains('/') && !name.contains('\\')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    const UPSCALER_VARS: [&str; 7] = [
        "PROTON_FSR3_UPGRADE",
        "PROTON_FSR4_UPGRADE",
        "PROTON_FSR4_RDNA3_UPGRADE",
        "PROTON_MLFG_UPGRADE",
        "PROTON_DLSS_UPGRADE",
        "PROTON_XESS_UPGRADE",
        "PROTON_USE_OPTISCALER",
    ];

    fn base_config() -> Config {
        Config {
            game_root: "/tmp".into(),
            proton_path: "/tmp".into(),
            game: None,
            game_id: None,
            game_exe: None,
            proton_use_wined3d: None,
            proton_disable_ntsync: None,
            proton_disable_upscalers: None,
            umu_runtime_update: None,
            dxvk_hud: None,
            auto_discover: None,
            mods_dir: None,
            default_profile: None,
            preload: None,
            proton_log: None,
            timings: None,
            prefix_ram: None,
            prefix_ram_proton: None,
            preload_proton: None,
            preload_steamrt: None,
        }
    }

    #[test]
    fn disable_upscalers_exports_upgrade_vars() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut cfg = base_config();
        cfg.proton_disable_upscalers = Some(true);
        LaunchEngine::setup_env(&cfg, Path::new("/tmp/pfx"), Path::new("/tmp/logs"), false);

        for var in UPSCALER_VARS {
            assert_eq!(std::env::var(var).unwrap(), "0", "{var} debe ser 0");
        }
        for var in UPSCALER_VARS {
            std::env::remove_var(var);
        }
    }

    #[test]
    fn upscalers_left_alone_by_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        let cfg = base_config();
        LaunchEngine::setup_env(&cfg, Path::new("/tmp/pfx"), Path::new("/tmp/logs"), false);
        assert!(UPSCALER_VARS
            .iter()
            .all(|var| std::env::var_os(var).is_none()));
    }

    #[test]
    fn umu_runtime_update_disabled_by_default() {
        let _guard = ENV_LOCK.lock().unwrap();
        let cfg = base_config();
        assert!(!cfg.umu_runtime_update());
        LaunchEngine::setup_env(&cfg, Path::new("/tmp/pfx"), Path::new("/tmp/logs"), false);
        assert_eq!(std::env::var("UMU_RUNTIME_UPDATE").unwrap(), "0");
        std::env::remove_var("UMU_RUNTIME_UPDATE");
    }

    #[test]
    fn umu_runtime_update_can_be_enabled() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut cfg = base_config();
        cfg.umu_runtime_update = Some(true);
        assert!(cfg.umu_runtime_update());
        LaunchEngine::setup_env(&cfg, Path::new("/tmp/pfx"), Path::new("/tmp/logs"), false);
        assert_eq!(std::env::var("UMU_RUNTIME_UPDATE").unwrap(), "1");
        std::env::remove_var("UMU_RUNTIME_UPDATE");
    }

    #[test]
    fn setup_env_points_logs_at_the_profile_dir() {
        let _guard = ENV_LOCK.lock().unwrap();
        let cfg = base_config();
        let logs = Path::new("/tmp/gta-mo-logs-test");
        LaunchEngine::setup_env(&cfg, Path::new("/tmp/pfx"), logs, false);
        assert_eq!(
            std::env::var("PROTON_LOG_DIR").unwrap(),
            "/tmp/gta-mo-logs-test"
        );
        assert_eq!(
            std::env::var("DXVK_LOG_PATH").unwrap(),
            "/tmp/gta-mo-logs-test"
        );
        assert_eq!(std::env::var("UMU_LOG").unwrap(), "1");
        // Proton's verbose log is opt-in (off by default to avoid the I/O).
        assert!(std::env::var_os("PROTON_LOG").is_none());
        // `--debug` switches umu to its verbose logger and enables PROTON_LOG.
        LaunchEngine::setup_env(&cfg, Path::new("/tmp/pfx"), logs, true);
        assert_eq!(std::env::var("UMU_LOG").unwrap(), "debug");
        assert_eq!(std::env::var("PROTON_LOG").unwrap(), "1");
        for v in ["PROTON_LOG", "PROTON_LOG_DIR", "DXVK_LOG_PATH", "UMU_LOG"] {
            std::env::remove_var(v);
        }
    }

    #[test]
    fn proton_log_can_be_enabled_from_config() {
        let _guard = ENV_LOCK.lock().unwrap();
        let mut cfg = base_config();
        cfg.proton_log = Some(true);
        LaunchEngine::setup_env(&cfg, Path::new("/tmp/pfx"), Path::new("/tmp/logs"), false);
        assert_eq!(std::env::var("PROTON_LOG").unwrap(), "1");
        std::env::remove_var("PROTON_LOG");
        std::env::remove_var("PROTON_LOG_DIR");
        std::env::remove_var("DXVK_LOG_PATH");
        std::env::remove_var("UMU_LOG");
    }

    #[test]
    fn launch_log_writes_and_prunes() {
        let dir = std::env::temp_dir().join(format!("gta-mo-launchlog-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..25 {
            std::fs::write(dir.join(format!("launch-{i:010}.log")), "viejo").unwrap();
        }

        let log = LaunchLog::create(&dir).unwrap();
        log.write("hola mundo");
        let content = std::fs::read_to_string(log.path()).unwrap();
        assert!(content.contains("hola mundo"));

        let count = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("launch-"))
            .count();
        assert!(
            count <= 21,
            "se conservan los 20 más nuevos + el actual: {count}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_lowerdir_expands_mount_lists() {
        let dir = std::env::temp_dir().join(format!("gta-mo-lowerdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("M1/models")).unwrap();
        std::fs::create_dir_all(dir.join("M1/data")).unwrap();
        std::fs::create_dir_all(dir.join("M2")).unwrap();
        std::fs::create_dir_all(dir.join("base")).unwrap();
        std::fs::write(dir.join("M1/mod.toml"), "mount = [\"models\", \"data\"]\n").unwrap();

        let lower = LaunchEngine::build_lowerdir(
            &dir,
            &["M1".to_string(), "M2".to_string()],
            &dir.join("base"),
            &HashMap::new(),
        );
        let layers: Vec<&str> = lower.split(':').collect();
        assert_eq!(layers[0], dir.join("M1/models").display().to_string());
        assert_eq!(layers[1], dir.join("M1/data").display().to_string());
        assert_eq!(layers[2], dir.join("M2").display().to_string());
        assert_eq!(layers[3], dir.join("base").display().to_string());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
