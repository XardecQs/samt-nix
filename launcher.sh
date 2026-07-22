#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG_FILE="${GTA_MO_CONFIG:-$ROOT_DIR/config.toml}"
DB_PATH="${GTA_MO_DB:-$ROOT_DIR/organizer.db}"
LOCKFILE="$ROOT_DIR/launcher.lock"
GUARD_PID=""

source "$ROOT_DIR/lib/common.sh"
source "$ROOT_DIR/lib/config.sh"
source "$ROOT_DIR/lib/db.sh"
source "$ROOT_DIR/lib/resolver.sh"
source "$ROOT_DIR/lib/overlay.sh"

# ──────────────────────────────────────────────
#  Argumentos
# ──────────────────────────────────────────────

DRY_RUN=0
DEBUG=0
DISCOVER=0
CLEAN=0
for arg in "$@"; do
    case "$arg" in
        --dry-run)  DRY_RUN=1 ;;
        --debug)    DEBUG=1 ;;
        --discover) DISCOVER=1 ;;
        --clean)    CLEAN=1 ;;
        *)
            echo "Error: argumento desconocido: $arg"
            echo "Uso: $(basename "$0") [--dry-run] [--debug] [--discover] [--clean]"
            exit 1
            ;;
    esac
done

# ──────────────────────────────────────────────
#  Lockfile (evitar ejecuciones simultáneas)
# ──────────────────────────────────────────────

exec {lock_fd}>"$LOCKFILE" || {
    echo "Error: no se pudo crear lockfile en $LOCKFILE"
    exit 1
}
if ! flock -n "$lock_fd"; then
    echo "Error: Ya hay una instancia en ejecución (lockfile: $LOCKFILE)"
    exit 1
fi
echo $$ > "$LOCKFILE"

# ──────────────────────────────────────────────
#  Verificación de dependencias del sistema
# ──────────────────────────────────────────────

echo "=== GTA SA Mod Organizer (SQLite Mode) ==="

missing=""
for cmd in fuse-overlayfs sqlite3 yj jq; do
    if ! command -v "$cmd" >/dev/null 2>&1; then
        missing="$missing $cmd"
    fi
done
if [ -n "$missing" ]; then
    echo "Error: Faltan dependencias:$missing. Asegúrate de estar dentro de 'nix-shell'."
    exit 1
fi

# ──────────────────────────────────────────────
#  Configuración
# ──────────────────────────────────────────────

if [ ! -f "$CONFIG_FILE" ]; then
    echo "Error: Archivo de configuración no encontrado: $CONFIG_FILE"
    exit 1
fi

GAME_ROOT=$(parse_toml "$CONFIG_FILE" "game_root" || true)
PROTONPATH=$(parse_toml "$CONFIG_FILE" "proton_path" || true)
GAMEID=$(parse_toml "$CONFIG_FILE" "game_id" || true)
GAME_EXE=$(parse_toml "$CONFIG_FILE" "game_exe" || true)
PROTON_USE_WINED3D=$(parse_toml "$CONFIG_FILE" "proton_use_wined3d" || true)
PROTON_DISABLE_NTSYNC=$(parse_toml "$CONFIG_FILE" "proton_disable_ntsync" || true)
DXVK_HUD=$(parse_toml "$CONFIG_FILE" "dxvk_hud" || true)
AUTO_DISCOVER=$(parse_toml "$CONFIG_FILE" "auto_discover" || true)

if [ -z "$GAME_ROOT" ]; then
    echo "Error: 'game_root' no definido en $CONFIG_FILE"
    exit 1
fi

if [ -z "$PROTONPATH" ]; then
    echo "Error: 'proton_path' no definido en $CONFIG_FILE"
    exit 1
fi

if [ ! -d "$GAME_ROOT" ]; then
    echo "Error: game_root no es un directorio válido: $GAME_ROOT"
    exit 1
fi

if [ ! -d "$PROTONPATH" ]; then
    echo "Error: proton_path no es un directorio válido: $PROTONPATH"
    exit 1
fi

