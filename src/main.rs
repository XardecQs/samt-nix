mod config;
mod ctl;
mod db;
mod overlay;
mod resolver;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "gta-mo",
    about = "GTA Mod Organizer — GTA SA mod launcher with overlayfs",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Launch(LaunchArgs),
    Ctl(CtlArgs),
}

#[derive(Args)]
struct LaunchArgs {
    #[arg(long, help = "Print overlay stack and exit without launching")]
    dry_run: bool,

    #[arg(long, help = "Enable Proton/DXVK debug logging")]
    debug: bool,

    #[arg(long, help = "Scan mods/ for new mods and exit")]
    discover: bool,

    #[arg(long, help = "Remove orphaned mod entries from database and exit")]
    clean: bool,
}

#[derive(Args)]
pub struct CtlArgs {
    #[command(subcommand)]
    pub command: CtlCommand,
}

#[derive(Subcommand)]
pub enum CtlCommand {
    #[command(about = "List all mods")]
    List {
        #[arg(short = 'v', long, help = "Show dependency info")]
        verbose: bool,

        #[arg(long = "enabled", group = "filter", help = "Only enabled mods")]
        enabled: bool,

        #[arg(long = "disabled", group = "filter", help = "Only disabled mods")]
        disabled: bool,
    },
    #[command(about = "Add a new mod")]
    Add {
        folder: String,
        #[arg(long, help = "Visible display name")]
        name: Option<String>,
        #[arg(long, help = "Load order (default: auto)")]
        order: Option<i64>,
    },
    #[command(about = "Remove a mod")]
    Remove {
        ident: String,
    },
    #[command(about = "Enable a mod")]
    Enable {
        ident: String,
    },
    #[command(about = "Disable a mod")]
    Disable {
        ident: String,
    },
    #[command(about = "Change load order")]
    Order {
        ident: String,
        new_order: i64,
    },
    #[command(about = "Rename a mod's display name")]
    Rename {
        ident: String,
        new_name: String,
    },
    #[command(about = "Show detailed mod info")]
    Info {
        ident: String,
    },
    #[command(about = "Manage dependencies")]
    Dep {
        #[command(subcommand)]
        action: DepAction,
    },
    #[command(about = "Interactive TUI mode")]
    Tui,
}

#[derive(Subcommand)]
pub enum DepAction {
    #[command(about = "Add a dependency")]
    Add {
        mod_ident: String,
        dep_ident: String,
    },
    #[command(about = "Remove a dependency")]
    Remove {
        mod_ident: String,
        dep_ident: String,
    },
}

fn main() {
    let cli = Cli::parse();

    let command = cli.command.unwrap_or(Command::Launch(LaunchArgs {
        dry_run: false,
        debug: false,
        discover: false,
        clean: false,
    }));

    let result = match command {
        Command::Launch(args) => cmd_launch(args),
        Command::Ctl(args) => {
            let db_path = config::db_path();
            db::ensure_db_dir(&db_path).unwrap_or_else(|e| {
                db::log::die(format!("No se pudo acceder a la base de datos: {e}"));
            });
            let conn = db::open_db(&db_path).unwrap_or_else(|e| {
                db::log::die(format!("No se pudo abrir la base de datos: {e}"));
            });
            db::run_migrations(&conn).unwrap_or_else(|e| {
                db::log::die(format!("Error en migraciones: {e}"));
            });
            ctl::run(&conn, &args)
        }
    };

    if let Err(e) = result {
        db::log::error(format!("{e:#}"));
        std::process::exit(1);
    }
}

