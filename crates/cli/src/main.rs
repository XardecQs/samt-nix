mod ctl;

use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::generate;
use clap_complete::shells::{Bash, Fish, Zsh};
use gta_mo_core::config;
use gta_mo_core::db;
use gta_mo_core::launcher::{LaunchEngine, LaunchOptions};
use gta_mo_core::log;
use std::io::Write;

#[derive(Parser)]
#[command(
    name = "gta-mo",
    about = "GTA Mod Organizer — GTA SA mod launcher with overlayfs",
    version
)]
struct Cli {
    #[arg(long, global = true, help = "Profile to use (name, slug or id)")]
    profile: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    #[command(hide = true, about = "Generate shell completions")]
    Completions {
        shell: String,
    },
    #[command(about = "Mount the overlay and launch the game")]
    Launch(LaunchArgs),
    #[command(about = "Launch from Steam (re-execs inside a user/mount namespace)")]
    Steam(LaunchArgs),
    #[command(about = "Manage mods, profiles, groups and dependencies")]
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

    #[arg(
        long,
        conflicts_with = "deps_ignore",
        help = "Auto-enable disabled dependencies without prompting"
    )]
    deps_enable: bool,

    #[arg(
        long,
        conflicts_with = "deps_enable",
        help = "Skip disabled dependencies without prompting"
    )]
    deps_ignore: bool,

    #[arg(
        long,
        help = "Do not auto-discover mods/ on launch, even if auto_discover is set"
    )]
    no_auto_discover: bool,
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

        #[arg(long, help = "Filter by tag")]
        tag: Option<String>,

        #[arg(long, help = "Filter by group (name, slug or id)")]
        group: Option<String>,

        #[arg(long, help = "Filter by author")]
        author: Option<String>,

        #[arg(long, help = "Filter by stable mod id (author:slug)")]
        id: Option<String>,

        #[arg(long, help = "Search name, folder, author, id, description and tags")]
        search: Option<String>,

        #[arg(long, value_parser = ["name", "folder", "author", "order", "mod_id", "version", "status"], help = "Sort by field (default: priority order)")]
        sort: Option<String>,

        #[arg(long, value_parser = ["asc", "desc"], help = "Sort direction (default asc, desc for --sort order)")]
        dir: Option<String>,

        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Add a new mod")]
    Add {
        folder: String,
        #[arg(long, help = "Visible display name")]
        name: Option<String>,
    },
    #[command(about = "Generate a mod.toml metadata template in a mod folder")]
    Init { folder: String },
    #[command(about = "Remove a mod")]
    Remove {
        ident: String,
        #[arg(long, help = "Skip confirmation")]
        yes: bool,
    },
    #[command(about = "Enable a mod")]
    Enable { ident: String },
    #[command(about = "Disable a mod")]
    Disable {
        ident: String,
        #[arg(long, help = "Skip confirmation")]
        yes: bool,
    },
    #[command(about = "Change load order")]
    Order {
        ident: String,
        #[arg(allow_negative_numbers = true)]
        new_order: i64,
    },
    #[command(about = "Rename a mod's display name (or its folder with --folder)")]
    Rename {
        ident: String,
        new_name: String,
        #[arg(long, help = "Rename the mod's folder on disk too")]
        folder: bool,
    },
    #[command(about = "Show detailed mod info")]
    Info {
        ident: String,
        #[arg(short, long, help = "Show the full output (lists, components, guides)")]
        verbose: bool,
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Open the mod folder (or its URL with --url)")]
    Open {
        ident: String,
        #[arg(long, help = "Open the mod's URL from mod.toml instead of its folder")]
        url: bool,
    },
    #[command(about = "Export the whole state to a JSON file (or stdout)")]
    Export {
        #[arg(value_name = "FILE", help = "Output file (defaults to stdout)")]
        path: Option<String>,
    },
    #[command(about = "Import a state from a JSON export (destructive)")]
    Import {
        #[arg(value_name = "FILE")]
        path: String,
        #[arg(long, help = "Skip confirmation")]
        force: bool,
    },
    #[command(about = "Check the health of the mod setup (folders, deps, manifests)")]
    Health {
        #[arg(long, help = "Also scan for file conflicts between enabled mods")]
        conflicts: bool,
    },
    #[command(about = "Report file conflicts between the enabled mods of the profile")]
    Conflicts {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Show which mods provide a game-relative path (and who wins)")]
    Which {
        #[arg(value_name = "PATH", help = "Game-relative path, e.g. models/x.dff")]
        path: String,
    },
    #[command(about = "Manage dependencies")]
    Dep {
        #[command(subcommand)]
        action: DepAction,
    },
    #[command(about = "Manage profiles")]
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },
    #[command(about = "Manage groups (global or per-profile collections of mods)")]
    Group {
        #[command(subcommand)]
        action: GroupAction,
    },
}

