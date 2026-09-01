use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_gta-mo");

struct TempDb {
    dir: PathBuf,
    db: PathBuf,
    config: Option<PathBuf>,
}

impl TempDb {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("gta-mo-it-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("test.db");
        Self {
            dir,
            db,
            config: None,
        }
    }

    fn with_config(mut self, game_root: &Path) -> Self {
        let cfg = self.dir.join("config.toml");
        std::fs::write(
            &cfg,
            format!(
                "game_root = \"{}\"\nproton_path = \"/tmp\"\n",
                game_root.display()
            ),
        )
        .unwrap();
        self.config = Some(cfg);
        self
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new(BIN);
        cmd.env("GTA_MO_DB", &self.db);
        if let Some(cfg) = &self.config {
            cmd.env("GTA_MO_CONFIG", cfg);
        }
        cmd.args(args).output().unwrap()
    }

    fn run_ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "comando falló: {args:?}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn json(s: &str) -> serde_json::Value {
    serde_json::from_str(s).expect("salida no es JSON válido")
}

#[test]
fn fresh_db_has_default_profile() {
    let t = TempDb::new("fresh");
    let v = json(&t.run_ok(&["ctl", "profile", "list", "--json"]));
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["slug"].as_str().unwrap(), "default");
    assert!(arr[0]["active"].as_bool().unwrap());
}

#[test]
fn mod_lifecycle_and_info_shows_profile_state() {
    let t = TempDb::new("lifecycle");
    t.run_ok(&["ctl", "add", "graphics_enhancer"]);
    t.run_ok(&["ctl", "enable", "graphics_enhancer"]);
    t.run_ok(&["ctl", "order", "graphics_enhancer", "40"]);

    let v = json(&t.run_ok(&["ctl", "info", "graphics_enhancer", "--json"]));
    assert!(v["enabled"].as_bool().unwrap());
    assert_eq!(v["order"].as_i64().unwrap(), 40);

    // list confirms the state too
    let v = json(&t.run_ok(&["ctl", "list", "--json"]));
    assert!(v[0]["enabled"].as_bool().unwrap());
}

#[test]
fn profiles_are_isolated() {
    let t = TempDb::new("profiles");
    t.run_ok(&["ctl", "add", "m1"]);
    t.run_ok(&["ctl", "enable", "m1"]);
    t.run_ok(&["ctl", "profile", "create", "Vanilla"]);

    let v = json(&t.run_ok(&["--profile", "vanilla", "ctl", "list", "--json"]));
    assert!(
        !v[0]["enabled"].as_bool().unwrap(),
        "nuevo perfil empieza desactivado"
    );

    let v = json(&t.run_ok(&["--profile", "default", "ctl", "list", "--json"]));
    assert!(v[0]["enabled"].as_bool().unwrap());

    // copy carries states
    t.run_ok(&["ctl", "profile", "copy", "default", "Copy"]);
    let v = json(&t.run_ok(&["--profile", "copy", "ctl", "list", "--json"]));
    assert!(v[0]["enabled"].as_bool().unwrap());

    // rename keeps slug (copy), use/delete work
    t.run_ok(&["ctl", "profile", "rename", "Copy", "Copy2"]);
    t.run_ok(&["ctl", "profile", "use", "vanilla"]);
    let v = json(&t.run_ok(&["ctl", "profile", "list", "--json"]));
    assert!(v
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["active"].as_bool().unwrap()));

    // delete down to a single profile
    t.run_ok(&["ctl", "profile", "delete", "Copy2", "--yes"]);
    t.run_ok(&["ctl", "profile", "delete", "default", "--yes"]);

    // the last remaining profile cannot be deleted
    let out = t.run(&["ctl", "profile", "delete", "vanilla", "--yes"]);
    assert!(!out.status.success());
}

#[test]
fn migrates_old_schema_to_profiles() {
    let t = TempDb::new("migrate");
    {
        let conn = rusqlite::Connection::open(&t.db).unwrap();
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
            INSERT INTO mods (folder_name, name, enabled, load_order) VALUES ('m1', 'M1', 1, 30);
            INSERT INTO mods (folder_name, name, enabled, load_order) VALUES ('m2', 'M2', 0, 10);
            INSERT INTO mod_dependencies (mod_id, dependency_id, required) VALUES (1, 2, 1);
        ",
        )
        .unwrap();
    }

    let v = json(&t.run_ok(&["--profile", "default", "ctl", "list", "--json"]));
    let arr = v.as_array().unwrap();
    let m1 = arr.iter().find(|m| m["folder"] == "m1").unwrap();
    assert!(m1["enabled"].as_bool().unwrap());
    assert_eq!(m1["order"].as_i64().unwrap(), 30);
    assert_eq!(m1["deps"][0]["folder"].as_str().unwrap(), "m2");
    let m2 = arr.iter().find(|m| m["folder"] == "m2").unwrap();
    assert!(!m2["enabled"].as_bool().unwrap());
}

