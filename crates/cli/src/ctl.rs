use comfy_table::{presets, Cell, ContentArrangement, Table};
use gta_mo_core::color;
use gta_mo_core::db;
use gta_mo_core::log;
use owo_colors::OwoColorize;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// Strip ANSI codes from stdout output when it is not a color-capable terminal
/// (pipe/redirect/`NO_COLOR`), so `gta-mo ctl ... | grep` stays clean.
macro_rules! println {
    () => {
        ::std::println!()
    };
    ($($arg:tt)*) => {{
        let msg = format!($($arg)*);
        if color::stdout_enabled() {
            ::std::println!("{}", msg);
        } else {
            ::std::println!("{}", color::strip_ansi(&msg));
        }
    }};
}

/// Renders a comfy-table with a clean preset, fitting the terminal width.
fn render_table(headers: Vec<String>, rows: Vec<Vec<String>>) -> String {
    let mut table = Table::new();
    table.load_preset(presets::UTF8_FULL_CONDENSED);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    if !headers.is_empty() {
        table.set_header(headers.into_iter().map(Cell::from).collect::<Vec<_>>());
    }
    for row in rows {
        table.add_row(row.into_iter().map(Cell::from).collect::<Vec<_>>());
    }
    table.to_string()
}

/// Mirrors the `folder_name` CHECK constraint from `schema.sql` so folder paths
/// can be validated *before* touching the filesystem (the DB constraint alone
/// runs too late for `cmd_init`, which writes first).
fn valid_folder_name(folder: &str) -> bool {
    !folder.is_empty()
        && !folder.starts_with('.')
        && !folder.contains('|')
        && !folder.contains('/')
        && !folder.contains('\\')
        && !folder.contains(':')
        // A comma would corrupt the fuse-overlayfs option string.
        && !folder.contains(',')
        && folder != "."
        && folder != ".."
        && !folder.starts_with(".. ")
        && !folder.starts_with("..\\")
        && !folder.starts_with("../")
        && folder.trim() == folder
}

#[derive(Serialize)]
struct DepJson {
    id: i64,
    folder: String,
    name: String,
    required: bool,
}

impl DepJson {
    fn from_entry(d: &db::ModEntry, required: bool) -> Self {
        Self {
            id: d.id,
            folder: d.folder_name.clone(),
            name: d.name.clone(),
            required,
        }
    }
}

#[derive(Serialize)]
struct VariantJson {
    group: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

impl VariantJson {
    fn from_cache(cache: &db::ModMetaCache) -> Option<Self> {
        cache.variant_group.as_ref().map(|g| Self {
            group: g.clone(),
            name: cache.variant_name.clone(),
        })
    }
}

#[derive(Serialize)]
struct ModJson {
    id: i64,
    folder: String,
    name: String,
    enabled: bool,
    order: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    mod_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    author: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cover: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    mount: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    guides: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    screenshots: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    deps: Vec<DepJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    dependents: Vec<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    variant: Option<VariantJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    conflicts: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    modloader_priority: Option<i64>,
}

pub fn run(
    conn: &Connection,
    args: &super::CtlArgs,
    profile_ident: Option<&str>,
) -> anyhow::Result<()> {
    let active = || -> anyhow::Result<db::Profile> {
        match profile_ident {
            Some(ident) => db::resolve_profile(conn, ident),
            None => db::active_profile(conn),
        }
    };

    match &args.command {
        super::CtlCommand::List {
            verbose,
            enabled,
            disabled,
            tag,
            group,
            author,
            id,
            search,
            sort,
            dir,
            json,
        } => {
            let filter = if *enabled {
                Some("enabled")
            } else if *disabled {
                Some("disabled")
            } else {
                None
            };
            let profile = active()?;
            let filters = ListFilters {
                tag: tag.clone(),
                group: group.clone(),
                author: author.clone(),
                id: id.clone(),
                search: search.clone(),
            };
            cmd_list(
                conn,
                &profile,
                *verbose,
                filter,
                filters,
                sort.clone(),
                dir.clone(),
                *json,
            )
        }
        super::CtlCommand::Add { folder, name } => cmd_add(conn, folder, name.as_deref()),
        super::CtlCommand::Init { folder } => cmd_init(conn, folder),
        super::CtlCommand::Remove { ident, yes } => cmd_remove(conn, ident, *yes),
        super::CtlCommand::Enable { ident } => {
            let profile = active()?;
            cmd_enable(conn, &profile, ident)
        }
        super::CtlCommand::Disable { ident, yes } => {
            let profile = active()?;
            cmd_disable(conn, &profile, ident, *yes)
        }
        super::CtlCommand::Order { ident, new_order } => {
            let profile = active()?;
            cmd_order(conn, &profile, ident, *new_order)
        }
        super::CtlCommand::Reorder { mods } => {
            let profile = active()?;
            cmd_reorder(conn, &profile, mods)
        }
        super::CtlCommand::Rename {
            ident,
            new_name,
            folder,
        } => cmd_rename(conn, ident, new_name, *folder),
        super::CtlCommand::Info {
            ident,
            verbose,
            json,
        } => {
            let profile = active()?;
            cmd_info(conn, &profile, ident, *verbose, *json)
        }
        super::CtlCommand::Open { ident, url } => cmd_open(conn, ident, *url),
        super::CtlCommand::Export { path } => cmd_export(conn, path.as_deref()),
        super::CtlCommand::Import { path, force } => cmd_import(conn, path, *force),
        super::CtlCommand::Health { conflicts } => cmd_health(conn, profile_ident, *conflicts),
        super::CtlCommand::Discover => cmd_discover(conn),
        super::CtlCommand::Clean => cmd_clean(conn),
        super::CtlCommand::Conflicts { json } => cmd_conflicts(conn, profile_ident, *json),
        super::CtlCommand::Which { path } => cmd_which(conn, profile_ident, path),
        super::CtlCommand::Dep { action } => match action {
            super::DepAction::Add {
                mod_ident,
                dep_ident,
                optional,
            } => cmd_dep_add(conn, mod_ident, dep_ident, *optional),
            super::DepAction::Remove {
                mod_ident,
                dep_ident,
            } => cmd_dep_rm(conn, mod_ident, dep_ident),
        },
        super::CtlCommand::Tag { action } => match action {
            super::TagAction::Set { ident, tags } => cmd_tag_set(conn, ident, tags),
            super::TagAction::Add { ident, tags } => cmd_tag_add(conn, ident, tags),
            super::TagAction::Remove { ident, tags } => cmd_tag_remove(conn, ident, tags),
        },
        super::CtlCommand::Manifest { action } => match action {
            super::ManifestAction::Set { ident } => cmd_manifest_set(conn, ident),
        },
        super::CtlCommand::Modloader { action } => match action {
            super::ModloaderAction::Show { json } => cmd_modloader_show(conn, profile_ident, *json),
        },
        super::CtlCommand::Data { action } => cmd_data(conn, action, profile_ident),
        super::CtlCommand::OpenUrl { url } => cmd_open_url(url),
        super::CtlCommand::Profile { action } => cmd_profile(conn, action),
        super::CtlCommand::Group { action } => cmd_group(conn, action, profile_ident),
    }
}

fn resolve_mod(conn: &Connection, ident: &str) -> anyhow::Result<db::ModIdentity> {
    let id = db::resolve_mod_ident(conn, ident)?;
    db::get_mod_by_id(conn, id)?.ok_or_else(|| anyhow::anyhow!("Mod no encontrado"))
}

/// Display metadata for a mod: the `mod.toml` manifest wins when the mods dir
/// is known; otherwise the cached DB metadata is used. The display name comes
/// from the manifest when present, falling back to the DB name.
fn display_meta(
    conn: &Connection,
    mods_dir: Option<&std::path::Path>,
    id: i64,
    folder: &str,
    db_name: &str,
) -> (String, db::ModMetaCache) {
    if let Some(mods_dir) = mods_dir {
        match gta_mo_core::meta::read_mod_meta(mods_dir, folder) {
            Ok(Some(meta)) => {
                let name = meta.name.clone().unwrap_or_else(|| db_name.to_string());
                return (name, db::meta_cache_from_meta(&meta));
            }
            Ok(None) => {}
            Err(e) => log::warn(format!(
                "{}: {e}",
                mods_dir.join(folder).join("mod.toml").display()
            )),
        }
    }
    (
        db_name.to_string(),
        db::load_mod_meta(conn, id).unwrap_or_default(),
    )
}

/// Optional mods dir resolved from config; `ctl` works without it (DB-only).
fn mods_dir_from_config() -> Option<std::path::PathBuf> {
    gta_mo_core::config::load_config()
        .ok()
        .map(|cfg| gta_mo_core::config::RuntimePaths::from_config(&cfg).mods_dir)
}

/// Scans `mods/` and refreshes metadata + manifest dependencies. Unlike
/// `launch --discover`, this never requires Proton or the overlay tools, so it
/// works even when the game is not launchable yet.
fn cmd_discover(conn: &Connection) -> anyhow::Result<()> {
    let mods_dir = mods_dir_from_config().ok_or_else(|| {
        anyhow::anyhow!(
            "No se pudo resolver el directorio de mods (revisa game_root/mods_dir en la config)."
        )
    })?;
    let (new_count, orphan_count) = db::discover_mods(conn, &mods_dir)?;
    log::info(format!(
        "Descubrimiento completado: {new_count} nuevo(s), {orphan_count} huérfano(s)."
    ));
    Ok(())
}

/// Removes orphaned mod entries (folders no longer on disk) from the database.
fn cmd_clean(conn: &Connection) -> anyhow::Result<()> {
    let mods_dir = mods_dir_from_config().ok_or_else(|| {
        anyhow::anyhow!(
            "No se pudo resolver el directorio de mods (revisa game_root/mods_dir en la config)."
        )
    })?;
    db::clean_orphans(conn, &mods_dir)?;
    Ok(())
}

/// Expands `guides` entries that point to a directory into their files, so a
/// manifest can use `guides = ["guides"]` to include a whole folder.
fn expand_guides(mods_dir: &std::path::Path, folder: &str, guides: Vec<String>) -> Vec<String> {
    let mod_dir = mods_dir.join(folder);
    let mut out = Vec::new();
    for g in guides {
        if !gta_mo_core::meta::valid_relative_path(&g) {
            log::warn(format!(
                "guía ignorada en '{}': '{g}' apunta fuera de la carpeta del mod",
                folder
            ));
            continue;
        }
        let p = mod_dir.join(&g);
        if p.is_dir() {
            let mut files: Vec<String> = std::fs::read_dir(&p)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    format!("{}/{}", g.trim_end_matches('/'), name)
                })
                .collect();
            files.sort();
            if files.is_empty() {
                out.push(g);
            } else {
                out.extend(files);
            }
        } else {
            out.push(g);
        }
    }
    out
}

const MOD_TOML_TEMPLATE: &str = r#"# GTA Mod Organizer manifest
# Todos los campos son opcionales. Descomenta y rellena los que quieras.

# id = "autor:slug"            # id estable (autor + nombre, ambos en minusculas)
# name = "NOMBRE"
# version = "1.0.0"
# author = ["AUTOR"]           # string o lista
# url = "https://..."
# description = "Descripción del mod."
# tags = ["tag1", "tag2"]      # para organizar/filtrar

# Carátula y guías (rutas relativas dentro de esta carpeta)
# cover = "cover.png"
# guides = ["guides/instalacion.md"]

# Galería de imágenes extra: una carpeta (se expande) o una lista de rutas.
# screenshots = ["capturas"]

# Subdirectorios cuyo CONTENIDO se monta sobre la raíz del juego.
# Sin esta clave se monta la carpeta entera (comportamiento por defecto).
# mount = ["content"]

# Dependencias (referencias por id autor:slug o por carpeta)
# [dependencies]
# required = ["otro:mod"]      # sin esto el mod no funciona
# optional = []

