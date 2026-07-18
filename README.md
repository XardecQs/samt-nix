# samt-nix

**SAMT** — San Andreas Mod Tool for Linux/NixOS.

> [!WARNING]
> **Alpha stage.** This project is under active development. APIs, config formats, and behavior may change without notice. Use at your own risk.

A bash-based mod organizer for GTA San Andreas on Linux, using SQLite for mod tracking and fuse-overlayfs for runtime layering of mods without touching the original game files.

## Requirements

- [Nix](https://nixos.org/) (with flakes enabled) or the dependencies installed manually:
  - `fuse-overlayfs`
  - `umu-launcher`
  - `sqlite3`
  - `yj` + `jq`
  - `bash` 4+

## Quick start

```bash
# Enter the development shell
nix develop

# Discover mods placed in the mods/ directory
./launcher.sh --discover

# Dry-run to preview the layer stack
./launcher.sh --dry-run

# Launch the game
./launcher.sh
```

## How it works

1. Mods live as subdirectories under `mods/`
2. `schema.sql` defines the SQLite database that tracks mods, their load order, and dependencies
3. `launcher.sh` builds a `fuse-overlayfs` layer stack from enabled mods and runs the game with `umu-launcher` (Proton)

## Options

| Flag | Description |
|---|---|
| `--dry-run` | Print the overlay layer stack without launching |
| `--debug` | Enable Proton/DXVK debug logging |
| `--discover` | Scan `mods/` for new mods and exit |
| `--clean` | Remove orphaned mod entries from the database |
