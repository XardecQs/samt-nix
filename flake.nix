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

    version =
      (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package.version;

    source = nixpkgs.lib.fileset.toSource {
      root = ./.;
      fileset = nixpkgs.lib.fileset.unions [
        ./crates
        ./Cargo.toml
        ./Cargo.lock
        ./README.md
        ./LICENSE
      ];
    };

    mkCliPackage = { pkgs, doCheck ? false }:
      pkgs.rustPlatform.buildRustPackage {
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

          # NOTE: do not add `fuse3` to PATH here. Its bin/ contains a
          # non-setuid fusermount3 that shadows the setuid wrapper in
          # /run/wrappers/bin (or /usr/bin), breaking both fuse-overlayfs
          # mounts and unmounts with "Permission denied".

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
          license = nixpkgs.lib.licenses.gpl3Plus;
          platforms = supportedSystems;
        };
      };

    mkGuiPackage = { pkgs }:
      let
        # winit/eframe load these at runtime via dlopen (not linked), so they must
        # also be reachable through LD_LIBRARY_PATH, not just at build time.
        guiRuntimeLibs = with pkgs; [
          libGL
          mesa
          libx11
          libxcursor
          libxrandr
          libxi
          libxkbcommon
          wayland
        ];
      in
      pkgs.rustPlatform.buildRustPackage {
        pname = "gta-mo-gui";
        inherit version;

        src = source;

        cargoLock = {
          lockFile = ./Cargo.lock;
        };

        cargoBuildFlags = [ "-p" "gta-mo-gui" ];

        doCheck = false;

        nativeBuildInputs = with pkgs; [ pkg-config makeWrapper autoPatchelfHook ];

        buildInputs = guiRuntimeLibs;

        postInstall = ''
          wrapProgram "$out/bin/gta-mo-gui" \
            --prefix PATH : ${self.packages.${pkgs.stdenv.hostPlatform.system}.gta-mod-organizer}/bin \
            --suffix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath guiRuntimeLibs}"
        '';

        meta = {
          description = "GTA San Andreas mod organizer GUI (egui/eframe frontend for gta-mo)";
          mainProgram = "gta-mo-gui";
          license = nixpkgs.lib.licenses.gpl3Plus;
          platforms = supportedSystems;
        };
      };

  in {
    packages = forAllSystems (pkgs: rec {
      default = gta-mod-organizer;
      gta-mod-organizer = mkCliPackage { inherit pkgs; };
      gta-mo-gui = mkGuiPackage { inherit pkgs; };
    });

    checks = forAllSystems (pkgs: let
      pkg = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
    in {
      build = pkg;

      clippy = pkgs.stdenv.mkDerivation {
        name = "gta-mod-organizer-clippy";
        src = source;
        nativeBuildInputs = with pkgs; [ rustc cargo clippy ];
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
      gta-mod-organizer = self.packages.${final.stdenv.hostPlatform.system}.default;
      gta-mo-gui = self.packages.${final.stdenv.hostPlatform.system}.gta-mo-gui;
    };

    nixosModules = {
      default = ./modules/nixos.nix;
      gta-mod-organizer = ./modules/nixos.nix;
    };

    homeManagerModules = {
      default = ./modules/home-manager.nix;
      gta-mod-organizer = ./modules/home-manager.nix;
    };

    devShells = forAllSystems (pkgs: let
      # winit/eframe dlopen these at runtime (they are not linked), so cargo-built
      # binaries also need them on LD_LIBRARY_PATH inside the dev shell.
      devRuntimeLibs = with pkgs; [
        libGL
        mesa
        libx11
        libxkbcommon
        wayland
      ];
    in {
      default = pkgs.mkShell {
        buildInputs = devRuntimeLibs ++ (with pkgs; [
          cargo
          rustc
          rustfmt
          clippy
          rust-analyzer
          fuse-overlayfs
          umu-launcher
          sqlite
          gcc
          pkg-config
          glfw3
          libxcb
          libxcursor
          libxrandr
          libxi
          libxinerama
          libxext
          libxxf86vm
          wayland-protocols
        ]);

        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath devRuntimeLibs;

        shellHook = ''
          export CARGO_TARGET_DIR="$HOME/.cache/gta-mo-target"
          echo "Listo — GTA Mod Organizer (Rust dev shell)"
        '';
      };
    });
  };
}
