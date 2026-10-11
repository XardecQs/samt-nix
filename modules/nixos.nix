{ config, lib, pkgs, ... }:

let
  cfg = config.programs.gta-mo;

  exampleConfig = ''
    # GTA Mod Organizer — Example configuration
    #
    # Copy this file to ~/.config/gta-mo/config.toml and edit it.
    # For declarative management, use the Home Manager module instead.

    game_root = "/home/user/Games/GTA_SA"
    proton_path = "/home/user/.steam/root/compatibilitytools.d/GE-Proton11-1"

    # Optional settings (defaults shown):
    #
    # game_id = "umu-gtasa"
    # game_exe = "gta_sa.exe"
    # proton_use_wined3d = true
    # proton_disable_ntsync = false
    # umu_runtime_update = false
    # auto_discover = false
    # dxvk_hud = "devinfo,fps,frametimes,submissions,compiler,version,api,pipelines,memory,gpuload,drawcalls"
    # mods_dir = "/home/user/path/to/custom/mods"

    # Directory layout expected under game_root:
    #   game_root/
    #   ├── base/     — Clean, unmodded game files
    #   ├── mods/     — One subdirectory per mod
    #   ├── pfx/      — Wine prefix (auto-created by umu-launcher)
    #   └── run/
    #       ├── merged/             — Overlay mount point
    #       └── profiles/<slug>/    — Per-profile upper/, work/, logs/
  '';

  gameDesktopEntry = pkgs.writeTextFile {
    name = "gta-sa.desktop";
    destination = "/share/applications/gta-sa.desktop";
    text = ''
      [Desktop Entry]
      Type=Application
      Name=${cfg.gameDesktopEntry.name}
      Comment=GTA San Andreas (via gta-mo)
      Exec=${cfg.gameDesktopEntry.exec}
      Terminal=false
      Categories=Game;
      StartupWMClass=${cfg.gameDesktopEntry.wmClass}
    '' + lib.optionalString (cfg.gameDesktopEntry.icon != "") ''
      Icon=${cfg.gameDesktopEntry.icon}
    '' + lib.optionalString cfg.gameDesktopEntry.noDisplay ''
      NoDisplay=true
    '';
  };
in {
  options.programs.gta-mo = {
    enable = lib.mkEnableOption "GTA Mod Organizer — install gta-mo system-wide";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.gta-mod-organizer;
      defaultText = lib.literalExpression "pkgs.gta-mod-organizer";
      description = ''
        The gta-mod-organizer CLI package. Only installed when
        `enableGui = false`; with the GUI enabled, `guiPackage` (which also
        provides the `gta-mo` binary) is installed for both.
      '';
    };

    enableGui = lib.mkEnableOption "the gta-mo-gui (egui) frontend";

    guiPackage = lib.mkOption {
      type = lib.types.package;
      default = pkgs.gta-mo-gui;
      defaultText = lib.literalExpression "pkgs.gta-mo-gui";
      description = ''
        The gta-mo-gui (egui frontend) package. It also provides the `gta-mo`
        CLI binary, so a single package serves both when the GUI is enabled.
      '';
    };

    gameDesktopEntry = lib.mkOption {
      default = { };
      description = ''
        Install a `.desktop` entry for the **game window**. GNOME does not
        implement `xdg_toplevel_icon_v1`, so a Proton/umu game window only shows
        an icon if a desktop entry with a matching `StartupWMClass` exists. The
        icon file is provided by the user (the game's icon is not redistributed).
        Find the real window class with `xprop WM_CLASS` (XWayland) or
        `wmctrl -lx` while the game is running.
      '';
      type = lib.types.submodule {
        options = {
          enable = lib.mkEnableOption "a desktop entry for the game window";

          name = lib.mkOption {
            type = lib.types.str;
            default = "GTA San Andreas";
            description = "Visible name of the desktop entry.";
          };

          exec = lib.mkOption {
            type = lib.types.str;
            default = "${if cfg.enableGui then cfg.guiPackage else cfg.package}/bin/gta-mo launch";
            defaultText = lib.literalExpression ''
              "''${if config.programs.gta-mo.enableGui
                  then config.programs.gta-mo.guiPackage
                  else config.programs.gta-mo.package}/bin/gta-mo launch"
            '';
            description = "Command run when the entry is clicked.";
          };

          icon = lib.mkOption {
            type = lib.types.str;
            default = "";
            description = ''
              Icon: an absolute path (e.g. a file in your Nix store) or a theme
              icon name. Leave empty to omit the `Icon=` line.
            '';
            example = "/home/user/icons/gtasa.png";
          };

          wmClass = lib.mkOption {
            type = lib.types.str;
            default = "gta_sa.exe";
            description = "`StartupWMClass` of the game window.";
          };

          noDisplay = lib.mkOption {
            type = lib.types.bool;
            default = true;
            description = "Hide the entry from the application grid (`NoDisplay=true`).";
          };
        };
      };
    };
  };

  config = lib.mkIf cfg.enable {
    # With the GUI enabled, `guiPackage` already provides the `gta-mo` CLI too,
    # so installing `package` as well would collide (both ship
    # `bin/.gta-mo-wrapped`) and recompile the shared crates.
    environment.systemPackages =
      (if cfg.enableGui then [ cfg.guiPackage ] else [ cfg.package ])
      ++ lib.optional cfg.gameDesktopEntry.enable gameDesktopEntry;

    environment.etc."gta-mo/config.toml.example" = {
      text = exampleConfig;
      mode = "0644";
    };
  };
}