#[derive(Subcommand)]
pub enum ProfileAction {
    #[command(about = "List profiles")]
    List {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Create a new profile")]
    Create { name: String },
    #[command(about = "Delete a profile (keeps the last one)")]
    Delete {
        ident: String,
        #[arg(long, help = "Skip confirmation")]
        yes: bool,
    },
    #[command(about = "Set the active profile")]
    Use { ident: String },
    #[command(about = "Rename a profile (slug and directories stay unchanged)")]
    Rename { ident: String, new_name: String },
    #[command(about = "Copy a profile's mod states to a new profile")]
    Copy { source: String, new_name: String },
    #[command(about = "Diff the enabled/order state of two profiles")]
    Diff { a: String, b: String },
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

#[derive(Subcommand)]
pub enum GroupAction {
    #[command(about = "List groups")]
    List {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Create a group")]
    Create { name: String },
    #[command(about = "Rename a group (slug stays unchanged)")]
    Rename { ident: String, new_name: String },
    #[command(about = "Delete a group and its memberships")]
    Delete {
        ident: String,
        #[arg(long, help = "Skip confirmation")]
        yes: bool,
    },
    #[command(about = "Add a mod to a group (--global for all profiles)")]
    Add {
        mod_ident: String,
        group_ident: String,
        #[arg(long, help = "Apply the membership in every profile")]
        global: bool,
    },
    #[command(about = "Remove a mod from a group (--global for the global membership)")]
    Remove {
        mod_ident: String,
        group_ident: String,
        #[arg(long, help = "Remove the global membership instead of the profile one")]
        global: bool,
    },
    #[command(about = "Enable all mods of a group in the profile (with required deps)")]
    Enable { group_ident: String },
    #[command(about = "Disable all mods of a group in the profile")]
    Disable { group_ident: String },
}

fn main() {
    let cli = Cli::parse();

    let profile = cli.profile.clone();

    let command = cli.command.unwrap_or(Command::Launch(LaunchArgs {
        dry_run: false,
        debug: false,
        discover: false,
        clean: false,
        deps_enable: false,
        deps_ignore: false,
        no_auto_discover: false,
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
        Command::Launch(args) => cmd_launch(args, profile.as_deref()),
        Command::Steam(args) => cmd_steam(args, profile.as_deref()),
        Command::Ctl(args) => {
            let db_path = config::db_path();
            db::ensure_db_dir(&db_path).unwrap_or_else(|e| {
                log::die(format!("No se pudo acceder a la base de datos: {e}"));
            });
            let conn = db::open_db(&db_path).unwrap_or_else(|e| {
                log::die(format!("No se pudo abrir la base de datos: {e}"));
            });
            db::run_migrations(&conn).unwrap_or_else(|e| {
                log::die(format!("Error en migraciones: {e}"));
            });
            ctl::run(&conn, &args, profile.as_deref())
        }
    };

    if let Err(e) = result {
        log::error(format!("{e:#}"));
        std::process::exit(1);
    }
}

fn cmd_launch(args: LaunchArgs, profile: Option<&str>) -> anyhow::Result<()> {
    if !args.dry_run {
        println!("=== GTA SA Mod Organizer ===");
    }

    let opts = LaunchOptions {
        dry_run: args.dry_run,
        debug: args.debug,
        discover: args.discover,
        clean: args.clean,
        auto_discover: !args.no_auto_discover,
        profile: profile.map(String::from),
        deps_enable: args.deps_enable,
        deps_ignore: args.deps_ignore,
    };

    let result = LaunchEngine::run(&opts, |msg| println!("{msg}"))?;

    if !result.log.is_empty() {
        println!("{}", result.log);
    }

    Ok(())
}

fn cmd_steam(args: LaunchArgs, profile: Option<&str>) -> anyhow::Result<()> {
    if std::env::var_os("GTA_MO_STEAM_NS").is_some() {
        return cmd_launch(args, profile);
    }

    let unshare = std::env::var("GTA_MO_UNSHARE")
        .ok()
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
        .or_else(|| which::which("unshare").ok())
        .ok_or_else(|| anyhow::anyhow!("No se encontró 'unshare'. Defínelo con GTA_MO_UNSHARE."))?;

    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };

    let mut steam_args = Vec::<std::ffi::OsString>::new();
    if args.dry_run {
        steam_args.push("--dry-run".into());
    }
    if args.debug {
        steam_args.push("--debug".into());
    }
    if args.discover {
        steam_args.push("--discover".into());
    }
    if args.clean {
        steam_args.push("--clean".into());
    }
    if args.deps_enable {
        steam_args.push("--deps-enable".into());
    }
    if args.deps_ignore {
        steam_args.push("--deps-ignore".into());
    }
    if args.no_auto_discover {
        steam_args.push("--no-auto-discover".into());
    }
    if let Some(p) = profile {
        steam_args.push("--profile".into());
        steam_args.push(p.into());
    }

    let self_exe = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("No se pudo resolver el binario gta-mo: {e}"))?;

    let mut cmd = std::process::Command::new(&unshare);
    cmd.arg("-m")
        .arg("-U")
        .arg("--map-root-user")
        .arg(&self_exe)
        .arg("steam")
        .args(&steam_args)
        .env("GTA_MO_STEAM_NS", "1")
        .env_remove("LD_PRELOAD")
        .env_remove("LD_LIBRARY_PATH");
    if uid != 0 {
        cmd.env("GTA_MO_DROP_UID", uid.to_string())
            .env("GTA_MO_DROP_GID", gid.to_string());
    }

    log::info("Entrando en user/mount namespace (modo Steam)...");
    let status = cmd.status().map_err(|e| {
        anyhow::anyhow!(
            "No se pudo ejecutar unshare en '{}': {e}",
            unshare.display()
        )
    })?;

    if !status.success() {
        let code = status.code().unwrap_or(1);
        anyhow::bail!("gta-mo steam terminó con error: exit status: {code}");
    }
    Ok(())
}
