#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DB_PATH="${GTA_MO_DB:-$ROOT_DIR/organizer.db}"

source "$ROOT_DIR/lib/common.sh"

# ─── Helpers ───

check_db() {
    if [ ! -f "$DB_PATH" ]; then
        die "Base de datos no encontrada: $DB_PATH. Ejecuta launcher.sh primero para crearla."
    fi
}

resolve_mod() {
    local ident="$1"
    if [[ "$ident" =~ ^[0-9]+$ ]]; then
        local exists
        exists=$(sqlite3 "$DB_PATH" \
            "SELECT COUNT(*) FROM mods WHERE id = $ident;" 2>/dev/null || echo "0")
        if [ "$exists" != "1" ]; then
            die "Mod con id=$ident no encontrado."
        fi
        echo "$ident"
    else
        local escaped
        escaped=$(sql_escape "$ident")
        local id
        id=$(sqlite3 "$DB_PATH" \
            "SELECT id FROM mods WHERE folder_name = '$escaped';" 2>/dev/null || true)
        if [ -z "$id" ]; then
            die "Mod '$ident' no encontrado."
        fi
        echo "$id"
    fi
}

mod_folder() {
    local mid="$1"
    sqlite3 "$DB_PATH" \
        "SELECT folder_name FROM mods WHERE id = $mid;" 2>/dev/null || true
}

mod_name() {
    local mid="$1"
    sqlite3 "$DB_PATH" \
        "SELECT name FROM mods WHERE id = $mid;" 2>/dev/null || true
}

mod_enabled() {
    local mid="$1"
    sqlite3 "$DB_PATH" \
        "SELECT enabled FROM mods WHERE id = $mid;" 2>/dev/null || true
}

# ─── Subcomandos ───

cmd_list() {
    check_db
    local verbose=0
    local filter=""
    while [ $# -gt 0 ]; do
        case "$1" in
            -v|--verbose) verbose=1; shift ;;
            --enabled)    filter="WHERE enabled = 1"; shift ;;
            --disabled)   filter="WHERE enabled = 0"; shift ;;
            *) die "Opción desconocida: $1" ;;
        esac
    done

    local count
    count=$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM mods $filter;" 2>/dev/null || echo "0")
    if [ "$count" = "0" ]; then
        echo "No hay mods registrados."
        return
    fi

    printf "${BOLD}%-4s %-6s %-7s %-30s %s${NC}\n" "ID" "ACTIVO" "ORDEN" "CARPETA" "NOMBRE"
    printf "%-4s %-6s %-7s %-30s %s\n" "---" "------" "------" "------------------------------" "------------------------------"

    sqlite3 -separator '|' "$DB_PATH" \
        "SELECT id, enabled, load_order, folder_name, name FROM mods $filter ORDER BY load_order DESC;" | \
    while IFS='|' read -r id enabled order folder name; do
        local status
        if [ "$enabled" = "1" ]; then
            status="${GREEN}SI${NC}"
        else
            status="${RED}NO${NC}"
        fi
        printf "%-4s %-6b %-7s %-30s %s\n" "$id" "$status" "$order" "$folder" "$name"

        if [ "$verbose" = "1" ]; then
            local deps
            deps=$(sqlite3 "$DB_PATH" \
                "SELECT m.folder_name FROM mod_dependencies d JOIN mods m ON d.dependency_id = m.id WHERE d.mod_id = $id ORDER BY m.load_order DESC;" \
                2>/dev/null || true)
            if [ -n "$deps" ]; then
                printf "     ${CYAN}-> depende de:${NC} %s\n" "$(echo "$deps" | tr '\n' ' ')"
            fi
            local dependents
            dependents=$(sqlite3 "$DB_PATH" \
                "SELECT m.folder_name FROM mod_dependencies d JOIN mods m ON d.mod_id = m.id WHERE d.dependency_id = $id ORDER BY m.load_order DESC;" \
                2>/dev/null || true)
            if [ -n "$dependents" ]; then
                printf "     ${YELLOW}<- requerido por:${NC} %s\n" "$(echo "$dependents" | tr '\n' ' ')"
            fi
        fi
    done

    echo ""
    info "Total: $count mod(s)"
}

