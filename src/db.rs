use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ModEntry {
    pub id: i64,
    pub folder_name: String,
    pub name: String,
    pub enabled: bool,
    pub load_order: i64,
}

pub fn ensure_db_dir(db_path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

pub fn open_db(db_path: &Path) -> anyhow::Result<Connection> {
    ensure_db_dir(db_path)?;
    let conn = Connection::open(db_path)?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
    Ok(conn)
}

pub fn run_migrations(conn: &Connection) -> anyhow::Result<()> {
    let schema = include_str!("../schema.sql");
    conn.execute_batch(schema)?;

    let has_cascade: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM pragma_foreign_key_list('mod_dependencies')
             WHERE \"from\" = 'mod_id' AND \"on_delete\" = 'CASCADE'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);

    if !has_cascade {
        log::info("[+] Migrando schema: añadiendo ON DELETE CASCADE en mod_dependencies...");
        conn.execute_batch(
            "BEGIN;
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
             COMMIT;",
        )?;
        log::info("[+] Migración completada.");
    }

    let has_colon_constraint: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='mods'",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();

    if !has_colon_constraint.contains("%:%") {
        let colon_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM mods WHERE folder_name LIKE '%:%'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if colon_count > 0 {
            log::warn(format!(
                "[!] Advertencia: {} mod(s) contienen ':' en folder_name.",
                colon_count
            ));
            log::warn("    El overlay usa ':' como separador de capas; el montaje fallará.");
            log::warn("    Renombra las carpetas y actualiza la base de datos antes de continuar.");
        } else {
            log::info("[+] Migrando schema: añadiendo restricción ':' en folder_name...");
            conn.execute_batch(
                "PRAGMA foreign_keys = OFF;
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
                         AND folder_name NOT LIKE '%\\%'
                         AND folder_name NOT LIKE '%:%'
                         AND folder_name != '.'
                         AND folder_name != '..'
                         AND folder_name NOT LIKE '.. %'
                         AND folder_name NOT LIKE '..\\%'
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
                 PRAGMA foreign_keys = ON;",
            )?;
            log::info("[+] Migración completada.");
        }
    }

    Ok(())
}

pub fn load_all_mods(conn: &Connection) -> anyhow::Result<HashMap<i64, ModEntry>> {
    let mut stmt = conn.prepare("SELECT id, folder_name, name, enabled, load_order FROM mods")?;
    let mods = stmt
        .query_map([], |row| {
            Ok(ModEntry {
                id: row.get(0)?,
                folder_name: row.get(1)?,
                name: row.get(2)?,
                enabled: row.get::<_, i64>(3)? != 0,
                load_order: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(mods.into_iter().map(|m| (m.id, m)).collect())
}

pub fn load_dependencies(conn: &Connection) -> anyhow::Result<HashMap<i64, Vec<i64>>> {
    let mut stmt = conn.prepare("SELECT mod_id, dependency_id FROM mod_dependencies")?;
    let mut deps: HashMap<i64, Vec<i64>> = HashMap::new();
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
    })?;

    for row in rows {
        let (mod_id, dep_id) = row?;
        deps.entry(mod_id).or_default().push(dep_id);
    }

    Ok(deps)
}

pub fn load_enabled_mod_ids(conn: &Connection) -> anyhow::Result<Vec<i64>> {
    let mut stmt =
        conn.prepare("SELECT id FROM mods WHERE enabled = 1 ORDER BY load_order DESC")?;
    let ids = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<i64>, _>>()?;
    Ok(ids)
}

pub fn mod_exists(conn: &Connection, folder_name: &str) -> anyhow::Result<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM mods WHERE folder_name = ?1",
        params![folder_name],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

pub fn add_mod(
    conn: &Connection,
    folder_name: &str,
    display_name: &str,
    order: Option<i64>,
) -> anyhow::Result<i64> {
    let order = match order {
        Some(o) => o,
        None => {
            conn.query_row(
                "SELECT COALESCE(MAX(load_order), 0) + 10 FROM mods",
                [],
                |row| row.get(0),
            )?
        }
    };

    conn.execute(
        "INSERT INTO mods (folder_name, name, enabled, load_order) VALUES (?1, ?2, 0, ?3)",
        params![folder_name, display_name, order],
    )?;

    Ok(conn.last_insert_rowid())
}

pub fn remove_mod(conn: &Connection, id: i64) -> anyhow::Result<()> {
    conn.execute("PRAGMA foreign_keys = ON", [])?;
    conn.execute("DELETE FROM mods WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn set_mod_enabled(conn: &Connection, id: i64, enabled: bool) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE mods SET enabled = ?1 WHERE id = ?2",
        params![enabled as i64, id],
    )?;
    Ok(())
}

pub fn set_mod_order(conn: &Connection, id: i64, order: i64) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE mods SET load_order = ?1 WHERE id = ?2",
        params![order, id],
    )?;
    Ok(())
}

pub fn set_mod_name(conn: &Connection, id: i64, name: &str) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE mods SET name = ?1 WHERE id = ?2",
        params![name, id],
    )?;
    Ok(())
}

