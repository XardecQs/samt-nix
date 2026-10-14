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

## Game window icon on GNOME

GNOME does not implement the `xdg_toplevel_icon_v1` Wayland protocol, so a
Proton/umu game window shows a generic icon unless a desktop entry with a
matching `StartupWMClass` exists (the same trick used for Minecraft). Both
modules offer a `gameDesktopEntry` option that installs such an entry; you
provide the icon (the game's own icon is not redistributed).

Find the real window class while the game is running:

```bash
xprop WM_CLASS      # click the game window (XWayland)
wmctrl -lx          # or list all windows with their class
```

It is usually `gta_sa.exe` (defaults to your configured `game_exe`). Example:

```nix
# Home Manager
programs.gta-mo.gameDesktopEntry = {
  enable = true;
  icon = "/home/user/icons/gtasa.png";  # path or theme icon name
  # wmClass = "gta_sa.exe";             # override if xprop shows another class
};

# NixOS
programs.gta-mo.gameDesktopEntry.enable = true;
programs.gta-mo.gameDesktopEntry.icon = "/home/user/icons/gtasa.png";
```

The entry (`NoDisplay=true` by default) uses `Exec = gta-mo launch`, so clicking
it also starts the game.

## Directory layout

The tool expects this structure under `game_root` (defined in `config.toml`):

```
game_root/
├── base/              # Clean, unmodded game files
├── mods/              # One subdirectory per mod
├── pfx/               # Wine prefix (auto-created by umu-launcher)
└── run/
    ├── merged/        # Overlay mount point (single)
    ├── mount.json     # State of a manual `gta-mo mount` (daemon pid, mount id)
    └── profiles/
        ├── <slug>/    # Per-profile state (saves, configs, logs)
        │   ├── upper/ # Overlay upperdir (game writes land here)
        │   ├── work/  # Overlay workdir
        │   └── logs/  # per-launch launch-<ts>.log + DXVK/Proton logs
        └── tmp-*/     # Throwaway upper of a `gta-mo mount --temp`
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
game = "gta_sa"
game_id = "umu-gtasa"
game_exe = "gta_sa.exe"
proton_use_wined3d = false
proton_disable_ntsync = false
auto_discover = true
# default_profile = "vanilla"   # used by `launch` when --profile is not given
# proton_disable_upscalers = false   # true = stop Proton (GE/CachyOS) from downloading/upgrading FSR/DLSS/XeSS/OptiScaler DLLs
# umu_runtime_update = false   # true = let umu-launcher update its steamrt4 runtime on launch
# Optional: custom mods directory (defaults to game_root/mods)
# mods_dir = "/path/to/mods"
# Page-cache preloading before launch (default): "off", "prefix" or "full"
# preload = "prefix"
# Per-phase launch timings (default on)
# timings = true
# Write Proton's verbose steam-<appid>.log every launch (default off; on with --debug)
# proton_log = false
# Run the game against a tmpfs copy of the Wine prefix (default on)
# prefix_ram = true
```

> **Startup / preloading**: on slow storage the game spends a long time reading
> the Wine prefix and the game's startup files on demand. `gta-mo` warms them in
> two steps: a **critical** one (the Wine prefix, the game executable and the mod
> loaders) runs synchronously at normal I/O priority *before* `umu`, so Wine
> never hits the loader stall; the **extra** startup files (the mods' startup
> configs/scripts —including those under `modloader/`, but not its on-demand
> assets like `.dff/.txd`) are warmed in the background and stop once the game
> process appears. `preload = "full"` also reads the whole base game and every enabled
> mod file (can be tens of GB); it prints a detailed warning and asks for
> confirmation on an interactive terminal, and is better used ahead of time with
> `gta-mo preload`. `preload = "off"` disables it.
>
> **Prefix in RAM (`prefix_ram`, default on)**: a long game session evicts the
> Wine prefix from the page cache, which makes the *next* launch slow. To avoid
> it, `gta-mo` `rsync`s the prefix to a tmpfs copy (`/dev/shm/gta-mo/pfx`) and
> runs the game against it; after the session it syncs back to disk (the source
> of truth) and **keeps** the copy, so the following launches reuse it without
> copying. The copy is only made once per login/reboot (or when the disk prefix
> changes); inspect it with `gta-mo prefix status` and free it with
> `gta-mo prefix clean`. `gta-mo preload` prepares the copy ahead of time (the GUI
> does it on startup and after each game), and the next `launch` reuses it. It is
> user-space only (no `mlock`/system changes) and falls back to the disk prefix
> if `/dev/shm` is unavailable. Disable with `--no-prefix-ram` or
> `prefix_ram = false`.
>
> When `prefix_ram` is on, `prefix_ram_proton` (default off) can also
> **materialize** into the tmpfs prefix the file symlinks that point into the
> Proton install
> (`drive_c/windows/{system32,syswow64}` → `GE-Proton/files/lib/wine/*`), so Wine
> reads its system DLLs from RAM instead of disk. It is off by default because,
> with the prefix already on tmpfs, it only duplicates hundreds of MB.
> `preload_proton` and
> `preload_steamrt` (both **off** by default) additionally prefetch the Proton
> runtime and umu's `steamrt4` into the page cache in the background. The RAM
> used by the tmpfs prefix is reported in the launch log and by
> `gta-mo prefix status`.
>
> `preload_umu` (on by default) warms umu's Python environment (interpreter,
> stdlib and site-packages) during the prewarm (`gta-mo preload`, which the GUI
> runs on startup and after each game), so the **next launch** does not read it
> cold. That cold read is the `⏱ umu responde` gap (measured ~20–47 s cold vs
> ~0.3 s warm). Only takes effect while `preload` is not `"off"`.
>
> `timings = true` (default) prints per-phase timings (`config/db`, `plan`,
> `mount`, `prefix-ram`, `preload`, `sesión`, `prefix-sync`) and, while the game
> boots, the time until `umu` produces its first output (`⏱ umu responde`), the
> time to the game process (`⏱ proceso del juego`) and the total from the `umu`
> spawn to the process (`⏱ arranque total`). The GUI's Log tab follows the newest
> lines unless you scroll up (there is an **Ir al final** button), and the status
> bar shows the running/queued jobs with a popover to cancel them.


