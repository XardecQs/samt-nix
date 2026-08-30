use crate::log;
use rusqlite::{params, Connection};
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

/// Identity data of a mod, without any profile-scoped state (enabled/order).
#[derive(Debug, Clone)]
pub struct ModIdentity {
    pub id: i64,
    pub folder_name: String,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct DepRef {
    pub id: i64,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct Profile {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub is_active: bool,
    pub created_at: String,
}

pub fn slugify(input: &str) -> String {
    let mut slug = String::new();
    for c in input.trim().to_lowercase().chars() {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if (c.is_whitespace() || c == '-' || c == '_')
            && !slug.is_empty()
            && !slug.ends_with('-')
        {
            slug.push('-');
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

pub fn unique_slug(conn: &Connection, base: &str) -> anyhow::Result<String> {
    let base = slugify(base);
    let base = if base.is_empty() {
        "p".to_string()
    } else {
        base
    };

    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM profiles WHERE slug = ?1",
        params![base],
        |row| row.get(0),
    )?;
    if exists == 0 {
        return Ok(base);
    }
    for i in 2.. {
        let cand = format!("{base}-{i}");
        let exists: i64 = conn.query_row(
            "SELECT COUNT(*) FROM profiles WHERE slug = ?1",
            params![cand],
            |row| row.get(0),
        )?;
        if exists == 0 {
            return Ok(cand);
        }
    }
    unreachable!("unique_slug agotó candidatos")
}

pub fn create_profile(conn: &Connection, name: &str) -> anyhow::Result<i64> {
    if name.trim().is_empty() {
        anyhow::bail!("El nombre del perfil no puede estar vacío.");
    }
    let slug = unique_slug(conn, name)?;
    conn.execute(
        "INSERT INTO profiles (name, slug) VALUES (?1, ?2)",
        params![name.trim(), slug],
    )?;
    let id = conn.last_insert_rowid();

    let mut stmt = conn.prepare("SELECT id FROM mods ORDER BY id")?;
    let ids = stmt
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for (i, mid) in ids.iter().enumerate() {
        conn.execute(
            "INSERT INTO profile_mods (profile_id, mod_id, enabled, load_order)
             VALUES (?1, ?2, 0, ?3)",
            params![id, mid, (i as i64 + 1) * 10],
        )?;
    }
    Ok(id)
}

fn row_to_profile(row: &rusqlite::Row) -> rusqlite::Result<Profile> {
    Ok(Profile {
        id: row.get(0)?,
        name: row.get(1)?,
        slug: row.get(2)?,
        is_active: row.get::<_, i64>(3)? != 0,
        created_at: row.get(4)?,
    })
}

pub fn get_profile_by_id(conn: &Connection, id: i64) -> anyhow::Result<Option<Profile>> {
    let mut stmt =
        conn.prepare("SELECT id, name, slug, is_active, created_at FROM profiles WHERE id = ?1")?;
    let mut rows = stmt.query_map(params![id], row_to_profile)?;
    Ok(rows.next().transpose()?)
}

pub fn get_profile_by_slug(conn: &Connection, slug: &str) -> anyhow::Result<Option<Profile>> {
    let mut stmt =
        conn.prepare("SELECT id, name, slug, is_active, created_at FROM profiles WHERE slug = ?1")?;
    let mut rows = stmt.query_map(params![slug], row_to_profile)?;
    Ok(rows.next().transpose()?)
}

pub fn get_profile_by_name(conn: &Connection, name: &str) -> anyhow::Result<Option<Profile>> {
    let mut stmt =
        conn.prepare("SELECT id, name, slug, is_active, created_at FROM profiles WHERE name = ?1")?;
    let mut rows = stmt.query_map(params![name], row_to_profile)?;
    Ok(rows.next().transpose()?)
}

pub fn list_profiles(conn: &Connection) -> anyhow::Result<Vec<Profile>> {
    let mut stmt =
        conn.prepare("SELECT id, name, slug, is_active, created_at FROM profiles ORDER BY id")?;
    let rows = stmt.query_map([], row_to_profile)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn resolve_profile(conn: &Connection, ident: &str) -> anyhow::Result<Profile> {
    if let Ok(id) = ident.parse::<i64>() {
        if let Some(p) = get_profile_by_id(conn, id)? {
            return Ok(p);
        }
    }
    if let Some(p) = get_profile_by_slug(conn, ident)? {
        return Ok(p);
    }
    if let Some(p) = get_profile_by_name(conn, ident)? {
        return Ok(p);
    }
    anyhow::bail!("Perfil '{}' no encontrado.", ident)
}

pub fn rename_profile(conn: &Connection, id: i64, new_name: &str) -> anyhow::Result<()> {
    if new_name.trim().is_empty() {
        anyhow::bail!("El nombre del perfil no puede estar vacío.");
    }
    conn.execute(
        "UPDATE profiles SET name = ?1 WHERE id = ?2",
        params![new_name.trim(), id],
    )?;
    Ok(())
}

pub fn delete_profile(conn: &Connection, id: i64) -> anyhow::Result<()> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM profiles", [], |row| row.get(0))?;
    if count <= 1 {
        anyhow::bail!("No se puede eliminar el último perfil.");
    }
    conn.execute("PRAGMA foreign_keys = ON", [])?;
    conn.execute("DELETE FROM profiles WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn set_active_profile(conn: &Connection, id: i64) -> anyhow::Result<()> {
    conn.execute("BEGIN", [])?;
    let res = (|| -> anyhow::Result<()> {
        conn.execute("UPDATE profiles SET is_active = 0", [])?;
        conn.execute(
            "UPDATE profiles SET is_active = 1 WHERE id = ?1",
            params![id],
        )?;
        Ok(())
    })();
    match res {
        Ok(()) => {
            conn.execute("COMMIT", [])?;
            Ok(())
        }
        Err(e) => {
            conn.execute("ROLLBACK", [])?;
            Err(e)
        }
    }
}

pub fn active_profile(conn: &Connection) -> anyhow::Result<Profile> {
    let mut stmt = conn.prepare(
        "SELECT id, name, slug, is_active, created_at FROM profiles WHERE is_active = 1",
    )?;
    let mut rows = stmt.query_map([], row_to_profile)?;
    if let Some(p) = rows.next().transpose()? {
        return Ok(p);
    }

    let mut stmt = conn.prepare(
        "SELECT id, name, slug, is_active, created_at FROM profiles ORDER BY id LIMIT 1",
    )?;
    let mut rows = stmt.query_map([], row_to_profile)?;
    if let Some(p) = rows.next().transpose()? {
        set_active_profile(conn, p.id)?;
        return Ok(p);
    }

    let id = create_profile(conn, "default")?;
    set_active_profile(conn, id)?;
    get_profile_by_id(conn, id)?
        .ok_or_else(|| anyhow::anyhow!("No se pudo crear el perfil default"))
}

pub fn copy_profile(conn: &Connection, src_id: i64, new_name: &str) -> anyhow::Result<i64> {
    let src = get_profile_by_id(conn, src_id)?
        .ok_or_else(|| anyhow::anyhow!("Perfil origen no encontrado."))?;
    if new_name.trim().is_empty() {
        anyhow::bail!("El nombre del perfil no puede estar vacío.");
    }
    let slug = unique_slug(conn, new_name)?;
    conn.execute(
        "INSERT INTO profiles (name, slug) VALUES (?1, ?2)",
        params![new_name.trim(), slug],
    )?;
    let new_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO profile_mods (profile_id, mod_id, enabled, load_order)
         SELECT ?1, mod_id, enabled, load_order FROM profile_mods WHERE profile_id = ?2",
        params![new_id, src.id],
    )?;
    Ok(new_id)
}

pub fn profile_mod_count(conn: &Connection, profile_id: i64) -> anyhow::Result<(i64, i64)> {
    let total: i64 = conn.query_row(
        "SELECT COUNT(*) FROM profile_mods WHERE profile_id = ?1",
        params![profile_id],
        |row| row.get(0),
    )?;
    let enabled: i64 = conn.query_row(
        "SELECT COUNT(*) FROM profile_mods WHERE profile_id = ?1 AND enabled = 1",
        params![profile_id],
        |row| row.get(0),
    )?;
    Ok((total, enabled))
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

/// Current schema version. Every new migration step bumps it.
const SCHEMA_VERSION: i64 = 1;

pub fn run_migrations(conn: &Connection) -> anyhow::Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    if version < 1 {
        bootstrap_to_v1(conn)?;
        conn.pragma_update(None, "user_version", 1)?;
    }

    // Future steps (add a numbered function per step):
    // if version < 2 {
    //     migrate_to_v2(conn)?;
    //     conn.pragma_update(None, "user_version", 2)?;
    // }

    debug_assert!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap_or(0)
            == SCHEMA_VERSION
    );
    Ok(())
}

/// Applies the base schema and the legacy migrations that bring any
/// pre-versioned database up to v1. Introspection-based, so it is idempotent
/// for fresh databases, databases from before profiles, and v1 databases.
fn bootstrap_to_v1(conn: &Connection) -> anyhow::Result<()> {
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
        log::info("Migrando schema: añadiendo ON DELETE CASCADE en mod_dependencies...");
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
            log::info("Migrando schema: añadiendo restricción ':' en folder_name...");
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

    let has_required: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM pragma_table_info('mod_dependencies')
             WHERE name = 'required'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(false);

    if !has_required {
        log::info("Migrando schema: añadiendo columna 'required' en mod_dependencies...");
        conn.execute_batch(
            "ALTER TABLE mod_dependencies
             ADD COLUMN required INTEGER NOT NULL DEFAULT 1 CHECK(required IN (0, 1));",
        )?;
        log::info("[+] Migración completada.");
    }

    let profiles_exist: i64 = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'profiles'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if profiles_exist > 0 {
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM profiles", [], |row| row.get(0))?;
        if count == 0 {
            log::info("Migrando schema: creando perfil 'default'...");
            conn.execute(
                "INSERT INTO profiles (name, slug, is_active) VALUES ('default', 'default', 1)",
                [],
            )?;
            log::info("[+] Perfil 'default' creado.");
        } else {
            let active: i64 = conn.query_row(
                "SELECT COUNT(*) FROM profiles WHERE is_active = 1",
                [],
                |row| row.get(0),
            )?;
            if active == 0 {
                let first: i64 =
                    conn.query_row("SELECT id FROM profiles ORDER BY id LIMIT 1", [], |row| {
                        row.get(0)
                    })?;
                conn.execute(
                    "UPDATE profiles SET is_active = 1 WHERE id = ?1",
                    params![first],
                )?;
            }
        }
    }

    let has_enabled_col: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('mods') WHERE name = 'enabled'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if has_enabled_col > 0 {
        log::info("Migrando schema: moviendo estados a perfiles...");
        conn.execute_batch(
            "PRAGMA foreign_keys = OFF;
             BEGIN;
             INSERT INTO profile_mods (profile_id, mod_id, enabled, load_order)
                 SELECT p.id, m.id, m.enabled, m.load_order
                 FROM mods m JOIN profiles p ON p.slug = 'default';
             CREATE TABLE mods_new (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 folder_name TEXT NOT NULL UNIQUE,
                 name TEXT NOT NULL CHECK(length(name) > 0),
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
             INSERT INTO mods_new (id, folder_name, name) SELECT id, folder_name, name FROM mods;
             DROP TABLE mods;
             ALTER TABLE mods_new RENAME TO mods;
             UPDATE sqlite_sequence SET seq = (SELECT MAX(id) FROM mods) WHERE name = 'mods';
             COMMIT;
             PRAGMA foreign_keys = ON;",
        )?;
        log::info("[+] Migración completada.");
    }

    Ok(())
}

pub fn load_all_mods(conn: &Connection) -> anyhow::Result<Vec<ModIdentity>> {
    let mut stmt = conn.prepare("SELECT id, folder_name, name FROM mods")?;
    let mods = stmt
        .query_map([], |row| {
            Ok(ModIdentity {
                id: row.get(0)?,
                folder_name: row.get(1)?,
                name: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(mods)
}

pub fn load_all_mods_for_profile(
    conn: &Connection,
    profile_id: i64,
) -> anyhow::Result<Vec<ModEntry>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.folder_name, m.name,
                COALESCE(pm.enabled, 0), COALESCE(pm.load_order, 0)
         FROM mods m
         LEFT JOIN profile_mods pm ON pm.mod_id = m.id AND pm.profile_id = ?1
         ORDER BY pm.load_order DESC",
    )?;
    let mods = stmt
        .query_map(params![profile_id], |row| {
            Ok(ModEntry {
                id: row.get(0)?,
                folder_name: row.get(1)?,
                name: row.get(2)?,
                enabled: row.get::<_, i64>(3)? != 0,
                load_order: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(mods)
}

pub fn load_dependencies(conn: &Connection) -> anyhow::Result<HashMap<i64, Vec<DepRef>>> {
    let mut stmt = conn.prepare("SELECT mod_id, dependency_id, required FROM mod_dependencies")?;
    let mut deps: HashMap<i64, Vec<DepRef>> = HashMap::new();
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)? != 0,
        ))
    })?;

    for row in rows {
        let (mod_id, dep_id, required) = row?;
        deps.entry(mod_id).or_default().push(DepRef {
            id: dep_id,
            required,
        });
    }

    Ok(deps)
}

pub fn load_enabled_mod_ids_for_profile(
    conn: &Connection,
    profile_id: i64,
) -> anyhow::Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT mod_id FROM profile_mods
         WHERE profile_id = ?1 AND enabled = 1
         ORDER BY load_order DESC",
    )?;
    let ids = stmt
        .query_map(params![profile_id], |row| row.get(0))?
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

pub fn add_mod_to_all_profiles(
    conn: &Connection,
    folder_name: &str,
    display_name: &str,
) -> anyhow::Result<i64> {
    conn.execute(
        "INSERT INTO mods (folder_name, name) VALUES (?1, ?2)",
        params![folder_name, display_name],
    )?;
    let mod_id = conn.last_insert_rowid();

    let profiles = list_profiles(conn)?;
    for p in &profiles {
        let order: i64 = conn.query_row(
            "SELECT COALESCE(MAX(load_order), 0) + 10 FROM profile_mods WHERE profile_id = ?1",
            params![p.id],
            |row| row.get(0),
        )?;
        conn.execute(
            "INSERT INTO profile_mods (profile_id, mod_id, enabled, load_order)
             VALUES (?1, ?2, 0, ?3)",
            params![p.id, mod_id, order],
        )?;
    }

    Ok(mod_id)
}

pub fn remove_mod(conn: &Connection, id: i64) -> anyhow::Result<()> {
    conn.execute("DELETE FROM mods WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn set_mod_enabled(
    conn: &Connection,
    profile_id: i64,
    mod_id: i64,
    enabled: bool,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO profile_mods (profile_id, mod_id, enabled, load_order)
         VALUES (?1, ?2, ?3, 0)
         ON CONFLICT(profile_id, mod_id) DO UPDATE SET enabled = excluded.enabled",
        params![profile_id, mod_id, enabled as i64],
    )?;
    Ok(())
}

pub fn profile_mod_state(
    conn: &Connection,
    profile_id: i64,
    mod_id: i64,
) -> anyhow::Result<(bool, i64)> {
    let mut stmt = conn.prepare(
        "SELECT enabled, load_order FROM profile_mods WHERE profile_id = ?1 AND mod_id = ?2",
    )?;
    let mut rows = stmt.query_map(params![profile_id, mod_id], |row| {
        Ok((row.get::<_, i64>(0)? != 0, row.get::<_, i64>(1)?))
    })?;
    Ok(rows.next().transpose()?.unwrap_or((false, 0)))
}

pub fn disable_mod_all_profiles(conn: &Connection, mod_id: i64) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE profile_mods SET enabled = 0 WHERE mod_id = ?1",
        params![mod_id],
    )?;
    Ok(())
}

pub fn set_mod_order(
    conn: &Connection,
    profile_id: i64,
    mod_id: i64,
    order: i64,
) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO profile_mods (profile_id, mod_id, enabled, load_order)
         VALUES (?1, ?2, 0, ?3)
         ON CONFLICT(profile_id, mod_id) DO UPDATE SET load_order = excluded.load_order",
        params![profile_id, mod_id, order],
    )?;
    Ok(())
}

