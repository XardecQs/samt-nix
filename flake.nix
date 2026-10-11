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
        ./data
        ./Cargo.toml
        ./Cargo.lock
        ./README.md
        ./LICENSE
      ];
    };

    # winit/eframe load these at runtime via dlopen (not linked), so they must
    # also be reachable through LD_LIBRARY_PATH, not just at build time.
    guiRuntimeLibs = pkgs: with pkgs; [
      libGL
      mesa
      libx11
      libxcursor
      libxrandr
      libxi
      libxkbcommon
      wayland
      # AVIF decoding (`image`'s avif-native) links libdav1d.
      dav1d
    ];

    # Wrapping + shell completions for the CLI binary.
    cliPostInstall = pkgs: ''
      wrapProgram "$out/bin/gta-mo" \
        --prefix PATH : ${pkgs.fuse-overlayfs}/bin \
        --prefix PATH : ${pkgs.umu-launcher}/bin \
        --prefix PATH : ${pkgs.rsync}/bin \
        --prefix PATH : ${pkgs.xdg-utils}/bin

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

    # CLI-only package: `enableGui = false` never compiles the GUI crates.
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
        postInstall = cliPostInstall pkgs;
        meta = {
          description = "GTA San Andreas mod organizer with fuse-overlayfs";
          mainProgram = "gta-mo";
          license = nixpkgs.lib.licenses.gpl3Plus;
          platforms = supportedSystems;
        };
      };

    # GUI-enabled package: builds the CLI and the GUI in a **single** cargo
    # invocation, so the dependency tree shared by both (gta-mo-core + rusqlite/
    # serde/toml/…) is compiled only once instead of once per package. It
    # provides both binaries (`gta-mo` and `gta-mo-gui`); when the GUI is enabled
    # the modules install this for the CLI too, so nothing is built twice.
    # `enableGui = false` uses `gta-mod-organizer` above (no GUI crates at all).
    mkGuiPackage = { pkgs }:
      let
        guiLibs = guiRuntimeLibs pkgs;
      in
      pkgs.rustPlatform.buildRustPackage {
        pname = "gta-mo-gui";
        inherit version;

        src = source;

        cargoLock = {
          lockFile = ./Cargo.lock;
        };

        cargoBuildFlags = [ "-p" "gta-mod-organizer" "-p" "gta-mo-gui" ];

        doCheck = false;

        nativeBuildInputs = with pkgs; [ pkg-config makeWrapper autoPatchelfHook ];

        buildInputs = guiLibs;

        postInstall = cliPostInstall pkgs + ''
          wrapProgram "$out/bin/gta-mo-gui" \
            --prefix PATH : "$out/bin" \
            --prefix PATH : ${pkgs.fontconfig}/bin \
            --suffix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath guiLibs}"

          install -Dm644 data/gta-mo-gui.desktop \
            "$out/share/applications/gta-mo-gui.desktop"
          install -Dm644 data/icons/hicolor/scalable/apps/gta-mo-gui.svg \
            "$out/share/icons/hicolor/scalable/apps/gta-mo-gui.svg"
          for size in 16 24 32 48 64 128 256 512; do
            install -Dm644 "data/icons/hicolor/''${size}x''${size}/apps/gta-mo-gui.png" \
              "$out/share/icons/hicolor/''${size}x''${size}/apps/gta-mo-gui.png"
          done
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

      clippy = pkgs.rustPlatform.buildRustPackage {
        pname = "gta-mod-organizer-clippy";
        inherit version;

        src = source;

        cargoLock = {
          lockFile = ./Cargo.lock;
        };

        nativeBuildInputs = with pkgs; [ clippy ];

        # Lint cli+core (including test code). The GUI needs X11/Wayland build
        # inputs that this derivation does not provide; CI lints it separately.
        cargoBuildCommand = "cargo clippy -p gta-mod-organizer -p gta-mo-core --all-targets -- -D warnings";

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

    overlays.default = self.overlay;

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
        libxcursor
        libxrandr
        libxi
        libxinerama
        libxext
        libxxf86vm
        libxkbcommon
        libxcb
        wayland
        # AVIF decoding (`image`'s avif-native) links libdav1d.
        dav1d
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