> **Proton upscalers**: GE and CachyOS Proton run `protonfixes.setup_upscalers()`
> on every launch, which downloads/installs upscaler DLLs (FSR3/FSR4/MLFG/DLSS/
> XeSS/OptiScaler). Set `proton_disable_upscalers = true` to export
> `PROTON_*_UPGRADE=0` and skip that. This works fully on GE-Proton; recent
> CachyOS builds may still fetch the FSR4 DLL on first use (their code enables
> it unconditionally), though the upgrade will not be applied to the game.

> **umu runtime (steamrt4)**: `umu-launcher` re-checks its steamrt runtime on
> every launch and, if the upstream archive digest does not match (a corrupted
> cache or a bad mirror), aborts the download with `Digest mismatched` while the
> game still starts with the installed runtime. gta-mo sets
> `UMU_RUNTIME_UPDATE=0` by default (`umu_runtime_update = false`) so an
> already-installed runtime is reused as-is. Inspect it with
> `gta-mo ctl runtime status`, and use `ctl runtime clean` (downloads cache) or
> `ctl runtime reinstall` (force umu to re-download on the next launch) to repair
> a broken install. Set `umu_runtime_update = true` to re-enable automatic
> updates.

> **Logs**: every launch writes a `launch-<ts>.log` under
> `run/profiles/<slug>/logs/` (high-level launcher messages plus the
> `umu`/Proton output, teed from the game process). The header records the
> `gta-mo` version, the local start time and an environment block (OS/kernel,
> CPU, RAM, GPU), followed by the profile, paths, an effective `config:` line and
> the full mod list (`[✔]` enabled, `[✘]` disabled/hidden). Every `umu`/Proton
> line is prefixed with `[HH:MM:SS.mmm +X.XXs]` (wall clock + time since the umu
> spawn) and the watcher reports the umu startup phases (`bwrap`, `python`,
> `wine`, `proton`) and the total boot. `PROTON_LOG_DIR` and `DXVK_LOG_PATH` point
> there too, and `UMU_LOG` keeps umu's output (`debug` with `--debug`). Proton's
> verbose `steam-<appid>.log` is only written with `--debug` (or
> `proton_log = true`). Standalone preload runs (`gta-mo preload`, which the GUI
> uses as its *prewarm*) write their own `preload-<ts>.log` in the same folder.
> The newest 20 `launch-*.log` and 20 `preload-*.log` per profile are kept. Pass
> `--redact` (or `GTA_MO_REDACT=1`) to mask `/home/<user>` as `/home/****` so the
> log is safe to share. The GUI's Log tab and ⋮ menu have an **Abrir carpeta de
> logs** action.

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
license = "MIT"                   # licencia/términos del autor (opcional)
tags = ["essential", "bugfix"]    # opcional, para organizar/filtrar

