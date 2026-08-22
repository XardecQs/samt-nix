mod ctl;

use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::generate;
use clap_complete::shells::{Bash, Fish, Zsh};
use gta_mo_core::config;
use gta_mo_core::db;
use gta_mo_core::launcher::{LaunchEngine, LaunchOptions};
use std::io::Write;

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
    #[command(hide = true, about = "Generate shell completions")]
    Completions {
        shell: String,
    },
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
    Remove { ident: String },
    #[command(about = "Enable a mod")]
    Enable { ident: String },
    #[command(about = "Disable a mod")]
    Disable { ident: String },
    #[command(about = "Change load order")]
    Order { ident: String, new_order: i64 },
    #[command(about = "Rename a mod's display name (or its folder with --folder)")]
    Rename {
        ident: String,
        new_name: String,
        #[arg(long, help = "Rename the mod's folder on disk too")]
        folder: bool,
    },
    #[command(about = "Show detailed mod info")]
    Info { ident: String },
    #[command(about = "Manage dependencies")]
    Dep {
        #[command(subcommand)]
        action: DepAction,
    },
}

#[derive(Subcommand)]
pub enum DepAction {
    #[command(about = "Add a dependency (--optional for recommended deps)")]
    Add {
        mod_ident: String,
        dep_ident: String,
        #[arg(
            long,
            help = "Optional (recommended) dependency, not required to launch"
        )]
        optional: bool,
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
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            let mut stdout = std::io::stdout().lock();
            let matched = match shell.as_str() {
                "bash" => {
                    generate(Bash, &mut cmd, &name, &mut stdout);
                    true
                }
                "zsh" => {
                    generate(Zsh, &mut cmd, &name, &mut stdout);
                    true
                }
                "fish" => {
                    generate(Fish, &mut cmd, &name, &mut stdout);
                    true
                }
                _ => false,
            };
            if matched {
                stdout.flush().ok();
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "Shell no soportado: {shell}. Usa: bash, zsh, fish"
                ))
            }
        }
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
    println!("=== GTA SA Mod Organizer ===");

    let opts = LaunchOptions {
        dry_run: args.dry_run,
        debug: args.debug,
        discover: args.discover,
        clean: args.clean,
        auto_discover: true,
    };

    let result = LaunchEngine::run(&opts, |msg| println!("{msg}"))?;

    if !result.log.is_empty() {
        println!("{}", result.log);
    }

    Ok(())
}