cmd_add() {
    check_db
    local folder=""
    local display_name=""
    local order=""

    while [ $# -gt 0 ]; do
        case "$1" in
            --name)
                display_name="$2"; shift 2 ;;
            --order)
                order="$2"; shift 2 ;;
            -*)
                die "Opción desconocida: $1" ;;
            *)
                folder="$1"; shift ;;
        esac
    done

    if [ -z "$folder" ]; then
        die "Uso: modctl add <carpeta> [--name \"Nombre\"] [--order N]"
    fi

    if echo "$folder" | grep -qE '[:|/\\]'; then
        die "El nombre de carpeta no puede contener ':', '|', '/' ni '\\'."
    fi

    if [ "$folder" = "." ] || [ "$folder" = ".." ]; then
        die "Nombre de carpeta no válido."
    fi

    local safe_folder
    safe_folder=$(sql_escape "$folder")

    local exists
    exists=$(sqlite3 "$DB_PATH" \
        "SELECT COUNT(*) FROM mods WHERE folder_name = '$safe_folder';" 2>/dev/null || echo "0")
    if [ "$exists" != "0" ]; then
        die "El mod '$folder' ya existe en la base de datos."
    fi

    if [ -z "$display_name" ]; then
        display_name="${folder//_/ }"
    fi
    local safe_name
    safe_name=$(sql_escape "$display_name")

    if [ -z "$order" ]; then
        order=$(sqlite3 "$DB_PATH" \
            "SELECT COALESCE(MAX(load_order), 0) + 10 FROM mods;" 2>/dev/null || echo "10")
    fi
    if [[ ! "$order" =~ ^[0-9]+$ ]]; then
        die "El orden de carga debe ser un número entero."
    fi

    sqlite3 "$DB_PATH" \
        "INSERT INTO mods (folder_name, name, enabled, load_order) VALUES ('$safe_folder', '$safe_name', 0, $order);" || {
        die "Error al insertar el mod en la base de datos."
    }

    local new_id
    new_id=$(sqlite3 "$DB_PATH" "SELECT last_insert_rowid();")
    ok "Mod añadido: [$new_id] '$folder' -> '$display_name' (orden=$order, desactivado)"
}

cmd_remove() {
    check_db
    local ident="${1:-}"
    [ -n "$ident" ] || die "Uso: modctl rm <id|carpeta>"
    local mid
    mid=$(resolve_mod "$ident")
    local folder
    folder=$(mod_folder "$mid")

    echo ""
    warn "Vas a eliminar el mod '$folder' (id=$mid)."
    local dep_count
    dep_count=$(sqlite3 "$DB_PATH" \
        "SELECT COUNT(*) FROM mod_dependencies WHERE mod_id = $mid OR dependency_id = $mid;" \
        2>/dev/null || echo "0")
    if [ "$dep_count" -gt 0 ]; then
        warn "Tiene $dep_count relación(es) de dependencia que se eliminarán también."
    fi
    echo ""
    read -rp "¿Confirmar eliminación? [s/N]: " confirm
    if [ "${confirm,,}" != "s" ] && [ "${confirm,,}" != "si" ]; then
        info "Cancelado."
        return
    fi

    sqlite3 "$DB_PATH" "PRAGMA foreign_keys = ON; DELETE FROM mods WHERE id = $mid;" || {
        die "Error al eliminar el mod."
    }
    ok "Mod '$folder' eliminado."
}

