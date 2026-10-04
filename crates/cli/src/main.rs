mod ctl;

use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::generate;
use clap_complete::shells::{Bash, Fish, Zsh};
use gta_mo_core::config;
use gta_mo_core::db;
use gta_mo_core::launcher::{LaunchEngine, LaunchOptions};
use gta_mo_core::log;
use std::io::{IsTerminal, Write};

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
    Completions { shell: String },
    #[command(about = "Mount the overlay and launch the game")]
    Launch(LaunchArgs),
    #[command(about = "Warm the page cache for the active profile without launching")]
    Preload(PreloadArgs),
    #[command(about = "Launch from Steam (re-execs inside a user/mount namespace)")]
    Steam(LaunchArgs),
    #[command(about = "Mount the overlay without launching the game (kept alive by a daemon)")]
    Mount(MountArgs),
    #[command(about = "Unmount a manual overlay, optionally capturing/discarding its upper")]
    Unmount(UnmountArgs),
    #[command(about = "Show the current manual overlay mount, if any")]
    Status {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Manage the tmpfs Wine prefix copy (prefix_ram)")]
    Prefix(PrefixArgs),
    #[command(about = "Manage mods, profiles, groups and dependencies")]
    Ctl(CtlArgs),
}

#[derive(Args)]
struct MountArgs {
    #[arg(
        long,
        help = "Ephemeral profile with a throwaway upper (for installers like MTA)"
    )]
    temp: bool,

    #[arg(long, help = "Print the layer stack and exit without mounting")]
    dry_run: bool,

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
        help = "Do not auto-discover mods/ before mounting, even if auto_discover is set"
    )]
    no_auto_discover: bool,
}

#[derive(Args)]
struct UnmountArgs {
    #[arg(
        long,
        value_name = "MOD",
        help = "Capture the mounted upper into a new mod (mods/<MOD>)"
    )]
    capture: Option<String>,

    #[arg(long, help = "Discard a temporary profile's upper")]
    discard: bool,
}

#[derive(Args)]
struct PrefixArgs {
    #[command(subcommand)]
    command: PrefixCommand,
}

#[derive(Subcommand)]
enum PrefixCommand {
    #[command(about = "Show whether a tmpfs Wine prefix copy is prepared")]
    Status,
    #[command(about = "Remove the tmpfs Wine prefix copy (the disk prefix is untouched)")]
    Clean {
        #[arg(long, help = "Do not ask for confirmation")]
        yes: bool,
    },
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

    #[arg(
        long,
        value_name = "MODE",
        help = "Page-cache preloading before launch: off, prefix or full"
    )]
    preload: Option<String>,

    #[arg(long, help = "Skip the 'full' preload confirmation prompt")]
    preload_yes: bool,

    #[command(flatten)]
    optimize: OptimizeArgs,
}

#[derive(Args)]
struct PreloadArgs {
    #[arg(
        long,
        help = "Preload everything (whole base game + every enabled mod), not just the startup files"
    )]
    full: bool,

    #[arg(long, help = "Skip the 'full' preload confirmation prompt")]
    yes: bool,

    #[command(flatten)]
    optimize: OptimizeArgs,
}

/// RAM/optimization toggles shared by `launch` and `preload`.
#[derive(Args, Clone, Default)]
struct OptimizeArgs {
    #[arg(
        long,
        help = "Run the game against a tmpfs copy of the Wine prefix (RAM)"
    )]
    prefix_ram: bool,

    #[arg(long = "no-prefix-ram", help = "Use the on-disk Wine prefix")]
    no_prefix_ram: bool,

    #[arg(
        long,
        help = "Materialize the Proton DLL symlinks into the tmpfs prefix"
    )]
    prefix_ram_proton: bool,

    #[arg(
        long = "no-prefix-ram-proton",
        help = "Do not materialize Proton symlinks into the tmpfs prefix"
    )]
    no_prefix_ram_proton: bool,

    #[arg(long, help = "Prefetch the Proton runtime into the page cache")]
    preload_proton: bool,

    #[arg(
        long = "no-preload-proton",
        help = "Do not prefetch the Proton runtime"
    )]
    no_preload_proton: bool,

    #[arg(long, help = "Prefetch umu's steamrt runtime into the page cache")]
    preload_steamrt: bool,

    #[arg(
        long = "no-preload-steamrt",
        help = "Do not prefetch umu's steamrt runtime"
    )]
    no_preload_steamrt: bool,
}

