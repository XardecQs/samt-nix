{
  description = "GTA Mod Organizer — GTA SA mod launcher with overlayfs";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }: let
    supportedSystems = [ "x86_64-linux" ];
    forAllSystems = f: nixpkgs.lib.genAttrs supportedSystems (system: f nixpkgs.legacyPackages.${system});
  in {
    packages = forAllSystems (pkgs: rec {
      default = gta-mod-organizer;
      gta-mod-organizer = pkgs.stdenv.mkDerivation {
        pname = "gta-mod-organizer";
        version = "0.1.0";

        src = with nixpkgs.lib.fileset;
          toSource {
            root = ./.;
            fileset = unions [
              ./lib
              ./launcher.sh
              ./modctl.sh
              ./schema.sql
            ];
          };

        nativeBuildInputs = with pkgs; [ makeWrapper ];

        dontBuild = true;

        installPhase = ''
          mkdir -p $out/share/gta-mod-organizer/lib
          mkdir -p $out/bin

          cp -r lib/* $out/share/gta-mod-organizer/lib/
          cp launcher.sh $out/share/gta-mod-organizer/
          cp modctl.sh $out/share/gta-mod-organizer/
          cp schema.sql $out/share/gta-mod-organizer/

          for script in launcher.sh modctl.sh; do
            substituteInPlace "$out/share/gta-mod-organizer/$script" \
              --replace-fail 'ROOT_DIR="$(cd "$(dirname "''${BASH_SOURCE[0]}")" && pwd)"' \
              'ROOT_DIR="'"$out/share/gta-mod-organizer"'"'
          done

          makeWrapper "$out/share/gta-mod-organizer/launcher.sh" "$out/bin/gta-mo-launcher" \
            --prefix PATH : "${pkgs.fuse-overlayfs}/bin" \
            --prefix PATH : "${pkgs.sqlite}/bin" \
            --prefix PATH : "${pkgs.yj}/bin" \
            --prefix PATH : "${pkgs.jq}/bin" \
            --prefix PATH : "${pkgs.umu-launcher}/bin" \
            --set-default GTA_MO_CONFIG "\''${XDG_CONFIG_HOME:-$HOME/.config}/gta-mo/config.toml" \
            --set-default GTA_MO_DB "\''${XDG_DATA_HOME:-$HOME/.local/share}/gta-mo/organizer.db"

          makeWrapper "$out/share/gta-mod-organizer/modctl.sh" "$out/bin/gta-mo-ctl" \
            --prefix PATH : "${pkgs.sqlite}/bin" \
            --set-default GTA_MO_DB "\''${XDG_DATA_HOME:-$HOME/.local/share}/gta-mo/organizer.db"
        '';
      };
    });

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        buildInputs = with pkgs; [
          fuse-overlayfs
          sqlite
          umu-launcher
          yj
          jq
        ];
        shellHook = ''
          echo "Listo — GTA Mod Organizer (dev shell)"
        '';
      };
    });
  };
}
