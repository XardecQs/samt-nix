# GTA Mod Organizer (SAMT)

**SAMT** — San Andreas Mod Tool for Linux/NixOS.

A Rust-based mod organizer for GTA San Andreas on Linux, using SQLite for mod tracking and fuse-overlayfs for runtime layering of mods without touching the original game files.

## Requirements

- [Nix](https://nixos.org/) (with flakes enabled)

## Installation

### Via Home Manager (recommended)

Add the flake to your Home Manager configuration:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    gta-mo.url = "github:XardecQs/samt-nix";
  };

  outputs = { nixpkgs, gta-mo, ... }: let
    system = "x86_64-linux";
    # The gta-mo module defaults to `pkgs.gta-mod-organizer`, which only exists
    # if the flake overlay is applied to the pkgs set it sees.
    pkgs = import nixpkgs {
      inherit system;
      overlays = [ gta-mo.overlays.default ];
    };
  in {
    homeConfigurations."tu-usuario" = nixpkgs.lib.homeManagerConfiguration {
      inherit pkgs;
      modules = [
        gta-mo.homeManagerModules.default
        {
          programs.gta-mo = {
            enable = true;
            # enableGui = true;   # also install the gta-mo-gui (egui) frontend
            settings = {
              game_root = "/home/user/Games/GTA_SA";
              proton_path = "/home/user/.steam/root/compatibilitytools.d/GE-Proton11-1";
              proton_use_wined3d = false;
              auto_discover = true;
            };
          };
        }
      ];
    };
  };
}
```

This generates `~/.config/gta-mo/config.toml`, installs the `gta-mo` binary, and sets up shell completions automatically.

### Via NixOS module

```nix
{
  inputs.gta-mo.url = "github:XardecQs/samt-nix";

  outputs = { nixpkgs, gta-mo, ... }: {
    nixosConfigurations.tu-host = nixpkgs.lib.nixosSystem {
      modules = [
        gta-mo.nixosModules.default
        ({ ... }: {
          nixpkgs.overlays = [ gta-mo.overlay ];
          programs.gta-mo.enable = true;
        })
      ];
    };
  };
}
```

This installs `gta-mo` system-wide and places a config template at `/etc/gta-mo/config.toml.example`. Add `programs.gta-mo.enableGui = true;` to also install the GUI.

### Development shell

```bash
nix develop
cargo build
```

### Manual

```bash
nix build
./result/bin/gta-mo --help
```

## Directory layout

The tool expects this structure under `game_root` (defined in `config.toml`):

```
game_root/
├── base/              # Clean, unmodded game files
├── mods/              # One subdirectory per mod
├── pfx/               # Wine prefix (auto-created by umu-launcher)
└── run/
    ├── merged/        # Overlay mount point (single)
    └── profiles/
        └── <slug>/    # Per-profile state (saves, configs, logs)
            ├── upper/ # Overlay upperdir (game writes land here)
            ├── work/  # Overlay workdir
            └── logs/  # DXVK/Proton logs (when --debug)