fn cmd_launch(args: LaunchArgs) -> anyhow::Result<()> {
    println!("=== GTA SA Mod Organizer (Rust Mode) ===");

    check_system_deps()?;

    let config = config::load_config_or_die();

    validate_config_paths(&config)?;

    let lock_file = config::lockfile_path();
    let lock = acquire_lock(&lock_file)
        .context("No se pudo adquirir el lockfile")?;

    let db_path = config::db_path();
    let conn = db::open_db(&db_path)?;
    db::run_migrations(&conn)?;

    let paths = config::RuntimePaths::from_config(&config);

    let do_discover = args.discover || config.auto_discover();
    if do_discover {
        println!();
        db::log::info("Ejecutando autodescubrimiento de mods...");
        db::discover_mods(&conn, &paths.mods_dir)?;
        println!();
    }

    if args.discover && !args.clean {
        return Ok(());
    }

    if args.clean {
        db::log::info("Eliminando mods huerfanos...");
        db::clean_orphans(&conn, &paths.mods_dir)?;
        println!();
        return Ok(());
    }

    db::log::info("Leyendo configuracion de mods desde SQLite...");

    let all_mods = db::load_all_mods(&conn)?;
    let deps = db::load_dependencies(&conn)?;
    let enabled_ids = db::load_enabled_mod_ids(&conn)?;

    let mut graph = resolver::DepGraph::new(all_mods, deps, enabled_ids);

    if !graph.validate_dependencies() || !graph.detect_cycles() {
        anyhow::bail!("Errores en la resolucion de dependencias.");
    }

    graph.enable_mods_for_deps();
    graph.sync_enabled_to_db(&conn)?;

    if graph.enabled_ids.is_empty() {
        println!();
        db::log::info("No hay mods activos. Se lanzara el juego limpio.");
        println!();

        if args.dry_run {
            print_dry_run(&config, &paths, &[], &args);
            return Ok(());
        }

        launch_game_clean(&config, &paths, &args)?;
        return Ok(());
    }

    let resolved = graph.resolve();

    for folder in &resolved {
        let mod_path = paths.mods_dir.join(folder);
        if !mod_path.exists() {
            db::log::error(format!(
                "La carpeta del mod '{folder}' no existe: {}",
                mod_path.display()
            ));
            anyhow::bail!("Falta la carpeta de un mod.");
        }
    }

    db::log::info("Mods activos detectados en orden de prioridad:");
    for folder in &resolved {
        println!("    - {folder}");
    }

    if args.dry_run {
        print_dry_run(&config, &paths, &resolved, &args);
        return Ok(());
    }

    let lowerdir = build_lowerdir(&paths.mods_dir, &resolved, &paths.base_game);

    let mut overlay = overlay::OverlayMount::mount(
        &lowerdir,
        &paths.upper,
        &paths.work,
        &paths.merged,
    )?;

    overlay.start_guard();

    let game_exe = paths.merged.join(config.game_exe());
    if !game_exe.exists() {
        anyhow::bail!(
            "{} no encontrado tras el montaje",
            game_exe.display()
        );
    }

    launch_game(&config, &paths, &overlay, &args)?;

    drop(overlay);
    drop(lock);

    db::log::info("Sesion terminada.");
    Ok(())
}