cover = "cover.png"                # ruta relativa dentro del mod
guides = ["guides/instalacion.md"] # lista de archivos de guía
screenshots = ["screenshots"]      # carpeta (se expande) o lista de imágenes

# Subdirectorios cuyo CONTENIDO se monta sobre la raíz del juego.
# Sin esta clave se monta la carpeta entera (comportamiento por defecto).
mount = ["content"]

# Dependencias (referencias por id autor:slug o por nombre de carpeta)
[dependencies]
required = ["xardec:asi-loader"]  # sin esto el mod no funciona
optional = []

# Miembro de una familia de alternativas mutuamente excluyentes (Reload Mod /
# ReloadSA, Project2DFX / Distant Car Upgrade…): solo un miembro del mismo `id`
# puede estar activo a la vez.
[family]
id = "reload"
name = "Reload Mod"

# Mods incompatibles (por id o carpeta); no se pueden activar juntos.
conflicts = ["mixmods:proper-fixes"]

# Al menos UNO de estos debe estar activo (alternativas, p. ej. familias o
# "Proper Shaders" o "SkyGfx"). No auto-activa ni afecta al orden: se valida.
requires_any = ["mixmods:proper-shaders", "aap:sa-skygfx-extended"]

# Opciones seleccionables por perfil (una entrada con variantes/configs). El
# valor elegido monta options/<grupo>/<valor>/ ENCIMA de mount. Sin [[options]],
# los grupos se auto-descubren de options/<grupo>/<valor>/.
[[options]]
group = "variant"                 # slug [a-z0-9_-]
name = "Variante"                 # etiqueta visible (opcional)
default = "4k"                    # opcional; sin default = solo mount base

  [[options.choices]]
  key = "4k"
  name = "4K"

  [[options.choices]]
  key = "lite"
  conflicts = ["mixmods:proper-fixes-vanilla"]   # restricción de la opción activa
  requires_any = ["link2012:open-limit-adjuster"]

# Prioridad en Mod Loader para las carpetas que este mod aporta a modloader/.
# El launcher la escribe en el modloader.ini del perfil al lanzar.
[modloader]
priority = 60                     # 1..100 (mayor gana); sin esto no se toca
# folders = ["Proper Fixes"]      # opcional; se autodetecta modloader/<X>

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
- `screenshots` accepts a directory or a list of paths; directories are
  expanded to their images (png/jpg/jpeg/webp/bmp/gif/avif), shown as a gallery in
  the GUI detail panel (and listed by `ctl info -v`/`--json`).
- `cover`, `guides`, `screenshots` and `components[].path` are read from inside
  the mod folder only: they must be **relative** paths and cannot contain `..`
  (absolute paths and escapes are ignored with a warning). This prevents a
  manifest from reading files outside its mod.
- Each `mount` entry is a folder treated as the **game root**: its CONTENTS
  are laid over the game. With `mount = ["content"]`, `content/d3d9.dll` lands
  on `<game root>/d3d9.dll` and `content/models/*` on `<game root>/models/*`.
  This is handy for mods with many loose files (e.g. Essentials_Pack): move
  them into a `content/` subfolder and list it. With no `mount`, the whole
  folder is treated as the game root, so legacy mods keep working untouched.
  Entries are relative paths (no `..`, no absolute paths) and must not contain
  `,` or `:` (those characters separate `fuse-overlayfs` mount options/layers).
  Mod folder names must also avoid `,`/`:`; such folders are skipped by
  `discover` with a warning.

