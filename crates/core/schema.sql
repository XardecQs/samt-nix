PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS mods (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    folder_name TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL CHECK(length(name) > 0),
    version TEXT,
    author TEXT,
    url TEXT,
    description TEXT,
    cover TEXT,
    mount TEXT,
    guides TEXT,
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

CREATE TABLE IF NOT EXISTS profiles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    slug TEXT NOT NULL UNIQUE,
    is_active INTEGER DEFAULT 0 CHECK(is_active IN (0, 1)),
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS profile_mods (
    profile_id INTEGER NOT NULL,
    mod_id INTEGER NOT NULL,
    enabled INTEGER DEFAULT 0 CHECK(enabled IN (0, 1)),
    load_order INTEGER DEFAULT 0,
    PRIMARY KEY (profile_id, mod_id),
    FOREIGN KEY (profile_id) REFERENCES profiles(id) ON DELETE CASCADE,
    FOREIGN KEY (mod_id) REFERENCES mods(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS mod_dependencies (
    mod_id INTEGER NOT NULL,
    dependency_id INTEGER NOT NULL,
    required INTEGER NOT NULL DEFAULT 1 CHECK(required IN (0, 1)),
    PRIMARY KEY (mod_id, dependency_id),
    FOREIGN KEY (mod_id) REFERENCES mods(id) ON DELETE CASCADE,
    FOREIGN KEY (dependency_id) REFERENCES mods(id) ON DELETE CASCADE,
    CHECK(mod_id != dependency_id)
);

CREATE INDEX IF NOT EXISTS idx_mod_deps_mod_id ON mod_dependencies(mod_id);
CREATE INDEX IF NOT EXISTS idx_mod_deps_dep_id ON mod_dependencies(dependency_id);
CREATE INDEX IF NOT EXISTS idx_profile_mods_mod_id ON profile_mods(mod_id);