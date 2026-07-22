{
  description = "GTA Mod Organizer — GTA SA mod launcher with overlayfs";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }: let
    supportedSystems = [ "x86_64-linux" ];
    forAllSystems = f: nixpkgs.lib.genAttrs supportedSystems (system:
      let pkgs = nixpkgs.legacyPackages.${system};
      in f pkgs);
  in {
    packages = forAllSystems (pkgs: rec {
      default = gta-mod-organizer;

      gta-mod-organizer = pkgs.rustPlatform.buildRustPackage {
        pname = "gta-mod-organizer";
        version = "0.2.0";

        src = with nixpkgs.lib.fileset;
          toSource {
            root = ./.;
            fileset = unions [
              ./src
              ./schema.sql
              ./Cargo.toml
              ./Cargo.lock
            ];
          };

        cargoLock = {
          lockFile = ./Cargo.lock;
        };

        meta = {
          description = "GTA San Andreas mod organizer with fuse-overlayfs";
          mainProgram = "gta-mo";
        };
      };
    });

    devShells = forAllSystems (pkgs: {
      default = pkgs.mkShell {
        buildInputs = with pkgs; [
          cargo
          rustc
          rustfmt
          clippy
          rust-analyzer
          fuse-overlayfs
          umu-launcher
          sqlite
        ];

        shellHook = ''
          echo "Listo — GTA Mod Organizer (Rust dev shell)"
        '';
      };
    });
  };
}
