RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

die()  { echo -e "${RED}[X]${NC} $*" >&2; exit 1; }
warn() { echo -e "${YELLOW}[!]${NC} $*" >&2; }
ok()   { echo -e "${GREEN}[+]${NC} $*"; }
info() { echo -e "${CYAN}[i]${NC} $*"; }

sql_escape() {
    printf '%s' "$1" | sed "s/'/''/g"
}
