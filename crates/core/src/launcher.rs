use std::io::IsTerminal;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use crate::{config, db, mount_state, overlay, resolver, upper};

pub struct LaunchEngine;

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
        }
    }
}

pub struct LaunchResult {
    pub log: String,
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

    pub fn build_lowerdir(mods_dir: &Path, resolved: &[String], base_game: &Path) -> String {
        let mut layers: Vec<String> = resolved
            .iter()
            .flat_map(|f| crate::meta::mod_layers(mods_dir, f))
            .map(|p| p.display().to_string())
            .collect();
        layers.push(base_game.display().to_string());
        layers.join(":")
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

        if debug {
            std::fs::create_dir_all(log_dir).ok();
            std::env::set_var("PROTON_LOG", "1");
            std::env::set_var("DXVK_LOG_LEVEL", "debug");
            std::env::set_var("DXVK_LOG_PATH", log_dir);
            std::env::set_var("DXVK_HUD", cfg.dxvk_hud());
            std::env::set_var("WINEDEBUG", "+loaddll");
        }
    }

    pub fn launch_game(exe_path: &Path, working_dir: &Path) -> anyhow::Result<()> {
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
                Some(uid) => Self::launch_game_dropped(exe_path, working_dir, uid, drop_gid),
                None => anyhow::bail!(
                    "gta-mo se ejecuta como root. Define GTA_MO_DROP_UID (y opcionalmente GTA_MO_DROP_GID) para lanzar el juego como usuario."
                ),
            };
        }

        Self::launch_game_direct(exe_path, working_dir)
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

    fn launch_game_direct(exe_path: &Path, working_dir: &Path) -> anyhow::Result<()> {
        let status = std::process::Command::new("umu-run")
            .arg(exe_path)
            .current_dir(working_dir)
            .status()
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
            let code = Self::drop_and_run_game(exe_path, working_dir, uid, gid);
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

    fn drop_and_run_game(exe_path: &Path, working_dir: &Path, uid: u32, gid: u32) -> i32 {
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
        match std::process::Command::new("umu-run")
            .arg(exe_path)
            .current_dir(working_dir)
            .status()
        {
            Ok(s) if s.success() => 0,
            Ok(s) => s.code().unwrap_or(1),
            Err(e) => {
                eprintln!("Error al ejecutar umu-run: {e}");
                1
            }
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

        let prepared = Self::plan(&boot, opts, false)?;

        if prepared.resolved.is_empty() {
            log("Sin mods activos — lanzando juego limpio.");

            if opts.dry_run {
                return Ok(LaunchResult {
                    log: Self::dry_run_report(&boot, &prepared, opts.debug),
                });
            }

            Self::setup_env(
                &boot.cfg,
                &boot.paths.wine_prefix,
                &prepared.ppaths.log_dir,
                opts.debug,
            );
            let exe = boot.paths.base_game.join(boot.cfg.game_exe());
            Self::launch_game(&exe, &boot.paths.base_game)?;
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

        let lowerdir = Self::build_lowerdir(
            &boot.paths.mods_dir,
            &prepared.resolved,
            &boot.paths.base_game,
        );

        let mut ov = overlay::OverlayMount::mount(
            &lowerdir,
            &prepared.ppaths.upper,
            &prepared.ppaths.work,
            &boot.paths.merged,
        )?;
        ov.start_guard();

        let game_exe = boot.paths.merged.join(boot.cfg.game_exe());
        if !game_exe.exists() {
            anyhow::bail!("{} no encontrado tras el montaje", game_exe.display());
        }

        Self::setup_env(
            &boot.cfg,
            &boot.paths.wine_prefix,
            &prepared.ppaths.log_dir,
            opts.debug,
        );
        log(log_output.trim_end());
        log("Lanzando juego...");
        Self::launch_game(&game_exe, &boot.paths.merged)?;

        drop(ov);
        Ok(LaunchResult { log: String::new() })
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
        let violations = crate::constraints::check(&paths.mods_dir, &all_folders, &enabled_folders);
        if !violations.is_empty() && !opts.dry_run {
            let mut msg = String::from("Conflictos entre mods activos:\n");
            for v in &violations {
                msg.push_str(&format!("  [X] {}\n", v.message));
            }
            anyhow::bail!("{}", msg.trim_end());
        }

        // Mod Loader priorities for the enabled mods (applied before mounting).
        let modloader_entries = crate::modloader::entries_for(&paths.mods_dir, &enabled_folders);
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

        for folder in &resolved {
            let layers = crate::meta::mod_layers(&paths.mods_dir, folder);
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
            for folder in resolved {
                let layers = crate::meta::mod_layers(&paths.mods_dir, folder);
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
            "AUTO_DISCOVER:             {}\n",
            cfg.auto_discover()
        ));

        if debug {
            out.push_str("\n[DEBUG] Modo debug activado:\n");
            out.push_str("  PROTON_LOG:              1\n");
            out.push_str("  DXVK_LOG_LEVEL:          debug\n");
            out.push_str(&format!(
                "  DXVK_LOG_PATH:           {}\n",
                ppaths.log_dir.display()
            ));
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
            dxvk_hud: None,
            auto_discover: None,
            mods_dir: None,
            default_profile: None,
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
        );
        let layers: Vec<&str> = lower.split(':').collect();
        assert_eq!(layers[0], dir.join("M1/models").display().to_string());
        assert_eq!(layers[1], dir.join("M1/data").display().to_string());
        assert_eq!(layers[2], dir.join("M2").display().to_string());
        assert_eq!(layers[3], dir.join("base").display().to_string());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