cmd_enable() {
    check_db
    local ident="${1:-}"
    [ -n "$ident" ] || die "Uso: modctl enable <id|carpeta>"
    local mid
    mid=$(resolve_mod "$ident")
    local folder
    folder=$(mod_folder "$mid")
    local enabled
    enabled=$(mod_enabled "$mid")

    if [ "$enabled" = "1" ]; then
        warn "'$folder' ya está activado."
        return
    fi

    sqlite3 "$DB_PATH" "UPDATE mods SET enabled = 1 WHERE id = $mid;" || {
        die "Error al activar el mod."
    }
    ok "Mod '$folder' activado."
}

cmd_disable() {
    check_db
    local ident="${1:-}"
    [ -n "$ident" ] || die "Uso: modctl disable <id|carpeta>"
    local mid
    mid=$(resolve_mod "$ident")
    local folder
    folder=$(mod_folder "$mid")
    local enabled
    enabled=$(mod_enabled "$mid")

    if [ "$enabled" = "0" ]; then
        warn "'$folder' ya está desactivado."
        return
    fi

    local dependents
    dependents=$(sqlite3 "$DB_PATH" \
        "SELECT m.folder_name FROM mod_dependencies d JOIN mods m ON d.mod_id = m.id WHERE d.dependency_id = $mid AND m.enabled = 1;" \
        2>/dev/null || true)
    if [ -n "$dependents" ]; then
        warn "'$folder' es requerido por los siguientes mods activos:"
        while IFS= read -r dep; do
            [ -z "$dep" ] && continue
            echo "       - $dep"
        done <<< "$dependents"
        echo ""
        read -rp "¿Desactivar de todas formas? [s/N]: " confirm
        if [ "${confirm,,}" != "s" ] && [ "${confirm,,}" != "si" ]; then
            info "Cancelado."
            return
        fi
    fi

    sqlite3 "$DB_PATH" "UPDATE mods SET enabled = 0 WHERE id = $mid;" || {
        die "Error al desactivar el mod."
    }
    ok "Mod '$folder' desactivado."
}

cmd_order() {
    check_db
    local ident="${1:-}"
    local new_order="${2:-}"
    [ -n "$ident" ] || die "Uso: modctl order <id|carpeta> <N>"
    [ -n "$new_order" ] || die "Uso: modctl order <id|carpeta> <N>"
    if [[ ! "$new_order" =~ ^[0-9]+$ ]]; then
        die "El orden debe ser un número entero."
    fi

    local mid
    mid=$(resolve_mod "$ident")
    local folder
    folder=$(mod_folder "$mid")
    local old_order
    old_order=$(sqlite3 "$DB_PATH" \
        "SELECT load_order FROM mods WHERE id = $mid;" 2>/dev/null || echo "?")

    sqlite3 "$DB_PATH" \
        "UPDATE mods SET load_order = $new_order WHERE id = $mid;" || {
        die "Error al cambiar el orden."
    }
    ok "'$folder': orden cambiado de $old_order a $new_order."
}

cmd_rename() {
    check_db
    local ident="${1:-}"
    local new_name="${2:-}"
    [ -n "$ident" ] || die "Uso: modctl rename <id|carpeta> <nombre>"
    [ -n "$new_name" ] || die "Uso: modctl rename <id|carpeta> <nombre>"
    [ "${#new_name}" -gt 0 ] || die "El nombre no puede estar vacío."

    local mid
    mid=$(resolve_mod "$ident")
    local folder
    folder=$(mod_folder "$mid")
    local old_name
    old_name=$(mod_name "$mid")

    local safe_name
    safe_name=$(sql_escape "$new_name")

    sqlite3 "$DB_PATH" \
        "UPDATE mods SET name = '$safe_name' WHERE id = $mid;" || {
        die "Error al renombrar el mod."
    }
    ok "'$folder': nombre cambiado de '$old_name' a '$new_name'."
}