# Si es un pack de mods, lista sus componentes (solo metadata)
# [[components]]
# name = "Componente"
# version = "1.0.0"
# author = "Autor"
# url = "https://..."
# path = "content/carpeta-del-componente"
"#;

fn cmd_init(conn: &Connection, folder: &str) -> anyhow::Result<()> {
    if !valid_folder_name(folder) {
        anyhow::bail!("Nombre de carpeta no válido: '{folder}'.");
    }
    let cfg =
        gta_mo_core::config::load_config().map_err(|e| anyhow::anyhow!("Error de config: {e}"))?;
    let paths = gta_mo_core::config::RuntimePaths::from_config(&cfg);

    let mod_dir = paths.mods_dir.join(folder);
    std::fs::create_dir_all(&mod_dir)?;

    let meta_path = mod_dir.join("mod.toml");
    if meta_path.exists() {
        anyhow::bail!("{} ya existe.", meta_path.display());
    }
    std::fs::write(&meta_path, MOD_TOML_TEMPLATE)?;
    log::info(format!("Plantilla creada: {}", meta_path.display()));

    let registered = match db::get_mod_by_folder(conn, folder)? {
        Some(m) => Some(m.id),
        None => {
            let display_name = folder.replace('_', " ");
            let id = db::add_mod_to_all_profiles(conn, folder, &display_name)?;
            log::info(format!(
                "Mod registrado: [{id}] '{folder}' -> '{display_name}' (desactivado)"
            ));
            Some(id)
        }
    };

    if let Some(id) = registered {
        let meta = gta_mo_core::meta::read_mod_meta(&paths.mods_dir, folder)?;
        db::update_mod_meta(conn, id, &meta)?;
        log::info(format!("Metadata actualizada para '{}'.", folder));
    }
    Ok(())
}

fn cmd_profile(conn: &Connection, action: &super::ProfileAction) -> anyhow::Result<()> {
    match action {
        super::ProfileAction::List { json } => {
            let profiles = db::list_profiles(conn)?;
            let active = db::active_profile(conn)?;

            if *json {
                #[derive(Serialize)]
                struct ProfileJson {
                    id: i64,
                    name: String,
                    slug: String,
                    active: bool,
                    mods: i64,
                    enabled: i64,
                }
                let mut out = Vec::new();
                for p in &profiles {
                    let (total, enabled) = db::profile_mod_count(conn, p.id)?;
                    out.push(ProfileJson {
                        id: p.id,
                        name: p.name.clone(),
                        slug: p.slug.clone(),
                        active: p.id == active.id,
                        mods: total,
                        enabled,
                    });
                }
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }

            let mut rows = Vec::new();
            for p in &profiles {
                let (total, enabled) = db::profile_mod_count(conn, p.id)?;
                let mark = if p.id == active.id {
                    "*".green().to_string()
                } else {
                    "".to_string()
                };
                rows.push(vec![
                    p.id.to_string(),
                    mark,
                    p.name.clone(),
                    p.slug.clone(),
                    total.to_string(),
                    enabled.to_string(),
                ]);
            }
            println!(
                "{}",
                render_table(
                    vec![
                        "ID".to_string(),
                        "ACTIVO".to_string(),
                        "NOMBRE".to_string(),
                        "SLUG".to_string(),
                        "MODS".to_string(),
                        "ON".to_string(),
                    ],
                    rows,
                )
            );
            Ok(())
        }
        super::ProfileAction::Create { name } => {
            let id = db::create_profile(conn, name)?;
            let p = db::get_profile_by_id(conn, id)?
                .ok_or_else(|| anyhow::anyhow!("Perfil no creado"))?;
            log::info(format!("Perfil '{}' creado (slug: {}).", p.name, p.slug));
            Ok(())
        }
        super::ProfileAction::Delete { ident, yes } => {
            let p = db::resolve_profile(conn, ident)?;

            if !yes {
                eprintln!();
                log::warn(format!(
                    "Vas a eliminar el perfil '{}' (slug: {}).",
                    p.name, p.slug
                ));
                log::warn("Se eliminarán sus estados de mods y su directorio en run/profiles/.");
                eprintln!();
                eprint!("Confirmar eliminación? [s/N]: ");
                std::io::Write::flush(&mut std::io::stderr()).ok();

                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                let confirm = input.trim().to_lowercase();
                if confirm != "s" && confirm != "si" {
                    log::info("Cancelado.");
                    return Ok(());
                }
            }

            let slug = p.slug.clone();
            db::delete_profile(conn, p.id)?;

            if let Ok(cfg) = gta_mo_core::config::load_config() {
                let paths = gta_mo_core::config::RuntimePaths::from_config(&cfg);
                let dir = paths.profiles_root.join(&slug);
                if dir.exists() {
                    std::fs::remove_dir_all(&dir).ok();
                    log::info(format!("Directorio '{}' eliminado.", dir.display()));
                }
            }
            log::info(format!("Perfil '{}' eliminado.", p.name));
            Ok(())
        }
        super::ProfileAction::Use { ident } => {
            let p = db::resolve_profile(conn, ident)?;
            db::set_active_profile(conn, p.id)?;
            log::info(format!("Perfil activo: '{}' (slug: {}).", p.name, p.slug));
            Ok(())
        }
        super::ProfileAction::Rename { ident, new_name } => {
            let p = db::resolve_profile(conn, ident)?;
            let old_name = p.name.clone();
            let (old_slug, new_slug) = db::rename_profile_with_slug(conn, p.id, new_name)?;

            // Move run/profiles/<slug> so the runtime directory follows the name.
            if old_slug != new_slug {
                if let Ok(cfg) = gta_mo_core::config::load_config() {
                    let root = gta_mo_core::config::RuntimePaths::from_config(&cfg).profiles_root;
                    let old_dir = root.join(&old_slug);
                    let new_dir = root.join(&new_slug);
                    if new_dir.exists() {
                        log::warn(format!(
                            "El directorio '{}' ya existe; no se mueve.",
                            new_dir.display()
                        ));
                    } else if old_dir.exists() {
                        if let Err(e) = std::fs::rename(&old_dir, &new_dir) {
                            let _ = conn.execute(
                                "UPDATE profiles SET name = ?1, slug = ?2 WHERE id = ?3",
                                params![old_name, old_slug, p.id],
                            );
                            return Err(anyhow::anyhow!(
                                "No se pudo mover '{}' a '{}': {e}",
                                old_dir.display(),
                                new_dir.display()
                            ));
                        }
                        log::info(format!(
                            "Directorio movido: {} -> {}.",
                            old_dir.display(),
                            new_dir.display()
                        ));
                    }
                }
            }

            log::info(format!(
                "Perfil renombrado de '{old_name}' a '{new_name}' (slug: {old_slug} -> {new_slug})."
            ));
            Ok(())
        }
        super::ProfileAction::Copy { source, new_name } => {
            let src = db::resolve_profile(conn, source)?;
            let id = db::copy_profile(conn, src.id, new_name)?;
            let p = db::get_profile_by_id(conn, id)?
                .ok_or_else(|| anyhow::anyhow!("Perfil no creado"))?;
            log::info(format!(
                "Perfil '{}' copiado a '{}' (slug: {}).",
                src.name, p.name, p.slug
            ));
            Ok(())
        }
        super::ProfileAction::Diff { a, b } => {
            let pa = db::resolve_profile(conn, a)?;
            let pb = db::resolve_profile(conn, b)?;
            let state_of = |conn: &Connection,
                            pid: i64|
             -> anyhow::Result<
                std::collections::HashMap<i64, (String, bool, i64)>,
            > {
                db::load_all_mods_for_profile(conn, pid)?
                    .into_iter()
                    .map(|m| Ok((m.id, (m.folder_name, m.enabled, m.load_order))))
                    .collect()
            };
            let sa = state_of(conn, pa.id)?;
            let sb = state_of(conn, pb.id)?;

            let mut only_in_a: Vec<&(String, bool, i64)> = sa
                .iter()
                .filter(|(id, (_, en, _))| *en && !sb.get(id).map(|(_, e, _)| *e).unwrap_or(false))
                .map(|(_, v)| v)
                .collect();
            only_in_a.sort();
            let mut only_in_b: Vec<&(String, bool, i64)> = sb
                .iter()
                .filter(|(id, (_, en, _))| *en && !sa.get(id).map(|(_, e, _)| *e).unwrap_or(false))
                .map(|(_, v)| v)
                .collect();
            only_in_b.sort();
            let mut order_diff: Vec<(&String, &i64, &i64)> = Vec::new();
            for (id, (folder, en, oa)) in &sa {
                if !*en {
                    continue;
                }
                if let Some((_, true, ob)) = sb.get(id) {
                    if oa != ob {
                        order_diff.push((folder, oa, ob));
                    }
                }
            }
            order_diff.sort();

            if only_in_a.is_empty() && only_in_b.is_empty() && order_diff.is_empty() {
                println!("Sin diferencias entre '{}' y '{}'.", pa.name, pb.name);
                return Ok(());
            }
            if !only_in_a.is_empty() {
                println!(
                    "{}",
                    render_table(
                        vec!["Solo en".to_string(), "Mod".to_string()],
                        only_in_a
                            .iter()
                            .map(|(f, _, _)| vec![pa.name.clone(), f.clone()])
                            .collect(),
                    )
                );
                println!();
            }
            if !only_in_b.is_empty() {
                println!(
                    "{}",
                    render_table(
                        vec!["Solo en".to_string(), "Mod".to_string()],
                        only_in_b
                            .iter()
                            .map(|(f, _, _)| vec![pb.name.clone(), f.clone()])
                            .collect(),
                    )
                );
                println!();
            }
            if !order_diff.is_empty() {
                println!();
                println!(
                    "{}",
                    render_table(
                        vec![
                            "Mod".to_string(),
                            format!("{} (orden)", pa.name),
                            format!("{} (orden)", pb.name)
                        ],
                        order_diff
                            .iter()
                            .map(|(f, a, b)| vec![f.to_string(), a.to_string(), b.to_string()])
                            .collect(),
                    )
                );
            }
            Ok(())
        }
    }
}

fn resolve_active_profile(
    conn: &Connection,
    profile_ident: Option<&str>,
) -> anyhow::Result<db::Profile> {
    match profile_ident {
        Some(ident) => db::resolve_profile(conn, ident),
        None => db::active_profile(conn),
    }
}

