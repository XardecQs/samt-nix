{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    fuse-overlayfs
    sqlite
    umu-launcher
    gcc
    gnumake
    pkg-config
    gtk3
  ];

  shellHook = ''
    echo "Listo — GTA Mod Organizer (bash + C)"
  '';
}