### Families, conflicts and Mod Loader

Some mods are **alternatives within a family** (Reload Mod / Reload Mod (Fixed) /
ReloadSA, Project2DFX / Distant Car Upgrade). Model them as separate folders that
share a `[family] id`: enabling one member automatically disables the other
members of the family (from `ctl enable`, which surfaces a toast in the GUI).
Per-member requirements/conflicts are declared in each folder's own
`[dependencies]` and `conflicts`. The detail panel shows the family and its
siblings (with **Ver** links to navigate between them), it does not switch them.

- `conflicts = [...]` lists incompatible mods (by `author:slug` or folder).
  Enabling one of a conflicting pair is refused, and `ctl health`/`launch`
  report the pair. A real launch **aborts** on a family/conflict violation
  (`--dry-run` prints them without launching).
- `requires_any = [...]` lists alternatives of which **at least one** must be
  enabled. Unlike `[dependencies].required` (an edge that auto-enables and
  affects the layer order), this is a validation: it never enables anything,
  `ctl enable` only warns, and `health`/`launch` report it.
- Mods under `modloader/` are ordered by Mod Loader itself, not by the overlay:
  set `[modloader] priority` (1..100, higher wins) and the launcher writes the
  `[Profiles.<P>.Priority]` entries into
  `run/profiles/<slug>/upper/modloader/modloader.ini` on every launch,
  preserving the rest of the file. Folder names are auto-detected from the
  mod's `modloader/<Name>` tree. Inspect them with `ctl modloader show`.
  Example: `Proper Fixes=60` must outrank `RoSA Project Evolved=50`.

### Options per profile

A mod can expose selectable **options**, so what used to be several "variant"
mods becomes **one entry with options**. Each choice is a folder
`options/<group>/<value>/` whose CONTENTS are overlaid on the game, on top of the
mod's `mount` (so it overrides the base). The selection is stored **per profile**
(schema v11), so the same mod can be a different variant in each profile.

```bash
gta-mo ctl option list <mod> [--json]        # groups and selected value
gta-mo ctl option set <mod> <group> <value>  # select a value
gta-mo ctl option unset <mod> <group>        # clear (base mount only)
```

- Without `[[options]]`, groups/values are **auto-discovered** from the
  `options/<group>/<value>/` folders. The optional `[[options]]` block adds
  labels, a `default` and per-choice constraints.
- Each choice may declare its own `conflicts`/`requires_any`, evaluated **only
  when that choice is selected**. This is what lets a single `proper-fixes` mod
  cover the RoSA and vanilla families.
- The launcher, `ctl conflicts`, `ctl which` and `ctl health` all account for the
  selected options, and the GUI's mod detail shows them as selectors.

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
  fields as a fallback for config-less `ctl` use. `ctl discover` (also run
  automatically on `launch` when `auto_discover` is on) imports `id`, `tags`
  and `[dependencies]` into the database, replacing the dependency rows of
  manifest mods. `ctl rename` writes the new name back into `mod.toml`, and
  `ctl dep add/remove` edits the `[dependencies]` section too. Covers and
  screenshots stay as files in the mod folder; the GUI renders them in the
  detail panel.

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
gta-mo launch [--dry-run] [--debug] [--discover] [--clean] [--profile <name>] [--no-auto-discover] [--preload <off|prefix|full>] [--preload-yes] [--prefix-ram|--no-prefix-ram]
gta-mo preload [--full] [--yes]       # warm the page cache without launching
gta-mo steam [launch flags...]        # same flags, but re-execs inside a user/mount namespace (for Steam)
gta-mo mount [--temp] [--dry-run] [--profile <name>]   # mount the overlay without launching the game
gta-mo unmount [--capture <mod>] [--discard]           # stop the mount daemon (capture/discard a temp upper)
gta-mo status [--json]                                 # show the current manual mount, if any
gta-mo prefix status                                   # show the prepared tmpfs Wine prefix copy
gta-mo prefix clean [--yes]                            # remove the tmpfs prefix copy
gta-mo ctl list [-v] [--enabled|--disabled] [--json] [--profile <name>]
              [--tag <tag>] [--group <group>] [--author <author>] [--id <id>]
              [--search <text>] [--sort name|folder|author|order|mod_id|version|status] [--dir asc|desc]
              [--hidden|--only-hidden]