#[test]
fn add_rejects_order_flag() {
    let t = TempDb::new("addorder");
    let out = t.run(&["ctl", "add", "m", "--order", "5"]);
    assert!(!out.status.success(), "ctl add --order debería fallar");
}

#[test]
fn duplicate_mod_is_rejected() {
    let t = TempDb::new("dup");
    t.run_ok(&["ctl", "add", "m1"]);
    let out = t.run(&["ctl", "add", "m1"]);
    assert!(!out.status.success());
}

#[test]
fn init_creates_folder_template_and_registers() {
    let t = TempDb::new("init");
    let game_root = t.dir.join("game");
    std::fs::create_dir_all(&game_root).unwrap();
    let t = t.with_config(&game_root);

    t.run_ok(&["ctl", "init", "my_mod"]);

    assert!(game_root.join("mods/my_mod/mod.toml").exists());
    let v = json(&t.run_ok(&["ctl", "list", "--json"]));
    assert_eq!(v[0]["folder"].as_str().unwrap(), "my_mod");
}

#[test]
fn info_reads_edited_mod_toml_without_launch() {
    let t = TempDb::new("info");
    let game_root = t.dir.join("game");
    std::fs::create_dir_all(&game_root).unwrap();
    let t = t.with_config(&game_root);

    t.run_ok(&["ctl", "init", "d3d9"]);
    std::fs::write(
        game_root.join("mods/d3d9/mod.toml"),
        "id = \"doitsujin:dxvk-d3d9\"\nname = \"DXVK - D3D9.dll\"\nversion = \"3.1\"\nauthor = [\"doitsujin\", \"xardec\"]\nmount = [\"content\"]\ntags = [\"essential\"]\n",
    )
    .unwrap();

    let v = json(&t.run_ok(&["ctl", "info", "d3d9", "--json"]));
    assert_eq!(v["name"].as_str().unwrap(), "DXVK - D3D9.dll");
    assert_eq!(v["mod_id"].as_str().unwrap(), "doitsujin:dxvk-d3d9");
    assert_eq!(v["version"].as_str().unwrap(), "3.1");
    assert_eq!(v["author"][0].as_str().unwrap(), "doitsujin");
    assert_eq!(v["author"][1].as_str().unwrap(), "xardec");
    assert_eq!(v["mount"][0].as_str().unwrap(), "content");
    assert_eq!(v["tags"][0].as_str().unwrap(), "essential");
}

#[test]
fn info_lists_profiles_with_enabled_state() {
    let t = TempDb::new("profilesinfo");
    t.run_ok(&["ctl", "add", "m1"]);
    t.run_ok(&["ctl", "enable", "m1"]);
    t.run_ok(&["ctl", "profile", "create", "Second"]);

    let v = json(&t.run_ok(&["ctl", "info", "m1", "--json"]));
    let profiles = v["profiles"].as_array().unwrap();
    assert_eq!(profiles.len(), 2);
    let by_name: Vec<(&str, bool)> = profiles
        .iter()
        .map(|p| (p["name"].as_str().unwrap(), p["enabled"].as_bool().unwrap()))
        .collect();
    assert!(by_name.contains(&("default", true)));
    assert!(by_name.contains(&("Second", false)));
}

#[test]
fn dep_add_writes_back_to_mod_toml() {
    let t = TempDb::new("depwrite");
    let game_root = t.dir.join("game");
    std::fs::create_dir_all(&game_root).unwrap();
    let t = t.with_config(&game_root);

    t.run_ok(&["ctl", "init", "m1"]);
    t.run_ok(&["ctl", "init", "m2"]);
    t.run_ok(&["ctl", "dep", "add", "m1", "m2", "--optional"]);

    let v = json(&t.run_ok(&["ctl", "info", "m1", "--json"]));
    assert_eq!(v["dependencies"][0]["folder"].as_str().unwrap(), "m2");
    assert!(!v["dependencies"][0]["required"].as_bool().unwrap());

    let content = std::fs::read_to_string(game_root.join("mods/m1/mod.toml")).unwrap();
    assert!(content.contains("optional"));
    assert!(content.contains("m2"));
}

