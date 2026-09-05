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
    ConflictScan(u64, Vec<ConflictView>),
}

/// One file conflict for the Conflicts tab.
#[derive(Debug, Clone)]
pub struct ConflictView {
    pub path: String,
    pub severity: String,
    pub providers: Vec<String>,
    pub duplicate: bool,
}

/// Dependencies and dependents of a mod within the active profile.
#[derive(Debug, Clone, Default)]
pub struct ModRelations {
    /// (folder, name, required, enabled-in-profile)
    pub depends: Vec<(String, String, bool, bool)>,
    /// (folder, name, enabled-in-profile)
    pub dependents: Vec<(String, String, bool)>,
}

pub struct Backend {
    conn: Option<rusqlite::Connection>,
    pub mods_dir: Option<PathBuf>,
    pub bin: String,
    pub error: Option<String>,
}

impl Backend {
    pub fn mods_dir_path(&self) -> Option<PathBuf> {
        self.mods_dir.clone()
    }

    pub fn error_str(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Tries to (re)initialize the backend (config + DB). Used to recover from
    /// a startup failure once the user fixes the environment.
    pub fn retry(&mut self) {
        *self = Self::new();
    }

    pub fn new() -> Self {
        let db_path = config::db_path();
        let conn = match db::open_db(&db_path) {
            Ok(c) => match db::run_migrations(&c) {
                Ok(()) => Some(c),
                Err(e) => {
                    return Self {
                        conn: None,
                        mods_dir: None,
                        bin: find_gta_mo_bin(),
                        error: Some(format!("No se pudo migrar la base de datos: {e:#}")),
                    };
                }
            },
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

    /// Resolves the dependency graph of a mod within the active profile.
    pub fn mod_relations(&self, mod_id: i64) -> Result<ModRelations, String> {
        let conn = self
            .conn
            .as_ref()
            .ok_or_else(|| "Sin conexión a la base de datos".to_string())?;
        let profile = db::active_profile(conn).map_err(|e| e.to_string())?;
        let depends = db::get_dependencies_of(conn, profile.id, mod_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(e, req)| (e.folder_name, e.name, req, e.enabled))
            .collect();
        let dependents = db::get_dependents_of(conn, profile.id, mod_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|e| (e.folder_name, e.name, e.enabled))
            .collect();
        Ok(ModRelations {
            depends,
            dependents,
        })
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
                let folder = m.folder_name.clone();
                let cached = db::load_mod_meta(conn, m.id).unwrap_or_default();
                // mod.toml es la fuente canónica: léelo en vivo y funde sus
                // campos con la caché (los ausentes caen a lo cacheado), así las
                // ediciones manuales (nombre, cover, tags…) se reflejan al
                // instante sin esperar un discover.
                let live = gta_mo_core::meta::read_mod_meta(&paths.mods_dir, &folder)
                    .ok()
                    .flatten();
                let meta = match live {
                    Some(ref lm) => merge_meta_caches(cached, db::meta_cache_from_meta(lm)),
                    None => cached,
                };
                let name = live
                    .as_ref()
                    .and_then(|lm| lm.name.clone())
                    .unwrap_or_else(|| m.name.clone());
                let groups = db::groups_of_mod_in_profile(conn, m.id, profile.id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|g| g.name)
                    .collect();
                ModView {
                    id: m.id,
                    folder,
                    name,
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

        let resolved = resolve_enabled_order(conn, &profile).unwrap_or_default();

        Ok(Snapshot {
            profiles,
            mods,
            active_slug: profile.slug,
            all_tags,
            all_groups,
            resolved,
        })
    }

    /// Scans file conflicts on a background thread so the UI thread is never
    /// blocked walking large mod trees. Results arrive as
    /// [`GuiEvent::ConflictScan`].
    pub fn scan_conflicts_async(
        gen: u64,
        mods_dir: PathBuf,
        resolved: Vec<String>,
        tx: Sender<GuiEvent>,
    ) {
        thread::spawn(move || {
            let list = match conflicts::scan_conflicts(&mods_dir, &resolved) {
                Ok(cs) => cs
                    .into_iter()
                    .map(|c| ConflictView {
                        path: c.path,
                        severity: match c.severity {
                            conflicts::Severity::High => "alta",
                            conflicts::Severity::Medium => "media",
                            conflicts::Severity::Info => "info",
                        }
                        .to_string(),
                        providers: c.providers,
                        duplicate: c.duplicate,
                    })
                    .collect(),
                Err(_) => Vec::new(),
            };
            let _ = tx.send(GuiEvent::ConflictScan(gen, list));
        });
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

/// Live `mod.toml` metadata wins; fields the manifest leaves absent/empty fall
/// back to the DB cache.
fn merge_meta_caches(cached: db::ModMetaCache, live: db::ModMetaCache) -> db::ModMetaCache {
    db::ModMetaCache {
        mod_id: live.mod_id.or(cached.mod_id),
        version: live.version.or(cached.version),
        author: if live.author.is_empty() {
            cached.author
        } else {
            live.author
        },
        url: live.url.or(cached.url),
        description: live.description.or(cached.description),
        cover: live.cover.or(cached.cover),
        mount: if live.mount.is_empty() {
            cached.mount
        } else {
            live.mount
        },
        guides: if live.guides.is_empty() {
            cached.guides
        } else {
            live.guides
        },
        tags: if live.tags.is_empty() {
            cached.tags
        } else {
            live.tags
        },
        components: if live.components.is_empty() {
            cached.components
        } else {
            live.components
        },
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

#[cfg(test)]
mod tests {
    use super::*;
    use gta_mo_core::meta::ModMeta;

    #[test]
    fn live_metadata_overrides_cache_and_falls_back() {
        let cached = db::ModMetaCache {
            mod_id: Some("a:b".into()),
            version: Some("1".into()),
            author: vec!["Old".into()],
            url: None,
            description: None,
            cover: Some("c.png".into()),
            mount: vec![],
            guides: vec![],
            tags: vec![],
            components: vec![],
        };
        let mut live = ModMeta::default();
        live.name = Some("Live Name".into());
        live.author = vec!["New".into()];
        live.tags = Some(vec!["essential".into()]);
        // version/cover ausentes en el manifest -> se mantienen de la caché
        let merged = merge_meta_caches(cached, db::meta_cache_from_meta(&live));
        assert_eq!(merged.mod_id.as_deref(), Some("a:b"));
        assert_eq!(merged.version.as_deref(), Some("1"));
        assert_eq!(merged.cover.as_deref(), Some("c.png"));
        assert_eq!(merged.author, vec!["New"]);
        assert_eq!(merged.tags, vec!["essential".to_string()]);
    }
}
