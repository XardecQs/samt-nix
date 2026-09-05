use crate::log;
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};
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

/// Cached mod metadata, populated from `mod.toml` on every `discover`.
#[derive(Debug, Clone, Default)]
pub struct ModMetaCache {
    /// Stable `author:slug` identifier.
    pub mod_id: Option<String>,
    pub version: Option<String>,
    pub author: Vec<String>,
    pub url: Option<String>,
    pub description: Option<String>,
    pub cover: Option<String>,
    pub mount: Vec<String>,
    pub guides: Vec<String>,
    pub tags: Vec<String>,
    /// Bundled components of a composite pack.
    pub components: Vec<crate::meta::MetaComponent>,
}

impl ModMetaCache {
    /// True when the mod is a composite pack (has bundled components).
    pub fn is_pack(&self) -> bool {
        !self.components.is_empty()
    }
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

#[derive(Debug, Clone)]
pub struct Group {
    pub id: i64,
    pub name: String,
    pub slug: String,
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

/// Every profile and whether `mod_id` is enabled in it (for `ctl info`).
pub fn mod_enabled_in_profiles(
    conn: &Connection,
    mod_id: i64,
) -> anyhow::Result<Vec<(Profile, bool)>> {
    let mut stmt = conn.prepare(
        "SELECT p.id, p.name, p.slug, p.is_active, p.created_at,
                COALESCE(pm.enabled, 0)
         FROM profiles p
         LEFT JOIN profile_mods pm ON pm.profile_id = p.id AND pm.mod_id = ?1
         ORDER BY p.id",
    )?;
    let rows = stmt.query_map(params![mod_id], |row| {
        Ok((row_to_profile(row)?, row.get::<_, i64>(5)? != 0))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

// ---------- Export / import helpers ----------

/// Inserts a profile with an explicit slug (used by `ctl import`). Callers
/// must ensure the slug is free.
pub fn insert_profile(
    conn: &Connection,
    name: &str,
    slug: &str,
    is_active: bool,
) -> anyhow::Result<i64> {
    conn.execute(
        "INSERT INTO profiles (name, slug, is_active) VALUES (?1, ?2, ?3)",
        params![name, slug, is_active as i64],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Inserts a mod without touching profile states (used by `ctl import`).
pub fn insert_mod(conn: &Connection, folder: &str, name: &str) -> anyhow::Result<i64> {
    conn.execute(
        "INSERT INTO mods (folder_name, name) VALUES (?1, ?2)",
        params![folder, name],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Inserts a group with an explicit slug (used by `ctl import`).
pub fn insert_group(conn: &Connection, name: &str, slug: &str) -> anyhow::Result<i64> {
    conn.execute(
        "INSERT INTO groups (name, slug) VALUES (?1, ?2)",
        params![name, slug],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Writes a metadata cache entry directly (used by `ctl import`).
pub fn set_mod_meta_cache(conn: &Connection, id: i64, cache: &ModMetaCache) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE mods SET mod_id = ?1, version = ?2, author = ?3, url = ?4, description = ?5,
         cover = ?6, mount = ?7, guides = ?8, tags = ?9, components = ?10 WHERE id = ?11",
        params![
            cache.mod_id,
            cache.version,
            json_vec(&cache.author),
            cache.url,
            cache.description,
            cache.cover,
            json_vec(&cache.mount),
            json_vec(&cache.guides),
            json_vec(&cache.tags),
            json_components(&cache.components),
            id,
        ],
    )?;
    Ok(())
}

/// (mod folder, dependency folder, required) for the whole dependency graph.
pub fn export_dependencies(conn: &Connection) -> anyhow::Result<Vec<(String, String, bool)>> {
    let mut stmt = conn.prepare(
        "SELECT m.folder_name, d.folder_name, md.required
         FROM mod_dependencies md
         JOIN mods m ON m.id = md.mod_id
         JOIN mods d ON d.id = md.dependency_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)? != 0))
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

/// (group slug, mod folder, optional profile slug) for every membership.
pub fn export_mod_groups(
    conn: &Connection,
) -> anyhow::Result<Vec<(String, String, Option<String>)>> {
    let mut stmt = conn.prepare(
        "SELECT g.slug, m.folder_name, p.slug
         FROM mod_groups mg
         JOIN groups g ON g.id = mg.group_id
         JOIN mods m ON m.id = mg.mod_id
         LEFT JOIN profiles p ON p.id = mg.profile_id",
    )?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

// ---------- Groups ----------

fn row_to_group(row: &rusqlite::Row) -> rusqlite::Result<Group> {
    Ok(Group {
        id: row.get(0)?,
        name: row.get(1)?,
        slug: row.get(2)?,
        created_at: row.get(3)?,
    })
}

fn unique_group_slug(conn: &Connection, base: &str) -> anyhow::Result<String> {
    let base = slugify(base);
    let base = if base.is_empty() {
        "g".to_string()
    } else {
        base
    };
    let exists = |slug: &str| -> anyhow::Result<bool> {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM groups WHERE slug = ?1",
            params![slug],
            |row| row.get(0),
        )?;
        Ok(n > 0)
    };
    if !exists(&base)? {
        return Ok(base);
    }
    for i in 2.. {
        let cand = format!("{base}-{i}");
        if !exists(&cand)? {
            return Ok(cand);
        }
    }
    unreachable!("unique_group_slug agotó candidatos")
}

pub fn create_group(conn: &Connection, name: &str) -> anyhow::Result<i64> {
    if name.trim().is_empty() {
        anyhow::bail!("El nombre del grupo no puede estar vacío.");
    }
    let slug = unique_group_slug(conn, name)?;
    conn.execute(
        "INSERT INTO groups (name, slug) VALUES (?1, ?2)",
        params![name.trim(), slug],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn get_group_by_id(conn: &Connection, id: i64) -> anyhow::Result<Option<Group>> {
    let mut stmt = conn.prepare("SELECT id, name, slug, created_at FROM groups WHERE id = ?1")?;
    let mut rows = stmt.query_map(params![id], row_to_group)?;
    Ok(rows.next().transpose()?)
}

pub fn get_group_by_slug(conn: &Connection, slug: &str) -> anyhow::Result<Option<Group>> {
    let mut stmt = conn.prepare("SELECT id, name, slug, created_at FROM groups WHERE slug = ?1")?;
    let mut rows = stmt.query_map(params![slug], row_to_group)?;
    Ok(rows.next().transpose()?)
}

pub fn get_group_by_name(conn: &Connection, name: &str) -> anyhow::Result<Option<Group>> {
    let mut stmt = conn.prepare("SELECT id, name, slug, created_at FROM groups WHERE name = ?1")?;
    let mut rows = stmt.query_map(params![name], row_to_group)?;
    Ok(rows.next().transpose()?)
}

pub fn resolve_group(conn: &Connection, ident: &str) -> anyhow::Result<Group> {
    if let Ok(id) = ident.parse::<i64>() {
        if let Some(g) = get_group_by_id(conn, id)? {
            return Ok(g);
        }
    }
    if let Some(g) = get_group_by_slug(conn, ident)? {
        return Ok(g);
    }
    if let Some(g) = get_group_by_name(conn, ident)? {
        return Ok(g);
    }
    anyhow::bail!("Grupo '{}' no encontrado.", ident)
}

pub fn list_groups(conn: &Connection) -> anyhow::Result<Vec<Group>> {
    let mut stmt = conn.prepare("SELECT id, name, slug, created_at FROM groups ORDER BY name")?;
    let rows = stmt.query_map([], row_to_group)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn rename_group(conn: &Connection, id: i64, new_name: &str) -> anyhow::Result<()> {
    if new_name.trim().is_empty() {
        anyhow::bail!("El nombre del grupo no puede estar vacío.");
    }
    conn.execute(
        "UPDATE groups SET name = ?1 WHERE id = ?2",
        params![new_name.trim(), id],
    )?;
    Ok(())
}

pub fn delete_group(conn: &Connection, id: i64) -> anyhow::Result<()> {
    conn.execute("PRAGMA foreign_keys = ON", [])?;
    conn.execute("DELETE FROM groups WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn group_mod_count(conn: &Connection, group_id: i64) -> anyhow::Result<i64> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM mod_groups WHERE group_id = ?1",
        params![group_id],
        |row| row.get(0),
    )?)
}

/// Adds a mod to a group. `profile_id = None` means the membership is global
/// (applies to every profile). Returns `false` when it already existed.
pub fn add_group_membership(
    conn: &Connection,
    group_id: i64,
    mod_id: i64,
    profile_id: Option<i64>,
) -> anyhow::Result<bool> {
    let exists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM mod_groups
         WHERE group_id = ?1 AND mod_id = ?2 AND profile_id IS ?3",
        params![group_id, mod_id, profile_id],
        |row| row.get(0),
    )?;
    if exists > 0 {
        return Ok(false);
    }
    conn.execute(
        "INSERT INTO mod_groups (group_id, mod_id, profile_id) VALUES (?1, ?2, ?3)",
        params![group_id, mod_id, profile_id],
    )?;
    Ok(true)
}

pub fn remove_group_membership(
    conn: &Connection,
    group_id: i64,
    mod_id: i64,
    profile_id: Option<i64>,
) -> anyhow::Result<bool> {
    let n = conn.execute(
        "DELETE FROM mod_groups WHERE group_id = ?1 AND mod_id = ?2 AND profile_id IS ?3",
        params![group_id, mod_id, profile_id],
    )?;
    Ok(n > 0)
}

/// Mod ids that belong to `group_id`, considering global memberships plus the
/// given profile's own memberships.
pub fn mods_in_group(
    conn: &Connection,
    group_id: i64,
    profile_id: i64,
) -> anyhow::Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT mod_id FROM mod_groups
         WHERE group_id = ?1 AND (profile_id IS NULL OR profile_id = ?2)",
    )?;
    let rows = stmt.query_map(params![group_id, profile_id], |row| row.get(0))?;
    rows.collect::<Result<Vec<i64>, _>>().map_err(Into::into)
}

/// Groups a mod belongs to (global memberships plus the profile's own).
pub fn groups_of_mod_in_profile(
    conn: &Connection,
    mod_id: i64,
    profile_id: i64,
) -> anyhow::Result<Vec<Group>> {
    let mut stmt = conn.prepare(
        "SELECT g.id, g.name, g.slug, g.created_at
         FROM groups g
         JOIN mod_groups mg ON mg.group_id = g.id
         WHERE mg.mod_id = ?1 AND (mg.profile_id IS NULL OR mg.profile_id = ?2)
         ORDER BY g.name",
    )?;
    let rows = stmt.query_map(params![mod_id, profile_id], row_to_group)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
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
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA journal_mode = WAL;
         PRAGMA busy_timeout = 5000;",
    )?;
    Ok(conn)
}

/// Current schema version. Every new migration step bumps it.
const SCHEMA_VERSION: i64 = 6;

/// Applies any pending schema migration. The whole chain runs inside a single
/// transaction: a failure rolls everything back and `user_version` is only
/// bumped to the target once the work is committed.
pub fn run_migrations(conn: &Connection) -> anyhow::Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    if version >= SCHEMA_VERSION {
        return Ok(());
    }

    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> anyhow::Result<()> {
        if version < 1 {
            bootstrap_to_v1(conn)?;
        }
        if version < 2 {
            migrate_to_v2(conn)?;
        }
        if version < 3 {
            migrate_to_v3(conn)?;
        }
        if version < 4 {
            migrate_to_v4(conn)?;
        }
        if version < 5 {
            migrate_to_v5(conn)?;
        }
        if version < 6 {
            migrate_to_v6(conn)?;
        }
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Deduplicates global group memberships (keeping the lowest rowid) and adds a
/// partial unique index so the database itself rejects duplicate `(group, mod)`
/// rows where `profile_id IS NULL`.
fn migrate_to_v6(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "DELETE FROM mod_groups WHERE profile_id IS NULL AND rowid NOT IN (
            SELECT MIN(rowid) FROM mod_groups WHERE profile_id IS NULL GROUP BY group_id, mod_id
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_mod_groups_global
            ON mod_groups(group_id, mod_id) WHERE profile_id IS NULL;",
    )?;
    Ok(())
}

/// Creates the `groups` and `mod_groups` tables (idempotent).
fn migrate_to_v5(conn: &Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS groups (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            name TEXT NOT NULL UNIQUE,
            slug TEXT NOT NULL UNIQUE,
            created_at TEXT DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS mod_groups (
            group_id INTEGER NOT NULL,
            mod_id INTEGER NOT NULL,
            profile_id INTEGER,
            PRIMARY KEY (group_id, mod_id, profile_id),
            FOREIGN KEY (group_id) REFERENCES groups(id) ON DELETE CASCADE,
            FOREIGN KEY (mod_id) REFERENCES mods(id) ON DELETE CASCADE,
            FOREIGN KEY (profile_id) REFERENCES profiles(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_mod_groups_mod_id ON mod_groups(mod_id);
        CREATE INDEX IF NOT EXISTS idx_mod_groups_profile_id ON mod_groups(profile_id);",
    )?;
    Ok(())
}

/// Adds the `components` cache column (JSON list of bundled components).
/// Introspection-based and idempotent.
fn migrate_to_v4(conn: &Connection) -> anyhow::Result<()> {
    let cols: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('mods')")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    if !cols.iter().any(|c| c.as_str() == "components") {
        log::info("Migrando schema: ALTER TABLE mods ADD COLUMN components TEXT");
        conn.execute("ALTER TABLE mods ADD COLUMN components TEXT", [])?;
    }
    Ok(())
}

/// Adds the stable `mod_id` (author:slug) and `tags` columns plus a unique
/// (nullable) index on `mod_id`. Introspection-based and idempotent.
fn migrate_to_v3(conn: &Connection) -> anyhow::Result<()> {
    let cols: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('mods')")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;

    for (name, sql) in [
        ("mod_id", "ALTER TABLE mods ADD COLUMN mod_id TEXT"),
        ("tags", "ALTER TABLE mods ADD COLUMN tags TEXT"),
    ] {
        if !cols.iter().any(|c| c.as_str() == name) {
            log::info(format!("Migrando schema: {sql}"));
            conn.execute(sql, [])?;
        }
    }
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_mods_mod_id ON mods(mod_id)",
        [],
    )?;
    Ok(())
}

/// Adds the mod metadata cache columns (version, author, url, description,
/// cover, mount, guides) to `mods`. Introspection-based, so it is idempotent
/// and works for fresh databases and pre-v2 ones alike.
fn migrate_to_v2(conn: &Connection) -> anyhow::Result<()> {
    let cols: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('mods')")?
        .query_map([], |row| row.get(0))?
        .collect::<Result<_, _>>()?;

    let additions = [
        ("version", "ALTER TABLE mods ADD COLUMN version TEXT"),
        ("author", "ALTER TABLE mods ADD COLUMN author TEXT"),
        ("url", "ALTER TABLE mods ADD COLUMN url TEXT"),
        (
            "description",
            "ALTER TABLE mods ADD COLUMN description TEXT",
        ),
        ("cover", "ALTER TABLE mods ADD COLUMN cover TEXT"),
        ("mount", "ALTER TABLE mods ADD COLUMN mount TEXT"),
        ("guides", "ALTER TABLE mods ADD COLUMN guides TEXT"),
    ];

    for (name, sql) in additions {
        if cols.iter().any(|c| c.as_str() == name) {
            continue;
        }
        log::info(format!("Migrando schema: {sql}"));
        conn.execute(sql, [])?;
    }
    Ok(())
}

/// Installs the schema on a brand-new database and makes sure a `default`
/// profile exists and is active. Databases created by pre-versioning builds of
/// gta-mo (the old bash/early-Rust schema) are refused with a clear message
/// instead of being migrated: no real databases of that era remain and their
/// migration paths relied on fragile table rebuilds.
fn bootstrap_to_v1(conn: &Connection) -> anyhow::Result<()> {
    let existing_tables: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'table' AND name NOT LIKE 'sqlite_%'",
        [],
        |row| row.get(0),
    )?;

    if existing_tables > 0 {
        anyhow::bail!(
            "Base de datos de una versión antigua de gta-mo (formato sin versionar). \
             La migración automática desde ese formato ya no está soportada.\n  \
             Haz una copia de seguridad y elimina el archivo: {}\n  \
             En la próxima ejecución se creará una base de datos nueva; los mods se \
             vuelven a registrar desde mods/ (auto_discover o `--discover`).",
            crate::config::db_path().display()
        );
    }

    let schema = include_str!("../schema.sql");
    conn.execute_batch(schema)?;

    let count: i64 = conn.query_row("SELECT COUNT(*) FROM profiles", [], |row| row.get(0))?;
    if count == 0 {
        log::info("Creando perfil 'default'...");
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

/// Enables a mod in a profile together with its required dependencies
/// (transitively). `visited` guards against dependency cycles.
pub fn enable_mod_with_deps(
    conn: &Connection,
    profile_id: i64,
    mod_id: i64,
    visited: &mut std::collections::HashSet<i64>,
) -> anyhow::Result<()> {
    if !visited.insert(mod_id) {
        return Ok(());
    }
    set_mod_enabled(conn, profile_id, mod_id, true)?;
    let deps = get_dependencies_of(conn, profile_id, mod_id)?;
    for (d, required) in deps {
        if required {
            enable_mod_with_deps(conn, profile_id, d.id, visited)?;
        }
    }
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

fn json_list(v: &Option<Vec<String>>) -> Option<String> {
    v.as_ref()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::to_string(l).unwrap_or_default())
}

fn json_vec(v: &[String]) -> Option<String> {
    if v.is_empty() {
        None
    } else {
        Some(serde_json::to_string(v).unwrap_or_default())
    }
}

fn parse_json_list(raw: Option<String>) -> Vec<String> {
    raw.and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn parse_components(raw: Option<String>) -> Vec<crate::meta::MetaComponent> {
    raw.and_then(|s| serde_json::from_str::<Vec<crate::meta::MetaComponent>>(&s).ok())
        .unwrap_or_default()
}

fn json_components(v: &[crate::meta::MetaComponent]) -> Option<String> {
    if v.is_empty() {
        None
    } else {
        serde_json::to_string(v).ok()
    }
}

/// Parses the cached author column, which may be a JSON list (new format) or a
/// legacy plain string.
fn parse_authors(raw: Option<String>) -> Vec<String> {
    let Some(s) = raw else {
        return vec![];
    };
    if s.trim().is_empty() {
        return vec![];
    }
    serde_json::from_str::<Vec<String>>(&s)
        .or_else(|_| serde_json::from_str::<String>(&s).map(|one| vec![one]))
        .unwrap_or_else(|_| vec![s])
}

/// Builds a cache entry straight from a `mod.toml` manifest (file wins over the
/// database cache when displaying a mod).
pub fn meta_cache_from_meta(meta: &crate::meta::ModMeta) -> ModMetaCache {
    ModMetaCache {
        mod_id: meta.id.clone(),
        version: meta.version.clone(),
        author: meta.author.clone(),
        url: meta.url.clone(),
        description: meta.description.clone(),
        cover: meta.cover.clone(),
        mount: meta.mount.clone().unwrap_or_default(),
        guides: meta.guides.clone().unwrap_or_default(),
        tags: meta.tags.clone().unwrap_or_default(),
        components: meta.components.clone().unwrap_or_default(),
    }
}

/// Loads the cached metadata for a mod (empty default if none was discovered).
pub fn load_mod_meta(conn: &Connection, id: i64) -> anyhow::Result<ModMetaCache> {
    let mut stmt = conn.prepare(
        "SELECT mod_id, version, author, url, description, cover, mount, guides, tags, components
         FROM mods WHERE id = ?1",
    )?;
    let mut rows = stmt.query_map(params![id], |row| {
        Ok(ModMetaCache {
            mod_id: row.get(0)?,
            version: row.get(1)?,
            author: parse_authors(row.get(2)?),
            url: row.get(3)?,
            description: row.get(4)?,
            cover: row.get(5)?,
            mount: parse_json_list(row.get(6)?),
            guides: parse_json_list(row.get(7)?),
            tags: parse_json_list(row.get(8)?),
            components: parse_components(row.get(9)?),
        })
    })?;
    Ok(rows.next().transpose()?.unwrap_or_default())
}

/// Stores (or clears) the metadata cache for a mod from its `mod.toml`.
pub fn update_mod_meta(
    conn: &Connection,
    id: i64,
    meta: &Option<crate::meta::ModMeta>,
) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE mods SET mod_id = ?1, version = ?2, author = ?3, url = ?4, description = ?5,
         cover = ?6, mount = ?7, guides = ?8, tags = ?9, components = ?10 WHERE id = ?11",
        params![
            meta.as_ref().and_then(|m| m.id.clone()),
            meta.as_ref().and_then(|m| m.version.clone()),
            meta.as_ref().and_then(|m| json_vec(&m.author)),
            meta.as_ref().and_then(|m| m.url.clone()),
            meta.as_ref().and_then(|m| m.description.clone()),
            meta.as_ref().and_then(|m| m.cover.clone()),
            json_list(&meta.as_ref().and_then(|m| m.mount.clone())),
            json_list(&meta.as_ref().and_then(|m| m.guides.clone())),
            json_list(&meta.as_ref().and_then(|m| m.tags.clone())),
            meta.as_ref()
                .and_then(|m| json_components(m.components.as_deref().unwrap_or(&[]))),
            id,
        ],
    )?;
    Ok(())
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

/// Resolves a dependency reference from a manifest: first by stable `mod_id`
/// (`author:slug`), then by folder name (legacy). `None` if not found.
fn resolve_dep_ref(conn: &Connection, reference: &str) -> Option<i64> {
    if crate::meta::valid_mod_id(reference) {
        conn.query_row(
            "SELECT id FROM mods WHERE mod_id = ?1",
            params![reference],
            |row| row.get(0),
        )
        .ok()
    } else {
        get_mod_by_folder(conn, reference)
            .ok()
            .flatten()
            .map(|m| m.id)
    }
}

/// Replaces a manifest mod's dependency rows in `mod_dependencies` with the
/// `[dependencies]` section of its `mod.toml`. Mods without a manifest keep
/// their manually managed DB dependencies untouched.
fn sync_mod_dependencies(
    conn: &Connection,
    mod_id: i64,
    meta: &Option<crate::meta::ModMeta>,
) -> anyhow::Result<()> {
    let Some(meta) = meta else {
        return Ok(());
    };
    let Some(deps) = &meta.dependencies else {
        return Ok(());
    };

    let mut required: Vec<i64> = Vec::new();
    let mut optional: Vec<i64> = Vec::new();
    for (reference, is_optional) in deps
        .required
        .iter()
        .map(|r| (r, false))
        .chain(deps.optional.iter().map(|r| (r, true)))
    {
        match resolve_dep_ref(conn, reference) {
            Some(dep_id) if dep_id != mod_id => {
                if is_optional {
                    optional.push(dep_id);
                } else {
                    required.push(dep_id);
                }
            }
            Some(_) => log::warn(format!(
                "    [!] Dependencia cíclica ignorada: un mod no puede depender de sí mismo ('{reference}')"
            )),
            None => log::warn(format!(
                "    [!] Dependencia no resuelta: '{reference}' (mod no instalado o referencia inválida)"
            )),
        }
    }

    conn.execute(
        "DELETE FROM mod_dependencies WHERE mod_id = ?1",
        params![mod_id],
    )?;
    for dep_id in required {
        conn.execute(
            "INSERT OR IGNORE INTO mod_dependencies (mod_id, dependency_id, required)
             VALUES (?1, ?2, 1)",
            params![mod_id, dep_id],
        )?;
    }
    for dep_id in optional {
        conn.execute(
            "INSERT OR IGNORE INTO mod_dependencies (mod_id, dependency_id, required)
             VALUES (?1, ?2, 0)",
            params![mod_id, dep_id],
        )?;
    }
    Ok(())
}

/// Validates the stable id (format + uniqueness) for a mod, returning a copy
/// of the manifest with an invalid or duplicated id cleared.
fn validate_meta_id(
    conn: &Connection,
    mod_id: i64,
    folder: &str,
    meta: &Option<crate::meta::ModMeta>,
) -> Option<crate::meta::ModMeta> {
    let mut meta = meta.clone();
    if let Some(id) = meta.as_ref().and_then(|m| m.id.clone()) {
        if !crate::meta::valid_mod_id(&id) {
            log::warn(format!(
                "    [!] {folder}: id '{id}' inválido (formato autor:slug); se ignora"
            ));
            meta.as_mut().unwrap().id = None;
        } else {
            let taken: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM mods WHERE mod_id = ?1 AND id != ?2",
                    params![id, mod_id],
                    |row| row.get(0),
                )
                .unwrap_or(0);
            if taken > 0 {
                log::warn(format!(
                    "    [!] {folder}: id '{id}' ya lo usa otro mod; se ignora"
                ));
                meta.as_mut().unwrap().id = None;
            }
        }
    }
    meta
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
    let db_folders_set: HashSet<&String> = db_folders.iter().collect();
    let mods_by_folder: HashMap<&str, &ModIdentity> = all_mods
        .iter()
        .map(|m| (m.folder_name.as_str(), m))
        .collect();

    let mut metas: Vec<Option<crate::meta::ModMeta>> = Vec::with_capacity(disk_folders.len());

    // Phase 1: register mods and store their metadata (id, tags, mount...).
    for folder in &disk_folders {
        let meta = match crate::meta::read_mod_meta(mods_dir, folder) {
            Ok(meta) => meta,
            Err(e) => {
                log::warn(format!("    [!] {folder}: {e}"));
                None
            }
        };
        if let Some(m) = &meta {
            if let Some(mount) = &m.mount {
                for entry in mount {
                    if !crate::meta::valid_mount_entry(entry) {
                        log::warn(format!(
                            "    [!] {folder}: 'mount' con entrada inválida '{entry}' (se ignora)"
                        ));
                    } else if !mods_dir.join(folder).join(entry).is_dir() {
                        log::warn(format!(
                            "    [!] {folder}: 'mount' '{}' no existe en el disco",
                            entry
                        ));
                    }
                }
            }
            if let Some(components) = &m.components {
                for c in components {
                    if c.name.as_deref().unwrap_or("").trim().is_empty() {
                        log::warn(format!(
                            "    [!] {folder}: componente sin nombre; se ignora"
                        ));
                    }
                    if let Some(p) = &c.path {
                        if !crate::meta::valid_mount_entry(p) {
                            log::warn(format!(
                                "    [!] {folder}: componente '{}' con path inválido '{}'",
                                c.name.as_deref().unwrap_or("?"),
                                p
                            ));
                        }
                    }
                }
            }
        }

        if !db_folders_set.contains(folder) {
            let display_name = meta
                .as_ref()
                .and_then(|m| m.name.clone())
                .unwrap_or_else(|| folder.replace('_', " "));

            match add_mod_to_all_profiles(conn, folder, &display_name) {
                Ok(_id) => {
                    log::info(format!("    [+] Nuevo: {folder} -> '{display_name}'"));
                    new_count += 1;
                }
                Err(e) => {
                    log::warn(format!("    [!] Error al insertar: {folder}: {e}"));
                }
            }
        } else if let Some(m) = mods_by_folder.get(folder.as_str()) {
            if let Some(name) = meta.as_ref().and_then(|m| m.name.clone()) {
                set_mod_name(conn, m.id, &name)?;
            }
        }

        if let Some(m) = get_mod_by_folder(conn, folder)? {
            let validated = validate_meta_id(conn, m.id, folder, &meta);
            update_mod_meta(conn, m.id, &validated)?;
            metas.push(Some(validated.unwrap_or_default()));
        } else {
            metas.push(meta);
        }
    }

    // Phase 2: sync `[dependencies]` once every mod is registered, so a
    // reference to a mod discovered later in the same pass still resolves.
    for (folder, meta) in disk_folders.iter().zip(metas.iter()) {
        if let Some(m) = get_mod_by_folder(conn, folder)? {
            sync_mod_dependencies(conn, m.id, meta)?;
        }
    }

    let disk_folders_set: HashSet<&String> = disk_folders.iter().collect();
    for db_folder in &db_folders {
        if !disk_folders_set.contains(db_folder) {
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
    fn refuses_pre_versioned_schema() {
        let conn = mem_conn();
        build_old_schema(&conn);
        let err = run_migrations(&conn).unwrap_err();
        assert!(
            format!("{err:#}").contains("versión antigua"),
            "el error debe explicar que la DB es antigua: {err:#}"
        );
        // nada se migra ni se toca
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM mods", [], |row| row.get(0))
            .unwrap();
        assert_eq!(n, 2);
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(v, 0);
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

    #[test]
    fn fresh_bootstrap_has_full_metadata_schema() {
        let conn = mem_conn();
        run_migrations(&conn).unwrap();

        let cols: Vec<String> = conn
            .prepare("SELECT name FROM pragma_table_info('mods')")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for c in [
            "version",
            "author",
            "url",
            "description",
            "cover",
            "mount",
            "guides",
            "mod_id",
            "tags",
            "components",
        ] {
            assert!(cols.contains(&c.to_string()), "falta columna {c}");
        }

        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);

        // idempotente
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();
    }

    #[test]
    fn discover_populates_metadata_cache() {
        let conn = mem_conn();
        run_migrations(&conn).unwrap();

        let dir = std::env::temp_dir().join(format!("gta-mo-disc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("MyMod/models")).unwrap();
        std::fs::write(
            dir.join("MyMod/mod.toml"),
            "id = \"xardec:my-mod\"\nname = \"My Mod\"\nversion = \"2.0\"\nauthor = [\"Author\", \"Co\"]\nmount = [\"models\"]\ntags = [\"essential\"]\n",
        )
        .unwrap();

        let (new_count, _) = discover_mods(&conn, &dir).unwrap();
        assert_eq!(new_count, 1);

        let m = get_mod_by_folder(&conn, "MyMod").unwrap().unwrap();
        assert_eq!(m.name, "My Mod");
        let meta = load_mod_meta(&conn, m.id).unwrap();
        assert_eq!(meta.mod_id.as_deref(), Some("xardec:my-mod"));
        assert_eq!(meta.version.as_deref(), Some("2.0"));
        assert_eq!(meta.author, vec!["Author", "Co"]);
        assert_eq!(meta.mount, vec!["models".to_string()]);
        assert_eq!(meta.tags, vec!["essential".to_string()]);

        // re-discover refreshes metadata and the name from the manifest
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("MyMod/models")).unwrap();
        std::fs::write(
            dir.join("MyMod/mod.toml"),
            "name = \"Renamed Mod\"\nversion = \"2.1\"\n",
        )
        .unwrap();
        discover_mods(&conn, &dir).unwrap();
        let meta = load_mod_meta(&conn, m.id).unwrap();
        assert_eq!(meta.version.as_deref(), Some("2.1"));
        assert!(meta.mod_id.is_none(), "id borrado del manifest se limpia");
        assert_eq!(
            get_mod_by_folder(&conn, "MyMod").unwrap().unwrap().name,
            "Renamed Mod"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_syncs_manifest_dependencies() {
        let conn = mem_conn();
        run_migrations(&conn).unwrap();

        let dir = std::env::temp_dir().join(format!("gta-mo-deps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("ModA")).unwrap();
        std::fs::create_dir_all(dir.join("ModB")).unwrap();
        std::fs::write(
            dir.join("ModA/mod.toml"),
            "id = \"x:mod-a\"\n[dependencies]\nrequired = [\"x:mod-b\"]\noptional = [\"x:mod-b\", \"no-existe\"]\n",
        )
        .unwrap();
        std::fs::write(dir.join("ModB/mod.toml"), "id = \"x:mod-b\"\n").unwrap();

        discover_mods(&conn, &dir).unwrap();

        let a = get_mod_by_folder(&conn, "ModA").unwrap().unwrap();
        let b = get_mod_by_folder(&conn, "ModB").unwrap().unwrap();
        let deps = load_dependencies(&conn).unwrap();
        let refs = deps.get(&a.id).unwrap();
        // required once (optional duplicate ignored), optional dropped
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].id, b.id);
        assert!(refs[0].required);

        // removing [dependencies] from the manifest leaves DB deps alone
        std::fs::write(dir.join("ModA/mod.toml"), "id = \"x:mod-a\"\n").unwrap();
        discover_mods(&conn, &dir).unwrap();
        let deps = load_dependencies(&conn).unwrap();
        assert!(
            deps.contains_key(&a.id),
            "sin [dependencies] no se tocan las deps"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discover_stores_components_cache() {
        let conn = mem_conn();
        run_migrations(&conn).unwrap();

        let dir = std::env::temp_dir().join(format!("gta-mo-comps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Pack")).unwrap();
        std::fs::write(
            dir.join("Pack/mod.toml"),
            "name = \"Pack\"\n[[components]]\nname = \"A\"\nversion = \"1.0\"\nurl = \"http://a\"\n",
        )
        .unwrap();

        discover_mods(&conn, &dir).unwrap();
        let m = get_mod_by_folder(&conn, "Pack").unwrap().unwrap();
        let meta = load_mod_meta(&conn, m.id).unwrap();
        assert!(meta.is_pack());
        assert_eq!(meta.components.len(), 1);
        assert_eq!(meta.components[0].name.as_deref(), Some("A"));
        assert_eq!(meta.components[0].url.as_deref(), Some("http://a"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn groups_global_and_per_profile_memberships() {
        let conn = mem_conn();
        run_migrations(&conn).unwrap();

        add_mod_to_all_profiles(&conn, "m1", "M1").unwrap();
        add_mod_to_all_profiles(&conn, "m2", "M2").unwrap();
        let gid = create_group(&conn, "Graphics").unwrap();
        let g = get_group_by_id(&conn, gid).unwrap().unwrap();
        assert_eq!(g.slug, "graphics");

        let default = active_profile(&conn).unwrap();
        let second = get_profile_by_id(&conn, create_profile(&conn, "Second").unwrap())
            .unwrap()
            .unwrap();

        // global membership: m1 in group everywhere
        assert!(add_group_membership(&conn, gid, 1, None).unwrap());
        // per-profile: m2 in group only in "default"
        assert!(add_group_membership(&conn, gid, 2, Some(default.id)).unwrap());
        // dedup
        assert!(!add_group_membership(&conn, gid, 1, None).unwrap());

        assert_eq!(mods_in_group(&conn, gid, default.id).unwrap(), vec![1, 2]);
        assert_eq!(mods_in_group(&conn, gid, second.id).unwrap(), vec![1]);

        let gs = groups_of_mod_in_profile(&conn, 2, second.id).unwrap();
        assert!(gs.is_empty(), "m2 no está en el grupo en 'second'");
        let gs = groups_of_mod_in_profile(&conn, 2, default.id).unwrap();
        assert_eq!(gs.len(), 1);

        // resolve by name/slug/id
        assert_eq!(resolve_group(&conn, "Graphics").unwrap().id, gid);
        assert_eq!(resolve_group(&conn, "graphics").unwrap().id, gid);
        assert_eq!(resolve_group(&conn, &gid.to_string()).unwrap().id, gid);

        // rename keeps slug
        rename_group(&conn, gid, "Gráficos").unwrap();
        let g = get_group_by_id(&conn, gid).unwrap().unwrap();
        assert_eq!(g.name, "Gráficos");
        assert_eq!(g.slug, "graphics");

        // remove per-profile membership
        assert!(remove_group_membership(&conn, gid, 2, Some(default.id)).unwrap());
        assert!(!remove_group_membership(&conn, gid, 2, Some(default.id)).unwrap());

        // delete cascades memberships
        delete_group(&conn, gid).unwrap();
        assert!(mods_in_group(&conn, gid, default.id).unwrap().is_empty());
    }

    #[test]
    fn mod_enabled_in_profiles_lists_all_profiles() {
        let conn = mem_conn();
        run_migrations(&conn).unwrap();
        add_mod_to_all_profiles(&conn, "m1", "M1").unwrap();
        create_profile(&conn, "Second").unwrap();

        let default = active_profile(&conn).unwrap();
        set_mod_enabled(&conn, default.id, 1, true).unwrap();

        let states = mod_enabled_in_profiles(&conn, 1).unwrap();
        assert_eq!(states.len(), 2);
        let by_name: Vec<(&str, bool)> =
            states.iter().map(|(p, e)| (p.name.as_str(), *e)).collect();
        assert!(by_name.contains(&("default", true)));
        assert!(by_name.contains(&("Second", false)));
    }
}

#[cfg(test)]
mod v6_tests {
    use super::*;

    #[test]
    fn v6_migration_dedups_global_memberships() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute_batch(
            "CREATE TABLE mods (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                folder_name TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL
            );
            CREATE TABLE profiles (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                slug TEXT NOT NULL UNIQUE,
                is_active INTEGER DEFAULT 0,
                created_at TEXT DEFAULT (datetime('now'))
            );
            CREATE TABLE groups (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                slug TEXT NOT NULL UNIQUE,
                created_at TEXT DEFAULT (datetime('now'))
            );
            CREATE TABLE mod_groups (
                group_id INTEGER NOT NULL,
                mod_id INTEGER NOT NULL,
                profile_id INTEGER,
                PRIMARY KEY (group_id, mod_id, profile_id),
                FOREIGN KEY (group_id) REFERENCES groups(id) ON DELETE CASCADE,
                FOREIGN KEY (mod_id) REFERENCES mods(id) ON DELETE CASCADE,
                FOREIGN KEY (profile_id) REFERENCES profiles(id) ON DELETE CASCADE
            );
            INSERT INTO mods (folder_name, name) VALUES ('m1', 'M1');
            INSERT INTO groups (name, slug) VALUES ('G', 'g');
            INSERT INTO mod_groups (group_id, mod_id, profile_id) VALUES (1, 1, NULL);
            INSERT INTO mod_groups (group_id, mod_id, profile_id) VALUES (1, 1, NULL);",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 5).unwrap();

        run_migrations(&conn).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM mod_groups", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "la migración v6 deduplica las globales");

        assert!(
            conn.execute(
                "INSERT INTO mod_groups (group_id, mod_id, profile_id) VALUES (1, 1, NULL)",
                [],
            )
            .is_err(),
            "el índice único parcial rechaza duplicados globales"
        );
    }
}
