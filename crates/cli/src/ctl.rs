use gta_mo_core::db::{self, log};
use owo_colors::OwoColorize;
use rusqlite::Connection;

pub fn run(conn: &Connection, args: &super::CtlArgs) -> anyhow::Result<()> {
    match &args.command {
        super::CtlCommand::List {
            verbose,
            enabled,
            disabled,
        } => {
            let filter = if *enabled {
                Some("enabled")
            } else if *disabled {
                Some("disabled")
            } else {
                None
            };
            cmd_list(conn, *verbose, filter)
        }
        super::CtlCommand::Add {
            folder,
            name,
            order,
        } => cmd_add(conn, folder, name.as_deref(), *order),
        super::CtlCommand::Remove { ident } => cmd_remove(conn, ident),
        super::CtlCommand::Enable { ident } => cmd_enable(conn, ident),
        super::CtlCommand::Disable { ident } => cmd_disable(conn, ident),
        super::CtlCommand::Order { ident, new_order } => cmd_order(conn, ident, *new_order),
        super::CtlCommand::Rename {
            ident,
            new_name,
            folder,
        } => cmd_rename(conn, ident, new_name, *folder),
        super::CtlCommand::Info { ident } => cmd_info(conn, ident),
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
    }
}

fn cmd_list(conn: &Connection, verbose: bool, filter: Option<&str>) -> anyhow::Result<()> {
    let filter_clause = match filter {
        Some("enabled") => "WHERE enabled = 1",
        Some("disabled") => "WHERE enabled = 0",
        _ => "",
    };

    let count: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM mods {filter_clause}"),
        [],
        |row| row.get(0),
    )?;

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

    let mut stmt = conn.prepare(&format!(
        "SELECT id, enabled, load_order, folder_name, name FROM mods {filter_clause} ORDER BY load_order DESC"
    ))?;

    let mods = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)? != 0,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
        ))
    })?;

    for m in mods {
        let (id, enabled, order, folder, name) = m?;
        let status = if enabled {
            "SI".green().to_string()
        } else {
            "NO".red().to_string()
        };

        print!(
            "{:<4} {:<6} {:<7} {:<30} {}",
            id, status, order, folder, name
        );
        println!();

        if verbose {
            let deps = db::get_dependencies_of(conn, id)?;
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

            let dependents = db::get_dependents_of(conn, id)?;
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

fn cmd_add(
    conn: &Connection,
    folder: &str,
    name: Option<&str>,
    order: Option<i64>,
) -> anyhow::Result<()> {
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

    let id = db::add_mod(conn, folder, &display_name, order)?;
    log::info(format!(
        "Mod añadido: [{id}] '{folder}' -> '{display_name}' (orden={}, desactivado)",
        order.unwrap_or_else(|| {
            conn.query_row(
                "SELECT COALESCE(MAX(load_order), 0) + 10 FROM mods",
                [],
                |row| row.get(0),
            )
            .unwrap_or(10)
        })
    ));
    Ok(())
}

fn cmd_remove(conn: &Connection, ident: &str) -> anyhow::Result<()> {
    let id = db::resolve_mod_ident(conn, ident)?;
    let m = db::get_mod_by_id(conn, id)?.ok_or_else(|| anyhow::anyhow!("Mod no encontrado"))?;
    let dep_count = db::count_deps_for_mod(conn, id)?;

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

    db::remove_mod(conn, id)?;
    log::info(format!("Mod '{}' eliminado.", m.folder_name));
    Ok(())
}

fn cmd_enable(conn: &Connection, ident: &str) -> anyhow::Result<()> {
    let id = db::resolve_mod_ident(conn, ident)?;
    let m = db::get_mod_by_id(conn, id)?.ok_or_else(|| anyhow::anyhow!("Mod no encontrado"))?;
    if m.enabled {
        log::warn(format!("'{}' ya esta activado.", m.folder_name));
        return Ok(());
    }
    db::set_mod_enabled(conn, id, true)?;
    log::info(format!("Mod '{}' activado.", m.folder_name));
    Ok(())
}

fn cmd_disable(conn: &Connection, ident: &str) -> anyhow::Result<()> {
    let id = db::resolve_mod_ident(conn, ident)?;
    let m = db::get_mod_by_id(conn, id)?.ok_or_else(|| anyhow::anyhow!("Mod no encontrado"))?;
    if !m.enabled {
        log::warn(format!("'{}' ya esta desactivado.", m.folder_name));
        return Ok(());
    }

    let dependents = db::get_dependents_of(conn, id)?;
    let active_dependents: Vec<_> = dependents.iter().filter(|d| d.enabled).collect();

    if !active_dependents.is_empty() {
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

    db::set_mod_enabled(conn, id, false)?;
    log::info(format!("Mod '{}' desactivado.", m.folder_name));
    Ok(())
}

fn cmd_order(conn: &Connection, ident: &str, new_order: i64) -> anyhow::Result<()> {
    let id = db::resolve_mod_ident(conn, ident)?;
    let m = db::get_mod_by_id(conn, id)?.ok_or_else(|| anyhow::anyhow!("Mod no encontrado"))?;
    let old_order = m.load_order;
    db::set_mod_order(conn, id, new_order)?;
    log::info(format!(
        "'{}': orden cambiado de {old_order} a {new_order}.",
        m.folder_name
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

fn cmd_rename_folder(conn: &Connection, m: &db::ModEntry, new_folder: &str) -> anyhow::Result<()> {
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

fn cmd_info(conn: &Connection, ident: &str) -> anyhow::Result<()> {
    let id = db::resolve_mod_ident(conn, ident)?;
    let m = db::get_mod_by_id(conn, id)?.ok_or_else(|| anyhow::anyhow!("Mod no encontrado"))?;

    let status = if m.enabled {
        "Activado".green().to_string()
    } else {
        "Desactivado".red().to_string()
    };

    println!();
    println!("  {}       {}", "ID:".bold(), m.id);
    println!("  {}  {}", "Carpeta:".bold(), m.folder_name);
    println!("  {}   {}", "Nombre:".bold(), m.name);
    println!("  {}   {}", "Estado:".bold(), status);
    println!("  {}    {}", "Orden:".bold(), m.load_order);
    println!();

    let deps = db::get_dependencies_of(conn, id)?;
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

    let dependents = db::get_dependents_of(conn, id)?;
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
