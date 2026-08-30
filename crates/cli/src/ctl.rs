use gta_mo_core::db;
use gta_mo_core::log;
use owo_colors::OwoColorize;
use rusqlite::Connection;
use serde::Serialize;

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
            cmd_list(conn, &profile, *verbose, filter, *json)
        }
        super::CtlCommand::Add { folder, name } => cmd_add(conn, folder, name.as_deref()),
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
        super::CtlCommand::Info { ident, json } => {
            let profile = active()?;
            cmd_info(conn, &profile, ident, *json)
        }
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
    }
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
    }
}

fn cmd_list(
    conn: &Connection,
    profile: &db::Profile,
    verbose: bool,
    filter: Option<&str>,
    json: bool,
) -> anyhow::Result<()> {
    let mods = db::load_all_mods_for_profile(conn, profile.id)?
        .into_iter()
        .filter(|m| match filter {
            Some("enabled") => m.enabled,
            Some("disabled") => !m.enabled,
            _ => true,
        })
        .collect::<Vec<_>>();

    if json {
        let mut out = Vec::new();
        for m in &mods {
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
                name: m.name.clone(),
                enabled: m.enabled,
                order: m.load_order,
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

    println!(
        "{:4} {:6} {:7} {:30} {}",
        "ID".bold(),
        "ACTIVO".bold(),
        "ORDEN".bold(),
        "CARPETA".bold(),
        "NOMBRE".bold()
    );
    println!(
        "{:4} {:6} {:7} {:30} ------------------------------",
        "---", "------", "------", "------------------------------"
    );

    for m in &mods {
        let status = if m.enabled {
            "SI".green().to_string()
        } else {
            "NO".red().to_string()
        };

        print!(
            "{:<4} {:<6} {:<7} {:<30} {}",
            m.id, status, m.load_order, m.folder_name, m.name
        );
        println!();

        if verbose {
            let deps = db::get_dependencies_of(conn, profile.id, m.id)?;
            if !deps.is_empty() {
                let names: Vec<String> = deps
                    .iter()
                    .map(|(d, req)| {
                        if *req {
                            d.folder_name.clone()
                        } else {
                            format!("{} {}", d.folder_name, "(opcional)".cyan())
                        }
                    })
                    .collect();
                println!("     {} {}", "-> depende de:".cyan(), names.join(" "));
            }

            let dependents = db::get_dependents_of(conn, profile.id, m.id)?;
            if !dependents.is_empty() {
                let names: Vec<&str> = dependents.iter().map(|d| d.folder_name.as_str()).collect();
                println!("     {} {}", "<- requerido por:".yellow(), names.join(" "));
            }
        }
    }

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
    let id = db::resolve_mod_ident(conn, ident)?;
    let m = db::get_mod_by_id(conn, id)?.ok_or_else(|| anyhow::anyhow!("Mod no encontrado"))?;
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
    let id = db::resolve_mod_ident(conn, ident)?;
    let m = db::get_mod_by_id(conn, id)?.ok_or_else(|| anyhow::anyhow!("Mod no encontrado"))?;
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
    let id = db::resolve_mod_ident(conn, ident)?;
    let m = db::get_mod_by_id(conn, id)?.ok_or_else(|| anyhow::anyhow!("Mod no encontrado"))?;
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
    let id = db::resolve_mod_ident(conn, ident)?;
    let m = db::get_mod_by_id(conn, id)?.ok_or_else(|| anyhow::anyhow!("Mod no encontrado"))?;
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
    let id = db::resolve_mod_ident(conn, ident)?;
    let m = db::get_mod_by_id(conn, id)?.ok_or_else(|| anyhow::anyhow!("Mod no encontrado"))?;

    if folder {
        return cmd_rename_folder(conn, &m, new_name);
    }

    let old_name = m.name.clone();
    db::set_mod_name(conn, id, new_name)?;
    log::info(format!(
        "'{}': nombre cambiado de '{old_name}' a '{new_name}'.",
        m.folder_name
    ));
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
    json: bool,
) -> anyhow::Result<()> {
    let id = db::resolve_mod_ident(conn, ident)?;
    let m = db::get_mod_by_id(conn, id)?.ok_or_else(|| anyhow::anyhow!("Mod no encontrado"))?;
    let (enabled, order) = db::profile_mod_state(conn, profile.id, id)?;

    let deps = db::get_dependencies_of(conn, profile.id, id)?;
    let dependents = db::get_dependents_of(conn, profile.id, id)?;

    if json {
        #[derive(Serialize)]
        struct InfoJson {
            id: i64,
            folder: String,
            name: String,
            enabled: bool,
            order: i64,
            dependencies: Vec<DepJson>,
            dependents: Vec<DepJson>,
        }
        let out = InfoJson {
            id: m.id,
            folder: m.folder_name,
            name: m.name,
            enabled,
            order,
            dependencies: deps
                .iter()
                .map(|(d, req)| DepJson::from_entry(d, *req))
                .collect(),
            dependents: dependents
                .iter()
                .map(|d| DepJson::from_entry(d, true))
                .collect(),
        };
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    let status = if enabled {
        "Activado".green().to_string()
    } else {
        "Desactivado".red().to_string()
    };

    println!();
    println!("  {}       {}", "ID:".bold(), m.id);
    println!("  {}  {}", "Carpeta:".bold(), m.folder_name);
    println!("  {}   {}", "Nombre:".bold(), m.name);
    println!("  {}   {}", "Estado:".bold(), status);
    println!("  {}    {}", "Orden:".bold(), order);
    println!("  {}   {}", "Perfil:".bold(), profile.name);
    println!();

    if deps.is_empty() {
        println!("  {} ninguna", "Dependencias:".cyan());
    } else {
        println!(
            "  {} ({} depende de):",
            "Dependencias".cyan(),
            m.folder_name
        );
        for (d, req) in &deps {
            let dstatus = if d.enabled {
                "SI".green().to_string()
            } else {
                "NO".red().to_string()
            };
            let dkind = if *req {
                "requerido".cyan().to_string()
            } else {
                "opcional".yellow().to_string()
            };
            println!(
                "    [{}] {} ({}) [activo: {}] [{}]",
                d.id, d.folder_name, d.name, dstatus, dkind
            );
        }
    }
    println!();

    if dependents.is_empty() {
        println!("  {} nadie", "Requerido por:".yellow());
    } else {
        println!("  {}:", "Requerido por".yellow());
        for d in &dependents {
            let rstatus = if d.enabled {
                "SI".green().to_string()
            } else {
                "NO".red().to_string()
            };
            println!(
                "    [{}] {} ({}) [activo: {}]",
                d.id, d.folder_name, d.name, rstatus
            );
        }
    }
    println!();
    Ok(())
}

fn cmd_dep_add(
    conn: &Connection,
    mod_ident: &str,
    dep_ident: &str,
    optional: bool,
) -> anyhow::Result<()> {
    let mod_id = db::resolve_mod_ident(conn, mod_ident)?;
    let dep_id = db::resolve_mod_ident(conn, dep_ident)?;

    let mod_folder = db::get_mod_by_id(conn, mod_id)?
        .map(|m| m.folder_name)
        .unwrap_or_default();
    let dep_folder = db::get_mod_by_id(conn, dep_id)?
        .map(|m| m.folder_name)
        .unwrap_or_default();

    db::add_dependency(conn, mod_id, dep_id, !optional)?;
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
    let mod_id = db::resolve_mod_ident(conn, mod_ident)?;
    let dep_id = db::resolve_mod_ident(conn, dep_ident)?;

    let mod_folder = db::get_mod_by_id(conn, mod_id)?
        .map(|m| m.folder_name)
        .unwrap_or_default();
    let dep_folder = db::get_mod_by_id(conn, dep_id)?
        .map(|m| m.folder_name)
        .unwrap_or_default();

    db::remove_dependency(conn, mod_id, dep_id)?;
    log::info(format!(
        "Dependencia eliminada: '{mod_folder}' ya no depende de '{dep_folder}'."
    ));
    Ok(())
}