pub fn get_mod_by_id(conn: &Connection, id: i64) -> anyhow::Result<Option<ModEntry>> {
    let mut stmt =
        conn.prepare("SELECT id, folder_name, name, enabled, load_order FROM mods WHERE id = ?1")?;
    let mut rows = stmt.query_map(params![id], |row| {
        Ok(ModEntry {
            id: row.get(0)?,
            folder_name: row.get(1)?,
            name: row.get(2)?,
            enabled: row.get::<_, i64>(3)? != 0,
            load_order: row.get(4)?,
        })
    })?;
    Ok(rows.next().transpose()?)
}

pub fn get_mod_by_folder(conn: &Connection, folder: &str) -> anyhow::Result<Option<ModEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, folder_name, name, enabled, load_order FROM mods WHERE folder_name = ?1",
    )?;
    let mut rows = stmt.query_map(params![folder], |row| {
        Ok(ModEntry {
            id: row.get(0)?,
            folder_name: row.get(1)?,
            name: row.get(2)?,
            enabled: row.get::<_, i64>(3)? != 0,
            load_order: row.get(4)?,
        })
    })?;
    Ok(rows.next().transpose()?)
}

pub fn resolve_mod_ident(conn: &Connection, ident: &str) -> anyhow::Result<i64> {
    if let Ok(id) = ident.parse::<i64>() {
        let exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM mods WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )?;
        if exists == 1 {
            return Ok(id);
        }
        anyhow::bail!("Mod con id={} no encontrado.", id);
    }

    let m = get_mod_by_folder(conn, ident)?;
    m.map(|m| m.id)
        .ok_or_else(|| anyhow::anyhow!("Mod '{}' no encontrado.", ident))
}

pub fn add_dependency(conn: &Connection, mod_id: i64, dep_id: i64) -> anyhow::Result<()> {
    if mod_id == dep_id {
        anyhow::bail!("Un mod no puede depender de sí mismo.");
    }
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM mod_dependencies WHERE mod_id = ?1 AND dependency_id = ?2",
        params![mod_id, dep_id],
        |row| row.get(0),
    )?;
    if exists > 0 {
        anyhow::bail!("La dependencia ya existe.");
    }
    conn.execute(
        "INSERT INTO mod_dependencies (mod_id, dependency_id) VALUES (?1, ?2)",
        params![mod_id, dep_id],
    )?;
    Ok(())
}

pub fn remove_dependency(conn: &Connection, mod_id: i64, dep_id: i64) -> anyhow::Result<()> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM mod_dependencies WHERE mod_id = ?1 AND dependency_id = ?2",
        params![mod_id, dep_id],
        |row| row.get(0),
    )?;
    if exists == 0 {
        anyhow::bail!("La dependencia no existe.");
    }
    conn.execute(
        "DELETE FROM mod_dependencies WHERE mod_id = ?1 AND dependency_id = ?2",
        params![mod_id, dep_id],
    )?;
    Ok(())
}