GAMEID_DEFAULT=0; [ -z "$GAMEID" ] && { GAMEID="umu-gtasa"; GAMEID_DEFAULT=1; }
GAME_EXE_DEFAULT=0; [ -z "$GAME_EXE" ] && { GAME_EXE="gta_sa.exe"; GAME_EXE_DEFAULT=1; }
WINE3D_DEFAULT=0; [ -z "$PROTON_USE_WINED3D" ] && { PROTON_USE_WINED3D="true"; WINE3D_DEFAULT=1; }
NTSYNC_DEFAULT=0; [ -z "$PROTON_DISABLE_NTSYNC" ] && { PROTON_DISABLE_NTSYNC="false"; NTSYNC_DEFAULT=1; }
PROTON_USE_WINED3D=$(toml_bool "$PROTON_USE_WINED3D")
PROTON_DISABLE_NTSYNC=$(toml_bool "$PROTON_DISABLE_NTSYNC")

DXVK_HUD_DEFAULT=0
if [ -z "$DXVK_HUD" ]; then
    DXVK_HUD="devinfo,fps,frametimes,submissions,compiler,version,api,pipelines,memory,gpuload,drawcalls"
    DXVK_HUD_DEFAULT=1
fi

AUTO_DISCOVER_DEFAULT=0
if [ -z "$AUTO_DISCOVER" ]; then
    AUTO_DISCOVER="false"
    AUTO_DISCOVER_DEFAULT=1
fi
AUTO_DISCOVER=$(toml_bool "$AUTO_DISCOVER")

BASE_JUEGO="$GAME_ROOT/base"
MODS_DIR="$GAME_ROOT/mods"
WINE_PREFIX_DIR="$GAME_ROOT/pfx"

UPPER="$GAME_ROOT/run/upper"
WORK="$GAME_ROOT/run/work"
MERGED="$GAME_ROOT/run/merged"

if [ ! -d "$BASE_JUEGO" ]; then
    echo "Error: directorio base del juego no encontrado: $BASE_JUEGO"
    exit 1
fi

export WINEPREFIX="$WINE_PREFIX_DIR"
export PROTONPATH
export GAMEID
export PROTON_USE_WINED3D
export PROTON_DISABLE_NTSYNC

if [ $DEBUG -eq 1 ]; then
    LOG_DIR="$GAME_ROOT/run/logs"
    mkdir -p "$LOG_DIR"
    export PROTON_LOG=1
    export DXVK_LOG_LEVEL=debug
    export DXVK_LOG_PATH="$LOG_DIR"
    export DXVK_HUD
    export WINEDEBUG="+loaddll"
    echo "[D] Modo debug activado: PROTON_LOG + DXVK debug + WINEDEBUG"
fi

# ──────────────────────────────────────────────
#  Limpieza
# ──────────────────────────────────────────────

trap cleanup EXIT

# ──────────────────────────────────────────────
#  Preparar mountpoints
# ──────────────────────────────────────────────

fusermount -u "$MERGED" 2>/dev/null || true
mkdir -p "$UPPER" "$WORK" "$MERGED"

# ──────────────────────────────────────────────
#  Migraciones de base de datos
# ──────────────────────────────────────────────

run_migrations "$DB_PATH"

# ──────────────────────────────────────────────
#  Autodescubrimiento de mods
# ──────────────────────────────────────────────

if [ "$DISCOVER" -eq 1 ] || [ "$AUTO_DISCOVER" -eq 1 ]; then
    echo "[+] Ejecutando autodescubrimiento de mods..."
    discover_mods
    echo ""
fi

if [ "$DISCOVER" -eq 1 ]; then
    exit 0
fi

# ──────────────────────────────────────────────
#  Limpieza de huérfanos
# ──────────────────────────────────────────────

if [ "$CLEAN" -eq 1 ]; then
    echo "[+] Eliminando mods huérfanos..."
    clean_orphans
    echo ""
    exit 0
fi

# ──────────────────────────────────────────────
#  Cargar mods desde SQLite
# ──────────────────────────────────────────────

echo "[+] Leyendo configuración de mods desde SQLite..."

declare -A MOD_FOLDER MOD_ENABLED MOD_ORDER DEPS VISITED DEPENDENCY_OF CYCLE_STATE SKIP_DEP
ENABLED_MODS=()
RESOLVED=()
HAS_ERRORS=0