#[test]
fn info_shows_pack_components_and_expands_guides() {
    let t = TempDb::new("packinfo");
    let game_root = t.dir.join("game");
    std::fs::create_dir_all(&game_root).unwrap();
    let t = t.with_config(&game_root);

    t.run_ok(&["ctl", "init", "pack"]);
    std::fs::create_dir_all(game_root.join("mods/pack/guides")).unwrap();
    std::fs::write(game_root.join("mods/pack/guides/readme.txt"), "hi").unwrap();
    std::fs::write(
        game_root.join("mods/pack/mod.toml"),
        "name = \"Pack\"\nguides = [\"guides\"]\n[[components]]\nname = \"SilentPatch\"\nversion = \"1.0.1\"\n",
    )
    .unwrap();

    let v = json(&t.run_ok(&["ctl", "info", "pack", "--json"]));
    assert!(v["pack"].as_bool().unwrap(), "pack con componentes");
    assert_eq!(v["components"][0]["name"].as_str().unwrap(), "SilentPatch");
    assert_eq!(v["components"][0]["version"].as_str().unwrap(), "1.0.1");
    assert_eq!(v["guides"][0].as_str().unwrap(), "guides/readme.txt");
}

#[test]
fn rename_writes_back_to_mod_toml() {
    let t = TempDb::new("rename");
    let game_root = t.dir.join("game");
    std::fs::create_dir_all(&game_root).unwrap();
    let t = t.with_config(&game_root);

    t.run_ok(&["ctl", "init", "m1"]);
    t.run_ok(&["ctl", "rename", "m1", "New Name"]);

    let content = std::fs::read_to_string(game_root.join("mods/m1/mod.toml")).unwrap();
    assert!(content.contains("name = \"New Name\""));

    // and it is reflected in info right away
    let v = json(&t.run_ok(&["ctl", "info", "m1", "--json"]));
    assert_eq!(v["name"].as_str().unwrap(), "New Name");
}

#[test]
fn human_output_commands_succeed() {
    let t = TempDb::new("human");
    t.run_ok(&["ctl", "add", "m1"]);
    for args in [
        &["ctl", "list"][..],
        &["ctl", "list", "-v"][..],
        &["ctl", "info", "m1"][..],
        &["ctl", "info", "m1", "-v"][..],
    ] {
        let out = t.run(args);
        assert!(
            out.status.success(),
            "{args:?} falló: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

#[test]
fn groups_global_and_per_profile_filters() {
    let t = TempDb::new("groups");
    t.run_ok(&["ctl", "add", "alpha_mod"]);
    t.run_ok(&["ctl", "add", "beta_mod"]);
    t.run_ok(&["ctl", "group", "create", "Graphics"]);
    t.run_ok(&["ctl", "group", "add", "alpha_mod", "Graphics"]);
    t.run_ok(&["ctl", "group", "add", "beta_mod", "Graphics", "--global"]);

    let v = json(&t.run_ok(&["ctl", "group", "list", "--json"]));
    assert_eq!(v[0]["name"].as_str().unwrap(), "Graphics");
    assert_eq!(v[0]["mods"].as_i64().unwrap(), 2);

    // default profile: both global and per-profile memberships
    let v = json(&t.run_ok(&["ctl", "list", "--group", "Graphics", "--json"]));
    let folders: Vec<&str> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["folder"].as_str().unwrap())
        .collect();
    assert!(folders.contains(&"alpha_mod"));
    assert!(folders.contains(&"beta_mod"));

    // second profile: only the global membership
    t.run_ok(&["ctl", "profile", "create", "Second"]);
    let v = json(&t.run_ok(&[
        "--profile",
        "second",
        "ctl",
        "list",
        "--group",
        "Graphics",
        "--json",
    ]));
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["folder"].as_str().unwrap(), "beta_mod");

    // info shows the groups of a mod
    let v = json(&t.run_ok(&["ctl", "info", "alpha_mod", "--json"]));
    assert_eq!(v["groups"][0]["name"].as_str().unwrap(), "Graphics");

    // remove membership
    t.run_ok(&["ctl", "group", "remove", "alpha_mod", "Graphics"]);
    let v = json(&t.run_ok(&["ctl", "list", "--group", "Graphics", "--json"]));
    assert_eq!(v.as_array().unwrap().len(), 1);
}

#[test]
fn list_filters_by_tag_author_and_search() {
    let t = TempDb::new("filters");
    let game_root = t.dir.join("game");
    std::fs::create_dir_all(&game_root).unwrap();
    let t = t.with_config(&game_root);

    t.run_ok(&["ctl", "add", "alpha_mod"]);
    t.run_ok(&["ctl", "add", "beta_mod"]);
    std::fs::create_dir_all(game_root.join("mods/alpha_mod")).unwrap();
    std::fs::create_dir_all(game_root.join("mods/beta_mod")).unwrap();
    std::fs::write(
        game_root.join("mods/alpha_mod/mod.toml"),
        "name = \"Alpha\"\nauthor = [\"xardec\"]\ntags = [\"essential\"]\n",
    )
    .unwrap();
    std::fs::write(
        game_root.join("mods/beta_mod/mod.toml"),
        "name = \"Beta\"\nauthor = [\"other\"]\ntags = [\"graphics\"]\n",
    )
    .unwrap();

    let v = json(&t.run_ok(&["ctl", "list", "--tag", "essential", "--json"]));
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["folder"].as_str().unwrap(), "alpha_mod");

    let v = json(&t.run_ok(&["ctl", "list", "--author", "xardec", "--json"]));
    assert_eq!(v.as_array().unwrap().len(), 1);

    let v = json(&t.run_ok(&["ctl", "list", "--search", "BETA", "--json"]));
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["folder"].as_str().unwrap(), "beta_mod");
}

