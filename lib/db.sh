run_migrations() {
    local db="$1"

    sqlite3 "$db" "PRAGMA foreign_keys = ON;" || {
        echo "[X] Error: no se pudo acceder a la base de datos"
        exit 1
    }

    sqlite3 "$db" "PRAGMA journal_mode=WAL;" > /dev/null || true

    local cascade_count
    if ! cascade_count=$(sqlite3 "$db" \
        "SELECT COUNT(*) FROM pragma_foreign_key_list('mod_dependencies') WHERE \"from\" = 'mod_id' AND \"on_delete\" = 'CASCADE';" \
        2>/dev/null); then
        echo "[X] Error: no se pudo consultar el schema de la base de datos"
        exit 1
    fi

    if [ "$cascade_count" = "0" ]; then
        echo "[+] Migrando schema: añadiendo ON DELETE CASCADE en mod_dependencies..."
        sqlite3 "$db" <<'SQL'
            PRAGMA foreign_keys = ON;
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

    local create_sql
    create_sql=$(sqlite3 "$db" "SELECT sql FROM sqlite_master WHERE type='table' AND name='mods';" 2>/dev/null || true)
    if [[ "$create_sql" != *"%:%"* ]]; then
        local colon_count
        colon_count=$(sqlite3 "$db" "SELECT COUNT(*) FROM mods WHERE folder_name LIKE '%:%';" 2>/dev/null || echo "0")
        if [ "$colon_count" != "0" ]; then
            echo "[!] Advertencia: $colon_count mod(s) contienen ':' en folder_name."
            echo "    El overlay usa ':' como separador de capas; el montaje fallará."
            echo "    Renombra las carpetas y actualiza la base de datos antes de continuar."
        else
            echo "[+] Migrando schema: añadiendo restricción ':' en folder_name..."
            if ! sqlite3 "$db" <<'SQL'
                PRAGMA foreign_keys = OFF;
                BEGIN;
                CREATE TABLE mod_deps_temp AS SELECT * FROM mod_dependencies;
                DROP TABLE mod_dependencies;
                CREATE TABLE mods_new (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    folder_name TEXT NOT NULL UNIQUE,
                    name TEXT NOT NULL CHECK(length(name) > 0),
                    enabled INTEGER DEFAULT 0 CHECK(enabled IN (0, 1)),
                    load_order INTEGER DEFAULT 0,
                    CHECK(
                        length(folder_name) > 0
                        AND folder_name NOT LIKE '%|%'
                        AND folder_name NOT LIKE '%/%'
                        AND folder_name NOT LIKE '%\%'
                        AND folder_name NOT LIKE '%:%'
                        AND folder_name != '.'
                        AND folder_name != '..'
                        AND folder_name NOT LIKE '.. %'
                        AND folder_name NOT LIKE '..\%'
                        AND folder_name NOT LIKE '../%'
                        AND trim(folder_name) = folder_name
                    )
                );
                INSERT INTO mods_new SELECT * FROM mods;
                DROP TABLE mods;
                ALTER TABLE mods_new RENAME TO mods;
                CREATE TABLE mod_dependencies (
                    mod_id INTEGER NOT NULL,
                    dependency_id INTEGER NOT NULL,
                    PRIMARY KEY (mod_id, dependency_id),
                    FOREIGN KEY (mod_id) REFERENCES mods(id) ON DELETE CASCADE,
                    FOREIGN KEY (dependency_id) REFERENCES mods(id) ON DELETE CASCADE,
                    CHECK(mod_id != dependency_id)
                );
                INSERT INTO mod_dependencies SELECT * FROM mod_deps_temp;
                DROP TABLE mod_deps_temp;
                CREATE INDEX IF NOT EXISTS idx_mod_deps_mod_id ON mod_dependencies(mod_id);
                CREATE INDEX IF NOT EXISTS idx_mod_deps_dep_id ON mod_dependencies(dependency_id);
                COMMIT;
                PRAGMA foreign_keys = ON;
SQL
            then
                echo "[+] Migración completada."
            else
                echo "[X] Error: no se pudo aplicar la migración."
                exit 1
            fi
        fi
    fi
}