```

Each profile gets its own `upper/`, so savegames (e.g. with PortableGTA),
game-written configs and logs never mix between profiles.

> **Upgrading from before 0.36.0:** the old shared `run/upper/` is moved to
> `run/profiles/default/upper` on the first `gta-mo launch` (this is done at
> launch time because `ctl` works without a config file).

## Configuration

Configuration is read from (in priority order):

1. `$GTA_MO_CONFIG` environment variable
2. `./config.toml` (current directory)
3. `$XDG_CONFIG_HOME/gta-mo/config.toml`

A template with every option is provided as
[`config.toml.example`](config.toml.example) — copy it to `config.toml` and
edit the paths (the file `config.toml` is intentionally not versioned).

The database is stored at `$GTA_MO_DB` or `$XDG_DATA_HOME/gta-mo/organizer.db`.

Example `config.toml`:

```toml
game_root = "/home/user/Games/GTA_SA"
proton_path = "/home/user/.steam/root/compatibilitytools.d/GE-Proton11-1"
game_id = "umu-gtasa"
game_exe = "gta_sa.exe"
proton_use_wined3d = false
proton_disable_ntsync = false
auto_discover = true
# default_profile = "vanilla"   # used by `launch` when --profile is not given
# proton_disable_upscalers = false   # true = stop Proton (GE/CachyOS) from downloading/upgrading FSR/DLSS/XeSS/OptiScaler DLLs
# Optional: custom mods directory (defaults to game_root/mods)
# mods_dir = "/path/to/mods"
```

> **Proton upscalers**: GE and CachyOS Proton run `protonfixes.setup_upscalers()`
> on every launch, which downloads/installs upscaler DLLs (FSR3/FSR4/MLFG/DLSS/
> XeSS/OptiScaler). Set `proton_disable_upscalers = true` to export
> `PROTON_*_UPGRADE=0` and skip that. This works fully on GE-Proton; recent
> CachyOS builds may still fetch the FSR4 DLL on first use (their code enables
> it unconditionally), though the upgrade will not be applied to the game.

## Mod packaging (`mod.toml`)

Each mod folder may carry a `mod.toml` manifest with metadata and mount
control. A template is provided as [`mod.toml.example`](mod.toml.example) and
can be generated per-mod with `gta-mo ctl init <folder>`:

```toml
# GTA Mod Organizer manifest
id = "xardec:hdcars"              # opcional, ESTABLE (autor:slug)
name = "Nombre visible"           # si falta, se usa el nombre de la carpeta
version = "1.2.0"
author = ["Autor1", "Autor2"]     # string o lista
url = "https://github.com/..."
description = "Descripción larga..."
tags = ["essential", "bugfix"]    # opcional, para organizar/filtrar

cover = "cover.png"                # ruta relativa dentro del mod
guides = ["guides/instalacion.md"] # lista de archivos de guía

# Subdirectorios cuyo CONTENIDO se monta sobre la raíz del juego.
# Sin esta clave se monta la carpeta entera (comportamiento por defecto).
mount = ["content"]

# Dependencias (referencias por id autor:slug o por nombre de carpeta)
[dependencies]
required = ["xardec:asi-loader"]  # sin esto el mod no funciona
optional = []