#[test]
fn export_import_roundtrip() {
    let t = TempDb::new("export");
    t.run_ok(&["ctl", "add", "m1"]);
    t.run_ok(&["ctl", "enable", "m1"]);
    t.run_ok(&["ctl", "group", "create", "G"]);
    t.run_ok(&["ctl", "group", "add", "m1", "G", "--global"]);
    let export_file = t.dir.join("state.json");
    t.run_ok(&["ctl", "export", export_file.to_str().unwrap()]);

    let t2 = TempDb::new("export2");
    t2.run_ok(&["ctl", "import", export_file.to_str().unwrap(), "--force"]);
    let v = json(&t2.run_ok(&["ctl", "list", "--json"]));
    assert_eq!(v[0]["folder"].as_str().unwrap(), "m1");
    assert!(v[0]["enabled"].as_bool().unwrap());
    let v = json(&t2.run_ok(&["ctl", "group", "list", "--json"]));
    assert_eq!(v[0]["name"].as_str().unwrap(), "G");
}

#[test]
fn conflicts_detected_between_enabled_mods() {
    let t = TempDb::new("conflict");
    let game_root = t.dir.join("game");
    std::fs::create_dir_all(&game_root).unwrap();
    let t = t.with_config(&game_root);

    t.run_ok(&["ctl", "add", "mod_a"]);
    t.run_ok(&["ctl", "add", "mod_b"]);
    std::fs::create_dir_all(game_root.join("mods/mod_a")).unwrap();
    std::fs::create_dir_all(game_root.join("mods/mod_b")).unwrap();
    std::fs::create_dir_all(game_root.join("mods/mod_a/models")).unwrap();
    std::fs::create_dir_all(game_root.join("mods/mod_b/models")).unwrap();
    std::fs::write(game_root.join("mods/mod_a/models/x.dff"), "AAA").unwrap();
    std::fs::write(game_root.join("mods/mod_b/models/x.dff"), "BBB").unwrap();
    t.run_ok(&["ctl", "enable", "mod_a"]);
    t.run_ok(&["ctl", "enable", "mod_b"]);

    let v = json(&t.run_ok(&["ctl", "conflicts", "--json"]));
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert_eq!(v[0]["path"].as_str().unwrap(), "models/x.dff");
    assert!(!v[0]["duplicate"].as_bool().unwrap());
    // higher load_order (mod_b) wins
    assert_eq!(v[0]["providers"][0].as_str().unwrap(), "mod_b");

    // identical files are not real conflicts
    std::fs::write(game_root.join("mods/mod_a/models/x.dff"), "SAME").unwrap();
    std::fs::write(game_root.join("mods/mod_b/models/x.dff"), "SAME").unwrap();
    let v = json(&t.run_ok(&["ctl", "conflicts", "--json"]));
    assert!(v[0]["duplicate"].as_bool().unwrap());
}

