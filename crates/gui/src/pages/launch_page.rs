use gtk4::glib;
use gtk4::prelude::*;
use gta_mo_core::config;
use gta_mo_core::{db, overlay, resolver};
use std::path::PathBuf;

pub fn create() -> gtk4::Box {
    let container = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(12)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .halign(gtk4::Align::Center)
        .valign(gtk4::Align::Center)
        .build();

    let title = gtk4::Label::builder()
        .label("Lanzar GTA San Andreas")
        .build();
    title.add_css_class("title-2");
    container.append(&title);

    let dry_run_check = gtk4::CheckButton::builder()
        .label("Dry-run (mostrar capas sin lanzar)")
        .build();

    let debug_check = gtk4::CheckButton::builder()
        .label("Modo debug (Proton/DXVK logging)")
        .build();

    let container_dry = dry_run_check.clone();
    let container_debug = debug_check.clone();

    container.append(&dry_run_check);
    container.append(&debug_check);

    let status = gtk4::TextView::builder()
        .editable(false)
        .monospace(true)
        .vexpand(true)
        .hexpand(true)
        .height_request(200)
        .build();

    let scroll = gtk4::ScrolledWindow::builder()
        .child(&status)
        .vexpand(true)
        .hexpand(true)
        .build();
    container.append(&scroll);

    let launch_btn = gtk4::Button::builder()
        .label("▶ Jugar")
        .halign(gtk4::Align::Center)
        .build();
    launch_btn.add_css_class("suggested-action");
    launch_btn.add_css_class("pill");

    let status_buf = status.buffer();

    launch_btn.connect_clicked(move |_| {
        let dry = container_dry.is_active();
        let debug = container_debug.is_active();
        let buf = status_buf.clone();

        std::thread::spawn(move || match do_launch(dry, debug) {
            Ok(log) => {
                glib::idle_add_once(move || {
                    buf.set_text(&format!("{}\n✓ Completado.", log));
                });
            }
            Err(e) => {
                glib::idle_add_once(move || {
                    buf.set_text(&format!("✗ Error: {e}"));
                });
            }
        });
    });

    container.append(&launch_btn);
    container
}

fn do_launch(dry_run: bool, debug: bool) -> anyhow::Result<String> {
    let mut output = String::new();

    let cfg = config::load_config().map_err(|e| anyhow::anyhow!("Config error: {e}"))?;

    let mut missing = Vec::new();
    for cmd in &["fuse-overlayfs", "umu-run"] {
        if which::which(cmd).is_err() {
            missing.push(*cmd);
        }
    }
    if !missing.is_empty() {
        anyhow::bail!("Faltan dependencias: {}", missing.join(", "));
    }

    let game_root = PathBuf::from(&cfg.game_root);
    if !game_root.is_dir() {
        anyhow::bail!(
            "game_root no es un directorio válido: {}",
            cfg.game_root
        );
    }

    let paths = config::RuntimePaths::from_config(&cfg);

    let lock_file = config::lockfile_path();
    let _lock = acquire_lock(&lock_file)?;

    let db_path = config::db_path();
    let conn = db::open_db(&db_path)?;
    db::run_migrations(&conn)?;

    if cfg.auto_discover() {
        output.push_str("Auto-descubriendo mods...\n");
        db::discover_mods(&conn, &paths.mods_dir)?;
    }

    let all_mods = db::load_all_mods(&conn)?;
    let deps = db::load_dependencies(&conn)?;
    let enabled_ids = db::load_enabled_mod_ids(&conn)?;

    let resolved_ids: Vec<i64> = enabled_ids.iter().copied().collect();
    let mut graph = resolver::DepGraph::new(all_mods, deps, resolved_ids);

    if !graph.validate_dependencies() || !graph.detect_cycles() {
        anyhow::bail!("Errores en la resolución de dependencias.");
    }

    graph.enable_mods_for_deps();

    if graph.enabled_ids.is_empty() {
        output.push_str("Sin mods activos — lanzando juego limpio.\n");

        if dry_run {
            output.push_str(&format!("DRY RUN: base = {}\n", paths.base_game.display()));
            output.push_str(&format!(
                "WINEPREFIX = {}\n",
                paths.wine_prefix.display()
            ));
            output.push_str(&format!("GAME_EXE = {}\n", cfg.game_exe()));
            return Ok(output);
        }

        launch_game_clean(&cfg, &paths, debug, &mut output)?;
        return Ok(output);
    }

    let resolved = graph.resolve();

    output.push_str("Mods en orden de prioridad:\n");
    for folder in &resolved {
        output.push_str(&format!("  - {}\n", folder));
    }

    if dry_run {
        output.push_str(&format!("DRY RUN: {} capas\n", resolved.len()));
        output.push_str(&format!(
            "WINEPREFIX = {}\n",
            paths.wine_prefix.display()
        ));
        output.push_str(&format!("GAME_EXE = {}\n", cfg.game_exe()));
        return Ok(output);
    }

    let lowerdir = resolved
        .iter()
        .map(|f| paths.mods_dir.join(f).display().to_string())
        .chain(std::iter::once(paths.base_game.display().to_string()))
        .collect::<Vec<_>>()
        .join(":");

    let mut ov =
        overlay::OverlayMount::mount(&lowerdir, &paths.upper, &paths.work, &paths.merged)?;

    ov.start_guard();

    let game_exe = paths.merged.join(cfg.game_exe());
    if !game_exe.exists() {
        anyhow::bail!("{} no encontrado tras el montaje", game_exe.display());
    }

    output.push_str("Lanzando juego...\n");
    launch_game_internal(&cfg, &paths, &ov, debug, &mut output)?;

    drop(ov);
    Ok(output)
}