impl OptimizeArgs {
    fn prefix_ram(&self) -> Option<bool> {
        tri(self.prefix_ram, self.no_prefix_ram)
    }
    fn prefix_ram_proton(&self) -> Option<bool> {
        tri(self.prefix_ram_proton, self.no_prefix_ram_proton)
    }
    fn preload_proton(&self) -> Option<bool> {
        tri(self.preload_proton, self.no_preload_proton)
    }
    fn preload_steamrt(&self) -> Option<bool> {
        tri(self.preload_steamrt, self.no_preload_steamrt)
    }
}

/// `Some(true)` / `Some(false)` / `None` from an `--x` / `--no-x` flag pair.
fn tri(enable: bool, disable: bool) -> Option<bool> {
    if enable {
        Some(true)
    } else if disable {
        Some(false)
    } else {
        None
    }
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

        #[arg(long = "hidden", group = "hidden_filter", help = "Include hidden mods")]
        hidden: bool,

        #[arg(
            long = "only-hidden",
            group = "hidden_filter",
            help = "Show only hidden mods"
        )]
        only_hidden: bool,

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
    Init {
        folder: String,
        #[arg(
            long,
            help = "Also create the content/, guides/ and screenshots/ directory skeleton"
        )]
        skeleton: bool,
    },
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
    #[command(about = "Rewrite the whole load order (top priority first)")]
    Reorder {
        #[arg(value_name = "FOLDER", num_args = 1..)]
        mods: Vec<String>,
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
    #[command(about = "Scan mods/ and refresh metadata/dependencies (no launch required)")]
    Discover,
    #[command(about = "Remove orphaned mod entries from the database")]
    Clean,
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
    #[command(about = "Edit a mod's tags in its mod.toml")]
    Tag {
        #[command(subcommand)]
        action: TagAction,
    },
    #[command(about = "Hide a mod from the list (per profile; --global for all profiles)")]
    Hide {
        ident: String,
        #[arg(long, help = "Hide it globally instead of only in the active profile")]
        global: bool,
    },
    #[command(about = "Unhide a mod")]
    Unhide {
        ident: String,
        #[arg(long, help = "Clear the global flag instead of the profile override")]
        global: bool,
    },
    #[command(about = "Show or edit a mod's notes for the active profile")]
    Note {
        ident: String,
        #[arg(
            value_name = "TEXT",
            num_args = 0..,
            help = "Note text (omit to print the current note)"
        )]
        text: Vec<String>,
        #[arg(long, help = "Clear the note")]
        clear: bool,
    },
    #[command(about = "Edit a mod's mod.toml manifest")]
    Manifest {
        #[command(subcommand)]
        action: ManifestAction,
    },
    #[command(about = "Inspect the Mod Loader priorities for a profile")]
    Modloader {
        #[command(subcommand)]
        action: ModloaderAction,
    },
    #[command(about = "View/manage the active profile's user data (saves, tracks, screenshots)")]
    Data {
        #[command(subcommand)]
        action: DataAction,
    },
    #[command(about = "Manage the profile's overlay upper (overwrite layer)")]
    Upper {
        #[command(subcommand)]
        action: UpperAction,
    },
    #[command(about = "Open an http(s) URL with xdg-open")]
    OpenUrl { url: String },
    #[command(about = "Open a local file or folder with xdg-open")]
    OpenPath { path: String },
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
    #[command(about = "Select a mod's options in the active profile")]
    Option {
        #[command(subcommand)]
        action: OptionAction,
    },
    #[command(about = "Inspect or repair the umu-launcher/steamrt runtime")]
    Runtime {
        #[command(subcommand)]
        action: RuntimeAction,
    },
}

