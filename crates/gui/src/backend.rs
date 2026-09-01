use gta_mo_core::config;
use gta_mo_core::conflicts;
use gta_mo_core::db;
use gta_mo_core::resolver;
use std::io::BufRead;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;

use crate::model::{ModView, ProfileView, Snapshot};

#[derive(Debug)]
pub enum GuiEvent {
    LogLine(String),
    CommandDone(bool, String),
}

pub struct Backend {
    conn: Option<rusqlite::Connection>,
    pub mods_dir: Option<PathBuf>,
    pub bin: String,
    pub error: Option<String>,
}

impl Backend {
    pub fn new() -> Self {
        let db_path = config::db_path();
        let conn = match db::open_db(&db_path) {
            Ok(c) => {
                let _ = db::run_migrations(&c);
                Some(c)
            }
            Err(e) => {
                return Self {
                    conn: None,
                    mods_dir: None,
                    bin: find_gta_mo_bin(),
                    error: Some(format!("No se pudo abrir la base de datos: {e}")),
                };
            }
        };

        let (mods_dir, error) = match config::load_config() {
            Ok(cfg) => {
                let paths = config::RuntimePaths::from_config(&cfg);
                (Some(paths.mods_dir), None)
            }
            Err(e) => (None, Some(format!("Error de config: {e}"))),
        };

        Self {
            conn,
            mods_dir,
            bin: find_gta_mo_bin(),
            error,
        }
    }

    pub fn cover_path(&self, folder: &str, cover: &str) -> Option<PathBuf> {
        self.mods_dir.as_ref().map(|d| d.join(folder).join(cover))
    }

    pub fn snapshot(&self) -> Result<Snapshot, String> {
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        let conn = self
            .conn
            .as_ref()
            .ok_or_else(|| "Sin conexión a la base de datos".to_string())?;
        let cfg = config::load_config().map_err(|e| format!("Error de config: {e}"))?;
        let paths = config::RuntimePaths::from_config(&cfg);
        let profile = db::active_profile(conn).map_err(|e| e.to_string())?;

        let profiles = db::list_profiles(conn)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|p| {
                let (total, enabled) = db::profile_mod_count(conn, p.id).unwrap_or((0, 0));
                ProfileView {
                    name: p.name.clone(),
                    slug: p.slug.clone(),
                    is_active: p.is_active,
                    total,
                    enabled,
                }
            })
            .collect::<Vec<_>>();

        let mods = db::load_all_mods_for_profile(conn, profile.id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|m| {
                let meta = db::load_mod_meta(conn, m.id).unwrap_or_default();
                let groups = db::groups_of_mod_in_profile(conn, m.id, profile.id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|g| g.name)
                    .collect();
                ModView {
                    id: m.id,
                    folder: m.folder_name,
                    name: m.name,
                    enabled: m.enabled,
                    order: m.load_order,
                    meta,
                    groups,
                }
            })
            .collect::<Vec<_>>();

        let mut all_tags: Vec<String> = Vec::new();
        for m in &mods {
            for t in &m.meta.tags {
                if !all_tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
                    all_tags.push(t.clone());
                }
            }
        }
        all_tags.sort();

        let all_groups = db::list_groups(conn)
            .unwrap_or_default()
            .into_iter()
            .map(|g| g.name)
            .collect::<Vec<_>>();

        let resolved = resolve_enabled_order(conn, &profile);
        let conflicts = match resolved {
            Ok(res) => conflicts::scan_conflicts(&paths.mods_dir, &res)
                .map(|c| c.iter().filter(|x| !x.duplicate).count())
                .unwrap_or(0),
            Err(_) => 0,
        };

        Ok(Snapshot {
            profiles,
            mods,
            active_slug: profile.slug,
            all_tags,
            all_groups,
            conflicts,
        })
    }

    /// Spawns `gta-mo <args>` in a background thread, streaming output to `tx`.
    pub fn run_cli_async(&self, args: Vec<String>, tx: Sender<GuiEvent>) {
        let bin = self.bin.clone();
        thread::spawn(move || {
            let mut cmd = Command::new(&bin);
            cmd.args(&args)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(GuiEvent::CommandDone(
                        false,
                        format!("No se pudo ejecutar gta-mo: {e}"),
                    ));
                    return;
                }
            };
            if let Some(out) = child.stdout.take() {
                let tx2 = tx.clone();
                thread::spawn(move || {
                    for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                        let _ = tx2.send(GuiEvent::LogLine(line));
                    }
                });
            }
            if let Some(err) = child.stderr.take() {
                let tx3 = tx.clone();
                thread::spawn(move || {
                    for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                        let _ = tx3.send(GuiEvent::LogLine(line));
                    }
                });
            }
            let ok = child.wait().map(|s| s.success()).unwrap_or(false);
            let _ = tx.send(GuiEvent::CommandDone(ok, String::new()));
        });
    }
}

impl Default for Backend {
    fn default() -> Self {
        Self::new()
    }
}

fn resolve_enabled_order(
    conn: &rusqlite::Connection,
    profile: &db::Profile,
) -> anyhow::Result<Vec<String>> {
    let all_mods = db::load_all_mods_for_profile(conn, profile.id)?;
    let mods_map = all_mods.into_iter().map(|m| (m.id, m)).collect();
    let deps = db::load_dependencies(conn)?;
    let enabled_ids = db::load_enabled_mod_ids_for_profile(conn, profile.id)?;
    let mut graph = resolver::DepGraph::new(mods_map, deps, enabled_ids);
    graph.prompt = resolver::DepPrompt::Ignore;
    let _ = graph.validate_dependencies();
    let _ = graph.detect_cycles();
    Ok(graph.resolve())
}

pub fn find_gta_mo_bin() -> String {
    if let Ok(p) = std::env::var("GTA_MO_BIN") {
        if std::path::Path::new(&p).exists() {
            return p;
        }
    }
    if let Ok(p) = which::which("gta-mo") {
        return p.to_string_lossy().into_owned();
    }
    for cand in [
        "target/debug/gta-mo",
        "../target/debug/gta-mo",
        "../../target/debug/gta-mo",
        "../../../target/debug/gta-mo",
    ] {
        if std::path::Path::new(cand).exists() {
            return cand.to_string();
        }
    }
    "gta-mo".into()
}
