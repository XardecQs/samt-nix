use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use crate::{config, db, overlay, resolver};

pub struct LaunchEngine;

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
        let mut missing = Vec::new();
        for cmd in &["fuse-overlayfs", "umu-run"] {
            if which::which(cmd).is_err() {
                missing.push(*cmd);
            }
        }
        if !missing.is_empty() {
            anyhow::bail!(
                "Faltan dependencias: {}. Asegurate de tener fuse-overlayfs y umu-launcher instalados.",
                missing.join(", ")
            );
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
        Self::check_system_deps()?;

        let cfg = config::load_config().map_err(|e| anyhow::anyhow!("Error de config: {e}"))?;
        Self::validate_paths(&cfg)?;

        let paths = config::RuntimePaths::from_config(&cfg);

        let lock_file = config::lockfile_path();
        let _lock = Self::acquire_lock(&lock_file)?;

        let db_path = config::db_path();
        let conn = db::open_db(&db_path)?;
        db::run_migrations(&conn)?;

        Self::migrate_legacy_upper(&cfg, &paths, &conn)?;

        let do_discover = opts.discover || (opts.auto_discover && cfg.auto_discover());
        if do_discover {
            log("Auto-descubriendo mods...");
            db::discover_mods(&conn, &paths.mods_dir)?;
        }

        if opts.discover && !opts.clean {
            return Ok(LaunchResult {
                log: "Descubrimiento completado.".into(),
            });
        }

        if opts.clean {
            log("Eliminando mods huérfanos...");
            db::clean_orphans(&conn, &paths.mods_dir)?;
            return Ok(LaunchResult {
                log: "Limpieza completada.".into(),
            });
        }

        let profile = Self::resolve_profile(&cfg, &conn, opts.profile.as_deref())?;
        let profile_id = profile.id;
        let ppaths = paths.profile_paths(&profile.slug);

        let all_mods = db::load_all_mods_for_profile(&conn, profile_id)?;
        let mods_map = all_mods.into_iter().map(|m| (m.id, m)).collect();
        let deps = db::load_dependencies(&conn)?;
        let enabled_ids = db::load_enabled_mod_ids_for_profile(&conn, profile_id)?;

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
            graph.sync_enabled_to_db(&conn, profile_id)?;
        }

        if graph.enabled_ids.is_empty() {
            log("Sin mods activos — lanzando juego limpio.");

            if opts.dry_run {
                return Ok(LaunchResult {
                    log: Self::dry_run_report(&cfg, &paths, &profile, &ppaths, &[], opts.debug),
                });
            }

            Self::setup_env(&cfg, &paths.wine_prefix, &ppaths.log_dir, opts.debug);
            let exe = paths.base_game.join(cfg.game_exe());
            Self::launch_game(&exe, &paths.base_game)?;
            return Ok(LaunchResult {
                log: "Juego lanzado (limpio).".into(),
            });
        }

        let resolved = graph.resolve();

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
            }
        }

        let mut log_output = String::new();
        log_output.push_str("Mods en orden de prioridad:\n");
        for folder in &resolved {
            log_output.push_str(&format!("  - {}\n", folder));
        }

        if opts.dry_run {
            log_output.push_str(&Self::dry_run_report(
                &cfg, &paths, &profile, &ppaths, &resolved, opts.debug,
            ));
            return Ok(LaunchResult { log: log_output });
        }

        let lowerdir = Self::build_lowerdir(&paths.mods_dir, &resolved, &paths.base_game);

        let mut ov =
            overlay::OverlayMount::mount(&lowerdir, &ppaths.upper, &ppaths.work, &paths.merged)?;
        ov.start_guard();

        let game_exe = paths.merged.join(cfg.game_exe());
        if !game_exe.exists() {
            anyhow::bail!("{} no encontrado tras el montaje", game_exe.display());
        }

        Self::setup_env(&cfg, &paths.wine_prefix, &ppaths.log_dir, opts.debug);
        log(log_output.trim_end());
        log("Lanzando juego...");
        Self::launch_game(&game_exe, &paths.merged)?;

        drop(ov);
        Ok(LaunchResult { log: String::new() })
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

    fn dry_run_report(
        cfg: &config::Config,
        paths: &config::RuntimePaths,
        profile: &db::Profile,
        ppaths: &config::ProfilePaths,
        resolved: &[String],
        debug: bool,
    ) -> String {
        let mut out = String::new();
        out.push_str("\n=== DRY RUN: no se montara overlay ni se lanzara el juego ===\n\n");
        out.push_str(&format!("Perfil: {} ({})\n", profile.name, profile.slug));
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

        out
    }
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
