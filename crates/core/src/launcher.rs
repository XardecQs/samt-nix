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
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            debug: false,
            discover: false,
            clean: false,
            auto_discover: true,
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
            .map(|f| mods_dir.join(f).display().to_string())
            .collect();
        layers.push(base_game.display().to_string());
        layers.join(":")
    }

    pub fn setup_env(cfg: &config::Config, paths: &config::RuntimePaths, debug: bool) {
        std::env::set_var("WINEPREFIX", &paths.wine_prefix);
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

        if debug {
            std::fs::create_dir_all(&paths.log_dir).ok();
            std::env::set_var("PROTON_LOG", "1");
            std::env::set_var("DXVK_LOG_LEVEL", "debug");
            std::env::set_var("DXVK_LOG_PATH", &paths.log_dir);
            std::env::set_var("DXVK_HUD", cfg.dxvk_hud());
            std::env::set_var("WINEDEBUG", "+loaddll");
        }
    }

    pub fn launch_game(exe_path: &Path, working_dir: &Path) -> anyhow::Result<()> {
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

        let all_mods = db::load_all_mods(&conn)?;
        let deps = db::load_dependencies(&conn)?;
        let enabled_ids = db::load_enabled_mod_ids(&conn)?;

        let mut graph = resolver::DepGraph::new(all_mods, deps, enabled_ids);

        if !graph.validate_dependencies() || !graph.detect_cycles() {
            anyhow::bail!("Errores en la resolución de dependencias.");
        }

        graph.enable_mods_for_deps();
        graph.sync_enabled_to_db(&conn)?;

        if graph.enabled_ids.is_empty() {
            log("Sin mods activos — lanzando juego limpio.");

            if opts.dry_run {
                return Ok(LaunchResult {
                    log: Self::dry_run_report(&cfg, &paths, &[], opts.debug),
                });
            }

            Self::setup_env(&cfg, &paths, opts.debug);
            let exe = paths.base_game.join(cfg.game_exe());
            Self::launch_game(&exe, &paths.base_game)?;
            return Ok(LaunchResult {
                log: "Juego lanzado (limpio).".into(),
            });
        }

        let resolved = graph.resolve();

        for folder in &resolved {
            let mod_path = paths.mods_dir.join(folder);
            if !mod_path.exists() {
                anyhow::bail!(
                    "La carpeta del mod '{}' no existe: {}",
                    folder,
                    mod_path.display()
                );
            }
        }

        let mut log_output = String::new();
        log_output.push_str("Mods en orden de prioridad:\n");
        for folder in &resolved {
            log_output.push_str(&format!("  - {}\n", folder));
        }

        if opts.dry_run {
            log_output.push_str(&Self::dry_run_report(&cfg, &paths, &resolved, opts.debug));
            return Ok(LaunchResult { log: log_output });
        }

        let lowerdir = Self::build_lowerdir(&paths.mods_dir, &resolved, &paths.base_game);

        let mut ov =
            overlay::OverlayMount::mount(&lowerdir, &paths.upper, &paths.work, &paths.merged)?;
        ov.start_guard();

        let game_exe = paths.merged.join(cfg.game_exe());
        if !game_exe.exists() {
            anyhow::bail!("{} no encontrado tras el montaje", game_exe.display());
        }

        Self::setup_env(&cfg, &paths, opts.debug);
        log("Lanzando juego...");
        Self::launch_game(&game_exe, &paths.merged)?;

        drop(ov);
        Ok(LaunchResult { log: log_output })
    }

    fn dry_run_report(
        cfg: &config::Config,
        paths: &config::RuntimePaths,
        resolved: &[String],
        debug: bool,
    ) -> String {
        let mut out = String::new();
        out.push_str("\n=== DRY RUN: no se montara overlay ni se lanzara el juego ===\n\n");
        out.push_str("lowerdir capas (ordenadas mayor -> menor prioridad):\n");

        if resolved.is_empty() {
            out.push_str(&format!(
                "  1. {} (juego limpio)\n",
                paths.base_game.display()
            ));
        } else {
            for (i, folder) in resolved.iter().enumerate() {
                out.push_str(&format!(
                    "  {}. {}\n",
                    i + 1,
                    paths.mods_dir.join(folder).display()
                ));
            }
            out.push_str(&format!(
                "  {}. {} (base)\n",
                resolved.len() + 1,
                paths.base_game.display()
            ));
        }

        out.push('\n');
        out.push_str(&format!("upperdir: {}\n", paths.upper.display()));
        out.push_str(&format!("workdir:  {}\n", paths.work.display()));
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
            "AUTO_DISCOVER:             {}\n",
            cfg.auto_discover()
        ));

        if debug {
            out.push_str("\n[DEBUG] Modo debug activado:\n");
            out.push_str("  PROTON_LOG:              1\n");
            out.push_str("  DXVK_LOG_LEVEL:          debug\n");
            out.push_str(&format!(
                "  DXVK_LOG_PATH:           {}\n",
                paths.log_dir.display()
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
