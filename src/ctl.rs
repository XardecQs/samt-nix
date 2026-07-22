use crate::db::{self, log};
use rusqlite::Connection;
use owo_colors::OwoColorize;

pub fn run(conn: &Connection, args: &super::CtlArgs) -> anyhow::Result<()> {
    match &args.command {
        super::CtlCommand::List { verbose, enabled, disabled } => {
            let filter = if *enabled { Some("enabled") } else if *disabled { Some("disabled") } else { None };
            cmd_list(conn, *verbose, filter)
        }
        super::CtlCommand::Add { folder, name, order } => cmd_add(conn, folder, name.as_deref(), *order),
        super::CtlCommand::Remove { ident } => cmd_remove(conn, ident),
        super::CtlCommand::Enable { ident } => cmd_enable(conn, ident),
        super::CtlCommand::Disable { ident } => cmd_disable(conn, ident),
        super::CtlCommand::Order { ident, new_order } => cmd_order(conn, ident, *new_order),
        super::CtlCommand::Rename { ident, new_name } => cmd_rename(conn, ident, new_name),
        super::CtlCommand::Info { ident } => cmd_info(conn, ident),
        super::CtlCommand::Dep { action } => match action {
            super::DepAction::Add { mod_ident, dep_ident } => cmd_dep_add(conn, mod_ident, dep_ident),
            super::DepAction::Remove { mod_ident, dep_ident } => cmd_dep_rm(conn, mod_ident, dep_ident),
        },
        super::CtlCommand::Tui => cmd_tui(conn),
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
        "{:4} {:6} {:7} {:30} {}",
        "---", "------", "------", "------------------------------", "------------------------------"
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

        print!("{:<4} {:<6} {:<7} {:<30} {}", id, status, order, folder, name);
        println!();

        if verbose {
            let deps = db::get_dependencies_of(conn, id)?;
            if !deps.is_empty() {
                let names: Vec<&str> = deps.iter().map(|d| d.folder_name.as_str()).collect();
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

fn cmd_add(conn: &Connection, folder: &str, name: Option<&str>, order: Option<i64>) -> anyhow::Result<()> {
    if folder.contains(':') || folder.contains('|') || folder.contains('/') || folder.contains('\\') {
        anyhow::bail!("El nombre de carpeta no puede contener ':', '|', '/' ni '\\'.");
    }
    if folder == "." || folder == ".." {
        anyhow::bail!("Nombre de carpeta no valido.");
    }
    if db::mod_exists(conn, folder)? {
        anyhow::bail!("El mod '{folder}' ya existe en la base de datos.");
    }

    let display_name = name.map(|n| n.to_string()).unwrap_or_else(|| folder.replace('_', " "));

    let id = db::add_mod(conn, folder, &display_name, order)?;
    log::info(format!("Mod añadido: [{id}] '{folder}' -> '{display_name}' (orden={}, desactivado)",
        order.unwrap_or_else(|| {
            conn.query_row("SELECT COALESCE(MAX(load_order), 0) + 10 FROM mods", [], |row| row.get(0)).unwrap_or(10)
        })
    ));
    Ok(())
}

fn cmd_remove(conn: &Connection, ident: &str) -> anyhow::Result<()> {
    let id = db::resolve_mod_ident(conn, ident)?;
    let m = db::get_mod_by_id(conn, id)?.ok_or_else(|| anyhow::anyhow!("Mod no encontrado"))?;
    let dep_count = db::count_deps_for_mod(conn, id)?;

    eprintln!();
    log::warn(format!("Vas a eliminar el mod '{}' (id={}).", m.folder_name, id));
    if dep_count > 0 {
        log::warn(format!("Tiene {dep_count} relacion(es) de dependencia que se eliminaran tambien."));
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
        log::warn(format!("'{}' es requerido por los siguientes mods activos:", m.folder_name));
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
    log::info(format!("'{}': orden cambiado de {old_order} a {new_order}.", m.folder_name));
    Ok(())
}

fn cmd_rename(conn: &Connection, ident: &str, new_name: &str) -> anyhow::Result<()> {
    if new_name.is_empty() {
        anyhow::bail!("El nombre no puede estar vacio.");
    }
    let id = db::resolve_mod_ident(conn, ident)?;
    let m = db::get_mod_by_id(conn, id)?.ok_or_else(|| anyhow::anyhow!("Mod no encontrado"))?;
    let old_name = m.name.clone();
    db::set_mod_name(conn, id, new_name)?;
    log::info(format!("'{}': nombre cambiado de '{old_name}' a '{new_name}'.", m.folder_name));
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
        println!("  {} ({} depende de):", "Dependencias".cyan(), m.folder_name);
        for d in &deps {
            let dstatus = if d.enabled { "SI".green().to_string() } else { "NO".red().to_string() };
            println!("    [{}] {} ({}) [activo: {}]", d.id, d.folder_name, d.name, dstatus);
        }
    }
    println!();

    let dependents = db::get_dependents_of(conn, id)?;
    if dependents.is_empty() {
        println!("  {} nadie", "Requerido por:".yellow());
    } else {
        println!("  {}:", "Requerido por".yellow());
        for d in &dependents {
            let rstatus = if d.enabled { "SI".green().to_string() } else { "NO".red().to_string() };
            println!("    [{}] {} ({}) [activo: {}]", d.id, d.folder_name, d.name, rstatus);
        }
    }
    println!();
    Ok(())
}

fn cmd_dep_add(conn: &Connection, mod_ident: &str, dep_ident: &str) -> anyhow::Result<()> {
    let mod_id = db::resolve_mod_ident(conn, mod_ident)?;
    let dep_id = db::resolve_mod_ident(conn, dep_ident)?;

    let mod_folder = db::get_mod_by_id(conn, mod_id)?.map(|m| m.folder_name).unwrap_or_default();
    let dep_folder = db::get_mod_by_id(conn, dep_id)?.map(|m| m.folder_name).unwrap_or_default();

    db::add_dependency(conn, mod_id, dep_id)?;
    log::info(format!("'{mod_folder}' ahora depende de '{dep_folder}'."));
    Ok(())
}

fn cmd_dep_rm(conn: &Connection, mod_ident: &str, dep_ident: &str) -> anyhow::Result<()> {
    let mod_id = db::resolve_mod_ident(conn, mod_ident)?;
    let dep_id = db::resolve_mod_ident(conn, dep_ident)?;

    let mod_folder = db::get_mod_by_id(conn, mod_id)?.map(|m| m.folder_name).unwrap_or_default();
    let dep_folder = db::get_mod_by_id(conn, dep_id)?.map(|m| m.folder_name).unwrap_or_default();

    db::remove_dependency(conn, mod_id, dep_id)?;
    log::info(format!("Dependencia eliminada: '{mod_folder}' ya no depende de '{dep_folder}'."));
    Ok(())
}

fn cmd_tui(conn: &Connection) -> anyhow::Result<()> {
    loop {
        println!();
        println!("  modctl -- Gestion de mods");
        println!("  --------------------------------");
        println!("  1) Listar mods");
        println!("  2) Listar mods (con dependencias)");
        println!("  3) Anadir mod");
        println!("  4) Eliminar mod");
        println!("  5) Activar mod");
        println!("  6) Desactivar mod");
        println!("  7) Cambiar orden de carga");
        println!("  8) Renombrar mod");
        println!("  9) Ver info de un mod");
        println!(" 10) Gestionar dependencias");
        println!("  0) Salir");
        println!();

        let input = read_input("Opcion [0-10]:");
        match input.trim() {
            "1" => { println!(); cmd_list(conn, false, None)?; }
            "2" => { println!(); cmd_list(conn, true, None)?; }
            "3" => {
                println!();
                let folder = read_input("    Carpeta del mod:");
                let dname = read_input("    Nombre visible (dejar vacio para auto):");
                let dorder = read_input("    Orden de carga (dejar vacio para auto):");
                let name_opt = if dname.is_empty() { None } else { Some(dname.as_str()) };
                let order_opt = dorder.trim().parse::<i64>().ok();
                if let Err(e) = cmd_add(conn, folder.trim(), name_opt, order_opt) {
                    log::warn(format!("{e}"));
                }
            }
            "4" => {
                println!();
                let ident = read_input("    ID o carpeta del mod a eliminar:");
                if let Err(e) = cmd_remove(conn, ident.trim()) {
                    log::warn(format!("{e}"));
                }
            }
            "5" => {
                println!();
                let ident = read_input("    ID o carpeta del mod a activar:");
                if let Err(e) = cmd_enable(conn, ident.trim()) {
                    log::warn(format!("{e}"));
                }
            }
            "6" => {
                println!();
                let ident = read_input("    ID o carpeta del mod a desactivar:");
                if let Err(e) = cmd_disable(conn, ident.trim()) {
                    log::warn(format!("{e}"));
                }
            }
            "7" => {
                println!();
                let ident = read_input("    ID o carpeta del mod:");
                let order = read_input("    Nuevo orden de carga:");
                if let Ok(order) = order.trim().parse::<i64>() {
                    if let Err(e) = cmd_order(conn, ident.trim(), order) {
                        log::warn(format!("{e}"));
                    }
                } else {
                    log::warn("El orden debe ser un numero entero.");
                }
            }
            "8" => {
                println!();
                let ident = read_input("    ID o carpeta del mod:");
                let name = read_input("    Nuevo nombre:");
                if let Err(e) = cmd_rename(conn, ident.trim(), name.trim()) {
                    log::warn(format!("{e}"));
                }
            }
            "9" => {
                println!();
                let ident = read_input("    ID o carpeta del mod:");
                if let Err(e) = cmd_info(conn, ident.trim()) {
                    log::warn(format!("{e}"));
                }
            }
            "10" => {
                println!();
                println!("    a) Anadir dependencia");
                println!("    b) Eliminar dependencia");
                let depopt = read_input("    Opcion [a/b]:");
                match depopt.trim() {
                    "a" => {
                        let mod_ident = read_input("      Mod:");
                        let dep_ident = read_input("      Dependencia (mod requerido):");
                        if let Err(e) = cmd_dep_add(conn, mod_ident.trim(), dep_ident.trim()) {
                            log::warn(format!("{e}"));
                        }
                    }
                    "b" => {
                        let mod_ident = read_input("      Mod:");
                        let dep_ident = read_input("      Dependencia a eliminar:");
                        if let Err(e) = cmd_dep_rm(conn, mod_ident.trim(), dep_ident.trim()) {
                            log::warn(format!("{e}"));
                        }
                    }
                    _ => log::warn("Opcion invalida."),
                }
            }
            "0" => {
                println!();
                log::info("Hasta luego!");
                return Ok(());
            }
            _ => log::warn("Opcion invalida."),
        }
    }
}

fn read_input(prompt: &str) -> String {
    eprint!("{prompt} ");
    std::io::Write::flush(&mut std::io::stderr()).ok();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).ok();
    input.trim_end_matches('\n').to_string()
}