gta-mo ctl add <folder> [--name <name>]
gta-mo ctl init <folder> [--skeleton]       # create folder + mod.toml (and content/guides/screenshots) and register the mod
gta-mo ctl remove <id|folder>
gta-mo ctl enable <id|folder> [--profile <name>]
gta-mo ctl disable <id|folder> [--profile <name>]
gta-mo ctl order <id|folder> <n> [--profile <name>]
gta-mo ctl reorder <folder>... [--profile <name>]
gta-mo ctl rename <id|folder> <name> [--folder]
gta-mo ctl info <id|folder> [-v] [--json] [--profile <name>]
gta-mo ctl open <id|folder> [--url]
gta-mo ctl discover                          # scan mods/ and refresh metadata + [dependencies]
gta-mo ctl clean                             # remove orphaned mod entries from the database
gta-mo ctl dep add <mod> <dependency> [--optional]
gta-mo ctl dep remove <mod> <dependency>
gta-mo ctl tag set <mod> [<tag>...]           # replace tags (no tags = clear)
gta-mo ctl tag add <mod> <tag>...
gta-mo ctl tag remove <mod> <tag>...
gta-mo ctl hide <id|folder> [--global]        # hide from the list (per profile, or all with --global)
gta-mo ctl unhide <id|folder> [--global]      # show it again
gta-mo ctl note <id|folder> [<text>...]       # show/set per-profile notes (--clear to remove)
gta-mo ctl manifest set <mod>                 # replace mod.toml with stdin content
gta-mo ctl data list [--json]                 # profile user data (saves/tracks/screenshots)
gta-mo ctl data dir                           # open the profile user-data folder
gta-mo ctl data open <rel>
gta-mo ctl data remove <rel>... [--yes]       # delete one or more user-data files
gta-mo ctl upper list [--include-userfiles] [--orphans] [--json]  # overwrite layer files
gta-mo ctl upper move <mod> <rel>...          # move upper files into a mod's mounted layer
gta-mo ctl upper discard <rel>... [--yes]     # delete upper files (lower layers show through)
gta-mo ctl upper capture <mod> [--from <dir>] # turn the upper into a new mod
gta-mo ctl open-url <url>                     # open an http(s) URL
gta-mo ctl open-path <path>                   # open a local file/folder
gta-mo ctl modloader show [--json]            # Mod Loader priorities for the profile
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
gta-mo ctl option list <id|folder> [--json] [--profile <name>]
gta-mo ctl option set <id|folder> <group> <value> [--profile <name>]
gta-mo ctl option unset <id|folder> <group> [--profile <name>]
gta-mo ctl health [--conflicts]
gta-mo ctl conflicts [--json]
gta-mo ctl which <game-relative-path>
gta-mo ctl runtime status [--json]             # umu/steamrt runtime + cache state
gta-mo ctl runtime clean                        # delete umu's partial-download cache
gta-mo ctl runtime reinstall [--yes]            # remove steamrt4 so umu re-downloads it
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
  keeps the load-order priority. Hidden mods are omitted unless `--hidden` (show
  them too) or `--only-hidden` is given.
- `hide`/`unhide` mark a mod as hidden so it disappears from the list without
  affecting the launch. Without `--global` the flag is per profile; `--global`
  hides it everywhere (a later per-profile `unhide` still overrides it).