fn launch_game_clean(
    config: &config::Config,
    paths: &config::RuntimePaths,
    debug: bool,
    output: &mut String,
) -> anyhow::Result<()> {
    std::env::set_var("WINEPREFIX", &paths.wine_prefix);
    std::env::set_var("PROTONPATH", &config.proton_path);
    std::env::set_var("GAMEID", config.game_id());
    set_common_env(config, debug, paths);
    output.push_str("Lanzando juego limpio...\n");

    let exe = paths.base_game.join(config.game_exe());
    std::env::set_current_dir(&paths.base_game)?;

    let status = std::process::Command::new("umu-run")
        .arg(&exe)
        .status()
        .map_err(|e| anyhow::anyhow!("Error al ejecutar umu-run: {e}"))?;

    if !status.success() {
        anyhow::bail!("umu-run terminó con error: {status}");
    }
    Ok(())
}

fn launch_game_internal(
    config: &config::Config,
    paths: &config::RuntimePaths,
    ov: &overlay::OverlayMount,
    debug: bool,
    output: &mut String,
) -> anyhow::Result<()> {
    let merged = ov.merged_path();
    std::env::set_var("WINEPREFIX", &paths.wine_prefix);
    std::env::set_var("PROTONPATH", &config.proton_path);
    std::env::set_var("GAMEID", config.game_id());
    set_common_env(config, debug, paths);
    output.push_str("Ejecutando umu-run...\n");

    let exe = merged.join(config.game_exe());
    std::env::set_current_dir(merged)?;

    let status = std::process::Command::new("umu-run")
        .arg(&exe)
        .status()
        .map_err(|e| anyhow::anyhow!("Error al ejecutar umu-run: {e}"))?;

    if !status.success() {
        anyhow::bail!("umu-run terminó con error: {status}");
    }
    Ok(())
}

fn set_common_env(config: &config::Config, debug: bool, paths: &config::RuntimePaths) {
    std::env::set_var(
        "PROTON_USE_WINED3D",
        if config.proton_use_wined3d() {
            "1"
        } else {
            "0"
        },
    );
    std::env::set_var(
        "PROTON_DISABLE_NTSYNC",
        if config.proton_disable_ntsync() {
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
        std::env::set_var("DXVK_HUD", config.dxvk_hud());
        std::env::set_var("WINEDEBUG", "+loaddll");
    }
}

fn acquire_lock(lock_path: &std::path::Path) -> anyhow::Result<std::fs::File> {
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
