{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  buildInputs = with pkgs; [
    fuse-overlayfs
    sqlite
    umu-launcher
    yj
    jq
    sqlite-utils
    #visidata
    #gcc
    #gnumake
    #pkg-config
    #gtk3
  ];

  shellHook = ''
    echo "Listo — GTA Mod Organizer"
  '';
}