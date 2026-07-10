#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONFIG_FILE="$ROOT_DIR/config.toml"
DB_PATH="$ROOT_DIR/organizer.db"
LOCKFILE="$ROOT_DIR/launcher.lock"
GUARD_PID=""

DRY_RUN=0
for arg in "$@"; do
    case "$arg" in
        --dry-run) DRY_RUN=1 ;;
        *)
            echo "Error: argumento desconocido: $arg"
            echo "Uso: $(basename "$0") [--dry-run]"
            exit 1
            ;;
    esac
done

# ──────────────────────────────────────────────
#  Funciones
# ──────────────────────────────────────────────

parse_toml() {
    local file="$1"
    local key="$2"
    yj -t < "$file" 2>/dev/null | jq -r --arg k "$key" 'if has($k) then .[$k] else empty end' 2>/dev/null
}

toml_bool() {
    case "$1" in
        true)  echo 1 ;;
        false) echo 0 ;;
        *)     echo "$1" ;;
    esac
}

run_migrations() {
    local db="$1"

    sqlite3 "$db" "PRAGMA foreign_keys = ON;" || {
        echo "[X] Error: no se pudo acceder a la base de datos"
        exit 1
    }

    sqlite3 "$db" "PRAGMA journal_mode=WAL;" > /dev/null || true

    local cascade_count
    cascade_count=$(sqlite3 "$db" \
        "SELECT COUNT(*) FROM pragma_foreign_key_list('mod_dependencies') WHERE \"from\" = 'mod_id' AND \"on_delete\" = 'CASCADE';" \
        2>/dev/null || echo "0")

    if [ "$cascade_count" = "0" ]; then
        echo "[+] Migrando schema: añadiendo ON DELETE CASCADE en mod_dependencies..."
        sqlite3 "$db" <<'SQL'
            BEGIN;
            CREATE TABLE mod_dependencies_new (
                mod_id INTEGER NOT NULL,
                dependency_id INTEGER NOT NULL,
                PRIMARY KEY (mod_id, dependency_id),
                FOREIGN KEY (mod_id) REFERENCES mods(id) ON DELETE CASCADE,
                FOREIGN KEY (dependency_id) REFERENCES mods(id) ON DELETE CASCADE,
                CHECK(mod_id != dependency_id)
            );
            INSERT INTO mod_dependencies_new SELECT * FROM mod_dependencies;
            DROP TABLE mod_dependencies;
            ALTER TABLE mod_dependencies_new RENAME TO mod_dependencies;
            COMMIT;
SQL
        echo "[+] Migración completada."
    fi

    sqlite3 "$db" "CREATE INDEX IF NOT EXISTS idx_mod_deps_mod_id ON mod_dependencies(mod_id);" 2>/dev/null || true
    sqlite3 "$db" "CREATE INDEX IF NOT EXISTS idx_mod_deps_dep_id ON mod_dependencies(dependency_id);" 2>/dev/null || true
}

detect_cycle() {
    local mid="$1"
    local path="$2"
    local did

    CYCLE_STATE[$mid]=1
    for did in ${DEPS[$mid]:-}; do
        [ -z "$did" ] && continue
        if [[ "${CYCLE_STATE[$did]:-}" == "1" ]]; then
            echo "[X] Error: ciclo de dependencias detectado: ${path}${MOD_FOLDER[$did]} -> ${MOD_FOLDER[$did]}"
            HAS_ERRORS=1
            return 1
        fi
        if [[ -z "${CYCLE_STATE[$did]:-}" ]]; then
            detect_cycle "$did" "${path}${MOD_FOLDER[$did]} -> " || return 1
        fi
    done
    CYCLE_STATE[$mid]=2
}

resolve() {
    local mid="$1"
    [[ -n "${VISITED[$mid]:-}" ]] && return
    [[ "${SKIP_DEP[$mid]:-0}" == "1" ]] && return

    VISITED[$mid]=1
    RESOLVED+=("${MOD_FOLDER[$mid]}")

    local dep_ids="${DEPS[$mid]:-}"
    if [ -n "$dep_ids" ]; then
        local dep_list=""
        local did
        for did in $dep_ids; do
            [ -z "$did" ] && continue
            dep_list+="${MOD_ORDER[$did]:-0}|$did"$'\n'
        done
        local sorted
        sorted=$(echo "$dep_list" | LC_ALL=C sort -t'|' -k1,1rn | cut -d'|' -f2 || true)
        for did in $sorted; do
            resolve "$did"
        done
    fi
}

start_guard() {
    local merged="$1"
    local ppid="$2"
    (
        while kill -0 "$ppid" 2>/dev/null; do
            sleep 1
        done
        fusermount -u "$merged" 2>/dev/null || true
    ) &
    GUARD_PID=$!
}

enable_recursive() {
    local did="$1"
    if [ "${MOD_ENABLED[$did]:-0}" = "1" ]; then
        return
    fi
    MOD_ENABLED[$did]=1
    sqlite3 "$DB_PATH" "UPDATE mods SET enabled = 1 WHERE id = $did;"
    echo "    [+] Activado: ${MOD_FOLDER[$did]}"
    for sub_did in ${DEPS[$did]:-}; do
        [ -z "$sub_did" ] && continue
        enable_recursive "$sub_did"
    done
}

cleanup() {
    fusermount -u "${MERGED:-}" 2>/dev/null || true
    [ -n "${GUARD_PID:-}" ] && kill "$GUARD_PID" 2>/dev/null || true
    rm -f "$LOCKFILE"
}

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
NTSYNC_DEFAULT=0; [ -z "$PROTON_DISABLE_NTSYNC" ] && { PROTON_DISABLE_NTSYNC="true"; NTSYNC_DEFAULT=1; }
PROTON_USE_WINED3D=$(toml_bool "$PROTON_USE_WINED3D")
PROTON_DISABLE_NTSYNC=$(toml_bool "$PROTON_DISABLE_NTSYNC")

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

# ──────────────────────────────────────────────
#  Limpieza
# ──────────────────────────────────────────────

trap cleanup EXIT

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
#  Preparar mountpoints
# ──────────────────────────────────────────────

fusermount -u "$MERGED" 2>/dev/null || true
mkdir -p "$UPPER" "$WORK" "$MERGED"

# ──────────────────────────────────────────────
#  Migraciones de base de datos
# ──────────────────────────────────────────────

run_migrations "$DB_PATH"

# ──────────────────────────────────────────────
#  Cargar mods desde SQLite
# ──────────────────────────────────────────────

echo "[+] Leyendo configuración de mods desde SQLite..."

declare -A MOD_FOLDER MOD_ENABLED MOD_ORDER DEPS VISITED DEPENDENCY_OF CYCLE_STATE SKIP_DEP
declare -a ENABLED_MODS
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
        ((i++))
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
    echo "Ejecutable:                $MERGED/$GAME_EXE"
    exit 0
fi

echo "[+] Montando capas..."
fuse-overlayfs -o lowerdir="$LOWERDIR",upperdir="$UPPER",workdir="$WORK" "$MERGED" || {
    echo "[X] Error al montar overlay"
    exit 1
}

start_guard "$MERGED" "$$"

if [ ! -f "$MERGED/$GAME_EXE" ]; then
    echo "[X] Error: $MERGED/$GAME_EXE no encontrado tras el montaje"
    exit 1
fi

cd "$MERGED" || exit 1
echo "[+] Lanzando juego desde merged..."
umu-run "$MERGED/$GAME_EXE"

echo "[+] Sesión terminada."