#[test]
fn health_reports_missing_folder_and_disabled_dep() {
    let t = TempDb::new("health");
    let game_root = t.dir.join("game");
    std::fs::create_dir_all(&game_root).unwrap();
    let t = t.with_config(&game_root);

    t.run_ok(&["ctl", "add", "ghost"]);
    let out = t.run_ok(&["ctl", "health"]);
    assert!(out.contains("carpeta no existe"), "salida: {out}");

    t.run_ok(&["ctl", "add", "a"]);
    t.run_ok(&["ctl", "add", "b"]);
    t.run_ok(&["ctl", "enable", "a"]);
    std::fs::create_dir_all(game_root.join("mods/a")).unwrap();
    std::fs::create_dir_all(game_root.join("mods/b")).unwrap();
    t.run_ok(&["ctl", "dep", "add", "a", "b"]);
    let out = t.run_ok(&["ctl", "health"]);
    assert!(
        out.contains("dependencia requerida desactivada"),
        "salida: {out}"
    );
}

#[test]
fn profile_diff_reports_differences() {
    let t = TempDb::new("diff");
    t.run_ok(&["ctl", "add", "m1"]);
    t.run_ok(&["ctl", "add", "m2"]);
    t.run_ok(&["ctl", "enable", "m1"]);
    t.run_ok(&["ctl", "enable", "m2"]);
    t.run_ok(&["ctl", "profile", "create", "Second"]);
    t.run_ok(&["--profile", "second", "ctl", "enable", "m1"]);
    let out = t.run_ok(&["ctl", "profile", "diff", "default", "second"]);
    assert!(out.contains("m2"), "m2 solo en default; salida: {out}");
}

#[test]
fn list_sort_by_name() {
    let t = TempDb::new("sort");
    t.run_ok(&["ctl", "add", "beta"]);
    t.run_ok(&["ctl", "add", "alpha"]);
    let v = json(&t.run_ok(&["ctl", "list", "--sort", "name", "--json"]));
    assert_eq!(v[0]["folder"].as_str().unwrap(), "alpha");
    assert_eq!(v[1]["folder"].as_str().unwrap(), "beta");
}

#[test]
fn group_enable_disables_members_and_deps() {
    let t = TempDb::new("genable");
    t.run_ok(&["ctl", "add", "m1"]);
    t.run_ok(&["ctl", "add", "dep1"]);
    t.run_ok(&["ctl", "dep", "add", "m1", "dep1"]);
    t.run_ok(&["ctl", "group", "create", "G"]);
    t.run_ok(&["ctl", "group", "add", "m1", "G"]);

    t.run_ok(&["ctl", "group", "enable", "G"]);
    let v = json(&t.run_ok(&["ctl", "list", "--json"]));
    let states: Vec<(&str, bool)> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["folder"].as_str().unwrap(),
                m["enabled"].as_bool().unwrap(),
            )
        })
        .collect();
    assert!(states.contains(&("m1", true)), "{states:?}");
    assert!(
        states.contains(&("dep1", true)),
        "dep transitiva: {states:?}"
    );

    t.run_ok(&["ctl", "group", "disable", "G"]);
    let v = json(&t.run_ok(&["ctl", "list", "--json"]));
    let states: Vec<(&str, bool)> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["folder"].as_str().unwrap(),
                m["enabled"].as_bool().unwrap(),
            )
        })
        .collect();
    assert!(states.contains(&("m1", false)), "{states:?}");
    assert!(
        states.contains(&("dep1", true)),
        "disable no toca deps: {states:?}"
    );
}

#[test]
fn which_reports_providers() {
    let t = TempDb::new("which");
    let game_root = t.dir.join("game");
    std::fs::create_dir_all(&game_root).unwrap();
    let t = t.with_config(&game_root);

    t.run_ok(&["ctl", "add", "mod_a"]);
    t.run_ok(&["ctl", "add", "mod_b"]);
    std::fs::create_dir_all(game_root.join("mods/mod_a/models")).unwrap();
    std::fs::create_dir_all(game_root.join("mods/mod_b/models")).unwrap();
    std::fs::write(game_root.join("mods/mod_a/models/x.dff"), "AAA").unwrap();
    std::fs::write(game_root.join("mods/mod_b/models/x.dff"), "BBB").unwrap();
    t.run_ok(&["ctl", "enable", "mod_a"]);
    t.run_ok(&["ctl", "enable", "mod_b"]);

    let out = t.run_ok(&["ctl", "which", "models/x.dff"]);
    assert!(out.contains("gana 'mod_b'"), "salida: {out}");

    let out = t.run_ok(&["ctl", "which", "data/gta.dat"]);
    assert!(out.contains("viene de la base"), "salida: {out}");
}
