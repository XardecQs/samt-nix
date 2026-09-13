#!/usr/bin/env bash
#
# Regenerates the bundled Lucide subset font (lucide.ttf) and the reference
# codepoints.json from the icon names in icons.txt.
#
# The subset is committed, so building gta-mo-gui never needs fonttools:
#
#   nix shell nixpkgs#python3Packages.fonttools -c \
#     bash crates/gui/assets/lucide/build.sh
#
# Override the Lucide version with LUCIDE_VERSION=x.y.z.
set -euo pipefail

VERSION="${LUCIDE_VERSION:-1.45.0}"
DIR="$(cd "$(dirname "$0")" && pwd)"
NAMES_FILE="$DIR/icons.txt"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

command -v pyftsubset >/dev/null 2>&1 || {
  echo "pyftsubset not found. Run under:" >&2
  echo "  nix shell nixpkgs#python3Packages.fonttools -c bash $0" >&2
  exit 1
}

python3 - "$VERSION" "$NAMES_FILE" "$TMP" <<'PY'
import json, pathlib, sys, urllib.request

version, names_file, tmp = sys.argv[1], sys.argv[2], pathlib.Path(sys.argv[3])
base = f"https://cdn.jsdelivr.net/npm/lucide-static@{version}/font"

def get(url):
    with urllib.request.urlopen(url) as r:
        return r.read()

names = [
    n.strip()
    for n in pathlib.Path(names_file).read_text().splitlines()
    if n.strip() and not n.strip().startswith("#")
]
codepoints = json.loads(get(f"{base}/codepoints.json"))
missing = [n for n in names if n not in codepoints]
if missing:
    sys.exit(f"icons not found in Lucide {version}: {missing}")

used = {n: codepoints[n] for n in names}
(tmp / "lucide-full.ttf").write_bytes(get(f"{base}/lucide.ttf"))
(tmp / "used.json").write_text(json.dumps(used, indent=2, sort_keys=True) + "\n")
(tmp / "unicodes.txt").write_text(",".join(f"U+{v:04X}" for v in used.values()))
print(f"lucide {version}: {len(used)} icons")
PY

pyftsubset "$TMP/lucide-full.ttf" \
  --unicodes-file="$TMP/unicodes.txt" \
  --output-file="$DIR/lucide.ttf" \
  --no-hinting \
  --desubroutinize \
  --name-IDs='*' \
  --name-legacy \
  --name-languages='*'

cp "$TMP/used.json" "$DIR/codepoints.json"
echo "wrote $DIR/lucide.ttf ($(wc -c < "$DIR/lucide.ttf") bytes) and codepoints.json"
