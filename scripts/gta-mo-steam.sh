#!/usr/bin/env bash
# Steam launcher for GTA Mod Organizer.
#
# gta-mo must run natively on the host: it mounts fuse-overlayfs and
# spawns its own container via umu-launcher. Do NOT set a Steam Play /
# Steam Linux Runtime compatibility tool on this entry, or Steam will run
# it inside pressure-vessel where neither gta-mo nor its dependencies
# exist.
#
# Why the extra user/mount namespace (unshare -Ur -m):
#   On NixOS, the Steam client runs inside a bubblewrap sandbox that has
#   its OWN user namespace where uid 0 is NOT mapped. Inside it, the
#   setuid bit of fusermount3 is ignored by the kernel, so fuse-overlayfs
#   cannot mount ("Operation not permitted"). By re-exec'ing into a fresh
#   user namespace where we map ourselves to root (and a private mount
#   namespace owned by it), fuse-overlayfs mounts directly without the
#   setuid helper. uid 0 in that namespace is the same host user, so file
#   ownership is unchanged.
#
# Usage (Steam):
#   1. Add a non-Steam game.
#   2. Target  -> absolute path to this script, e.g. /path/to/gta-mo-steam.sh
#   3. Start In -> a directory that exists (any, e.g. $HOME)
#   4. Compatibility -> "Do not use a compatibility tool"
#   5. (Optional) Launch Options -> extra gta-mo flags (--debug, --discover, ...)
#
# The binary is resolved from, in order:
#   $GTA_MO_BIN, PATH, /etc/profiles/per-user/$USER/bin, ~/.nix-profile/bin
set -euo pipefail

bin="${GTA_MO_BIN:-}"
if [ -z "$bin" ]; then
    bin="$(command -v gta-mo 2>/dev/null || true)"
fi
if [ -z "$bin" ] && [ -x "/etc/profiles/per-user/$USER/bin/gta-mo" ]; then
    bin="/etc/profiles/per-user/$USER/bin/gta-mo"
fi
if [ -z "$bin" ] && [ -x "$HOME/.nix-profile/bin/gta-mo" ]; then
    bin="$HOME/.nix-profile/bin/gta-mo"
fi
if [ -z "$bin" ]; then
    echo "gta-mo-steam: no se encontro el binario gta-mo (usa \$GTA_MO_BIN)" >&2
    exit 1
fi

unshare="${UNSHARE:-}"
if [ -z "$unshare" ]; then
    unshare="$(command -v unshare 2>/dev/null || true)"
fi
if [ -z "$unshare" ] || [ ! -x "$unshare" ]; then
    echo "gta-mo-steam: no se encontro el binario unshare (usa \$UNSHARE)" >&2
    exit 1
fi

uid="$(id -u)"
gid="$(id -g)"

# Steam injects gameoverlayrenderer.so via LD_PRELOAD; clear it (the overlay
# will not attach anyway) together with Steam's LD_LIBRARY_PATH, since
# umu-launcher sets up its own runtime.
#
# GTA_MO_DROP_UID/GID let gta-mo re-exec the game as the original user:
# umu-run refuses to run as root, and inside the nested namespace only root
# is mapped, so gta-mo forks a child that maps this uid and drops privileges.
exec "$unshare" -m -U --map-root-user \
    env -u LD_PRELOAD -u LD_LIBRARY_PATH \
    GTA_MO_DROP_UID="$uid" GTA_MO_DROP_GID="$gid" \
    "$bin" launch "$@"