- `note` stores free-form text per mod and per profile (e.g. "causa CTD en la
  zona X"); `ctl info`/`--json` and the GUI show it.
- The overlay order respects the configured priority: a dependency is only
  placed below the mod that requires it, never hoisted above unrelated
  higher-priority mods. This is the order `launch` mounts and the order the GUI
  shows as the effective list.
- `info` shows a compact summary by default; `-v` prints the full output
  (components, guides, dependents, profiles as tables). `open` opens the mod
  folder (or its `--url`) with `xdg-open`.
- `health` checks folders, manifests, mount entries and dependencies of the
  profile; `--conflicts` also scans for file conflicts. `conflicts` reports
  paths provided by more than one enabled mod (the first provider in priority
  wins), grouping by severity and ignoring identical duplicates.
- `mount`/`unmount`/`status` manage the overlay **without launching the game**:
  `mount` starts a detached daemon that keeps `run/merged` mounted (and holds
  the launcher lock, so `launch` is refused until you `unmount`). With `--temp`
  the writes go to a throwaway profile (for installers like MTA) that
  `unmount --capture <mod>` turns into a mod, or `unmount --discard` deletes.
- `ctl upper` browses the profile's overlay upper (the "overwrite" layer): files
  written through `run/merged` that no enabled mod provides are flagged
  `huérfano`. `upper move` puts files into a mod's mounted layer (respecting
  its `mount = [...]`), `upper discard` deletes them, and `upper capture` packs
  the whole upper into `content/` + `mod.toml`. `move`, `discard` and `data
  remove` accept several paths at once (best-effort, with a per-file error
  summary). Overlayfs whiteout markers are reported but cannot be replayed from
  a mod (a lower layer cannot delete base-game files).
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

The built-in `gta-mo steam` subcommand performs the user/mount namespace
dance required by Steam.

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
| `--redact` | Mask `/home/<user>` as `/home/****` in the launch log (also `GTA_MO_REDACT=1`) |
| `--deps-enable` | Auto-enable disabled dependencies without prompting |
| `--deps-ignore` | Skip disabled dependencies without prompting |
| `--no-auto-discover` | Do not auto-scan `mods/`, even if `auto_discover` is set |
| `--preload <off\|prefix\|full>` | Page-cache preloading before launch (overrides the config) |
| `--preload-yes` | Skip the `full` preload confirmation prompt |
| `--prefix-ram` / `--no-prefix-ram` | Run the Wine prefix from tmpfs / from disk (overrides the config) |
| `--prefix-ram-proton` / `--no-prefix-ram-proton` | Materialize the Proton DLL symlinks into the tmpfs prefix |
| `--preload-proton` / `--no-preload-proton` | Prefetch the Proton runtime into the page cache |
| `--preload-steamrt` / `--no-preload-steamrt` | Prefetch umu's steamrt runtime into the page cache |
| `--preload-umu` / `--no-preload-umu` | Warm umu's Python environment during the prewarm |

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

Features: mods list with search/tag/group/enabled-status filters and sorting,
enable/disable and drag&drop load-order controls (with an insertion bar), a rich
detail panel (author, Mod ID, URL, tags, groups, mount, description, cover,
screenshot gallery, components, a one-click **Crear mod.toml** for mods
without a manifest, a **tag editor** and a raw **mod.toml editor**, per-profile
**notes** and **hide** toggles (per profile and global, with a "Mostrar ocultos"
filter)), a **family** badge and declared **conflicts** in the detail panel,
profiles tab with create/use/rename/copy/delete
(renaming also moves `run/profiles/<slug>`), a **Datos** tab listing the active
profile's saves, settings, User Tracks and screenshots written under
`userfiles/` (with PortableGTA) plus a recommendation to install PortableGTA,
an **Overwrite** tab that browses the profile's `upper/` (the overwrite layer)
and flags files no mod provides as *huérfanos*, with Mount/Unmount controls
(including a temporary profile and its capture/discard), groups tab, a
**Dependencies** tab
that flags required dependencies that are disabled/missing, a Conflicts tab
(background scan), a launch button (`--deps-enable`, plus `--debug`/dry-run) that
becomes a **Detener** button while the game runs, Discover/Clean actions, a
streaming Log tab, and a status bar.

### Dependency graph

