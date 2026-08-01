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

This installs `gta-mo` system-wide and places a config template at `/etc/gta-mo/config.toml.example`.

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
├── base/          # Clean, unmodded game files
├── mods/          # One subdirectory per mod
├── pfx/           # Wine prefix (auto-created by umu-launcher)
└── run/           # Overlay runtime (upper/, work/, merged/)
```

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
# Optional: custom mods directory (defaults to game_root/mods)
# mods_dir = "/path/to/mods"
```

## CLI

```
gta-mo launch [--dry-run] [--debug] [--discover] [--clean]
gta-mo ctl list [-v] [--enabled|--disabled]
gta-mo ctl add <folder> [--name <name>] [--order <n>]
gta-mo ctl remove <id|folder>
gta-mo ctl enable <id|folder>
gta-mo ctl disable <id|folder>
gta-mo ctl order <id|folder> <n>
gta-mo ctl rename <id|folder> <name>
gta-mo ctl info <id|folder>
gta-mo ctl dep add <mod> <dependency>
gta-mo ctl dep rm <mod> <dependency>
gta-mo ctl tui
```

Shell completions are installed automatically when using the Home Manager module. To generate them manually:

```bash
gta-mo completions bash > gta-mo.bash
gta-mo completions zsh  > _gta-mo
gta-mo completions fish > gta-mo.fish
```

## Options

| Flag | Description |
|---|---|
| `--dry-run` | Print the overlay layer stack without launching |
| `--debug` | Enable Proton/DXVK debug logging |
| `--discover` | Scan `mods/` for new mods and exit |
| `--clean` | Remove orphaned mod entries from the database |

## How it works

1. Mods live as subdirectories under `mods/`
2. `schema.sql` defines the SQLite database that tracks mods, their load order, and dependencies
3. The binary builds a `fuse-overlayfs` layer stack from enabled mods and runs the game with `umu-launcher` (Proton)

## Legacy version

The original bash implementation is preserved in [`bash-legacy/`](bash-legacy/).