mods_data=$(sqlite3 "$DB_PATH" "SELECT id, folder_name, enabled, load_order FROM mods;") || {
    echo "[X] Error: no se pudo leer la tabla de mods"
    exit 1
}
while IFS='|' read -r id folder enabled order; do
    [ -z "$id" ] && continue
    MOD_FOLDER[$id]="$folder"
    MOD_ENABLED[$id]="$enabled"
    MOD_ORDER[$id]="$order"
done <<< "$mods_data"

deps_data=$(sqlite3 "$DB_PATH" "SELECT mod_id, dependency_id FROM mod_dependencies;") || {
    echo "[X] Error: no se pudo leer las dependencias"
    exit 1
}
while IFS='|' read -r mod_id dep_id; do
    [ -z "$mod_id" ] && continue
    DEPS[$mod_id]+="$dep_id "
done <<< "$deps_data"

enabled_data=$(sqlite3 "$DB_PATH" \
    "SELECT id FROM mods WHERE enabled = 1 ORDER BY load_order DESC;") || {
    echo "[X] Error: no se pudo consultar mods habilitados"
    exit 1
}
while IFS='|' read -r id; do
    [ -z "$id" ] && continue
    ENABLED_MODS+=("$id")
done <<< "$enabled_data"

# ──────────────────────────────────────────────
#  Resolución de dependencias
# ──────────────────────────────────────────────

for mid in "${ENABLED_MODS[@]}"; do
    dep_ids="${DEPS[$mid]:-}"
    for did in $dep_ids; do
        [ -z "$did" ] && continue
        DEPENDENCY_OF[$did]=1
    done
done

for mid in "${!DEPS[@]}"; do
    for did in ${DEPS[$mid]}; do
        [ -z "$did" ] && continue
        if [ -z "${MOD_FOLDER[$did]:-}" ]; then
            echo "[X] Error: '${MOD_FOLDER[$mid]}' depende del mod con id=$did, que no existe en la base de datos."
            HAS_ERRORS=1
        fi
    done
done

for mid in "${!DEPS[@]}"; do
    [[ -n "${CYCLE_STATE[$mid]:-}" ]] && continue
    detect_cycle "$mid" ""
done

if [ $HAS_ERRORS -ne 0 ]; then
    exit 1
fi

# ──────────────────────────────────────────────
#  Dependencias deshabilitadas (interactivo)
# ──────────────────────────────────────────────

declare -a DISABLED_DEPS_INFO=()
for mid in "${ENABLED_MODS[@]}"; do
    for did in ${DEPS[$mid]:-}; do
        [ -z "$did" ] && continue
        if [ "${MOD_ENABLED[$did]:-0}" = "0" ]; then
            DISABLED_DEPS_INFO+=("$mid|$did|${MOD_FOLDER[$mid]}|${MOD_FOLDER[$did]}")
        fi
    done
done

if [ ${#DISABLED_DEPS_INFO[@]} -gt 0 ]; then
    echo ""
    echo "[!] Se detectaron dependencias deshabilitadas:"
    for entry in "${DISABLED_DEPS_INFO[@]}"; do
        IFS='|' read -r _ _ mod_name dep_name <<< "$entry"
        echo "    - '$mod_name' requiere '$dep_name' (deshabilitado)"
    done
    echo ""
    echo "Opciones:"
    echo "  1) Activar dependencias (incluyendo transitivas) y continuar"
    echo "  2) Continuar sin las dependencias (ignorar)"
    echo "  3) Cancelar"
    read -rp "Elige una opción [1-3]: " choice
    case "$choice" in
        1)
            for entry in "${DISABLED_DEPS_INFO[@]}"; do
                IFS='|' read -r _ did _ _ <<< "$entry"
                enable_recursive "$did"
            done
            echo ""
            ;;
        2)
            echo "[!] Continuando sin las dependencias. Puede que el juego falle."
            for entry in "${DISABLED_DEPS_INFO[@]}"; do
                IFS='|' read -r _ did _ _ <<< "$entry"
                SKIP_DEP[$did]=1
            done
            echo ""
            ;;
        3)
            echo "Cancelado."
            exit 1
            ;;
        *)
            echo "Opción inválida. Cancelando."
            exit 1
            ;;
    esac
fi

# ──────────────────────────────────────────────
#  Resolver orden de carga (DFS pre-order)
# ──────────────────────────────────────────────