cmd_info() {
    check_db
    local ident="${1:-}"
    [ -n "$ident" ] || die "Uso: modctl info <id|carpeta>"
    local mid
    mid=$(resolve_mod "$ident")

    local folder name enabled order
    IFS='|' read -r folder name enabled order <<< \
        "$(sqlite3 "$DB_PATH" "SELECT folder_name, name, enabled, load_order FROM mods WHERE id = $mid;")"

    local status_str
    if [ "$enabled" = "1" ]; then
        status_str="${GREEN}Activado${NC}"
    else
        status_str="${RED}Desactivado${NC}"
    fi

    echo ""
    echo -e "  ${BOLD}ID:${NC}       $mid"
    echo -e "  ${BOLD}Carpeta:${NC}  $folder"
    echo -e "  ${BOLD}Nombre:${NC}   $name"
    echo -e "  ${BOLD}Estado:${NC}   $status_str"
    echo -e "  ${BOLD}Orden:${NC}    $order"
    echo ""

    local deps
    deps=$(sqlite3 "$DB_PATH" \
        "SELECT m.id, m.folder_name, m.name, m.enabled
         FROM mod_dependencies d
         JOIN mods m ON d.dependency_id = m.id
         WHERE d.mod_id = $mid
         ORDER BY m.load_order DESC;" 2>/dev/null || true)

    if [ -n "$deps" ]; then
        echo -e "  ${CYAN}Dependencias ($mid depende de):${NC}"
        while IFS='|' read -r did dfolder dname denabled; do
            [ -z "$did" ] && continue
            local dstatus
            if [ "$denabled" = "1" ]; then
                dstatus="${GREEN}SI${NC}"
            else
                dstatus="${RED}NO${NC}"
            fi
            echo -e "    [$did] $dfolder ($dname) [activo: $dstatus]"
        done <<< "$deps"
    else
        echo -e "  ${CYAN}Dependencias:${NC} ninguna"
    fi
    echo ""

    local dependents
    dependents=$(sqlite3 "$DB_PATH" \
        "SELECT m.id, m.folder_name, m.name, m.enabled
         FROM mod_dependencies d
         JOIN mods m ON d.mod_id = m.id
         WHERE d.dependency_id = $mid
         ORDER BY m.load_order DESC;" 2>/dev/null || true)

    if [ -n "$dependents" ]; then
        echo -e "  ${YELLOW}Requerido por:${NC}"
        while IFS='|' read -r rid rfolder rname renabled; do
            [ -z "$rid" ] && continue
            local rstatus
            if [ "$renabled" = "1" ]; then
                rstatus="${GREEN}SI${NC}"
            else
                rstatus="${RED}NO${NC}"
            fi
            echo -e "    [$rid] $rfolder ($rname) [activo: $rstatus]"
        done <<< "$dependents"
    else
        echo -e "  ${YELLOW}Requerido por:${NC} nadie"
    fi
    echo ""
}

cmd_dep() {
    check_db
    local action="${1:-}"
    [ -n "$action" ] || die "Uso: modctl dep <add|rm> <mod> <dependencia>"
    shift

    case "$action" in
        add) cmd_dep_add "$@" ;;
        rm)  cmd_dep_rm "$@" ;;
        *)   die "Acción desconocida: $action. Usa 'add' o 'rm'." ;;
    esac
}

cmd_dep_add() {
    local mod_ident="${1:-}"
    local dep_ident="${2:-}"
    [ -n "$mod_ident" ] || die "Uso: modctl dep add <mod> <dependencia>"
    [ -n "$dep_ident" ] || die "Uso: modctl dep add <mod> <dependencia>"

    local mod_id dep_id
    mod_id=$(resolve_mod "$mod_ident")
    dep_id=$(resolve_mod "$dep_ident")

    local mod_folder dep_folder
    mod_folder=$(mod_folder "$mod_id")
    dep_folder=$(mod_folder "$dep_id")

    if [ "$mod_id" = "$dep_id" ]; then
        die "Un mod no puede depender de sí mismo."
    fi

    local exists
    exists=$(sqlite3 "$DB_PATH" \
        "SELECT COUNT(*) FROM mod_dependencies WHERE mod_id = $mod_id AND dependency_id = $dep_id;" \
        2>/dev/null || echo "0")
    if [ "$exists" != "0" ]; then
        warn "'$mod_folder' ya depende de '$dep_folder'."
        return
    fi

    sqlite3 "$DB_PATH" \
        "INSERT INTO mod_dependencies (mod_id, dependency_id) VALUES ($mod_id, $dep_id);" || {
        die "Error al añadir la dependencia (¿ciclo de dependencias?)."
    }
    ok "'$mod_folder' ahora depende de '$dep_folder'."
}