The **Dependencies** tab has a **Lista / Grafo** toggle. The graph view (built
with [`egui_graphs`](https://crates.io/crates/egui_graphs)) renders the mods as
nodes (green = enabled, gray = disabled, dark = hidden) and the relations as
edges: required dependencies, optional ones, conflicts, `requires_any`
alternatives and `[family]` families. Edges can be filtered with the
*Opcionales*, *Conflictos* and *Familias* toggles. It supports free zoom/pan
(the mouse wheel zooms towards the cursor and drag pans, even outside the
viewport) plus **＋/－/⤢ Ajustar** buttons, node dragging, a **Fuerza**
layout (it settles and pauses on its own) and a **Reorganizar** button. A single
click selects a node (shown
in an info line) and a double click opens its detail; labels have a background
and stay at a constant screen size, and edges/arrows do not grow with the zoom.
Positions are kept when only the enabled state changes. All of this (plus the
advanced force parameters) is configurable in
**Preferencias → Grafo** and persisted in `gui.toml`.

**Datos** and **Overwrite** share one explorer widget: a folder-tree sidebar and
a clickable breadcrumb, a list or thumbnail-grid content area (with an
adjustable thumbnail size), search, kind filter, an **images only** quick filter
and a hidden-files toggle, sorting by name/size/date/path, multi-selection
(Ctrl/Shift toggles and ranges, `Ctrl+A`, invert) with bulk move/delete,
keyboard navigation (arrows, Enter, Supr, `Ctrl+F`), folder and selection size
totals, and thumbnails that open in the full-screen image viewer. Images are
decoded on a background pool, downscaled and cached (in memory and on disk under
`$XDG_CACHE_HOME/gta-mo/thumbs`, invalidated by mtime), so opening a folder with
many images never blocks the UI. The view/sort/thumbnail size are remembered per
browser.

### Appearance

The GUI follows the system **light/dark** theme by default and can be forced in
**Preferencias…** (⋮ menu), together with:

- **Accent color** (8 presets — blue, purple, magenta, red, amber, yellow,
  green, gray — plus a custom color picker), high-contrast mode,
  UI density, interface scale, "reduce motion" and whether to show covers.
- A **Grafo** section configures the dependency graph: which edge kinds to show
  and "only enabled", edge width/arrow
  size and arrows on/off (kept at a constant screen size across zoom), node
  radius, label font/background, highlight the selected node, labels/dragging/
  selection, zoom speed and fit padding, "settle and pause" for the force
  layout, and advanced **Fuerza** parameters (center gravity,
  attraction, repulsion, damping, spacing…).
- A semantic color palette (`crates/gui/src/theme.rs`) whose text/background
  pairs are verified to meet **WCAG AA** contrast in both themes by unit tests.

Preferences are stored in `~/.config/gta-mo/gui.toml` (only the changed fields
are written; older files keep working). The interface **text uses the system UI
font** when it can be resolved through fontconfig (`fc-match`) or the GNOME
settings, falling back to the bundled egui fonts; set `GTA_MO_NO_SYSTEM_FONT=1`
to disable it. The ⋮ menu also has **Acerca de** and **Atajos de teclado**, and
the keyboard shortcuts include `Ctrl+F` (search), `Ctrl+R` (refresh) and
`Ctrl+,` (preferences).

### Adaptive layout

The interface reflows by window width (breakpoint 760 px):

- **Wide**: left navigation sidebar, with a single ⋮ menu for preferences/about/
  theme.
- **Narrow (vertical/phone)**: a **gradual bottom bar** that keeps Mods,
  Perfiles and Log always visible and shows Grupos/Dependencias/Conflictos while
  their measured width fits (labels are dropped at the smallest sizes); a "más"
  menu holds the rest (actions, Debug/dry-run, Preferencias, Acerca de, atajos y
  tema). The header keeps only the profile selector and the launch button, and
  the Mods tab puts the search on its own line with a compact **Filtros** menu
  plus "Nuevo mod".

The mod detail always opens as a **full-screen overlay page** (8 px margins) with
a pinned header and a back button (libadwaita `NavigationSplitView` style), so
the back button is always visible. The dialogs (new/rename/copy, tag editor,
`mod.toml` editor, delete confirmation, Preferences, About, Keyboard shortcuts)
use the same full-screen overlay style, so they always appear above the detail
page and are never floating windows.

Each control appears in exactly one place per layout (no duplicated
preferences/theme menus).

In the detail panel, clicking the cover or any screenshot opens a **full-screen
image viewer** with previous/next arrows, a counter and page dots (also `←`/`→`
and `Esc`, or click the backdrop). The viewer and its controls scale to fit the
window on both axes, and the image animates from its thumbnail (shared-element
"hero" transition). Animated **GIF** files play in the viewer (via the
`egui_extras` gif loader); `bmp`, `gif` and `avif` also get static thumbnails.
AVIF decoding uses the `avif-native` `image` feature (`libdav1d`).

### Motion

Animations are physics-based (springs) and gated by **"Reducir movimiento"**:
spring-driven hover/toggle states, staggered list rows, the cover hero
transition, overlay reveal and a subtle counter pulse. There is no background
blur (egui cannot do it without a custom shader).

### Icons

The UI icons are a subset of the bundled [Lucide](https://lucide.dev) font
(`crates/gui/assets/lucide/`). To add icons: list their canonical names in
`assets/lucide/icons.txt`, then regenerate the subset (the committed `.ttf` is
what the build embeds, so Nix never needs the tool):

```bash
nix shell nixpkgs#python3Packages.fonttools -c \
  bash crates/gui/assets/lucide/build.sh
```

Then add the codepoints from `assets/lucide/codepoints.json` to
`crates/gui/src/icons.rs`.

The package installs a desktop entry and the app icon (a scalable SVG plus PNGs
at the standard sizes) from `data/gta-mo-gui.desktop` and
`data/icons/hicolor/`; with `enableGui` it appears in your application menu.

To develop it: `nix develop`, then `cargo run -p gta-mo-gui`.

## How it works

1. Mods live as subdirectories under `mods/`
2. `schema.sql` defines the SQLite database that tracks mods, their
   dependencies, and per-profile enabled/load-order state
3. The binary builds a `fuse-overlayfs` layer stack from the enabled mods of
   the active profile (writing through to that profile's `upper/`) and runs
   the game with `umu-launcher` (Proton)

## Overlay without launching the game

`gta-mo mount` builds the same layer stack as `launch` but stops after mounting
it at `run/merged`, leaving it available to any program. A detached daemon keeps
it mounted (and holds the launcher lock, so a concurrent `launch` is refused)
until `gta-mo unmount`. `gta-mo status` reports the current mount. The GUI's
**Overwrite** tab offers the same controls.

This is useful to run an installer by hand (e.g. an MTA `.exe` under the same
`WINEPREFIX`) and inspect what it changed.

**Temporary profiles.** `gta-mo mount --temp` mounts the active profile's mods
but writes to a throwaway `run/profiles/tmp-*/upper`. After the installer ran:

- `gta-mo unmount --capture <mod>` packs the upper into `mods/<mod>/content/`
  (plus a minimal `mod.toml` with `mount = ["content"]`) and registers it.
- `gta-mo unmount --discard` throws the upper away.
- `gta-mo unmount` keeps it, so it can be captured later with
  `gta-mo ctl upper capture <mod> --from <dir>`.

The capture skips `userfiles/` (PortableGTA saves/settings/screenshots) and the
launcher-generated `modloader/modloader.ini`. Overlayfs whiteouts (deletions)
are **not** captured: a lower-layer mod cannot remove base-game files, so they
are reported and omitted.

## Profile user data

Per-profile savegames, settings, User Tracks and screenshots need the game to
write *inside* its directory (so the overlay routes those writes to the
profile's `upper/`). The recommended way is **[PortableGTA](https://www.mixmods.com.br/2021/06/iii-vc-sa-portablegta-change-saves-folder-mudar-pasta-user-files/)**
(MIT, by GTA modding): install it as a mod (its `portablegta.asi` inside
`modloader/`) and enable it. Each profile then keeps its own
`userfiles/` (saves, `gta_sa.set`, `User Tracks/`, `Gallery/`), browsable and
manageable from the GUI's **Datos** tab or with `gta-mo ctl data ...`.

The launcher only *recommends* PortableGTA (a third-party mod, MIT-licensed); it
does not redistribute its binary.

## License

GPL-3.0-or-later — see [`LICENSE`](LICENSE).

The bundled GUI icon font is a subset of the [Lucide](https://lucide.dev) font,
licensed under ISC — see `crates/gui/assets/lucide/LICENSE`.