pub fn set_mod_name(conn: &Connection, id: i64, name: &str) -> anyhow::Result<()> {
    conn.execute("UPDATE mods SET name = ?1 WHERE id = ?2", params![name, id])?;
    Ok(())
}

pub fn set_mod_folder(conn: &Connection, id: i64, folder: &str) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE mods SET folder_name = ?1 WHERE id = ?2",
        params![folder, id],
    )?;
    Ok(())
}

pub fn get_mod_by_id(conn: &Connection, id: i64) -> anyhow::Result<Option<ModIdentity>> {
    let mut stmt = conn.prepare("SELECT id, folder_name, name FROM mods WHERE id = ?1")?;
    let mut rows = stmt.query_map(params![id], |row| {
        Ok(ModIdentity {
            id: row.get(0)?,
            folder_name: row.get(1)?,
            name: row.get(2)?,
        })
    })?;
    Ok(rows.next().transpose()?)
}

pub fn get_mod_by_folder(conn: &Connection, folder: &str) -> anyhow::Result<Option<ModIdentity>> {
    let mut stmt = conn.prepare("SELECT id, folder_name, name FROM mods WHERE folder_name = ?1")?;
    let mut rows = stmt.query_map(params![folder], |row| {
        Ok(ModIdentity {
            id: row.get(0)?,
            folder_name: row.get(1)?,
            name: row.get(2)?,
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
        if let Some(m) = get_mod_by_folder(conn, ident)? {
            return Ok(m.id);
        }
        anyhow::bail!("Mod con id={} no encontrado.", id);
    }

    let m = get_mod_by_folder(conn, ident)?;
    m.map(|m| m.id)
        .ok_or_else(|| anyhow::anyhow!("Mod '{}' no encontrado.", ident))
}

pub fn add_dependency(
    conn: &Connection,
    mod_id: i64,
    dep_id: i64,
    required: bool,
) -> anyhow::Result<()> {
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
        "INSERT INTO mod_dependencies (mod_id, dependency_id, required) VALUES (?1, ?2, ?3)",
        params![mod_id, dep_id, required as i64],
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

pub fn get_dependencies_of(
    conn: &Connection,
    profile_id: i64,
    mod_id: i64,
) -> anyhow::Result<Vec<(ModEntry, bool)>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.folder_name, m.name,
                COALESCE(pm.enabled, 0), COALESCE(pm.load_order, 0), d.required
         FROM mod_dependencies d
         JOIN mods m ON d.dependency_id = m.id
         LEFT JOIN profile_mods pm ON pm.mod_id = m.id AND pm.profile_id = ?1
         WHERE d.mod_id = ?2 ORDER BY pm.load_order DESC",
    )?;
    let rows = stmt.query_map(params![profile_id, mod_id], |row| {
        Ok((
            ModEntry {
                id: row.get(0)?,
                folder_name: row.get(1)?,
                name: row.get(2)?,
                enabled: row.get::<_, i64>(3)? != 0,
                load_order: row.get(4)?,
            },
            row.get::<_, i64>(5)? != 0,
        ))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn get_dependents_of(
    conn: &Connection,
    profile_id: i64,
    mod_id: i64,
) -> anyhow::Result<Vec<ModEntry>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.folder_name, m.name,
                COALESCE(pm.enabled, 0), COALESCE(pm.load_order, 0)
         FROM mod_dependencies d
         JOIN mods m ON d.mod_id = m.id
         LEFT JOIN profile_mods pm ON pm.mod_id = m.id AND pm.profile_id = ?1
         WHERE d.dependency_id = ?2 ORDER BY pm.load_order DESC",
    )?;
    let rows = stmt.query_map(params![profile_id, mod_id], |row| {
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

pub fn discover_mods(conn: &Connection, mods_dir: &Path) -> anyhow::Result<(usize, usize)> {
    if !mods_dir.exists() {
        std::fs::create_dir_all(mods_dir)?;
        log::info(format!(
            "    [-] Directorio de mods creado: {}",
            mods_dir.display()
        ));
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
    let db_folders: Vec<String> = all_mods.iter().map(|m| m.folder_name.clone()).collect();

    for folder in &disk_folders {
        if !db_folders.contains(folder) {
            let display_name = folder.replace('_', " ");

            match add_mod_to_all_profiles(conn, folder, &display_name) {
                Ok(_id) => {
                    log::info(format!("    [+] Nuevo: {folder} -> '{display_name}'"));
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
            if let Some(m) = all_mods.iter().find(|m| &m.folder_name == db_folder) {
                let _ = disable_mod_all_profiles(conn, m.id);
                log::warn(format!(
                    "    [!] Huérfano desactivado: '{}' (carpeta eliminada del disco)",
                    db_folder
                ));
                orphan_count += 1;
            }
        }
    }

    if new_count > 0 {
        log::info(format!("[+] {new_count} mod(s) nuevo(s) registrado(s)."));
    }
    if orphan_count > 0 {
        log::warn(format!(
            "[!] {orphan_count} mod(s) huérfano(s) detectado(s)."
        ));
    }

    Ok((new_count, orphan_count))
}

pub fn clean_orphans(conn: &Connection, mods_dir: &Path) -> anyhow::Result<usize> {
    let all_mods = load_all_mods(conn)?;
    let mut deleted = 0usize;

    for m in &all_mods {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn
    }

    /// Old pre-profiles schema (enabled/load_order in mods), matching the
    /// schema.sql as it existed before the profiles feature.
    fn build_old_schema(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE mods (
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
            CREATE TABLE mod_dependencies (
                mod_id INTEGER NOT NULL,
                dependency_id INTEGER NOT NULL,
                required INTEGER NOT NULL DEFAULT 1 CHECK(required IN (0, 1)),
                PRIMARY KEY (mod_id, dependency_id),
                FOREIGN KEY (mod_id) REFERENCES mods(id) ON DELETE CASCADE,
                FOREIGN KEY (dependency_id) REFERENCES mods(id) ON DELETE CASCADE,
                CHECK(mod_id != dependency_id)
            );
            INSERT INTO mods (folder_name, name, enabled, load_order) VALUES ('m1', 'M1', 1, 20);
            INSERT INTO mods (folder_name, name, enabled, load_order) VALUES ('m2', 'M2', 0, 10);
            INSERT INTO mod_dependencies (mod_id, dependency_id, required) VALUES (1, 2, 1);
        ",
        )
        .unwrap();
    }

    #[test]
    fn migration_from_old_schema_preserves_states() {
        let conn = mem_conn();
        build_old_schema(&conn);
        run_migrations(&conn).unwrap();

        let p = active_profile(&conn).unwrap();
        assert_eq!(p.slug, "default");
        assert!(p.is_active);

        let mods = load_all_mods_for_profile(&conn, p.id).unwrap();
        let m1 = mods.iter().find(|m| m.folder_name == "m1").unwrap();
        assert!(m1.enabled);
        assert_eq!(m1.load_order, 20);
        let m2 = mods.iter().find(|m| m.folder_name == "m2").unwrap();
        assert!(!m2.enabled);
        assert_eq!(m2.load_order, 10);

        let deps = load_dependencies(&conn).unwrap();
        assert!(deps.get(&1).map(|d| d.len()).unwrap_or(0) == 1);
        assert!(deps.get(&1).unwrap()[0].required);

        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();
    }

    #[test]
    fn slugify_is_kebab_and_stable() {
        assert_eq!(slugify("Vanilla Play"), "vanilla-play");
        assert_eq!(slugify("  Graphics   Mods  "), "graphics-mods");
        assert_eq!(slugify("A.B/C_1"), "abc-1");
        assert_eq!(slugify("---"), "");
    }

    #[test]
    fn profile_lifecycle_and_isolation() {
        let conn = mem_conn();
        run_migrations(&conn).unwrap();

        add_mod_to_all_profiles(&conn, "m1", "M1").unwrap();
        add_mod_to_all_profiles(&conn, "m2", "M2").unwrap();

        let p1 = active_profile(&conn).unwrap();
        set_mod_enabled(&conn, p1.id, 1, true).unwrap();
        set_mod_order(&conn, p1.id, 1, 50).unwrap();
        assert_eq!(profile_mod_state(&conn, p1.id, 1).unwrap(), (true, 50));

        let id2 = create_profile(&conn, "Second").unwrap();
        let p2 = get_profile_by_id(&conn, id2).unwrap().unwrap();
        assert_eq!(p2.slug, "second");
        let (enabled2, _) = profile_mod_state(&conn, p2.id, 1).unwrap();
        assert!(!enabled2, "perfil nuevo arranca con mods desactivados");

        assert!(active_profile(&conn).unwrap().id == p1.id);
        set_active_profile(&conn, p2.id).unwrap();
        assert!(active_profile(&conn).unwrap().id == p2.id);
        set_active_profile(&conn, p1.id).unwrap();

        let cid = copy_profile(&conn, p1.id, "Copy").unwrap();
        assert_eq!(profile_mod_state(&conn, cid, 1).unwrap(), (true, 50));

        delete_profile(&conn, p2.id).unwrap();
        delete_profile(&conn, cid).unwrap();
        assert!(
            delete_profile(&conn, p1.id).is_err(),
            "no se puede borrar el último"
        );

        rename_profile(&conn, p1.id, "Renamed").unwrap();
        let pr = get_profile_by_id(&conn, p1.id).unwrap().unwrap();
        assert_eq!(pr.name, "Renamed");
        assert_eq!(pr.slug, "default");
    }

    #[test]
    fn new_mod_registered_in_all_profiles() {
        let conn = mem_conn();
        run_migrations(&conn).unwrap();
        create_profile(&conn, "Vanilla").unwrap();

        add_mod_to_all_profiles(&conn, "newmod", "New Mod").unwrap();

        for p in list_profiles(&conn).unwrap() {
            let state = profile_mod_state(&conn, p.id, 1).unwrap();
            assert!(!state.0, "nuevo mod registrado desactivado en cada perfil");
            assert!(state.1 > 0, "nuevo mod registrado con orden auto");
        }
    }
}