#[derive(Subcommand)]
pub enum RuntimeAction {
    #[command(about = "Show the installed umu runtime and cache state")]
    Status {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Delete umu's partial-download cache")]
    Clean,
    #[command(about = "Remove the installed steamrt runtime so umu reinstalls it")]
    Reinstall {
        #[arg(long, help = "Skip confirmation")]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum OptionAction {
    #[command(about = "List a mod's option groups and the selected value")]
    List {
        ident: String,
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Select an option value for a mod in the profile")]
    Set {
        ident: String,
        group: String,
        value: String,
    },
    #[command(about = "Clear a selected option group for a mod in the profile")]
    Unset { ident: String, group: String },
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
pub enum TagAction {
    #[command(about = "Replace all tags")]
    Set {
        ident: String,
        #[arg(value_name = "TAG", num_args = 0..)]
        tags: Vec<String>,
    },
    #[command(about = "Add tags (keeping the existing ones)")]
    Add {
        ident: String,
        #[arg(value_name = "TAG", num_args = 1..)]
        tags: Vec<String>,
    },
    #[command(about = "Remove tags")]
    Remove {
        ident: String,
        #[arg(value_name = "TAG", num_args = 1..)]
        tags: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum ManifestAction {
    #[command(about = "Replace the mod.toml with the manifest read from stdin")]
    Set { ident: String },
}

#[derive(Subcommand)]
pub enum ModloaderAction {
    #[command(about = "Show the effective Mod Loader priorities for a profile")]
    Show {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
}

#[derive(Subcommand)]
pub enum DataAction {
    #[command(about = "List the profile's user-data files")]
    List {
        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Open the profile's user-data folder")]
    Dir,
    #[command(about = "Open a user-data file/folder")]
    Open {
        #[arg(value_name = "PATH", help = "Path relative to the user-data folder")]
        path: String,
    },
    #[command(about = "Delete one or more user-data files")]
    Remove {
        #[arg(
            value_name = "PATH",
            num_args = 1..,
            help = "Path(s) relative to the user-data folder"
        )]
        paths: Vec<String>,
        #[arg(long, help = "Skip confirmation")]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub enum UpperAction {
    #[command(about = "List the files written to the profile's upper")]
    List {
        #[arg(long, help = "Include the PortableGTA userfiles/ subtree")]
        include_userfiles: bool,

        #[arg(long, help = "Show only files no enabled mod provides")]
        orphans: bool,

        #[arg(long, help = "Output as JSON")]
        json: bool,
    },
    #[command(about = "Move one or more upper files into a mod (their mounted layer)")]
    Move {
        #[arg(value_name = "MOD", help = "Target mod (id, folder or name)")]
        mod_ident: String,
        #[arg(
            value_name = "PATH",
            num_args = 1..,
            help = "Path(s) relative to the upper"
        )]
        paths: Vec<String>,
    },
    #[command(about = "Delete upper files so the lower layers show through again")]
    Discard {
        #[arg(
            value_name = "PATH",
            num_args = 1..,
            help = "Path(s) relative to the upper"
        )]
        paths: Vec<String>,
        #[arg(long, help = "Skip confirmation")]
        yes: bool,
    },
    #[command(about = "Capture the whole upper into a new mod")]
    Capture {
        #[arg(value_name = "MOD", help = "Name of the mod to create")]
        name: String,
        #[arg(
            long,
            value_name = "DIR",
            help = "Upper directory to capture (defaults to the active profile's)"
        )]
        from: Option<String>,
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
        preload: None,
        preload_yes: false,
        optimize: OptimizeArgs::default(),
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
        Command::Preload(args) => cmd_preload(args, profile.as_deref()),
        Command::Steam(args) => cmd_steam(args, profile.as_deref()),
        Command::Mount(args) => cmd_mount(args, profile.as_deref()),
        Command::Unmount(args) => cmd_unmount(args),
        Command::Status { json } => cmd_status(json),
        Command::Prefix(args) => cmd_prefix(args),
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
        preload: args
            .preload
            .as_deref()
            .map(|v| gta_mo_core::preload::PreloadMode::parse(Some(v))),
        preload_yes: args.preload_yes,
        prefix_ram: args.optimize.prefix_ram(),
        prefix_ram_proton: args.optimize.prefix_ram_proton(),
        preload_proton: args.optimize.preload_proton(),
        preload_steamrt: args.optimize.preload_steamrt(),
    };

    let result = LaunchEngine::run(&opts, |msg| println!("{msg}"))?;

    if !result.log.is_empty() {
        println!("{}", result.log);
    }

    Ok(())
}

fn cmd_preload(args: PreloadArgs, profile: Option<&str>) -> anyhow::Result<()> {
    println!("=== GTA SA Mod Organizer — precarga ===");
    let opts = LaunchOptions {
        dry_run: false,
        debug: false,
        discover: false,
        clean: false,
        auto_discover: true,
        profile: profile.map(String::from),
        deps_enable: false,
        deps_ignore: false,
        preload: args.full.then_some(gta_mo_core::preload::PreloadMode::Full),
        preload_yes: args.yes,
        prefix_ram: args.optimize.prefix_ram(),
        prefix_ram_proton: args.optimize.prefix_ram_proton(),
        preload_proton: args.optimize.preload_proton(),
        preload_steamrt: args.optimize.preload_steamrt(),
    };
    LaunchEngine::preload(&opts, |msg| println!("{msg}"))?;
    Ok(())
}

fn cmd_mount(args: MountArgs, profile: Option<&str>) -> anyhow::Result<()> {
    let opts = LaunchOptions {
        dry_run: args.dry_run,
        debug: false,
        discover: false,
        clean: false,
        auto_discover: !args.no_auto_discover,
        profile: profile.map(String::from),
        deps_enable: args.deps_enable,
        deps_ignore: args.deps_ignore,
        preload: None,
        preload_yes: false,
        prefix_ram: None,
        prefix_ram_proton: None,
        preload_proton: None,
        preload_steamrt: None,
    };
    let outcome = LaunchEngine::mount(&opts, args.temp)?;
    if let Some(report) = outcome.dry_run {
        println!("{report}");
        return Ok(());
    }
    if let Some(info) = outcome.info {
        let kind = if info.temp { "temporal" } else { "perfil" };
        println!(
            "Overlay montado en '{}' ({kind} '{}').",
            info.merged.display(),
            info.profile_name
        );
        println!("upper:  {}", info.upper.display());
        if info.temp {
            println!(
                "Captura el upper con `gta-mo unmount --capture <mod>` o descártalo con \
                 `gta-mo unmount --discard`."
            );
        } else {
            println!("Desmonta con `gta-mo unmount`.");
        }
    }
    Ok(())
}

fn cmd_unmount(args: UnmountArgs) -> anyhow::Result<()> {
    let outcome = LaunchEngine::unmount(args.capture.as_deref(), args.discard)?;
    println!(
        "Overlay desmontado (perfil '{}').",
        outcome.info.profile_name
    );

    if let Some(report) = &outcome.capture {
        println!("Mod capturado: {} archivo(s).", report.moved);
        if report.deleted_skipped > 0 {
            println!(
                "[!] {} borrado(s) del upper no se pueden reproducir desde un mod (se omitieron).",
                report.deleted_skipped
            );
        }
        if report.userdata_skipped > 0 {
            println!(
                "  {} archivo(s) de userfiles omitidos.",
                report.userdata_skipped
            );
        }
        if report.generated_skipped > 0 {
            println!(
                "  {} archivo(s) generados omitidos.",
                report.generated_skipped
            );
        }
        if !outcome.registered {
            println!("[!] No se pudo registrar el mod; ejecuta `gta-mo ctl discover`.");
        }
    }

    if outcome.discarded {
        println!("Perfil temporal descartado.");
    } else if outcome.info.temp {
        let dir = outcome.info.upper.parent().unwrap_or(&outcome.info.upper);
        println!(
            "Perfil temporal conservado en '{}' (captura o descarta más tarde).",
            dir.display()
        );
    }
    Ok(())
}

fn cmd_status(json: bool) -> anyhow::Result<()> {
    let info = LaunchEngine::status()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&info)?);
        return Ok(());
    }
    match info {
        Some(i) => {
            let kind = if i.temp { "temporal" } else { "perfil" };
            println!(
                "Overlay montado: {} ({kind} '{}', pid {}).",
                i.merged.display(),
                i.profile_name,
                i.pid
            );
            println!("upper:  {}", i.upper.display());
        }
        None => println!("No hay ningún overlay montado manualmente."),
    }
    Ok(())
}

