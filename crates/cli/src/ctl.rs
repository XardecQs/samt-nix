use comfy_table::{presets, Cell, ContentArrangement, Table};
use gta_mo_core::db;
use gta_mo_core::log;
use owo_colors::OwoColorize;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

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
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    deps: Vec<DepJson>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    dependents: Vec<i64>,
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
        super::CtlCommand::Conflicts { json } => cmd_conflicts(conn, profile_ident, *json),
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

/// Expands `guides` entries that point to a directory into their files, so a
/// manifest can use `guides = ["guides"]` to include a whole folder.
fn expand_guides(mods_dir: &std::path::Path, folder: &str, guides: Vec<String>) -> Vec<String> {
    let mod_dir = mods_dir.join(folder);
    let mut out = Vec::new();
    for g in guides {
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

            println!(
                "{:4} {:6} {:24} {:30} {:5} {:5}",
                "ID".bold(),
                "ACTIVO".bold(),
                "NOMBRE".bold(),
                "SLUG".bold(),
                "MODS".bold(),
                "ON".bold()
            );
            for p in &profiles {
                let (total, enabled) = db::profile_mod_count(conn, p.id)?;
                let mark = if p.id == active.id {
                    "*".green().to_string()
                } else {
                    "".to_string()
                };
                println!(
                    "{:<4} {:<6} {:<24} {:<30} {:<5} {:<5}",
                    p.id, mark, p.name, p.slug, total, enabled
                );
            }
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
                std::io::Write::flush(&mut std::io::stdout()).ok();

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
            let old = p.name.clone();
            db::rename_profile(conn, p.id, new_name)?;
            log::info(format!(
                "Perfil renombrado de '{old}' a '{new_name}' (slug '{}' sin cambios).",
                p.slug
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

            let only_in_a: Vec<&(String, bool, i64)> = sa
                .iter()
                .filter(|(id, (_, en, _))| *en && !sb.get(id).map(|(_, e, _)| *e).unwrap_or(false))
                .map(|(_, v)| v)
                .collect();
            let only_in_b: Vec<&(String, bool, i64)> = sb
                .iter()
                .filter(|(id, (_, en, _))| *en && !sa.get(id).map(|(_, e, _)| *e).unwrap_or(false))
                .map(|(_, v)| v)
                .collect();
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
                std::io::Write::flush(&mut std::io::stdout()).ok();
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

    let mut mods = db::load_all_mods_for_profile(conn, profile.id)?
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
                tags: meta.tags,
                deps,
                dependents,
            });
        }
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    let count = mods.len();
    if count == 0 {
        println!("No hay mods registrados.");
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
    if folder.contains(':') || folder.contains('|') || folder.contains('/') || folder.contains('\\')
    {
        anyhow::bail!("El nombre de carpeta no puede contener ':', '|', '/' ni '\\'.");
    }
    if folder == "." || folder == ".." {
        anyhow::bail!("Nombre de carpeta no valido.");
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
        std::io::Write::flush(&mut std::io::stdout()).ok();

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

fn cmd_enable(conn: &Connection, profile: &db::Profile, ident: &str) -> anyhow::Result<()> {
    let m = resolve_mod(conn, ident)?;
    let id = m.id;
    let (enabled, _) = db::profile_mod_state(conn, profile.id, id)?;
    if enabled {
        log::warn(format!("'{}' ya esta activado.", m.folder_name));
        return Ok(());
    }
    db::set_mod_enabled(conn, profile.id, id, true)?;
    log::info(format!(
        "Mod '{}' activado (perfil '{}').",
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
        std::io::Write::flush(&mut std::io::stdout()).ok();

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
    db::set_mod_name(conn, id, new_name)?;

    let mut manifest_updated = false;
    if let Ok(cfg) = gta_mo_core::config::load_config() {
        let paths = gta_mo_core::config::RuntimePaths::from_config(&cfg);
        let manifest = paths.mods_dir.join(&m.folder_name).join("mod.toml");
        if manifest.exists() {
            gta_mo_core::meta::set_meta_name(&paths.mods_dir, &m.folder_name, new_name)?;
            manifest_updated = true;
        }
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
    if new_folder.contains(':')
        || new_folder.contains('|')
        || new_folder.contains('/')
        || new_folder.contains('\\')
    {
        anyhow::bail!("El nombre de carpeta no puede contener ':', '|', '/' ni '\\'.");
    }
    if new_folder == "." || new_folder == ".." {
        anyhow::bail!("Nombre de carpeta no valido.");
    }
    if new_folder.trim() != new_folder {
        anyhow::bail!("El nombre de carpeta no puede tener espacios al inicio o final.");
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
            tags: Vec<String>,
            pack: bool,
            components: Vec<ComponentJson>,
            groups: Vec<GroupJson>,
            dependencies: Vec<DepJson>,
            dependents: Vec<DepJson>,
            profiles: Vec<ProfileStateJson>,
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
        println!();
        println!("  {}       {}", "ID:".bold(), m.id);
        println!("  {}  {}", "Carpeta:".bold(), m.folder_name);
        println!("  {}   {}", "Nombre:".bold(), name);
        if let Some(id) = &meta.mod_id {
            println!("  {}      {}", "Mod ID:".bold(), id.green());
        }
        if let Some(v) = &meta.version {
            println!("  {}      {}", "Versión:".bold(), v.cyan());
        }
        if !meta.author.is_empty() {
            println!(
                "  {}    {}",
                "Autor:".bold(),
                meta.author.join(", ").magenta()
            );
        }
        if let Some(u) = &meta.url {
            println!("  {}       {}", "URL:".bold(), u.blue().underline());
        }
        if let Some(d) = &meta.description {
            println!("  {} {}", "Descripción:".bold(), d.yellow());
        }
        if let Some(c) = &meta.cover {
            println!("  {}  {}", "Carátula:".bold(), c);
        }
        if !meta.tags.is_empty() {
            println!("  {}     {}", "Tags:".bold(), meta.tags.join(", "));
        }
        if !meta.mount.is_empty() {
            println!("  {}     {}", "Mount:".bold(), meta.mount.join(", "));
        }
        println!("  {}   {}", "Tipo:".bold(), kind.magenta());
        println!("  {}   {}", "Estado:".bold(), status);
        println!("  {}    {}", "Orden:".bold(), order);
        if !groups.is_empty() {
            let names: Vec<String> = groups.iter().map(|g| g.name.clone()).collect();
            println!("  {}     {}", "Grupos:".bold(), names.join(", ").cyan());
        }

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

fn cmd_open(conn: &Connection, ident: &str, url: bool) -> anyhow::Result<()> {
    let m = resolve_mod(conn, ident)?;
    let cfg =
        gta_mo_core::config::load_config().map_err(|e| anyhow::anyhow!("Error de config: {e}"))?;
    let paths = gta_mo_core::config::RuntimePaths::from_config(&cfg);

    let target = if url {
        let meta =
            gta_mo_core::meta::read_mod_meta(&paths.mods_dir, &m.folder_name)?.unwrap_or_default();
        match meta.url {
            Some(u) => u,
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

    db::add_dependency(conn, m.id, d.id, !optional)?;
    dep_writeback(conn, &m, &d, optional, true)?;

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

    db::remove_dependency(conn, m.id, d.id)?;
    dep_writeback(conn, &m, &d, false, false)?;

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
    tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    components: Vec<gta_mo_core::meta::MetaComponent>,
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
            tags: cache.tags.clone(),
            components: cache.components.clone(),
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
            tags: self.tags.clone(),
            components: self.components.clone(),
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
        std::io::Write::flush(&mut std::io::stdout()).ok();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let confirm = input.trim().to_lowercase();
        if confirm != "s" && confirm != "si" {
            log::info("Cancelado.");
            return Ok(());
        }
    }

    conn.execute("PRAGMA foreign_keys = ON", [])?;
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

    log::info("Importación completada.");
    Ok(())
}

// ---------- Health / conflicts ----------

fn resolve_enabled_order(conn: &Connection, profile: &db::Profile) -> anyhow::Result<Vec<String>> {
    let all_mods = db::load_all_mods_for_profile(conn, profile.id)?;
    let mods_map = all_mods.into_iter().map(|m| (m.id, m)).collect();
    let deps = db::load_dependencies(conn)?;
    let enabled_ids = db::load_enabled_mod_ids_for_profile(conn, profile.id)?;
    let mut graph = gta_mo_core::resolver::DepGraph::new(mods_map, deps, enabled_ids);
    graph.prompt = gta_mo_core::resolver::DepPrompt::Ignore;
    let _ = graph.validate_dependencies();
    let _ = graph.detect_cycles();
    Ok(graph.resolve())
}

fn cmd_conflicts(conn: &Connection, profile_ident: Option<&str>, json: bool) -> anyhow::Result<()> {
    let profile = resolve_active_profile(conn, profile_ident)?;
    let cfg =
        gta_mo_core::config::load_config().map_err(|e| anyhow::anyhow!("Error de config: {e}"))?;
    let paths = gta_mo_core::config::RuntimePaths::from_config(&cfg);
    let resolved = resolve_enabled_order(conn, &profile)?;
    let conflicts = gta_mo_core::conflicts::scan_conflicts(&paths.mods_dir, &resolved)?;

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
            Ok(_) => {}
            Err(e) => warnings.push(format!("{}: {e}", m.folder_name)),
        }
        if let Ok(Some(meta)) = gta_mo_core::meta::read_mod_meta(&paths.mods_dir, &m.folder_name) {
            if let Some(mount) = meta.mount {
                for entry in mount {
                    if !gta_mo_core::meta::valid_mount_entry(&entry) {
                        warnings.push(format!("{}: mount inválido '{entry}'", m.folder_name));
                    } else if !dir.join(&entry).is_dir() {
                        warnings.push(format!("{}: mount '{}' no existe", m.folder_name, entry));
                    }
                }
            }
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

    if conflicts {
        println!();
        cmd_conflicts(conn, profile_ident, false)?;
    }
    Ok(())
}