fn cmd_group(
    conn: &Connection,
    action: &super::GroupAction,
    profile_ident: Option<&str>,
) -> anyhow::Result<()> {
    match action {
        super::GroupAction::List { json } => {
            let groups = db::list_groups(conn)?;
            if *json {
                #[derive(Serialize)]
                struct GroupJson {
                    id: i64,
                    name: String,
                    slug: String,
                    mods: i64,
                }
                let mut out = Vec::new();
                for g in &groups {
                    out.push(GroupJson {
                        id: g.id,
                        name: g.name.clone(),
                        slug: g.slug.clone(),
                        mods: db::group_mod_count(conn, g.id)?,
                    });
                }
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
            if groups.is_empty() {
                println!("No hay grupos.");
                return Ok(());
            }
            let rows: Vec<Vec<String>> = groups
                .iter()
                .map(|g| {
                    vec![
                        g.id.to_string(),
                        g.name.clone(),
                        g.slug.clone(),
                        db::group_mod_count(conn, g.id).unwrap_or(0).to_string(),
                    ]
                })
                .collect();
            println!(
                "{}",
                render_table(
                    vec![
                        "ID".to_string(),
                        "NOMBRE".to_string(),
                        "SLUG".to_string(),
                        "MODS".to_string(),
                    ],
                    rows,
                )
            );
            Ok(())
        }
        super::GroupAction::Create { name } => {
            let id = db::create_group(conn, name)?;
            let g =
                db::get_group_by_id(conn, id)?.ok_or_else(|| anyhow::anyhow!("Grupo no creado"))?;
            log::info(format!("Grupo '{}' creado (slug: {}).", g.name, g.slug));
            Ok(())
        }
        super::GroupAction::Rename { ident, new_name } => {
            let g = db::resolve_group(conn, ident)?;
            let old = g.name.clone();
            db::rename_group(conn, g.id, new_name)?;
            log::info(format!(
                "Grupo renombrado de '{old}' a '{new_name}' (slug '{}' sin cambios).",
                g.slug
            ));
            Ok(())
        }
        super::GroupAction::Delete { ident, yes } => {
            let g = db::resolve_group(conn, ident)?;
            if !yes {
                let count = db::group_mod_count(conn, g.id)?;
                eprintln!();
                log::warn(format!(
                    "Vas a eliminar el grupo '{}' (slug: {}) con {count} membresías.",
                    g.name, g.slug
                ));
                eprintln!();
                eprint!("Confirmar eliminación? [s/N]: ");
                std::io::Write::flush(&mut std::io::stderr()).ok();
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                let confirm = input.trim().to_lowercase();
                if confirm != "s" && confirm != "si" {
                    log::info("Cancelado.");
                    return Ok(());
                }
            }
            db::delete_group(conn, g.id)?;
            log::info(format!("Grupo '{}' eliminado.", g.name));
            Ok(())
        }
        super::GroupAction::Add {
            mod_ident,
            group_ident,
            global,
        } => {
            let m = resolve_mod(conn, mod_ident)?;
            let g = db::resolve_group(conn, group_ident)?;
            let profile_id = if *global {
                None
            } else {
                Some(resolve_active_profile(conn, profile_ident)?.id)
            };
            if db::add_group_membership(conn, g.id, m.id, profile_id)? {
                let scope = if *global { "global" } else { "perfil actual" };
                log::info(format!(
                    "'{}' añadido al grupo '{}' ({scope}).",
                    m.folder_name, g.name
                ));
            } else {
                log::warn(format!(
                    "'{}' ya está en el grupo '{}'.",
                    m.folder_name, g.name
                ));
            }
            Ok(())
        }
        super::GroupAction::Remove {
            mod_ident,
            group_ident,
            global,
        } => {
            let m = resolve_mod(conn, mod_ident)?;
            let g = db::resolve_group(conn, group_ident)?;
            let profile_id = if *global {
                None
            } else {
                Some(resolve_active_profile(conn, profile_ident)?.id)
            };
            if db::remove_group_membership(conn, g.id, m.id, profile_id)? {
                let scope = if *global { "global" } else { "perfil actual" };
                log::info(format!(
                    "'{}' quitado del grupo '{}' ({scope}).",
                    m.folder_name, g.name
                ));
            } else {
                log::warn(format!(
                    "'{}' no está en el grupo '{}'.",
                    m.folder_name, g.name
                ));
            }
            Ok(())
        }
        super::GroupAction::Enable { group_ident } => {
            let g = db::resolve_group(conn, group_ident)?;
            let profile = resolve_active_profile(conn, profile_ident)?;
            let members = db::mods_in_group(conn, g.id, profile.id)?;
            let mut visited = std::collections::HashSet::new();
            let mut enabled = 0usize;
            for mod_id in members {
                if let Ok(Some(m)) = db::get_mod_by_id(conn, mod_id) {
                    db::enable_mod_with_deps(conn, profile.id, mod_id, &mut visited)?;
                    enabled += 1;
                    log::info(format!("    [+] {} activado", m.folder_name));
                }
            }
            log::info(format!(
                "Grupo '{}': {enabled} mod(s) activado(s) en '{}' (con dependencias requeridas).",
                g.name, profile.name
            ));
            Ok(())
        }
        super::GroupAction::Disable { group_ident } => {
            let g = db::resolve_group(conn, group_ident)?;
            let profile = resolve_active_profile(conn, profile_ident)?;
            let members = db::mods_in_group(conn, g.id, profile.id)?;
            let mut disabled = 0usize;
            for mod_id in members {
                if let Ok(Some(m)) = db::get_mod_by_id(conn, mod_id) {
                    db::set_mod_enabled(conn, profile.id, mod_id, false)?;
                    disabled += 1;
                    log::info(format!("    [-] {} desactivado", m.folder_name));
                }
            }
            log::info(format!(
                "Grupo '{}': {disabled} mod(s) desactivado(s) en '{}'.",
                g.name, profile.name
            ));
            Ok(())
        }
    }
}

/// Filters for `ctl list`. All of them combine with AND.
#[derive(Default, Clone)]
struct ListFilters {
    tag: Option<String>,
    group: Option<String>,
    author: Option<String>,
    id: Option<String>,
    search: Option<String>,
}

fn mod_matches_filters(
    conn: &Connection,
    mods_dir: Option<&std::path::Path>,
    m: &db::ModEntry,
    filters: &ListFilters,
    group_ids: &std::collections::HashSet<i64>,
) -> bool {
    if filters.group.is_some() && !group_ids.contains(&m.id) {
        return false;
    }
    if filters.tag.is_none()
        && filters.author.is_none()
        && filters.id.is_none()
        && filters.search.is_none()
    {
        return true;
    }

    let (name, meta) = display_meta(conn, mods_dir, m.id, &m.folder_name, &m.name);

    if let Some(tag) = &filters.tag {
        if !meta.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
            return false;
        }
    }
    if let Some(author) = &filters.author {
        let a = author.to_lowercase();
        if !meta.author.iter().any(|x| x.to_lowercase().contains(&a)) {
            return false;
        }
    }
    if let Some(id) = &filters.id {
        let matches_mod_id = meta
            .mod_id
            .as_deref()
            .map(|x| x.eq_ignore_ascii_case(id))
            .unwrap_or(false);
        let matches_folder = m.folder_name.eq_ignore_ascii_case(id);
        if !matches_mod_id && !matches_folder {
            return false;
        }
    }
    if let Some(search) = &filters.search {
        let s = search.to_lowercase();
        let mut haystack = name.to_lowercase();
        haystack.push(' ');
        haystack.push_str(&m.folder_name.to_lowercase());
        haystack.push(' ');
        haystack.push_str(&meta.author.join(" ").to_lowercase());
        if let Some(id) = &meta.mod_id {
            haystack.push(' ');
            haystack.push_str(&id.to_lowercase());
        }
        if let Some(d) = &meta.description {
            haystack.push(' ');
            haystack.push_str(&d.to_lowercase());
        }
        haystack.push(' ');
        haystack.push_str(&meta.tags.join(" ").to_lowercase());
        if !haystack.contains(&s) {
            return false;
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn cmd_list(
    conn: &Connection,
    profile: &db::Profile,
    verbose: bool,
    filter: Option<&str>,
    filters: ListFilters,
    sort: Option<String>,
    dir: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    let mods_dir = mods_dir_from_config();

    let group_ids: std::collections::HashSet<i64> = match &filters.group {
        Some(g) => {
            let group = db::resolve_group(conn, g)?;
            db::mods_in_group(conn, group.id, profile.id)?
                .into_iter()
                .collect()
        }
        None => std::collections::HashSet::new(),
    };

    let all_profile_mods = db::load_all_mods_for_profile(conn, profile.id)?;
    let any_registered = !all_profile_mods.is_empty();
    let mut mods = all_profile_mods
        .into_iter()
        .filter(|m| match filter {
            Some("enabled") => m.enabled,
            Some("disabled") => !m.enabled,
            _ => true,
        })
        .filter(|m| mod_matches_filters(conn, mods_dir.as_deref(), m, &filters, &group_ids))
        .collect::<Vec<_>>();

    if let Some(field) = &sort {
        let desc = dir.as_deref() == Some("desc") || (field == "order" && dir.is_none());
        let mut decorated: Vec<(db::ModEntry, String, db::ModMetaCache)> = mods
            .iter()
            .map(|m| {
                let (name, meta) =
                    display_meta(conn, mods_dir.as_deref(), m.id, &m.folder_name, &m.name);
                (m.clone(), name, meta)
            })
            .collect();
        let key = |m: &db::ModEntry, name: &str, meta: &db::ModMetaCache| -> String {
            match field.as_str() {
                "name" => name.to_lowercase(),
                "folder" => m.folder_name.to_lowercase(),
                "author" => meta.author.join(" ").to_lowercase(),
                "order" => format!("{:010}", m.load_order),
                "mod_id" => meta.mod_id.clone().unwrap_or_default().to_lowercase(),
                "version" => meta.version.clone().unwrap_or_default(),
                "status" => format!("{}", m.enabled as u8),
                _ => String::new(),
            }
        };
        decorated.sort_by(|a, b| {
            let ka = key(&a.0, &a.1, &a.2);
            let kb = key(&b.0, &b.1, &b.2);
            let ord = ka.cmp(&kb);
            if desc {
                ord.reverse()
            } else {
                ord
            }
        });
        mods = decorated.into_iter().map(|(m, _, _)| m).collect();
    }

    if json {
        let mut out = Vec::new();
        for m in &mods {
            let (name, meta) =
                display_meta(conn, mods_dir.as_deref(), m.id, &m.folder_name, &m.name);
            let deps = db::get_dependencies_of(conn, profile.id, m.id)?
                .into_iter()
                .map(|(d, req)| DepJson::from_entry(&d, req))
                .collect::<Vec<_>>();
            let dependents = db::get_dependents_of(conn, profile.id, m.id)?
                .into_iter()
                .map(|d| d.id)
                .collect::<Vec<_>>();
            let variant = VariantJson::from_cache(&meta);
            out.push(ModJson {
                id: m.id,
                folder: m.folder_name.clone(),
                name,
                enabled: m.enabled,
                order: m.load_order,
                mod_id: meta.mod_id,
                version: meta.version,
                author: meta.author,
                url: meta.url,
                description: meta.description,
                cover: meta.cover,
                mount: meta.mount,
                guides: meta.guides,
                screenshots: meta.screenshots,
                tags: meta.tags,
                deps,
                dependents,
                variant,
                conflicts: meta.conflicts,
                modloader_priority: meta.modloader_priority,
            });
        }
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    let count = mods.len();
    if count == 0 {
        if any_registered {
            println!("Ningún mod coincide con los filtros.");
        } else {
            println!("No hay mods registrados.");
        }
        return Ok(());
    }

    let mut headers = vec!["ID", "ACTIVO", "ORDEN", "CARPETA", "NOMBRE"];
    if verbose {
        headers.extend(["VERSIÓN", "AUTOR", "DEPS"]);
    }
    let headers: Vec<String> = headers.into_iter().map(String::from).collect();

    let mut rows: Vec<Vec<String>> = Vec::new();
    for m in &mods {
        let status = if m.enabled {
            "SI".green().to_string()
        } else {
            "NO".red().to_string()
        };
        let (name, meta) = display_meta(conn, mods_dir.as_deref(), m.id, &m.folder_name, &m.name);
        let mut row = vec![
            m.id.to_string(),
            status,
            m.load_order.to_string(),
            m.folder_name.clone(),
            name,
        ];
        if verbose {
            row.push(meta.version.clone().unwrap_or_default());
            row.push(meta.author.join(", "));
            let deps = db::get_dependencies_of(conn, profile.id, m.id)?;
            let dependents = db::get_dependents_of(conn, profile.id, m.id)?;
            let mut dep_lines: Vec<String> = Vec::new();
            for (d, req) in &deps {
                let kind = if *req { "requerido" } else { "opcional" };
                dep_lines.push(format!("-> {} ({kind})", d.folder_name));
            }
            for d in &dependents {
                dep_lines.push(format!("<- {}", d.folder_name));
            }
            row.push(dep_lines.join("\n"));
        }
        rows.push(row);
    }

    println!("{}", render_table(headers, rows));
    println!();
    log::info(format!("Total: {count} mod(s)"));
    Ok(())
}

fn cmd_add(conn: &Connection, folder: &str, name: Option<&str>) -> anyhow::Result<()> {
    if !valid_folder_name(folder) {
        anyhow::bail!("El nombre de carpeta no puede contener ':', '|', '/' ni '\\', ni una coma, ni empezar por '.', ni ser '.', '..', ni llevar espacios alrededor.");
    }
    if db::mod_exists(conn, folder)? {
        anyhow::bail!("El mod '{folder}' ya existe en la base de datos.");
    }

    let display_name = name
        .map(|n| n.to_string())
        .unwrap_or_else(|| folder.replace('_', " "));

    let id = db::add_mod_to_all_profiles(conn, folder, &display_name)?;
    log::info(format!(
        "Mod añadido: [{id}] '{folder}' -> '{display_name}' (desactivado en todos los perfiles)"
    ));
    Ok(())
}

fn cmd_remove(conn: &Connection, ident: &str, yes: bool) -> anyhow::Result<()> {
    let m = resolve_mod(conn, ident)?;
    let id = m.id;
    let dep_count = db::count_deps_for_mod(conn, id)?;

    if !yes {
        eprintln!();
        log::warn(format!(
            "Vas a eliminar el mod '{}' (id={}).",
            m.folder_name, id
        ));
        if dep_count > 0 {
            log::warn(format!(
                "Tiene {dep_count} relacion(es) de dependencia que se eliminaran tambien."
            ));
        }
        eprintln!();
        eprint!("Confirmar eliminacion? [s/N]: ");
        std::io::Write::flush(&mut std::io::stderr()).ok();

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let confirm = input.trim().to_lowercase();

        if confirm != "s" && confirm != "si" {
            log::info("Cancelado.");
            return Ok(());
        }
    }

    db::remove_mod(conn, id)?;
    log::info(format!("Mod '{}' eliminado.", m.folder_name));
    Ok(())
}

/// Resolves a conflict reference (by `author:slug` or folder) to a folder.
fn resolve_conflict_ref(
    reference: &str,
    by_id: &std::collections::HashMap<String, String>,
    folders: &std::collections::HashSet<String>,
) -> Option<String> {
    let normalized = gta_mo_core::meta::normalize_mod_id(reference);
    if gta_mo_core::meta::valid_mod_id(&normalized) {
        if let Some(f) = by_id.get(&normalized) {
            return Some(f.clone());
        }
    }
    let trimmed = reference.trim().to_string();
    folders.contains(&trimmed).then_some(trimmed)
}

/// Enforces variant exclusivity and declared conflicts before enabling `target`.
///
/// Aborts when a declared conflict with an already-enabled mod would result,
/// then disables the other enabled variants of `target`'s family (returning
/// them) so only one member of a family stays active.
fn enforce_enable_constraints(
    conn: &Connection,
    profile: &db::Profile,
    target: &db::ModIdentity,
    mods_dir: &std::path::Path,
) -> anyhow::Result<Vec<String>> {
    let target_meta =
        gta_mo_core::meta::read_mod_meta(mods_dir, &target.folder_name)?.unwrap_or_default();
    let all = db::load_all_mods_for_profile(conn, profile.id)?;

    let mut by_id: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut folders: std::collections::HashSet<String> = std::collections::HashSet::new();
    for m in &all {
        folders.insert(m.folder_name.clone());
        if let Some(id) = gta_mo_core::meta::read_mod_meta(mods_dir, &m.folder_name)
            .ok()
            .flatten()
            .and_then(|mm| mm.id)
        {
            by_id.insert(
                gta_mo_core::meta::normalize_mod_id(&id),
                m.folder_name.clone(),
            );
        }
    }

    // Conflicts in both directions (target -> other, other -> target).
    for other in all.iter().filter(|m| m.enabled && m.id != target.id) {
        let other_meta =
            gta_mo_core::meta::read_mod_meta(mods_dir, &other.folder_name)?.unwrap_or_default();
        let hits = |meta: &gta_mo_core::meta::ModMeta, folder: &str| {
            meta.conflicts
                .iter()
                .any(|r| resolve_conflict_ref(r, &by_id, &folders).as_deref() == Some(folder))
        };
        if hits(&target_meta, &other.folder_name) || hits(&other_meta, &target.folder_name) {
            anyhow::bail!(
                "No se puede activar '{}': es incompatible con '{}' (activado).",
                target.folder_name,
                other.folder_name
            );
        }
    }

    // requires_any: only warn (the alternative can be enabled right after).
    if !target_meta.requires_any.is_empty() {
        let enabled: std::collections::HashSet<String> = all
            .iter()
            .filter(|m| m.enabled)
            .map(|m| m.folder_name.clone())
            .collect();
        let any = target_meta.requires_any.iter().any(|r| {
            resolve_conflict_ref(r, &by_id, &folders)
                .map(|f| enabled.contains(&f))
                .unwrap_or(false)
        });
        if !any {
            log::warn(format!(
                "'{}' requiere al menos uno de: {} (ninguno activado).",
                target.folder_name,
                target_meta.requires_any.join(", ")
            ));
        }
    }

    // Variant exclusivity: disable the other enabled members of the family.
    let mut disabled = Vec::new();
    if let Some(variant) = &target_meta.variant {
        for other in all.iter().filter(|m| m.enabled && m.id != target.id) {
            let om =
                gta_mo_core::meta::read_mod_meta(mods_dir, &other.folder_name)?.unwrap_or_default();
            if om
                .variant
                .as_ref()
                .map(|v| v.group == variant.group)
                .unwrap_or(false)
            {
                db::set_mod_enabled(conn, profile.id, other.id, false)?;
                disabled.push(other.folder_name.clone());
            }
        }
    }
    Ok(disabled)
}

fn cmd_enable(conn: &Connection, profile: &db::Profile, ident: &str) -> anyhow::Result<()> {
    let m = resolve_mod(conn, ident)?;
    let id = m.id;

    // Variant exclusivity + declared conflicts (via the live manifests).
    if let Some(mods_dir) = mods_dir_from_config() {
        let disabled = enforce_enable_constraints(conn, profile, &m, &mods_dir)?;
        for folder in disabled {
            log::warn(format!(
                "Variante '{}' desactivada automáticamente (familia de '{}').",
                folder, m.folder_name
            ));
        }
    }

    let (already_enabled, _) = db::profile_mod_state(conn, profile.id, id)?;

    let before: std::collections::HashSet<i64> =
        db::load_enabled_mod_ids_for_profile(conn, profile.id)?
            .into_iter()
            .collect();
    let mut visited = std::collections::HashSet::new();
    db::enable_mod_with_deps(conn, profile.id, id, &mut visited)?;
    let after: std::collections::HashSet<i64> =
        db::load_enabled_mod_ids_for_profile(conn, profile.id)?
            .into_iter()
            .collect();

    let mut new_folders: Vec<String> = after
        .difference(&before)
        .filter_map(|nid| db::get_mod_by_id(conn, *nid).ok().flatten())
        .map(|x| x.folder_name)
        .collect();
    new_folders.sort();

    if already_enabled {
        if new_folders.is_empty() {
            log::warn(format!("'{}' ya esta activado.", m.folder_name));
        } else {
            log::info(format!(
                "'{}' ya estaba activado; dependencias requeridas activadas: {}.",
                m.folder_name,
                new_folders.join(", ")
            ));
        }
        return Ok(());
    }

    let deps_note = if new_folders.is_empty() {
        String::new()
    } else {
        format!(" (con dependencias: {})", new_folders.join(", "))
    };
    log::info(format!(
        "Mod '{}' activado (perfil '{}'){deps_note}.",
        m.folder_name, profile.name
    ));
    Ok(())
}

fn cmd_disable(
    conn: &Connection,
    profile: &db::Profile,
    ident: &str,
    yes: bool,
) -> anyhow::Result<()> {
    let m = resolve_mod(conn, ident)?;
    let id = m.id;
    let (enabled, _) = db::profile_mod_state(conn, profile.id, id)?;
    if !enabled {
        log::warn(format!("'{}' ya esta desactivado.", m.folder_name));
        return Ok(());
    }

    let dependents = db::get_dependents_of(conn, profile.id, id)?;
    let active_dependents: Vec<_> = dependents.iter().filter(|d| d.enabled).collect();

    if !active_dependents.is_empty() && !yes {
        log::warn(format!(
            "'{}' es requerido por los siguientes mods activos:",
            m.folder_name
        ));
        for d in &active_dependents {
            eprintln!("       - {}", d.folder_name);
        }
        eprintln!();
        eprint!("Desactivar de todas formas? [s/N]: ");
        std::io::Write::flush(&mut std::io::stderr()).ok();

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let confirm = input.trim().to_lowercase();

        if confirm != "s" && confirm != "si" {
            log::info("Cancelado.");
            return Ok(());
        }
    }

    db::set_mod_enabled(conn, profile.id, id, false)?;
    log::info(format!(
        "Mod '{}' desactivado (perfil '{}').",
        m.folder_name, profile.name
    ));
    Ok(())
}

fn cmd_order(
    conn: &Connection,
    profile: &db::Profile,
    ident: &str,
    new_order: i64,
) -> anyhow::Result<()> {
    let m = resolve_mod(conn, ident)?;
    let id = m.id;
    let (_, old_order) = db::profile_mod_state(conn, profile.id, id)?;
    db::set_mod_order(conn, profile.id, id, new_order)?;
    log::info(format!(
        "'{}': orden cambiado de {old_order} a {new_order} (perfil '{}').",
        m.folder_name, profile.name
    ));
    Ok(())
}

fn cmd_reorder(conn: &Connection, profile: &db::Profile, folders: &[String]) -> anyhow::Result<()> {
    if folders.is_empty() {
        anyhow::bail!("Indica al menos un mod para reordenar.");
    }
    let mut ids = Vec::with_capacity(folders.len());
    for folder in folders {
        let m = db::get_mod_by_folder(conn, folder)?
            .ok_or_else(|| anyhow::anyhow!("Mod '{folder}' no encontrado."))?;
        ids.push(m.id);
    }
    db::set_profile_order(conn, profile.id, &ids)?;
    log::info(format!(
        "Orden del perfil '{}' actualizado ({} mod(s), prioridad de arriba a abajo).",
        profile.name,
        ids.len()
    ));
    Ok(())
}

fn cmd_rename(conn: &Connection, ident: &str, new_name: &str, folder: bool) -> anyhow::Result<()> {
    if new_name.is_empty() {
        anyhow::bail!("El nombre no puede estar vacio.");
    }
    let m = resolve_mod(conn, ident)?;
    let id = m.id;

    if folder {
        return cmd_rename_folder(conn, &m, new_name);
    }

    let old_name = m.name.clone();

    // The manifest is the source of truth, so write it first: if it fails the
    // DB is left untouched and the command reports a clean error (instead of
    // half-applying and then failing).
    let mut manifest_updated = false;
    if let Ok(cfg) = gta_mo_core::config::load_config() {
        let paths = gta_mo_core::config::RuntimePaths::from_config(&cfg);
        let manifest = paths.mods_dir.join(&m.folder_name).join("mod.toml");
        if manifest.exists() {
            gta_mo_core::meta::set_meta_name(&paths.mods_dir, &m.folder_name, new_name)?;
            manifest_updated = true;
        }
    }

    if let Err(e) = db::set_mod_name(conn, id, new_name) {
        // Roll back the manifest so both stay consistent.
        if manifest_updated {
            if let Ok(cfg) = gta_mo_core::config::load_config() {
                let paths = gta_mo_core::config::RuntimePaths::from_config(&cfg);
                let _ =
                    gta_mo_core::meta::set_meta_name(&paths.mods_dir, &m.folder_name, &old_name);
            }
        }
        return Err(e);
    }

    if manifest_updated {
        log::info(format!(
            "'{}': nombre cambiado en mod.toml y en la DB de '{old_name}' a '{new_name}'.",
            m.folder_name
        ));
    } else {
        log::info(format!(
            "'{}': nombre cambiado de '{old_name}' a '{new_name}'.",
            m.folder_name
        ));
    }
    Ok(())
}

fn cmd_rename_folder(
    conn: &Connection,
    m: &db::ModIdentity,
    new_folder: &str,
) -> anyhow::Result<()> {
    if !valid_folder_name(new_folder) {
        anyhow::bail!("El nombre de carpeta no puede contener ':', '|', '/' ni '\\', ni una coma, ni empezar por '.', ni ser '.', '..', ni llevar espacios alrededor.");
    }

    let cfg =
        gta_mo_core::config::load_config().map_err(|e| anyhow::anyhow!("Error de config: {e}"))?;
    let paths = gta_mo_core::config::RuntimePaths::from_config(&cfg);

    let old_dir = paths.mods_dir.join(&m.folder_name);
    let new_dir = paths.mods_dir.join(new_folder);

    if new_dir.exists() {
        anyhow::bail!(
            "Ya existe una carpeta '{}' en {}",
            new_folder,
            paths.mods_dir.display()
        );
    }

    let renamed = if old_dir.exists() {
        std::fs::rename(&old_dir, &new_dir).map_err(|e| {
            anyhow::anyhow!(
                "No se pudo renombrar '{}' a '{}': {e}",
                old_dir.display(),
                new_dir.display()
            )
        })?;
        true
    } else {
        log::warn(format!(
            "La carpeta '{}' no existe en disco; solo se actualiza la base de datos.",
            old_dir.display()
        ));
        false
    };

    if let Err(e) = db::set_mod_folder(conn, m.id, new_folder) {
        if renamed {
            let _ = std::fs::rename(&new_dir, &old_dir);
        }
        anyhow::bail!("No se pudo actualizar la base de datos: {e}");
    }

    if renamed {
        log::info(format!(
            "'{}': carpeta renombrada a '{}'.",
            m.folder_name, new_folder
        ));
    } else {
        log::info(format!(
            "'{}': carpeta actualizada en la base de datos a '{}'.",
            m.folder_name, new_folder
        ));
    }
    Ok(())
}

fn cmd_info(
    conn: &Connection,
    profile: &db::Profile,
    ident: &str,
    verbose: bool,
    json: bool,
) -> anyhow::Result<()> {
    let m = resolve_mod(conn, ident)?;
    let id = m.id;
    let (enabled, order) = db::profile_mod_state(conn, profile.id, id)?;

    let deps = db::get_dependencies_of(conn, profile.id, id)?;
    let dependents = db::get_dependents_of(conn, profile.id, id)?;
    let mods_dir = mods_dir_from_config();
    let (name, meta) = display_meta(conn, mods_dir.as_deref(), id, &m.folder_name, &m.name);
    let guides = match mods_dir.as_deref() {
        Some(dir) => expand_guides(dir, &m.folder_name, meta.guides.clone()),
        None => meta.guides.clone(),
    };
    let screenshots = match mods_dir.as_deref() {
        Some(dir) => gta_mo_core::meta::mod_screenshots(dir, &m.folder_name),
        None => meta.screenshots.clone(),
    };
    let profiles = db::mod_enabled_in_profiles(conn, id)?;
    let groups = db::groups_of_mod_in_profile(conn, id, profile.id)?;

    if json {
        #[derive(Serialize)]
        struct InfoJson {
            id: i64,
            folder: String,
            name: String,
            enabled: bool,
            order: i64,
            mod_id: Option<String>,
            version: Option<String>,
            author: Vec<String>,
            url: Option<String>,
            description: Option<String>,
            cover: Option<String>,
            mount: Vec<String>,
            guides: Vec<String>,
            screenshots: Vec<String>,
            tags: Vec<String>,
            pack: bool,
            components: Vec<ComponentJson>,
            groups: Vec<GroupJson>,
            dependencies: Vec<DepJson>,
            dependents: Vec<DepJson>,
            profiles: Vec<ProfileStateJson>,
            variant: Option<VariantJson>,
            conflicts: Vec<String>,
            modloader_priority: Option<i64>,
        }

        #[derive(Serialize)]
        struct GroupJson {
            id: i64,
            name: String,
            slug: String,
        }

        #[derive(Serialize)]
        struct ComponentJson {
            #[serde(skip_serializing_if = "Option::is_none")]
            name: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            version: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            author: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            url: Option<String>,
            #[serde(skip_serializing_if = "Option::is_none")]
            path: Option<String>,
        }

        #[derive(Serialize)]
        struct ProfileStateJson {
            name: String,
            slug: String,
            enabled: bool,
        }

        let pack = meta.is_pack();
        let components_json: Vec<ComponentJson> = meta
            .components
            .iter()
            .map(|c| ComponentJson {
                name: c.name.clone(),
                version: c.version.clone(),
                author: c.author.clone(),
                url: c.url.clone(),
                path: c.path.clone(),
            })
            .collect();

        let profiles_json: Vec<ProfileStateJson> = profiles
            .iter()
            .map(|(p, enabled)| ProfileStateJson {
                name: p.name.clone(),
                slug: p.slug.clone(),
                enabled: *enabled,
            })
            .collect();

        let groups_json: Vec<GroupJson> = groups
            .iter()
            .map(|g| GroupJson {
                id: g.id,
                name: g.name.clone(),
                slug: g.slug.clone(),
            })
            .collect();

        let variant = VariantJson::from_cache(&meta);
        let out = InfoJson {
            id: m.id,
            folder: m.folder_name,
            name,
            enabled,
            order,
            mod_id: meta.mod_id,
            version: meta.version,
            author: meta.author,
            url: meta.url,
            description: meta.description,
            cover: meta.cover,
            mount: meta.mount,
            guides,
            screenshots,
            tags: meta.tags,
            pack,
            components: components_json,
            groups: groups_json,
            dependencies: deps
                .iter()
                .map(|(d, req)| DepJson::from_entry(d, *req))
                .collect(),
            dependents: dependents
                .iter()
                .map(|d| DepJson::from_entry(d, true))
                .collect(),
            profiles: profiles_json,
            variant,
            conflicts: meta.conflicts,
            modloader_priority: meta.modloader_priority,
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    let status = if enabled {
        "Activado".green().to_string()
    } else {
        "Desactivado".red().to_string()
    };
    let kind = if meta.is_pack() {
        format!("pack ({} componentes)", meta.components.len())
    } else {
        "mod".to_string()
    };

    if verbose {
        let label = |l: &str| l.bold().to_string();
        let mut rows: Vec<Vec<String>> = vec![
            vec![label("ID"), m.id.to_string()],
            vec![label("Carpeta"), m.folder_name.clone()],
            vec![label("Nombre"), name.clone()],
        ];
        if let Some(id) = &meta.mod_id {
            rows.push(vec![label("Mod ID"), id.green().to_string()]);
        }
        if let Some(v) = &meta.version {
            rows.push(vec![label("Versión"), v.cyan().to_string()]);
        }
        if !meta.author.is_empty() {
            rows.push(vec![
                label("Autor"),
                meta.author.join(", ").magenta().to_string(),
            ]);
        }
        if let Some(u) = &meta.url {
            rows.push(vec![label("URL"), u.blue().underline().to_string()]);
        }
        if let Some(d) = &meta.description {
            rows.push(vec![label("Descripción"), d.yellow().to_string()]);
        }
        if let Some(c) = &meta.cover {
            rows.push(vec![label("Carátula"), c.clone()]);
        }
        if !meta.tags.is_empty() {
            rows.push(vec![label("Tags"), meta.tags.join(", ")]);
        }
        if !meta.mount.is_empty() {
            rows.push(vec![label("Mount"), meta.mount.join(", ")]);
        }
        rows.push(vec![label("Tipo"), kind.clone().magenta().to_string()]);
        rows.push(vec![label("Estado"), status.clone()]);
        rows.push(vec![label("Orden"), order.to_string()]);
        if !groups.is_empty() {
            let names: Vec<String> = groups.iter().map(|g| g.name.clone()).collect();
            rows.push(vec![label("Grupos"), names.join(", ").cyan().to_string()]);
        }
        println!("\n{}", render_table(vec![], rows));

        if !guides.is_empty() {
            println!("\n  {}", "Guías:".bold());
            println!(
                "{}",
                render_table(
                    vec!["Guías".to_string()],
                    guides.iter().map(|g| vec![g.clone()]).collect(),
                )
            );
        }

        if !screenshots.is_empty() {
            println!("\n  {}", "Capturas:".bold());
            println!(
                "{}",
                render_table(
                    vec!["Imagen".to_string()],
                    screenshots.iter().map(|g| vec![g.clone()]).collect(),
                )
            );
        }

        if !meta.components.is_empty() {
            println!("\n  {}", "Componentes:".bold());
            let rows = meta
                .components
                .iter()
                .map(|c| {
                    vec![
                        c.name.clone().unwrap_or_default(),
                        c.version
                            .clone()
                            .map(|v| format!("v{v}"))
                            .unwrap_or_default(),
                        c.author.clone().unwrap_or_default(),
                        c.url.clone().unwrap_or_default(),
                        c.path.clone().unwrap_or_default(),
                    ]
                })
                .collect();
            println!(
                "{}",
                render_table(
                    vec![
                        "Componente".to_string(),
                        "Versión".to_string(),
                        "Autor".to_string(),
                        "URL".to_string(),
                        "Path".to_string(),
                    ],
                    rows,
                )
            );
        }

        println!("\n  {}", "Perfiles:".bold());
        let rows = profiles
            .iter()
            .map(|(p, enabled)| {
                vec![
                    format!("{}{}", p.name, if p.is_active { " (activo)" } else { "" }),
                    if *enabled {
                        "SI".green().to_string()
                    } else {
                        "NO".red().to_string()
                    },
                ]
            })
            .collect();
        println!(
            "{}",
            render_table(vec!["Perfil".to_string(), "Estado".to_string()], rows)
        );

        if deps.is_empty() {
            println!("\n  {} ninguna", "Dependencias:".cyan());
        } else {
            println!("\n  {}:", "Dependencias".cyan());
            let rows = deps
                .iter()
                .map(|(d, req)| {
                    vec![
                        d.id.to_string(),
                        d.folder_name.clone(),
                        d.name.clone(),
                        if d.enabled {
                            "SI".green().to_string()
                        } else {
                            "NO".red().to_string()
                        },
                        if *req {
                            "requerido".cyan().to_string()
                        } else {
                            "opcional".yellow().to_string()
                        },
                    ]
                })
                .collect();
            println!(
                "{}",
                render_table(
                    vec![
                        "ID".to_string(),
                        "Mod".to_string(),
                        "Nombre".to_string(),
                        "Activo".to_string(),
                        "Tipo".to_string(),
                    ],
                    rows,
                )
            );
        }

        if dependents.is_empty() {
            println!("\n  {} nadie", "Requerido por:".yellow());
        } else {
            println!("\n  {}:", "Requerido por".yellow());
            let rows = dependents
                .iter()
                .map(|d| {
                    vec![
                        d.id.to_string(),
                        d.folder_name.clone(),
                        d.name.clone(),
                        if d.enabled {
                            "SI".green().to_string()
                        } else {
                            "NO".red().to_string()
                        },
                    ]
                })
                .collect();
            println!(
                "{}",
                render_table(
                    vec![
                        "ID".to_string(),
                        "Mod".to_string(),
                        "Nombre".to_string(),
                        "Activo".to_string(),
                    ],
                    rows,
                )
            );
        }
        println!();
        return Ok(());
    }

    // Compact summary (2-column table)
    let mut rows: Vec<(String, String)> = Vec::new();
    rows.push(("Nombre".to_string(), name.clone()));
    rows.push(("Tipo".to_string(), kind.clone()));
    if let Some(id) = &meta.mod_id {
        rows.push(("Mod ID".to_string(), id.green().to_string()));
    }
    if let Some(v) = &meta.version {
        rows.push(("Versión".to_string(), v.cyan().to_string()));
    }
    if !meta.author.is_empty() {
        rows.push((
            "Autor".to_string(),
            meta.author.join(", ").magenta().to_string(),
        ));
    }
    rows.push((
        "Estado".to_string(),
        format!(
            "{} · Orden {}",
            if enabled {
                "Activado".green().to_string()
            } else {
                "Desactivado".red().to_string()
            },
            order
        ),
    ));
    rows.push(("Perfil".to_string(), profile.name.clone()));
    if !groups.is_empty() {
        let names: Vec<String> = groups.iter().map(|g| g.name.clone()).take(3).collect();
        let shown = if groups.len() > 3 {
            format!("{}, …", names.join(", "))
        } else {
            names.join(", ")
        };
        rows.push((
            "Grupos".to_string(),
            format!("{} ({shown})", groups.len()).cyan().to_string(),
        ));
    }
    if !guides.is_empty() {
        rows.push((
            "Guías".to_string(),
            format!("{} archivos (usa -v para listar)", guides.len()),
        ));
    }
    if !meta.components.is_empty() {
        let names: Vec<String> = meta
            .components
            .iter()
            .filter_map(|c| c.name.clone())
            .collect();
        let preview = names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
        let shown = if names.len() > 3 {
            format!("{preview}, …")
        } else {
            preview
        };
        rows.push((
            "Componentes".to_string(),
            format!("{} ({shown})", meta.components.len()),
        ));
    }
    if !deps.is_empty() {
        let names = deps.iter().map(|(d, _)| d.name.clone()).collect::<Vec<_>>();
        rows.push(("Depende de".to_string(), names.join(", ")));
    }
    if !dependents.is_empty() {
        let names: Vec<String> = dependents.iter().map(|d| d.name.clone()).take(3).collect();
        rows.push((
            "Requerido por".to_string(),
            format!("{}, … ({} mods)", names.join(", "), dependents.len()),
        ));
    }

    let table_rows: Vec<Vec<String>> = rows
        .into_iter()
        .map(|(k, v)| vec![k.bold().to_string(), v])
        .collect();
    println!();
    println!("{}", render_table(Vec::new(), table_rows));
    println!();
    Ok(())
}

/// True when `u` is an `http://`/`https://` URL. `ctl open --url` only hands
/// these to `xdg-open`, so a mod manifest cannot invoke arbitrary URI handlers
/// (`file://`, custom `x-scheme-handler`, …).
fn is_http_url(u: &str) -> bool {
    let l = u.trim().to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

fn cmd_open(conn: &Connection, ident: &str, url: bool) -> anyhow::Result<()> {
    let m = resolve_mod(conn, ident)?;
    let cfg =
        gta_mo_core::config::load_config().map_err(|e| anyhow::anyhow!("Error de config: {e}"))?;
    let paths = gta_mo_core::config::RuntimePaths::from_config(&cfg);

    let target = if url {
        let meta =
            gta_mo_core::meta::read_mod_meta(&paths.mods_dir, &m.folder_name)?.unwrap_or_default();
        match meta.url {
            Some(u) => {
                if !is_http_url(&u) {
                    anyhow::bail!(
                        "La URL del mod '{}' no usa http/https y no se abrirá por seguridad: {u}",
                        m.folder_name
                    );
                }
                u
            }
            None => anyhow::bail!("El mod '{}' no tiene URL en su mod.toml.", m.folder_name),
        }
    } else {
        let dir = paths.mods_dir.join(&m.folder_name);
        if !dir.exists() {
            anyhow::bail!("La carpeta del mod no existe: {}", dir.display());
        }
        dir.display().to_string()
    };

    let status = std::process::Command::new("xdg-open")
        .arg(&target)
        .status()
        .map_err(|e| anyhow::anyhow!("No se pudo ejecutar xdg-open: {e}"))?;
    if !status.success() {
        anyhow::bail!("xdg-open terminó con error: {status}");
    }
    log::info(format!("Abriendo '{}'...", target));
    Ok(())
}

fn dep_exists(conn: &Connection, mod_id: i64, dep_id: i64) -> anyhow::Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM mod_dependencies WHERE mod_id = ?1 AND dependency_id = ?2",
        params![mod_id, dep_id],
        |row| row.get(0),
    )?;
    Ok(n > 0)
}

fn cmd_dep_add(
    conn: &Connection,
    mod_ident: &str,
    dep_ident: &str,
    optional: bool,
) -> anyhow::Result<()> {
    let m = resolve_mod(conn, mod_ident)?;
    let d = resolve_mod(conn, dep_ident)?;
    let mod_folder = m.folder_name.clone();
    let dep_folder = d.folder_name.clone();

    if dep_exists(conn, m.id, d.id)? {
        anyhow::bail!("La dependencia ya existe.");
    }

    // The mod.toml manifest is updated first and the DB row last (single
    // source of truth), so a failing write-back never leaves the DB changed
    // while the command reports failure.
    if let Err(e) = dep_writeback(conn, &m, &d, optional, true) {
        anyhow::bail!("No se pudo actualizar el manifest: {e:#}");
    }
    if let Err(e) = db::add_dependency(conn, m.id, d.id, !optional) {
        let _ = dep_writeback(conn, &m, &d, optional, false);
        return Err(e);
    }

    if optional {
        log::info(format!(
            "'{mod_folder}' ahora recomienda '{dep_folder}' (opcional)."
        ));
    } else {
        log::info(format!("'{mod_folder}' ahora depende de '{dep_folder}'."));
    }
    Ok(())
}

fn cmd_dep_rm(conn: &Connection, mod_ident: &str, dep_ident: &str) -> anyhow::Result<()> {
    let m = resolve_mod(conn, mod_ident)?;
    let d = resolve_mod(conn, dep_ident)?;
    let mod_folder = m.folder_name.clone();
    let dep_folder = d.folder_name.clone();

    if !dep_exists(conn, m.id, d.id)? {
        anyhow::bail!("La dependencia no existe.");
    }

    if let Err(e) = dep_writeback(conn, &m, &d, false, false) {
        anyhow::bail!("No se pudo actualizar el manifest: {e:#}");
    }
    if let Err(e) = db::remove_dependency(conn, m.id, d.id) {
        let _ = dep_writeback(conn, &m, &d, false, true);
        return Err(e);
    }

    log::info(format!(
        "Dependencia eliminada: '{mod_folder}' ya no depende de '{dep_folder}'."
    ));
    Ok(())
}

/// If the mod has a manifest, mirrors a DB dependency change into its
/// `[dependencies]` section, referencing the dependency by its stable id when
/// it has one, otherwise by folder name.
fn dep_writeback(
    conn: &Connection,
    m: &db::ModIdentity,
    d: &db::ModIdentity,
    optional: bool,
    add: bool,
) -> anyhow::Result<()> {
    let Ok(cfg) = gta_mo_core::config::load_config() else {
        return Ok(());
    };
    let paths = gta_mo_core::config::RuntimePaths::from_config(&cfg);
    if !paths
        .mods_dir
        .join(&m.folder_name)
        .join("mod.toml")
        .exists()
    {
        return Ok(());
    }
    let dep_ref = db::load_mod_meta(conn, d.id)?
        .mod_id
        .unwrap_or_else(|| d.folder_name.clone());
    gta_mo_core::meta::set_mod_dependency(
        &paths.mods_dir,
        &m.folder_name,
        &dep_ref,
        optional,
        add,
    )?;
    Ok(())
}

// ---------- Tags / manifest editing ----------

/// Normalizes and validates tag arguments (trim, drop a leading `#`, lowercase,
/// deduplicate). Errors on a malformed tag.
fn normalize_tags(raw: &[String]) -> anyhow::Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for t in raw {
        let t = t.trim().trim_start_matches('#').trim().to_lowercase();
        if t.is_empty() {
            continue;
        }
        if !gta_mo_core::meta::valid_tag(&t) {
            anyhow::bail!("Tag inválido: '{t}' (usa minúsculas, dígitos, '-' o '_').");
        }
        if !out.contains(&t) {
            out.push(t);
        }
    }
    Ok(out)
}

fn mods_dir_or_bail() -> anyhow::Result<std::path::PathBuf> {
    mods_dir_from_config().ok_or_else(|| {
        anyhow::anyhow!(
            "No se pudo resolver el directorio de mods (revisa game_root/mods_dir en la config)."
        )
    })
}

/// Refreshes the cached metadata of a mod after editing its `mod.toml`.
fn refresh_meta_cache(
    conn: &Connection,
    mods_dir: &std::path::Path,
    id: i64,
    folder: &str,
) -> anyhow::Result<()> {
    let meta = gta_mo_core::meta::read_mod_meta(mods_dir, folder)?;
    db::update_mod_meta(conn, id, &meta)?;
    Ok(())
}

fn tags_display(tags: &[String]) -> String {
    if tags.is_empty() {
        "(ninguno)".to_string()
    } else {
        tags.iter()
            .map(|t| format!("#{t}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn cmd_tag_set(conn: &Connection, ident: &str, tags: &[String]) -> anyhow::Result<()> {
    let m = resolve_mod(conn, ident)?;
    let mods_dir = mods_dir_or_bail()?;
    let tags = normalize_tags(tags)?;
    gta_mo_core::meta::set_meta_tags(&mods_dir, &m.folder_name, &tags)?;
    refresh_meta_cache(conn, &mods_dir, m.id, &m.folder_name)?;
    log::info(format!(
        "'{}': tags -> {}",
        m.folder_name,
        tags_display(&tags)
    ));
    Ok(())
}

fn cmd_tag_add(conn: &Connection, ident: &str, tags: &[String]) -> anyhow::Result<()> {
    let m = resolve_mod(conn, ident)?;
    let mods_dir = mods_dir_or_bail()?;
    let add = normalize_tags(tags)?;
    let mut current = gta_mo_core::meta::read_mod_meta(&mods_dir, &m.folder_name)?
        .and_then(|meta| meta.tags)
        .unwrap_or_default();
    for t in add {
        if !current.contains(&t) {
            current.push(t);
        }
    }
    gta_mo_core::meta::set_meta_tags(&mods_dir, &m.folder_name, &current)?;
    refresh_meta_cache(conn, &mods_dir, m.id, &m.folder_name)?;
    log::info(format!(
        "'{}': tags -> {}",
        m.folder_name,
        tags_display(&current)
    ));
    Ok(())
}

fn cmd_tag_remove(conn: &Connection, ident: &str, tags: &[String]) -> anyhow::Result<()> {
    let m = resolve_mod(conn, ident)?;
    let mods_dir = mods_dir_or_bail()?;
    let remove = normalize_tags(tags)?;
    let current = gta_mo_core::meta::read_mod_meta(&mods_dir, &m.folder_name)?
        .and_then(|meta| meta.tags)
        .unwrap_or_default();
    let kept: Vec<String> = current
        .into_iter()
        .filter(|t| !remove.contains(t))
        .collect();
    gta_mo_core::meta::set_meta_tags(&mods_dir, &m.folder_name, &kept)?;
    refresh_meta_cache(conn, &mods_dir, m.id, &m.folder_name)?;
    log::info(format!(
        "'{}': tags -> {}",
        m.folder_name,
        tags_display(&kept)
    ));
    Ok(())
}

/// Replaces a mod's whole `mod.toml` with the manifest read from stdin.
fn cmd_manifest_set(conn: &Connection, ident: &str) -> anyhow::Result<()> {
    use std::io::Read;
    let m = resolve_mod(conn, ident)?;
    let mods_dir = mods_dir_or_bail()?;
    let mut content = String::new();
    std::io::stdin()
        .read_to_string(&mut content)
        .map_err(|e| anyhow::anyhow!("No se pudo leer stdin: {e}"))?;
    if content.trim().is_empty() {
        anyhow::bail!("No se recibió ningún mod.toml por stdin.");
    }
    gta_mo_core::meta::write_manifest_validated(&mods_dir, &m.folder_name, &content)?;
    refresh_meta_cache(conn, &mods_dir, m.id, &m.folder_name)?;
    log::info(format!("'{}': mod.toml actualizado.", m.folder_name));
    Ok(())
}

// ---------- Mod Loader priorities ----------

fn cmd_modloader_show(
    conn: &Connection,
    profile_ident: Option<&str>,
    json: bool,
) -> anyhow::Result<()> {
    let profile = resolve_active_profile(conn, profile_ident)?;
    let cfg =
        gta_mo_core::config::load_config().map_err(|e| anyhow::anyhow!("Error de config: {e}"))?;
    let paths = gta_mo_core::config::RuntimePaths::from_config(&cfg);
    let enabled: Vec<String> = db::load_all_mods_for_profile(conn, profile.id)?
        .into_iter()
        .filter(|m| m.enabled)
        .map(|m| m.folder_name)
        .collect();
    let entries = gta_mo_core::modloader::entries_for(&paths.mods_dir, &enabled);
    let ini = gta_mo_core::modloader::ini_path(&paths.profile_paths(&profile.slug).upper);

    if json {
        #[derive(Serialize)]
        struct EntryJson {
            name: String,
            priority: i64,
        }
        #[derive(Serialize)]
        struct Out {
            path: String,
            profile: String,
            entries: Vec<EntryJson>,
        }
        let out = Out {
            path: ini.display().to_string(),
            profile: profile.slug.clone(),
            entries: entries
                .iter()
                .map(|(n, p)| EntryJson {
                    name: n.clone(),
                    priority: *p,
                })
                .collect(),
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    if entries.is_empty() {
        println!("No hay prioridades de ModLoader declaradas por los mods activos.");
        return Ok(());
    }
    let rows: Vec<Vec<String>> = entries
        .iter()
        .map(|(n, p)| vec![n.clone(), p.to_string()])
        .collect();
    println!(
        "{}",
        render_table(vec!["Carpeta".to_string(), "Prioridad".to_string()], rows,)
    );
    log::info(format!("Archivo: {}", ini.display()));
    Ok(())
}

// ---------- Profile user data (saves/tracks/screenshots) ----------

fn xdg_open(target: &std::path::Path) -> anyhow::Result<()> {
    let status = std::process::Command::new("xdg-open")
        .arg(target)
        .status()
        .map_err(|e| anyhow::anyhow!("No se pudo ejecutar xdg-open: {e}"))?;
    if !status.success() {
        anyhow::bail!("xdg-open terminó con error: {status}");
    }
    Ok(())
}

fn cmd_open_url(url: &str) -> anyhow::Result<()> {
    if !is_http_url(url) {
        anyhow::bail!("Solo se permiten URLs http/https: '{url}'.");
    }
    let status = std::process::Command::new("xdg-open")
        .arg(url)
        .status()
        .map_err(|e| anyhow::anyhow!("No se pudo ejecutar xdg-open: {e}"))?;
    if !status.success() {
        anyhow::bail!("xdg-open terminó con error: {status}");
    }
    log::info(format!("Abriendo '{url}'..."));
    Ok(())
}

fn cmd_data(
    conn: &Connection,
    action: &super::DataAction,
    profile_ident: Option<&str>,
) -> anyhow::Result<()> {
    let profile = resolve_active_profile(conn, profile_ident)?;
    let cfg =
        gta_mo_core::config::load_config().map_err(|e| anyhow::anyhow!("Error de config: {e}"))?;
    let spec = cfg.game_spec();
    let subdir = spec
        .user_data_dir
        .ok_or_else(|| anyhow::anyhow!("El juego '{}' no define datos de usuario.", spec.name))?;
    let upper = gta_mo_core::config::RuntimePaths::from_config(&cfg)
        .profile_paths(&profile.slug)
        .upper;

    match action {
        super::DataAction::List { json } => {
            let entries = gta_mo_core::userdata::scan(&upper, subdir);
            if *json {
                #[derive(Serialize)]
                struct DataJson {
                    path: String,
                    name: String,
                    size: u64,
                    category: String,
                }
                let out: Vec<DataJson> = entries
                    .iter()
                    .map(|e| DataJson {
                        path: e.rel.clone(),
                        name: e.name.clone(),
                        size: e.size,
                        category: e.category.key().to_string(),
                    })
                    .collect();
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(());
            }
            if entries.is_empty() {
                println!(
                    "No hay datos de usuario en el perfil '{}' \
                     (¿PortableGTA instalado y activo?).",
                    profile.name
                );
                return Ok(());
            }
            let rows: Vec<Vec<String>> = entries
                .iter()
                .map(|e| {
                    vec![
                        e.category.label().to_string(),
                        e.rel.clone(),
                        gta_mo_core::userdata::human_size(e.size),
                    ]
                })
                .collect();
            println!(
                "{}",
                render_table(
                    vec![
                        "Tipo".to_string(),
                        "Archivo".to_string(),
                        "Tamaño".to_string(),
                    ],
                    rows,
                )
            );
            log::info(format!(
                "Total: {} archivo(s) en el perfil '{}'.",
                entries.len(),
                profile.name
            ));
            Ok(())
        }
        super::DataAction::Dir => {
            let dir = upper.join(subdir);
            if !dir.exists() {
                anyhow::bail!("La carpeta de datos no existe todavía: {}", dir.display());
            }
            xdg_open(&dir)
        }
        super::DataAction::Open { path } => {
            let target = gta_mo_core::userdata::resolve_entry(&upper, subdir, path)?;
            xdg_open(&target)
        }
        super::DataAction::Remove { path, yes } => {
            // Validate before asking.
            gta_mo_core::userdata::resolve_entry(&upper, subdir, path)?;
            if !yes {
                eprintln!();
                log::warn(format!(
                    "Vas a eliminar '{}' del perfil '{}'.",
                    path, profile.name
                ));
                eprint!("Confirmar eliminación? [s/N]: ");
                std::io::Write::flush(&mut std::io::stderr()).ok();
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
                let confirm = input.trim().to_lowercase();
                if confirm != "s" && confirm != "si" {
                    log::info("Cancelado.");
                    return Ok(());
                }
            }
            gta_mo_core::userdata::remove(&upper, subdir, path)?;
            log::info(format!("Eliminado: '{path}'."));
            Ok(())
        }
    }
}

// ---------- Export / import ----------

#[derive(Serialize, Deserialize)]
struct ExportProfile {
    name: String,
    slug: String,
    is_active: bool,
}

#[derive(Serialize, Deserialize)]
struct ExportMod {
    folder: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mod_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    author: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cover: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    mount: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    guides: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    screenshots: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    components: Vec<gta_mo_core::meta::MetaComponent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    variant: Option<gta_mo_core::meta::ModVariant>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    conflicts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    modloader: Option<gta_mo_core::meta::ModLoaderMeta>,
}

impl ExportMod {
    fn from_identity(m: &gta_mo_core::db::ModIdentity, cache: &db::ModMetaCache) -> Self {
        Self {
            folder: m.folder_name.clone(),
            name: m.name.clone(),
            mod_id: cache.mod_id.clone(),
            version: cache.version.clone(),
            author: cache.author.clone(),
            url: cache.url.clone(),
            description: cache.description.clone(),
            cover: cache.cover.clone(),
            mount: cache.mount.clone(),
            guides: cache.guides.clone(),
            screenshots: cache.screenshots.clone(),
            tags: cache.tags.clone(),
            components: cache.components.clone(),
            variant: cache
                .variant_group
                .as_ref()
                .map(|g| gta_mo_core::meta::ModVariant {
                    group: g.clone(),
                    name: cache.variant_name.clone(),
                }),
            conflicts: cache.conflicts.clone(),
            modloader: if cache.modloader_priority.is_some() || !cache.modloader_folders.is_empty()
            {
                Some(gta_mo_core::meta::ModLoaderMeta {
                    priority: cache.modloader_priority,
                    folders: (!cache.modloader_folders.is_empty())
                        .then(|| cache.modloader_folders.clone()),
                })
            } else {
                None
            },
        }
    }

    fn to_cache(&self) -> db::ModMetaCache {
        db::ModMetaCache {
            mod_id: self.mod_id.clone(),
            version: self.version.clone(),
            author: self.author.clone(),
            url: self.url.clone(),
            description: self.description.clone(),
            cover: self.cover.clone(),
            mount: self.mount.clone(),
            guides: self.guides.clone(),
            screenshots: self.screenshots.clone(),
            tags: self.tags.clone(),
            components: self.components.clone(),
            variant_group: self.variant.as_ref().map(|v| v.group.clone()),
            variant_name: self.variant.as_ref().and_then(|v| v.name.clone()),
            conflicts: self.conflicts.clone(),
            modloader_priority: self.modloader.as_ref().and_then(|m| m.priority),
            modloader_folders: self
                .modloader
                .as_ref()
                .and_then(|m| m.folders.clone())
                .unwrap_or_default(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct ExportProfileMod {
    profile: String,
    folder: String,
    enabled: bool,
    load_order: i64,
}

#[derive(Serialize, Deserialize)]
struct ExportDep {
    folder: String,
    dep: String,
    required: bool,
}

#[derive(Serialize, Deserialize)]
struct ExportGroup {
    name: String,
    slug: String,
}

#[derive(Serialize, Deserialize)]
struct ExportModGroup {
    group: String,
    folder: String,
    #[serde(default)]
    profile: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ExportFile {
    profiles: Vec<ExportProfile>,
    mods: Vec<ExportMod>,
    profile_mods: Vec<ExportProfileMod>,
    dependencies: Vec<ExportDep>,
    groups: Vec<ExportGroup>,
    mod_groups: Vec<ExportModGroup>,
}

fn cmd_export(conn: &Connection, path: Option<&str>) -> anyhow::Result<()> {
    let profiles = db::list_profiles(conn)?;
    let profiles_export: Vec<ExportProfile> = profiles
        .iter()
        .map(|p| ExportProfile {
            name: p.name.clone(),
            slug: p.slug.clone(),
            is_active: p.is_active,
        })
        .collect();

    let mods = db::load_all_mods(conn)?;
    let mods_export: Vec<ExportMod> = mods
        .iter()
        .map(|m| {
            let cache = db::load_mod_meta(conn, m.id).unwrap_or_default();
            ExportMod::from_identity(m, &cache)
        })
        .collect();

    let mut profile_mods_export = Vec::new();
    for p in &profiles {
        for e in db::load_all_mods_for_profile(conn, p.id)? {
            profile_mods_export.push(ExportProfileMod {
                profile: p.slug.clone(),
                folder: e.folder_name,
                enabled: e.enabled,
                load_order: e.load_order,
            });
        }
    }

    let dependencies: Vec<ExportDep> = db::export_dependencies(conn)?
        .into_iter()
        .map(|(folder, dep, required)| ExportDep {
            folder,
            dep,
            required,
        })
        .collect();
    let groups: Vec<ExportGroup> = db::list_groups(conn)?
        .into_iter()
        .map(|g| ExportGroup {
            name: g.name,
            slug: g.slug,
        })
        .collect();
    let mod_groups: Vec<ExportModGroup> = db::export_mod_groups(conn)?
        .into_iter()
        .map(|(group, folder, profile)| ExportModGroup {
            group,
            folder,
            profile,
        })
        .collect();

    let out = ExportFile {
        profiles: profiles_export,
        mods: mods_export,
        profile_mods: profile_mods_export,
        dependencies,
        groups,
        mod_groups,
    };
    let json = serde_json::to_string_pretty(&out)?;
    match path {
        Some(p) => {
            std::fs::write(p, json)?;
            log::info(format!("Estado exportado a '{}'.", p));
        }
        None => println!("{json}"),
    }
    Ok(())
}

fn cmd_import(conn: &Connection, path: &str, force: bool) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(path)?;
    let data: ExportFile = serde_json::from_str(&content)?;

    if !force {
        eprintln!();
        log::warn("Esto reemplazará el estado actual de la base de datos con el backup.");
        eprintln!();
        eprint!("Continuar? [s/N]: ");
        std::io::Write::flush(&mut std::io::stderr()).ok();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let confirm = input.trim().to_lowercase();
        if confirm != "s" && confirm != "si" {
            log::info("Cancelado.");
            return Ok(());
        }
    }

    // All-or-nothing: replace the live DB state inside a single transaction so
    // a mid-import failure rolls back instead of leaving a wiped database.
    conn.execute("BEGIN IMMEDIATE", [])?;
    let result = do_import(conn, &data);
    match result {
        Ok(()) => {
            conn.execute("COMMIT", [])?;
            log::info("Importación completada.");
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute("ROLLBACK", []);
            Err(e)
        }
    }
}

fn do_import(conn: &Connection, data: &ExportFile) -> anyhow::Result<()> {
    conn.execute("DELETE FROM mod_groups", [])?;
    conn.execute("DELETE FROM groups", [])?;
    conn.execute("DELETE FROM mod_dependencies", [])?;
    conn.execute("DELETE FROM profile_mods", [])?;
    conn.execute("DELETE FROM mods", [])?;
    conn.execute("DELETE FROM profiles", [])?;
    let _ = conn.execute("DELETE FROM sqlite_sequence", []);
    let mut profile_ids: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    let mut active_id: Option<i64> = None;
    for p in &data.profiles {
        let id = db::insert_profile(conn, &p.name, &p.slug, p.is_active)?;
        if p.is_active {
            active_id = Some(id);
        }
        profile_ids.insert(p.slug.clone(), id);
    }
    if profile_ids.is_empty() {
        let id = db::insert_profile(conn, "default", "default", true)?;
        active_id = Some(id);
    }
    let first_active = active_id.or_else(|| profile_ids.values().next().copied());
    conn.execute("UPDATE profiles SET is_active = 0", [])?;
    if let Some(id) = first_active {
        conn.execute(
            "UPDATE profiles SET is_active = 1 WHERE id = ?1",
            params![id],
        )?;
    }

    let mut mod_ids: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for m in &data.mods {
        let id = db::insert_mod(conn, &m.folder, &m.name)?;
        let cache = m.to_cache();
        db::set_mod_meta_cache(conn, id, &cache)?;
        mod_ids.insert(m.folder.clone(), id);
    }

    for pm in &data.profile_mods {
        let (Some(pid), Some(mid)) = (profile_ids.get(&pm.profile), mod_ids.get(&pm.folder)) else {
            continue;
        };
        conn.execute(
            "INSERT INTO profile_mods (profile_id, mod_id, enabled, load_order)
             VALUES (?1, ?2, ?3, ?4)",
            params![pid, mid, pm.enabled as i64, pm.load_order],
        )?;
    }

    for d in &data.dependencies {
        let (Some(mid), Some(did)) = (mod_ids.get(&d.folder), mod_ids.get(&d.dep)) else {
            continue;
        };
        conn.execute(
            "INSERT OR IGNORE INTO mod_dependencies (mod_id, dependency_id, required)
             VALUES (?1, ?2, ?3)",
            params![mid, did, d.required as i64],
        )?;
    }

    let mut group_ids: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for g in &data.groups {
        let id = db::insert_group(conn, &g.name, &g.slug)?;
        group_ids.insert(g.slug.clone(), id);
    }
    for mg in &data.mod_groups {
        let (Some(gid), Some(mid)) = (group_ids.get(&mg.group), mod_ids.get(&mg.folder)) else {
            continue;
        };
        let pid = mg.profile.as_deref().and_then(|s| profile_ids.get(s));
        conn.execute(
            "INSERT INTO mod_groups (group_id, mod_id, profile_id) VALUES (?1, ?2, ?3)",
            params![gid, mid, pid],
        )?;
    }
    Ok(())
}

// ---------- Health / conflicts ----------

fn resolve_enabled_order(conn: &Connection, profile: &db::Profile) -> anyhow::Result<Vec<String>> {
    gta_mo_core::resolver::enabled_order_for_profile(conn, profile.id)
}

fn cmd_conflicts(conn: &Connection, profile_ident: Option<&str>, json: bool) -> anyhow::Result<()> {
    let profile = resolve_active_profile(conn, profile_ident)?;
    let cfg =
        gta_mo_core::config::load_config().map_err(|e| anyhow::anyhow!("Error de config: {e}"))?;
    let paths = gta_mo_core::config::RuntimePaths::from_config(&cfg);
    let resolved = resolve_enabled_order(conn, &profile)?;
    let conflicts =
        gta_mo_core::conflicts::scan_conflicts(&paths.mods_dir, &resolved, cfg.game_spec())?;

    if json {
        #[derive(Serialize)]
        struct ConflictJson {
            path: String,
            providers: Vec<String>,
            duplicate: bool,
            severity: String,
        }
        let out: Vec<ConflictJson> = conflicts
            .iter()
            .map(|c| ConflictJson {
                path: c.path.clone(),
                providers: c.providers.clone(),
                duplicate: c.duplicate,
                severity: format!("{:?}", c.severity).to_lowercase(),
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    let real: Vec<&gta_mo_core::conflicts::Conflict> =
        conflicts.iter().filter(|c| !c.duplicate).collect();
    let dups = conflicts.iter().filter(|c| c.duplicate).count();
    if real.is_empty() {
        println!(
            "No hay conflictos de archivos entre los mods activos del perfil '{}'.",
            profile.name
        );
        if dups > 0 {
            println!("({dups} duplicado(s) idéntico(s) ignorados)");
        }
        return Ok(());
    }

    let rows: Vec<Vec<String>> = real
        .iter()
        .map(|c| {
            let severity = match c.severity {
                gta_mo_core::conflicts::Severity::High => "ALTO".red().to_string(),
                gta_mo_core::conflicts::Severity::Medium => "MEDIO".yellow().to_string(),
                gta_mo_core::conflicts::Severity::Info => "INFO".cyan().to_string(),
            };
            vec![c.path.clone(), severity, c.providers.join(" -> ")]
        })
        .collect();
    println!(
        "{}",
        render_table(
            vec![
                "Archivo".to_string(),
                "Gravedad".to_string(),
                "Proveedores (el primero gana)".to_string(),
            ],
            rows,
        )
    );
    if dups > 0 {
        println!("\n({dups} duplicado(s) idéntico(s) ignorados)");
    }
    Ok(())
}

fn cmd_which(conn: &Connection, profile_ident: Option<&str>, path: &str) -> anyhow::Result<()> {
    let profile = resolve_active_profile(conn, profile_ident)?;
    let cfg =
        gta_mo_core::config::load_config().map_err(|e| anyhow::anyhow!("Error de config: {e}"))?;
    let paths = gta_mo_core::config::RuntimePaths::from_config(&cfg);
    let resolved = resolve_enabled_order(conn, &profile)?;

    match gta_mo_core::conflicts::providers_for_path(
        &paths.mods_dir,
        &resolved,
        path,
        cfg.game_spec(),
    )? {
        None => println!(
            "'{}' no lo provee ningún mod del perfil '{}' (viene de la base).",
            path, profile.name
        ),
        Some(p) if p.providers.len() == 1 => {
            println!("'{}' lo provee '{}'.", path, p.providers[0]);
        }
        Some(p) => {
            let winner = &p.providers[0];
            let overridden: Vec<&str> = p.providers[1..].iter().map(|s| s.as_str()).collect();
            let severity = match p.severity {
                gta_mo_core::conflicts::Severity::High => "ALTO".red().to_string(),
                gta_mo_core::conflicts::Severity::Medium => "MEDIO".yellow().to_string(),
                gta_mo_core::conflicts::Severity::Info => "INFO".cyan().to_string(),
            };
            println!("'{}' → gana '{}' (gravedad: {severity})", path, winner);
            if p.duplicate {
                println!(
                    "  (los {} mods aportan el mismo contenido)",
                    p.providers.len()
                );
            } else {
                println!("  pisados: {}", overridden.join(", "));
            }
        }
    }
    Ok(())
}

fn cmd_health(
    conn: &Connection,
    profile_ident: Option<&str>,
    conflicts: bool,
) -> anyhow::Result<()> {
    let profile = resolve_active_profile(conn, profile_ident)?;
    let cfg =
        gta_mo_core::config::load_config().map_err(|e| anyhow::anyhow!("Error de config: {e}"))?;
    let paths = gta_mo_core::config::RuntimePaths::from_config(&cfg);

    let mut errors: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut folders: std::collections::HashMap<i64, String> = std::collections::HashMap::new();

    for m in db::load_all_mods(conn)? {
        let dir = paths.mods_dir.join(&m.folder_name);
        folders.insert(m.id, m.folder_name.clone());
        if !dir.is_dir() {
            errors.push(format!("{}: carpeta no existe en disco", m.folder_name));
            continue;
        }
        match gta_mo_core::meta::read_mod_meta(&paths.mods_dir, &m.folder_name) {
            Ok(Some(meta)) => {
                if let Some(mount) = meta.mount {
                    for entry in mount {
                        if !gta_mo_core::meta::valid_mount_entry(&entry) {
                            warnings.push(format!("{}: mount inválido '{entry}'", m.folder_name));
                        } else if !dir.join(&entry).is_dir() {
                            warnings
                                .push(format!("{}: mount '{}' no existe", m.folder_name, entry));
                        }
                    }
                }
            }
            Ok(None) => {}
            Err(e) => warnings.push(format!("{}: {e}", m.folder_name)),
        }
    }

    let enabled_ids = db::load_enabled_mod_ids_for_profile(conn, profile.id)?;
    let enabled_set: std::collections::HashSet<i64> = enabled_ids.iter().copied().collect();
    let deps = db::load_dependencies(conn)?;
    for mid in &enabled_ids {
        if let Some(refs) = deps.get(mid) {
            for r in refs {
                let name = folders.get(mid).cloned().unwrap_or_default();
                match folders.get(&r.id) {
                    Some(dep_folder) => {
                        if r.required && !enabled_set.contains(&r.id) {
                            warnings.push(format!(
                                "{name}: dependencia requerida desactivada '{dep_folder}'"
                            ));
                        }
                    }
                    None => errors.push(format!(
                        "{name}: dependencia no resuelta (id={} no instalado)",
                        r.id
                    )),
                }
            }
        }
    }

    // Variant/conflict constraints (live manifests) and ModLoader priorities.
    let all_folders: Vec<String> = folders.values().cloned().collect();
    let enabled_folders: Vec<String> = db::load_all_mods_for_profile(conn, profile.id)?
        .into_iter()
        .filter(|m| m.enabled)
        .map(|m| m.folder_name)
        .collect();
    for v in gta_mo_core::constraints::check(&paths.mods_dir, &all_folders, &enabled_folders) {
        errors.push(v.message);
    }
    let ml_entries = gta_mo_core::modloader::entries_for(&paths.mods_dir, &enabled_folders);

    let mut lines: Vec<String> = Vec::new();
    for e in &errors {
        lines.push(format!("[X] {e}"));
    }
    for w in &warnings {
        lines.push(format!("[!] {w}"));
    }

    if errors.is_empty() && warnings.is_empty() {
        println!(
            "Estado saludable: sin problemas en los mods del perfil '{}'.",
            profile.name
        );
    } else {
        for l in &lines {
            println!("{l}");
        }
        println!();
        println!(
            "Resumen: {} error(es), {} advertencia(s)",
            errors.len(),
            warnings.len()
        );
    }

    if !ml_entries.is_empty() {
        println!();
        println!("modloader.ini:");
        for (name, prio) in &ml_entries {
            println!("  {name}={prio}");
        }
        println!(
            "  -> {}",
            gta_mo_core::modloader::ini_path(&paths.profile_paths(&profile.slug).upper).display()
        );
    }

    if conflicts {
        println!();
        cmd_conflicts(conn, profile_ident, false)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_http_url;

    #[test]
    fn only_http_urls_are_accepted() {
        assert!(is_http_url("http://example.com"));
        assert!(is_http_url("https://example.com/a?b=1#c"));
        assert!(is_http_url("  HTTPS://Example.ORG  "));
        // esquemas peligrosos o no web se rechazan
        assert!(!is_http_url("file:///etc/passwd"));
        assert!(!is_http_url("javascript:alert(1)"));
        assert!(!is_http_url("ftp://example.com"));
        assert!(!is_http_url("http:example.com"));
        assert!(!is_http_url(""));
    }
}
