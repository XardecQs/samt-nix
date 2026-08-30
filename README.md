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

  outputs = { nixpkgs, gta-mo, ... }: {
    homeConfigurations."tu-usuario" = nixpkgs.lib.homeManagerConfiguration {
      modules = [
        gta-mo.homeManagerModules.default
        {
          programs.gta-mo = {
            enable = true;
            # enableGui = true;   # also install the gta-mo-gui (Fyne) frontend
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
# Optional: custom mods directory (defaults to game_root/mods)
# mods_dir = "/path/to/mods"
```

## CLI

```
gta-mo launch [--dry-run] [--debug] [--discover] [--clean] [--profile <name>] [--no-auto-discover]
gta-mo steam [launch flags...]        # same flags, but re-execs inside a user/mount namespace (for Steam)
gta-mo ctl list [-v] [--enabled|--disabled] [--json] [--profile <name>]
gta-mo ctl add <folder> [--name <name>]
gta-mo ctl remove <id|folder>
gta-mo ctl enable <id|folder> [--profile <name>]
gta-mo ctl disable <id|folder> [--profile <name>]
gta-mo ctl order <id|folder> <n> [--profile <name>]
gta-mo ctl rename <id|folder> <name> [--folder]
gta-mo ctl info <id|folder> [--json] [--profile <name>]
gta-mo ctl dep add <mod> <dependency> [--optional]
gta-mo ctl dep rm <mod> <dependency>
gta-mo ctl profile list [--json]
gta-mo ctl profile create <name>
gta-mo ctl profile delete <name>
gta-mo ctl profile use <name>
gta-mo ctl profile rename <old> <new>
gta-mo ctl profile copy <source> <new-name>
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

There is a lightweight Fyne (Go) frontend that drives the CLI — it never
touches the database or the overlay directly, it only spawns `gta-mo` and
parses the `--json` output.

```
nix build .#gta-mo-gui
./result/bin/gta-mo-gui
```

The binary finds `gta-mo` via `GTA_MO_BIN`, `PATH`, or the dev
`target/debug/gta-mo`. The Nix package is wrapped so it always finds the
installed `gta-mo`.

Features: list of mods with enable/disable toggles, move up/down load order,
profile selector plus create/use/rename/copy/delete, and a launch button
(uses `--deps-enable`, so missing dependencies are auto-enabled instead of
prompting).

To develop it: `nix develop` (the shell includes Go and the Fyne
dependencies), then `cd gui && go run .`.

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