# Si es un pack de mods, lista sus componentes (solo metadata)
[[components]]
name = "SilentPatch"
version = "1.0.1"
author = "Silent"
url = "http://mixmods.com.br/2015/03/SilentPatch.html"
path = "content/modloader/_ESSENTIALS/SilentPatch"
```

- All fields are optional. `author` accepts a single string or a list. The
  `id` is a **stable** `author:slug` identifier (lowercase) that survives
  folder/display renames and is used by `[dependencies]` (a folder name works
  as a legacy fallback reference).
- A manifest with `[[components]]` is treated as a **pack**: `ctl info` shows
  the component list and flags it (`pack: true` in `--json`). `guides` entries
  may point to a directory (`guides = ["guides"]`) — `ctl info` expands it to
  its files.
- Each `mount` entry is a folder treated as the **game root**: its CONTENTS
  are laid over the game. With `mount = ["content"]`, `content/d3d9.dll` lands
  on `<game root>/d3d9.dll` and `content/models/*` on `<game root>/models/*`.
  This is handy for mods with many loose files (e.g. Essentials_Pack): move
  them into a `content/` subfolder and list it. With no `mount`, the whole
  folder is treated as the game root, so legacy mods keep working untouched.
  Entries are relative paths (no `..`, no absolute paths).

**Recommended pack layout** (keeps the game dir clean):

```
Essentials_Pack/
├── mod.toml        # mount = ["content"], guides = ["guides"], cover, [[components]]
├── cover.png
├── content/        # only this is mounted onto the game
└── guides/         # readmes, changelogs, licenses
```

- The manifest is the canonical source of metadata: `ctl info`, `ctl list`
  (and their `--json` output) read `mod.toml` directly whenever the mods dir is
  known, so editing the file shows up immediately. The database caches the
  fields as a fallback for config-less `ctl` use. `discover` (on `launch`)
  imports `id`, `tags` and `[dependencies]` into the database, replacing the
  dependency rows of manifest mods. `ctl rename` writes the new name back into
  `mod.toml`, and `ctl dep add/remove` edits the `[dependencies]` section too.
  Covers and guides stay as files in the mod folder; rendering them in the GUI
  is planned for a future GUI rewrite.

## Groups

Groups are user-curated collections of mods (unlike tags, which are manifest
metadata). A membership (mod → group) can be:

- **Global** (`ctl group add <mod> <group> --global`): applies in every profile.
- **Per-profile** (`ctl group add <mod> <group>` without `--global`): only in
  the active profile (or `--profile`).

`ctl list --group <group>` shows the mods of that group in the current profile
(global memberships plus the profile's own). `ctl info` lists the groups a mod
belongs to. Deleting a group (`ctl group delete`) removes its memberships.
`ctl group enable <group>` activates every member of the group in the profile
(plus its required dependencies, transitively); `ctl group disable <group>`
deactivates just the members.

```bash
gta-mo ctl group create Graphics
gta-mo ctl group add d3d9 Graphics --global        # every profile
gta-mo ctl group add essentials-pack Graphics      # only the active profile
gta-mo ctl list --group Graphics                   # members of "Graphics" here
gta-mo ctl group enable Graphics                   # enable them all (+ deps)
gta-mo ctl group disable Graphics
```

## Health and conflicts

- `ctl health [--profile <p>]` checks the profile: mods whose folder is missing
  on disk, malformed `mod.toml` files, `mount` entries pointing to non-existent
  directories, and required dependencies that are disabled or unresolved.
- `ctl conflicts [--profile <p>] [--json]` scans the enabled mods of a profile
  (in overlay priority order) and reports **file conflicts**: the same
  game-relative path provided by more than one mod with different content. The
  first provider in priority order is the winner (the others are silently
  overridden by the overlay). Overlaps are classified by severity — high
  (executables), medium (data/textures) and info (paths under `modloader/`,
  where Mod Loader manages its own order) — and identical duplicates are
  reported separately (not as real conflicts). `ctl health --conflicts` runs
  both checks at once.
- `ctl which <game-relative-path>` answers "who provides this file and who
  wins" for a single path (base game, a single mod, or a conflict).

```bash
gta-mo ctl health                          # folder/manifest/dep problems
gta-mo ctl health --conflicts              # plus a conflict scan
gta-mo ctl conflicts                       # all file conflicts of the profile
gta-mo ctl which models/player.dff         # who wins that file
```

## CLI

```
gta-mo launch [--dry-run] [--debug] [--discover] [--clean] [--profile <name>] [--no-auto-discover]
gta-mo steam [launch flags...]        # same flags, but re-execs inside a user/mount namespace (for Steam)
gta-mo ctl list [-v] [--enabled|--disabled] [--json] [--profile <name>]
              [--tag <tag>] [--group <group>] [--author <author>] [--id <id>]
              [--search <text>] [--sort name|folder|author|order|mod_id|version|status] [--dir asc|desc]
gta-mo ctl add <folder> [--name <name>]
gta-mo ctl init <folder>                    # create folder + mod.toml template and register the mod
gta-mo ctl remove <id|folder>
gta-mo ctl enable <id|folder> [--profile <name>]
gta-mo ctl disable <id|folder> [--profile <name>]
gta-mo ctl order <id|folder> <n> [--profile <name>]
gta-mo ctl rename <id|folder> <name> [--folder]
gta-mo ctl info <id|folder> [-v] [--json] [--profile <name>]
gta-mo ctl open <id|folder> [--url]
gta-mo ctl dep add <mod> <dependency> [--optional]
gta-mo ctl dep remove <mod> <dependency>
gta-mo ctl profile list [--json]
gta-mo ctl profile create <name>
gta-mo ctl profile delete <name>
gta-mo ctl profile use <name>
gta-mo ctl profile rename <old> <new>
gta-mo ctl profile copy <source> <new-name>
gta-mo ctl profile diff <a> <b>
gta-mo ctl group list [--json]
gta-mo ctl group create <name>
gta-mo ctl group rename <id|slug|name> <new-name>
gta-mo ctl group delete <id|slug|name> [--yes]
gta-mo ctl group add <mod> <group> [--global]
gta-mo ctl group remove <mod> <group> [--global]
gta-mo ctl group enable <group> [--profile <name>]
gta-mo ctl group disable <group> [--profile <name>]
gta-mo ctl health [--conflicts]
gta-mo ctl conflicts [--json]
gta-mo ctl which <game-relative-path>
gta-mo ctl export [<file>]
gta-mo ctl import <file> [--force]
```

- `--profile <name>` (global flag, also `--profile <slug|id>`): selects which
  profile a command operates on. Defaults to the active profile.
- Profiles group the `enabled`/`load_order` state of mods. Mods themselves
  (folders, names, dependencies) are global. A `default` profile is created
  automatically on first run.
- `profile use` persists the active profile; `launch` uses it unless
  `--profile` or `default_profile` in the config says otherwise.
- `profile rename` only changes the display name — the directory slug (and
  therefore the saves/logs in `run/profiles/<slug>/`) stays unchanged.
- `profile copy` duplicates the mod states of another profile.
- `--json` prints structured output (list, info, profile list) to stdout,
  useful for the GUI and scripting.
- `rename --folder`: also renames the mod's directory on disk (and keeps the
  database consistent), instead of only the display name. Dependencies are
  stored by id, so they survive the rename.
- `dep add --optional`: marks a dependency as optional/recommended. A missing
  or disabled optional dependency only prints a warning at launch; it does
  not block the game nor get forced on.
- `list` filters (combinable with AND): `--tag`, `--group` (global memberships
  plus the profile's own), `--author`, `--id` (stable `author:slug` or folder),
  and `--search` (case-insensitive over name, folder, authors, id, description
  and tags). `--sort` orders by a field (`--dir asc|desc`); without it the list
  keeps the load-order priority.
- `info` shows a compact summary by default; `-v` prints the full output
  (components, guides, dependents, profiles as tables). `open` opens the mod
  folder (or its `--url`) with `xdg-open`.
- `health` checks folders, manifests, mount entries and dependencies of the
  profile; `--conflicts` also scans for file conflicts. `conflicts` reports
  paths provided by more than one enabled mod (the first provider in priority
  wins), grouping by severity and ignoring identical duplicates.
- `export`/`import` dump and restore the whole state (mods, metadata, profiles,
  dependencies, groups) as JSON. Import replaces the database (asks for
  confirmation unless `--force`).

Shell completions are installed automatically when using the Home Manager module. To generate them manually:

```bash
gta-mo completions bash > gta-mo.bash
gta-mo completions zsh  > _gta-mo
gta-mo completions fish > gta-mo.fish
```

## Steam integration

`gta-mo` must run **natively on the host**: it mounts `fuse-overlayfs` and then
spawns its own container via `umu-launcher`. Do not force a Steam Play /
Steam Linux Runtime compatibility tool on the entry, or Steam will execute
`gta-mo` inside `pressure-vessel`, where neither the binary nor its
dependencies exist (`Failed to execute child process ?gta-mo?`).

The built-in `gta-mo steam` subcommand does the namespace dance that the old
shell wrapper did (the legacy wrapper lives in
[`bash-legacy/gta-mo-steam.sh`](bash-legacy/gta-mo-steam.sh)).

To integrate with Steam:

1. In Steam: **Add a Non-Steam Game**.
2. Set **Target** to the absolute path of the `gta-mo` binary.
3. Set **Start In** to an existing directory (e.g. `$HOME`).
4. In **Properties → Compatibility**, select **"Do not use a compatibility tool"**.
5. In **Launch Options** put `steam` plus any extra flags
   (`steam --debug --profile vanilla`, ...).

`gta-mo steam` re-execs itself inside a fresh user/mount namespace
(`unshare -m -U --map-root-user`). It resolves `unshare` from `$GTA_MO_UNSHARE`
or `PATH`.

Why the namespace is required: on NixOS the Steam client runs inside a
bubblewrap sandbox with its **own user namespace**, where uid 0 is not
mapped. The kernel therefore ignores the setuid bit of `fusermount3`, and
`fuse-overlayfs` fails to mount with `Operation not permitted`. Inside a
fresh namespace where the user is root, `fuse-overlayfs` mounts directly
(uid 0 in that namespace is the same host user, so file ownership is
unchanged). The subcommand also clears `LD_PRELOAD`/`LD_LIBRARY_PATH`, which
Steam populates with `gameoverlayrenderer.so`.

`umu-launcher` refuses to run as root, so `gta-mo steam` passes
`GTA_MO_DROP_UID`/`GTA_MO_DROP_GID` (the real user). When running with
euid 0, `gta-mo` forks a child that enters a nested user namespace mapping
that uid and drops privileges before launching `umu-run`, keeping the
overlay mount from the parent.

Note: the game process is started by `umu-launcher`, not by Steam, so the
Steam overlay will not attach to the game window.

## Options

| Flag | Description |
|---|---|
| `--dry-run` | Print the overlay layer stack without launching |
| `--debug` | Enable Proton/DXVK debug logging |
| `--discover` | Scan `mods/` for new mods and exit |
| `--clean` | Remove orphaned mod entries from the database |
| `--profile` | Launch with a specific profile (name, slug or id) |
| `--deps-enable` | Auto-enable disabled dependencies without prompting |
| `--deps-ignore` | Skip disabled dependencies without prompting |
| `--no-auto-discover` | Do not auto-scan `mods/`, even if `auto_discover` is set |

## GUI

There is a **Rust (egui/eframe)** graphical frontend (`gta-mo-gui`) that uses
the **hybrid** architecture: it reads directly from the SQLite database via
`gta-mo-core` for instant listing, and spawns `gta-mo ctl ...` for every
mutation and `gta-mo launch` (streaming its output to the Log tab).

```
nix build .#gta-mo-gui
./result/bin/gta-mo-gui
```

The binary finds `gta-mo` via `GTA_MO_BIN`, `PATH`, or the dev
`target/debug/gta-mo`. The Nix package is wrapped so it always finds the
installed `gta-mo`.

Features (milestone 1): mods list with search/tag/group filters and sorting,
enable/disable and load-order controls, a rich detail panel (author, Mod ID,
URL, tags, groups, mount, description, cover, components), profiles tab with
create/use/rename/copy/delete, a launch button (`--deps-enable`), a streaming
Log tab, and a status bar (active profile, enabled count, conflict count).

To develop it: `nix develop`, then `cargo run -p gta-mo-gui`.

## How it works

1. Mods live as subdirectories under `mods/`
2. `schema.sql` defines the SQLite database that tracks mods, their
   dependencies, and per-profile enabled/load-order state
3. The binary builds a `fuse-overlayfs` layer stack from the enabled mods of
   the active profile (writing through to that profile's `upper/`) and runs
   the game with `umu-launcher` (Proton)

## Legacy version

The original bash implementation is preserved in [`bash-legacy/`](bash-legacy/).

## License

GPL-3.0-or-later — see [`LICENSE`](LICENSE).
