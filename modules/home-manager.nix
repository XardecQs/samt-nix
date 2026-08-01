{ config, lib, pkgs, ... }:

let
  cfg = config.programs.gta-mo;

  settingsFormat = pkgs.formats.toml { };

  settingsType = lib.types.submodule {
    freeformType = settingsFormat.type;
    options = {
      game_root = lib.mkOption {
        type = lib.types.str;
        description = "Root directory for the game files.";
        example = "/home/user/Games/GTA_SA";
      };
      proton_path = lib.mkOption {
        type = lib.types.str;
        description = "Path to the Proton compatibility tool directory.";
        example = "/home/user/.steam/root/compatibilitytools.d/GE-Proton11-1";
      };
    };
  };
in {
  options.programs.gta-mo = {
    enable = lib.mkEnableOption "GTA Mod Organizer — manage GTA SA mods with overlayfs";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.gta-mod-organizer;
      defaultText = lib.literalExpression "pkgs.gta-mod-organizer";
      description = "The gta-mod-organizer CLI package to use.";
    };

    gui = {
      enable = lib.mkEnableOption "GTA Mod Organizer GTK4 graphical interface";
      package = lib.mkOption {
        type = lib.types.nullOr lib.types.package;
        default = null;
        description = "The gta-mod-organizer GUI package. Set this to use the GUI (e.g. inputs.gta-mo.packages.\${pkgs.system}.gta-mod-organizer-gui).";
      };
    };

    settings = lib.mkOption {
      type = settingsType;
      default = { };
      description = ''
        Configuration for gta-mo. Required fields: `game_root` and `proton_path`.
        Any other fields from Config are accepted (game_id, game_exe, etc.).
        Written to `~/.config/gta-mo/config.toml`.
      '';
      example = lib.literalExpression ''
        {
          game_root = "/home/user/Games/GTA_SA";
          proton_path = "/home/user/.steam/root/compatibilitytools.d/GE-Proton11-1";
          game_id = "umu-gtasa";
          game_exe = "gta_sa.exe";
          proton_use_wined3d = false;
          proton_disable_ntsync = false;
          auto_discover = true;
          dxvk_hud = "devinfo,fps,frametimes";
          mods_dir = "/home/user/Games/GTA_SA/custom_mods";
        }
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ]
      ++ lib.optional (cfg.gui.enable && cfg.gui.package != null) cfg.gui.package;

    xdg.configFile."gta-mo/config.toml".source =
      settingsFormat.generate "gta-mo-config.toml" cfg.settings;
  };
}