discover_mods() {
    local new_count=0
    local orphan_count=0

    local disk_folders=()
    while IFS= read -r -d '' dir; do
        local name
        name=$(basename "$dir")
        [[ "$name" == .* ]] && continue
        disk_folders+=("$name")
    done < <(find "$MODS_DIR" -mindepth 1 -maxdepth 1 -type d -print0 2>/dev/null || true)

    local db_folders
    db_folders=$(sqlite3 "$DB_PATH" "SELECT folder_name FROM mods;" 2>/dev/null || true)

    for folder in "${disk_folders[@]}"; do
        if ! echo "$db_folders" | grep -qxF "$folder"; then
            local display_name="${folder//_/ }"
            local safe_folder
            safe_folder=$(printf '%s' "$folder" | sed "s/'/''/g")
            local safe_name
            safe_name=$(printf '%s' "$display_name" | sed "s/'/''/g")
            local max_order
            max_order=$(sqlite3 "$DB_PATH" "SELECT COALESCE(MAX(load_order), 0) + 10 FROM mods;")
            if [[ ! "$max_order" =~ ^[0-9]+$ ]]; then
                echo "    [!] Error interno: load_order inesperado ($max_order), omitiendo '$folder'"
                continue
            fi
            if sqlite3 "$DB_PATH" \
                "INSERT INTO mods (folder_name, name, enabled, load_order) VALUES ('$safe_folder', '$safe_name', 0, $max_order);"; then
                echo "    [+] Nuevo: $folder → '$display_name' (load_order=$max_order)"
                (( ++new_count ))
            else
                echo "    [!] Error al insertar: $folder"
            fi
        fi
    done

    if [ -n "$db_folders" ]; then
        while IFS= read -r db_folder; do
            [ -z "$db_folder" ] && continue
            local found=0
            for df in "${disk_folders[@]}"; do
                [ "$df" = "$db_folder" ] && { found=1; break; }
            done
            if [ $found -eq 0 ]; then
                local was_enabled
                was_enabled=$(sqlite3 "$DB_PATH" \
                    "SELECT enabled FROM mods WHERE folder_name = '$(printf '%s' "$db_folder" | sed "s/'/''/g")';" \
                    2>/dev/null || echo "0")
                if [ "$was_enabled" = "1" ]; then
                    sqlite3 "$DB_PATH" \
                        "UPDATE mods SET enabled = 0 WHERE folder_name = '$(printf '%s' "$db_folder" | sed "s/'/''/g")';"
                    echo "    [!] Huérfano desactivado: '$db_folder' (carpeta eliminada del disco)"
                else
                    echo "    [!] Huérfano: '$db_folder' (carpeta eliminada del disco)"
                fi
                (( ++orphan_count ))
            fi
        done <<< "$db_folders"
    fi

    [ $new_count -gt 0 ] && echo "[+] $new_count mod(s) nuevo(s) registrado(s)."
    [ $orphan_count -gt 0 ] && echo "[!] $orphan_count mod(s) huérfano(s) detectado(s)."
    return 0
}

clean_orphans() {
    local deleted=0

    local db_folders
    db_folders=$(sqlite3 "$DB_PATH" "SELECT folder_name FROM mods;" 2>/dev/null || true)
    [ -z "$db_folders" ] && { echo "[+] No hay mods en la base de datos."; return 0; }

    while IFS= read -r db_folder; do
        [ -z "$db_folder" ] && continue
        if [ ! -d "$MODS_DIR/$db_folder" ]; then
            local safe_folder
            safe_folder=$(printf '%s' "$db_folder" | sed "s/'/''/g")
            sqlite3 "$DB_PATH" \
                "DELETE FROM mods WHERE folder_name = '$safe_folder';" 2>/dev/null
            echo "    [-] Eliminado: '$db_folder'"
            (( ++deleted ))
        fi
    done <<< "$db_folders"

    [ $deleted -gt 0 ] && echo "[+] $deleted mod(s) huérfano(s) eliminado(s)."
    [ $deleted -eq 0 ] && echo "[+] No hay mods huérfanos que eliminar."
    return 0
}
