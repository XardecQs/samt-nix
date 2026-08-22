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
    # auto_discover = false
    # dxvk_hud = "devinfo,fps,frametimes,submissions,compiler,version,api,pipelines,memory,gpuload,drawcalls"
    # mods_dir = "/home/user/path/to/custom/mods"

    # Directory layout expected under game_root:
    #   game_root/
    #   ├── base/    — Clean, unmodded game files
    #   ├── mods/    — One subdirectory per mod
    #   ├── pfx/     — Wine prefix (auto-created by umu-launcher)
    #   └── run/     — Overlay runtime (upper/, work/, merged/ and logs/)
  '';
in {
  options.programs.gta-mo = {
    enable = lib.mkEnableOption "GTA Mod Organizer — install gta-mo system-wide";

    package = lib.mkOption {
      type = lib.types.package;
      default = pkgs.gta-mod-organizer;
      defaultText = lib.literalExpression "pkgs.gta-mod-organizer";
      description = "The gta-mod-organizer CLI package to use.";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];

    environment.etc."gta-mo/config.toml.example" = {
      text = exampleConfig;
      mode = "0644";
    };
  };
}