fn cmd_prefix(args: PrefixArgs) -> anyhow::Result<()> {
    match args.command {
        PrefixCommand::Status => {
            match gta_mo_core::prefix_ram::status() {
                Some(p) => {
                    let bytes = gta_mo_core::prefix_ram::prepared_bytes();
                    println!(
                        "Prefijo en tmpfs: {} ({:.1} MiB).",
                        p.display(),
                        bytes as f64 / 1_048_576.0
                    );
                }
                None => println!("No hay copia del prefijo en tmpfs."),
            }
            Ok(())
        }
        PrefixCommand::Clean { yes } => {
            // Refuse to run while a launch/mount holds the launcher lock.
            let _lock = LaunchEngine::acquire_lock(&config::lockfile_path()).map_err(|e| {
                anyhow::anyhow!("No se puede limpiar con una instancia en ejecución: {e}")
            })?;
            if !yes && !confirm("¿Eliminar la copia del prefijo en tmpfs? [s/N]: ") {
                println!("Cancelado.");
                return Ok(());
            }
            let report = gta_mo_core::prefix_ram::clean();
            if report.existed {
                println!(
                    "Copia tmpfs eliminada ({:.1} MiB liberados).",
                    report.bytes as f64 / 1_048_576.0
                );
            } else {
                println!("No había copia en tmpfs.");
            }
            Ok(())
        }
    }
}

/// Short interactive confirmation (empty/`n`/headless means no).
fn confirm(prompt: &str) -> bool {
    if !std::io::stdin().is_terminal() {
        return false;
    }
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    matches!(
        input.trim().to_ascii_lowercase().as_str(),
        "s" | "si" | "sí" | "y" | "yes"
    )
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
    if let Some(p) = &args.preload {
        steam_args.push("--preload".into());
        steam_args.push(p.into());
    }
    if args.preload_yes {
        steam_args.push("--preload-yes".into());
    }
    for (on, off, flag) in [
        (
            args.optimize.prefix_ram,
            args.optimize.no_prefix_ram,
            "--prefix-ram",
        ),
        (
            args.optimize.prefix_ram_proton,
            args.optimize.no_prefix_ram_proton,
            "--prefix-ram-proton",
        ),
        (
            args.optimize.preload_proton,
            args.optimize.no_preload_proton,
            "--preload-proton",
        ),
        (
            args.optimize.preload_steamrt,
            args.optimize.no_preload_steamrt,
            "--preload-steamrt",
        ),
    ] {
        if on {
            steam_args.push(flag.into());
        } else if off {
            steam_args.push(format!("--no-{}", &flag[2..]).into());
        }
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