cmd_dep_rm() {
    local mod_ident="${1:-}"
    local dep_ident="${2:-}"
    [ -n "$mod_ident" ] || die "Uso: modctl dep rm <mod> <dependencia>"
    [ -n "$dep_ident" ] || die "Uso: modctl dep rm <mod> <dependencia>"

    local mod_id dep_id
    mod_id=$(resolve_mod "$mod_ident")
    dep_id=$(resolve_mod "$dep_ident")

    local mod_folder dep_folder
    mod_folder=$(mod_folder "$mod_id")
    dep_folder=$(mod_folder "$dep_id")

    local exists
    exists=$(sqlite3 "$DB_PATH" \
        "SELECT COUNT(*) FROM mod_dependencies WHERE mod_id = $mod_id AND dependency_id = $dep_id;" \
        2>/dev/null || echo "0")
    if [ "$exists" = "0" ]; then
        warn "'$mod_folder' no depende de '$dep_folder'."
        return
    fi

    sqlite3 "$DB_PATH" \
        "DELETE FROM mod_dependencies WHERE mod_id = $mod_id AND dependency_id = $dep_id;" || {
        die "Error al eliminar la dependencia."
    }
    ok "Dependencia eliminada: '$mod_folder' ya no depende de '$dep_folder'."
}

# ─── TUI ───

cmd_tui() {
    check_db

    while true; do
        echo ""
        echo -e "  ${BOLD}modctl — Gestión de mods${NC}"
        echo "  ───────────────────────────"
        echo "  1) Listar mods"
        echo "  2) Listar mods (con dependencias)"
        echo "  3) Añadir mod"
        echo "  4) Eliminar mod"
        echo "  5) Activar mod"
        echo "  6) Desactivar mod"
        echo "  7) Cambiar orden de carga"
        echo "  8) Renombrar mod"
        echo "  9) Ver info de un mod"
        echo " 10) Gestionar dependencias"
        echo "  0) Salir"
        echo ""
        read -rp "  Opción [0-10]: " opt

        case "$opt" in
            1) echo ""; cmd_list ;;
            2) echo ""; cmd_list -v ;;
            3)
                echo ""
                read -rp "    Carpeta del mod: " folder
                read -rp "    Nombre visible (dejar vacío para auto): " dname
                read -rp "    Orden de carga (dejar vacío para auto): " dorder
                local args=("$folder")
                [ -n "$dname" ] && args+=(--name "$dname")
                [ -n "$dorder" ] && args+=(--order "$dorder")
                set -- "${args[@]}"
                cmd_add "$@" 2>/dev/null || true
                ;;
            4)
                echo ""
                read -rp "    ID o carpeta del mod a eliminar: " ident
                cmd_remove "$ident" 2>/dev/null || true
                ;;
            5)
                echo ""
                read -rp "    ID o carpeta del mod a activar: " ident
                cmd_enable "$ident" 2>/dev/null || true
                ;;
            6)
                echo ""
                read -rp "    ID o carpeta del mod a desactivar: " ident
                cmd_disable "$ident" 2>/dev/null || true
                ;;
            7)
                echo ""
                read -rp "    ID o carpeta del mod: " ident
                read -rp "    Nuevo orden de carga: " new_order
                cmd_order "$ident" "$new_order" 2>/dev/null || true
                ;;
            8)
                echo ""
                read -rp "    ID o carpeta del mod: " ident
                read -rp "    Nuevo nombre: " new_name
                cmd_rename "$ident" "$new_name" 2>/dev/null || true
                ;;
            9)
                echo ""
                read -rp "    ID o carpeta del mod: " ident
                cmd_info "$ident" 2>/dev/null || true
                ;;
            10)
                echo ""
                echo "    a) Añadir dependencia"
                echo "    b) Eliminar dependencia"
                read -rp "    Opción [a/b]: " depopt
                case "$depopt" in
                    a)
                        read -rp "      Mod: " mod_ident
                        read -rp "      Dependencia (mod requerido): " dep_ident
                        cmd_dep_add "$mod_ident" "$dep_ident" 2>/dev/null || true
                        ;;
                    b)
                        read -rp "      Mod: " mod_ident
                        read -rp "      Dependencia a eliminar: " dep_ident
                        cmd_dep_rm "$mod_ident" "$dep_ident" 2>/dev/null || true
                        ;;
                    *) warn "Opción inválida." ;;
                esac
                ;;
            0)
                echo ""
                info "¡Hasta luego!"
                exit 0
                ;;
            *) warn "Opción inválida." ;;
        esac
    done
}