fn check_system_deps() -> anyhow::Result<()> {
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

fn validate_config_paths(config: &config::Config) -> anyhow::Result<()> {
    let game_root = PathBuf::from(&config.game_root);
    if !game_root.is_dir() {
        anyhow::bail!("game_root no es un directorio valido: {}", config.game_root);
    }

    let proton_path = PathBuf::from(&config.proton_path);
    if !proton_path.is_dir() {
        anyhow::bail!("proton_path no es un directorio valido: {}", config.proton_path);
    }

    let paths = config::RuntimePaths::from_config(config);
    if !paths.base_game.is_dir() {
        anyhow::bail!(
            "directorio base del juego no encontrado: {}",
            paths.base_game.display()
        );
    }

    Ok(())
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
        .context("Ya hay una instancia en ejecucion")?;

    Ok(file)
}

fn build_lowerdir(mods_dir: &std::path::Path, resolved: &[String], base_game: &std::path::Path) -> String {
    let mut layers: Vec<String> = resolved
        .iter()
        .map(|f| mods_dir.join(f).display().to_string())
        .collect();
    layers.push(base_game.display().to_string());
    layers.join(":")
}

fn launch_game(
    config: &config::Config,
    paths: &config::RuntimePaths,
    overlay: &overlay::OverlayMount,
    args: &LaunchArgs,
) -> anyhow::Result<()> {
    let merged = overlay.merged_path();

    std::env::set_var("WINEPREFIX", &paths.wine_prefix);
    std::env::set_var("PROTONPATH", &config.proton_path);
    std::env::set_var("GAMEID", config.game_id());
    std::env::set_var("PROTON_USE_WINED3D", if config.proton_use_wined3d() { "1" } else { "0" });
    std::env::set_var("PROTON_DISABLE_NTSYNC", if config.proton_disable_ntsync() { "1" } else { "0" });

    if args.debug {
        std::fs::create_dir_all(&paths.log_dir)?;
        std::env::set_var("PROTON_LOG", "1");
        std::env::set_var("DXVK_LOG_LEVEL", "debug");
        std::env::set_var("DXVK_LOG_PATH", &paths.log_dir);
        std::env::set_var("DXVK_HUD", config.dxvk_hud());
        std::env::set_var("WINEDEBUG", "+loaddll");
        db::log::info("Modo debug activado: PROTON_LOG + DXVK debug + WINEDEBUG");
    }

    db::log::info("Lanzando juego desde merged...");

    std::env::set_current_dir(merged)?;

    let exe = merged.join(config.game_exe());

    let status = std::process::Command::new("umu-run")
        .arg(&exe)
        .status()
        .context("Error al ejecutar umu-run")?;

    if !status.success() {
        anyhow::bail!("umu-run termino con error: {status}");
    }

    Ok(())
}

fn launch_game_clean(
    config: &config::Config,
    paths: &config::RuntimePaths,
    args: &LaunchArgs,
) -> anyhow::Result<()> {
    std::env::set_var("WINEPREFIX", &paths.wine_prefix);
    std::env::set_var("PROTONPATH", &config.proton_path);
    std::env::set_var("GAMEID", config.game_id());
    std::env::set_var("PROTON_USE_WINED3D", if config.proton_use_wined3d() { "1" } else { "0" });
    std::env::set_var("PROTON_DISABLE_NTSYNC", if config.proton_disable_ntsync() { "1" } else { "0" });

    if args.debug {
        std::fs::create_dir_all(&paths.log_dir)?;
        std::env::set_var("PROTON_LOG", "1");
        std::env::set_var("DXVK_LOG_LEVEL", "debug");
        std::env::set_var("DXVK_LOG_PATH", &paths.log_dir);
        std::env::set_var("DXVK_HUD", config.dxvk_hud());
        std::env::set_var("WINEDEBUG", "+loaddll");
    }

    let exe = paths.base_game.join(config.game_exe());
    db::log::info("Lanzando juego limpio...");

    std::env::set_current_dir(&paths.base_game)?;

    let status = std::process::Command::new("umu-run")
        .arg(&exe)
        .status()
        .context("Error al ejecutar umu-run")?;

    if !status.success() {
        anyhow::bail!("umu-run termino con error: {status}");
    }

    Ok(())
}

fn print_dry_run(
    config: &config::Config,
    paths: &config::RuntimePaths,
    resolved: &[String],
    args: &LaunchArgs,
) {
    println!();
    println!("=== DRY RUN: no se montara overlay ni se lanzara el juego ===");
    println!();
    println!("lowerdir capas (ordenadas mayor -> menor prioridad):");

    if resolved.is_empty() {
        println!("  1. {} (juego limpio)", paths.base_game.display());
    } else {
        let mut i = 1;
        for folder in resolved {
            println!("  {}. {}", i, paths.mods_dir.join(folder).display());
            i += 1;
        }
        println!("  {}. {} (base)", i, paths.base_game.display());
    }

    println!();
    println!("upperdir: {}", paths.upper.display());
    println!("workdir:  {}", paths.work.display());
    println!("merged:   {}", paths.merged.display());
    println!();
    println!("WINEPREFIX:                {}", paths.wine_prefix.display());
    println!("PROTONPATH:                {}", config.proton_path);
    println!("GAMEID:                    {}", config.game_id());
    println!("GAME_EXE:                  {}", config.game_exe());
    println!("PROTON_USE_WINED3D:        {}", config.proton_use_wined3d());
    println!("PROTON_DISABLE_NTSYNC:     {}", config.proton_disable_ntsync());
    println!("AUTO_DISCOVER:             {}", config.auto_discover());

    if args.debug {
        println!();
        println!("[DEBUG] Modo debug activado:");
        println!("  PROTON_LOG:              1");
        println!("  DXVK_LOG_LEVEL:          debug");
        println!("  DXVK_LOG_PATH:           {}", paths.log_dir.display());
        println!("  DXVK_HUD:                {}", config.dxvk_hud());
        println!("  WINEDEBUG:               +loaddll");
    }
    println!("Ejecutable:                {}", paths.merged.join(config.game_exe()).display());
}