if [ ${#ENABLED_MODS[@]} -eq 0 ]; then
    echo "[!] No hay mods activos en la base de datos. Se lanzará el juego limpio."
    LOWERDIR="$BASE_JUEGO"
else
    for mid in "${ENABLED_MODS[@]}"; do
        [[ -n "${DEPENDENCY_OF[$mid]:-}" ]] && continue
        resolve "$mid"
    done

    for folder in "${RESOLVED[@]}"; do
        [ -z "$folder" ] && continue
        mod_path="$MODS_DIR/$folder"
        if [ ! -d "$mod_path" ]; then
            echo "[X] Error: la carpeta del mod '$folder' no existe: $mod_path"
            HAS_ERRORS=1
        fi
    done

    if [ $HAS_ERRORS -ne 0 ]; then
        exit 1
    fi

    echo "[+] Mods activos detectados en orden de prioridad:"
    ACTIVE_MODS=()
    for folder in "${RESOLVED[@]}"; do
        [ -z "$folder" ] && continue
        echo "    - $folder"
        ACTIVE_MODS+=("$MODS_DIR/$folder")
    done
    LOWERDIR=$(IFS=:; echo "${ACTIVE_MODS[*]}"):$BASE_JUEGO
fi

# ──────────────────────────────────────────────
#  Montar overlay y lanzar
# ──────────────────────────────────────────────

if [ $DRY_RUN -eq 1 ]; then
    echo ""
    echo "=== DRY RUN: no se montará overlay ni se lanzará el juego ==="
    echo ""
    echo "lowerdir capas (ordenadas mayor → menor prioridad):"
    IFS=':' read -ra CAPAS <<< "$LOWERDIR"
    i=1
    for capa in "${CAPAS[@]}"; do
        echo "  $i. $capa"
        (( ++i ))
    done
    echo ""
    echo "upperdir: $UPPER"
    echo "workdir:  $WORK"
    echo "merged:   $MERGED"
    echo ""
    echo "WINEPREFIX:                $WINE_PREFIX_DIR"
    echo "PROTONPATH:                $PROTONPATH"
    echo "GAMEID:                    $GAMEID$([ $GAMEID_DEFAULT -eq 1 ] && echo ' (por defecto)')"
    echo "GAME_EXE:                  $GAME_EXE$([ $GAME_EXE_DEFAULT -eq 1 ] && echo ' (por defecto)')"
    echo "PROTON_USE_WINED3D:        $PROTON_USE_WINED3D$([ $WINE3D_DEFAULT -eq 1 ] && echo ' (por defecto)')"
    echo "PROTON_DISABLE_NTSYNC:     $PROTON_DISABLE_NTSYNC$([ $NTSYNC_DEFAULT -eq 1 ] && echo ' (por defecto)')"
    echo "AUTO_DISCOVER:             $AUTO_DISCOVER$([ $AUTO_DISCOVER_DEFAULT -eq 1 ] && echo ' (por defecto)')"
    if [ $DEBUG -eq 1 ]; then
        echo ""
        echo "[DEBUG] Modo debug activado:"
        echo "  PROTON_LOG:              1"
        echo "  DXVK_LOG_LEVEL:          debug"
        echo "  DXVK_LOG_PATH:           $LOG_DIR"
        echo "  DXVK_HUD:                $DXVK_HUD$([ $DXVK_HUD_DEFAULT -eq 1 ] && echo ' (por defecto)')"
        echo "  WINEDEBUG:               +loaddll"
    fi
    echo "Ejecutable:                $MERGED/$GAME_EXE"
    exit 0
fi

start_guard "$MERGED" "$$"

echo "[+] Montando capas..."
fuse-overlayfs -o lowerdir="$LOWERDIR",upperdir="$UPPER",workdir="$WORK" "$MERGED" || {
    echo "[X] Error al montar overlay"
    exit 1
}

if [ ! -f "$MERGED/$GAME_EXE" ]; then
    echo "[X] Error: $MERGED/$GAME_EXE no encontrado tras el montaje"
    exit 1
fi

cd "$MERGED" || exit 1
echo "[+] Lanzando juego desde merged..."
umu-run "$MERGED/$GAME_EXE"

echo "[+] Sesión terminada."