pub fn get_dependencies_of(conn: &Connection, mod_id: i64) -> anyhow::Result<Vec<ModEntry>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.folder_name, m.name, m.enabled, m.load_order
         FROM mod_dependencies d JOIN mods m ON d.dependency_id = m.id
         WHERE d.mod_id = ?1 ORDER BY m.load_order DESC",
    )?;
    let rows = stmt.query_map(params![mod_id], |row| {
        Ok(ModEntry {
            id: row.get(0)?,
            folder_name: row.get(1)?,
            name: row.get(2)?,
            enabled: row.get::<_, i64>(3)? != 0,
            load_order: row.get(4)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn get_dependents_of(conn: &Connection, mod_id: i64) -> anyhow::Result<Vec<ModEntry>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.folder_name, m.name, m.enabled, m.load_order
         FROM mod_dependencies d JOIN mods m ON d.mod_id = m.id
         WHERE d.dependency_id = ?1 ORDER BY m.load_order DESC",
    )?;
    let rows = stmt.query_map(params![mod_id], |row| {
        Ok(ModEntry {
            id: row.get(0)?,
            folder_name: row.get(1)?,
            name: row.get(2)?,
            enabled: row.get::<_, i64>(3)? != 0,
            load_order: row.get(4)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn count_deps_for_mod(conn: &Connection, mod_id: i64) -> anyhow::Result<i64> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM mod_dependencies WHERE mod_id = ?1 OR dependency_id = ?1",
        params![mod_id],
        |row| row.get(0),
    )?;
    Ok(count)
}

pub fn discover_mods(
    conn: &Connection,
    mods_dir: &Path,
) -> anyhow::Result<(usize, usize)> {
    if !mods_dir.exists() {
        std::fs::create_dir_all(mods_dir)?;
        log::info(format!("    [-] Directorio de mods creado: {}", mods_dir.display()));
    }

    let mut new_count = 0usize;
    let mut orphan_count = 0usize;

    let mut disk_folders: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(mods_dir) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                disk_folders.push(name);
            }
        }
    }

    let all_mods = load_all_mods(conn)?;
    let db_folders: Vec<String> = all_mods.values().map(|m| m.folder_name.clone()).collect();

    for folder in &disk_folders {
        if !db_folders.contains(folder) {
            let display_name = folder.replace('_', " ");
            let order: i64 = conn
                .query_row(
                    "SELECT COALESCE(MAX(load_order), 0) + 10 FROM mods",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(10);

            match add_mod(conn, folder, &display_name, Some(order)) {
                Ok(_id) => {
                    log::info(format!(
                        "    [+] Nuevo: {folder} -> '{display_name}' (load_order={order})"
                    ));
                    new_count += 1;
                }
                Err(e) => {
                    log::warn(format!("    [!] Error al insertar: {folder}: {e}"));
                }
            }
        }
    }

    for db_folder in &db_folders {
        if !disk_folders.contains(db_folder) {
            if let Some(m) = all_mods.values().find(|m| &m.folder_name == db_folder) {
                if m.enabled {
                    let _ = set_mod_enabled(conn, m.id, false);
                    log::warn(format!(
                        "    [!] Huérfano desactivado: '{}' (carpeta eliminada del disco)",
                        db_folder
                    ));
                } else {
                    log::warn(format!(
                        "    [!] Huérfano: '{}' (carpeta eliminada del disco)",
                        db_folder
                    ));
                }
                orphan_count += 1;
            }
        }
    }

    if new_count > 0 {
        log::info(format!("[+] {new_count} mod(s) nuevo(s) registrado(s)."));
    }
    if orphan_count > 0 {
        log::warn(format!("[!] {orphan_count} mod(s) huérfano(s) detectado(s)."));
    }

    Ok((new_count, orphan_count))
}

pub fn clean_orphans(conn: &Connection, mods_dir: &Path) -> anyhow::Result<usize> {
    let all_mods = load_all_mods(conn)?;
    let mut deleted = 0usize;

    for m in all_mods.values() {
        let mod_path = mods_dir.join(&m.folder_name);
        if !mod_path.exists() {
            conn.execute("DELETE FROM mods WHERE id = ?1", params![m.id])?;
            log::info(format!("    [-] Eliminado: '{}'", m.folder_name));
            deleted += 1;
        }
    }

    if deleted > 0 {
        log::info(format!("[+] {deleted} mod(s) huérfano(s) eliminado(s)."));
    } else {
        log::info("[+] No hay mods huérfanos que eliminar.");
    }

    Ok(deleted)
}

/// Logging helpers that mimic the bash output style
pub mod log {
    use owo_colors::OwoColorize;
    use std::fmt::Display;

    pub fn info(msg: impl Display) {
        eprintln!("{} {}", "[+]".green().bold(), msg);
    }

    pub fn warn(msg: impl Display) {
        eprintln!("{} {}", "[!]".yellow().bold(), msg);
    }

    pub fn error(msg: impl Display) {
        eprintln!("{} {}", "[X]".red().bold(), msg);
    }

    pub fn die(msg: impl Display) -> ! {
        error(msg);
        std::process::exit(1);
    }
}
