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
        "name = \"DXVK - D3D9.dll\"\nversion = \"3.1\"\nauthor = \"doitsujin\"\nmount = [\"content\"]\n",
    )
    .unwrap();

    let v = json(&t.run_ok(&["ctl", "info", "d3d9", "--json"]));
    assert_eq!(v["name"].as_str().unwrap(), "DXVK - D3D9.dll");
    assert_eq!(v["version"].as_str().unwrap(), "3.1");
    assert_eq!(v["author"].as_str().unwrap(), "doitsujin");
    assert_eq!(v["mount"][0].as_str().unwrap(), "content");
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