# ─── Help ───

cmd_help() {
    echo ""
    echo -e "${BOLD}modctl — Gestión de mods para GTA Mod Organizer${NC}"
    echo ""
    echo "Uso: modctl <comando> [opciones]"
    echo ""
    echo "Comandos:"
    echo "  list                    Listar todos los mods"
    echo "  list -v                 Listar con dependencias"
    echo "  list --enabled          Solo mods activados"
    echo "  list --disabled         Solo mods desactivados"
    echo "  add <carpeta>           Añadir un mod nuevo"
    echo "        [--name N]          Nombre visible (opcional)"
    echo "        [--order N]         Orden de carga (opcional)"
    echo "  rm <id|carpeta>         Eliminar un mod"
    echo "  enable <id|carpeta>     Activar un mod"
    echo "  disable <id|carpeta>    Desactivar un mod"
    echo "  order <id|carpeta> <N>  Cambiar orden de carga"
    echo "  rename <id|carpeta> <N> Cambiar nombre visible"
    echo "  info <id|carpeta>       Mostrar información detallada"
    echo "  dep add <mod> <dep>     Añadir dependencia"
    echo "  dep rm <mod> <dep>      Eliminar dependencia"
    echo "  tui                     Abrir menú interactivo"
    echo "  help                    Mostrar esta ayuda"
    echo ""
    echo "Ejemplos:"
    echo "  modctl list -v"
    echo "  modctl add mi_mod --name \"Mi Mod\" --order 50"
    echo "  modctl enable mi_mod"
    echo "  modctl dep add mod_b mod_a"
    echo ""
}

# ─── Main ───

case "${1:-}" in
    list)     shift; cmd_list "$@" ;;
    add)      shift; cmd_add "$@" ;;
    rm|remove) shift; cmd_remove "$@" ;;
    enable)   shift; cmd_enable "$@" ;;
    disable)  shift; cmd_disable "$@" ;;
    order)    shift; cmd_order "$@" ;;
    rename)   shift; cmd_rename "$@" ;;
    info)     shift; cmd_info "$@" ;;
    dep)      shift; cmd_dep "$@" ;;
    tui)      shift; cmd_tui "$@" ;;
    help|-h|--help) cmd_help ;;
    "")
        cmd_help
        echo -e "${CYAN}Sugerencia:${NC} ejecuta ${BOLD}modctl tui${NC} para el menú interactivo."
        echo ""
        ;;
    *) die "Comando desconocido: $1. Usa 'modctl help' para ver las opciones." ;;
esac
