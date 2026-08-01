{
  description = "GTA Mod Organizer — GTA SA mod launcher with overlayfs";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }: let
    supportedSystems = [ "x86_64-linux" "aarch64-linux" ];
    forAllSystems = f: nixpkgs.lib.genAttrs supportedSystems (system:
      let pkgs = nixpkgs.legacyPackages.${system};
      in f pkgs);

    version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;

    source = nixpkgs.lib.fileset.toSource {
      root = ./.;
      fileset = nixpkgs.lib.fileset.unions [
        ./src
        ./schema.sql
        ./Cargo.toml
        ./Cargo.lock
      ];
    };

    mkPackage = { pkgs, doCheck ? false }: pkgs.rustPlatform.buildRustPackage {
      pname = "gta-mod-organizer";
      inherit version;

      src = source;

      cargoLock = {
        lockFile = ./Cargo.lock;
      };

      inherit doCheck;

      nativeBuildInputs = with pkgs; [ makeWrapper ];

      postInstall = ''
        wrapProgram "$out/bin/gta-mo" \
          --prefix PATH : ${pkgs.fuse-overlayfs}/bin \
          --prefix PATH : ${pkgs.umu-launcher}/bin

        mkdir -p "$out/share/bash-completion/completions" \
                 "$out/share/zsh/site-functions" \
                 "$out/share/fish/vendor_completions.d"
        "$out/bin/gta-mo" completions bash > "$out/share/bash-completion/completions/gta-mo"
        "$out/bin/gta-mo" completions zsh  > "$out/share/zsh/site-functions/_gta-mo"
        "$out/bin/gta-mo" completions fish > "$out/share/fish/vendor_completions.d/gta-mo.fish"
      '';

      meta = {
        description = "GTA San Andreas mod organizer with fuse-overlayfs";
        mainProgram = "gta-mo";
        license = nixpkgs.lib.licenses.mit;
        platforms = supportedSystems;
      };
    };

  in {
    packages = forAllSystems (pkgs: rec {
      default = gta-mod-organizer;
      gta-mod-organizer = mkPackage { inherit pkgs; };
    });

    checks = forAllSystems (pkgs: let
      pkg = self.packages.${pkgs.system}.default;
    in {
      build = pkg;

      clippy = pkgs.stdenv.mkDerivation {
        name = "gta-mod-organizer-clippy";
        src = source;
        nativeBuildInputs = with pkgs; [ rustc cargo clippy pkg-config ];
        buildPhase = "cargo clippy --all-targets -- -D warnings";
        installPhase = "mkdir $out";
      };

      fmt = pkgs.stdenv.mkDerivation {
        name = "gta-mod-organizer-fmt";
        src = source;
        nativeBuildInputs = with pkgs; [ rustc cargo rustfmt ];
        buildPhase = "cargo fmt --all --check";
        installPhase = "mkdir $out";
      };
    });

    overlay = final: prev: {
      gta-mod-organizer = self.packages.${final.system}.default;
    };

    nixosModules = {
      default = ./modules/nixos.nix;
      gta-mod-organizer = ./modules/nixos.nix;
    };

    homeManagerModules = {
      default = ./modules/home-manager.nix;
      gta-mod-organizer = ./modules/home-manager.nix;
    };

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
