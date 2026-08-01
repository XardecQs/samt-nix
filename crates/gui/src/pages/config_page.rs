use gtk4::prelude::*;
use gta_mo_core::config;
use gta_mo_core::config::UserOverrides;
use gta_mo_core::db::log;

pub fn create() -> gtk4::Box {
    let container = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(8)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let nix_managed = config::is_nix_managed();

    if nix_managed {
        let badge = gtk4::Label::new(Some(
            "Configuración gestionada por Nix/Home Manager (solo lectura).\nUsa los campos de abajo para sobrescribir valores.",
        ));
        badge.add_css_class("caption");
        badge.set_margin_bottom(6);
        container.append(&badge);
    }

    let scroll = gtk4::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .build();

    let list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .build();
    scroll.set_child(Some(&list));
    container.append(&scroll);

    let cfg = match config::load_config() {
        Ok(c) => c,
        Err(e) => {
            list.append(&error_row(&format!("Error cargando config: {e}")));
            return container;
        }
    };

    add_row(
        &list,
        "Directorio raíz del juego",
        &cfg.game_root,
        nix_managed,
    );
    add_row(&list, "Ruta de Proton", &cfg.proton_path, nix_managed);
    add_row(&list, "ID del juego", cfg.game_id(), nix_managed);
    add_row(&list, "Ejecutable", cfg.game_exe(), nix_managed);
    add_row(
        &list,
        "Usar WineD3D",
        &cfg.proton_use_wined3d().to_string(),
        nix_managed,
    );
    add_row(
        &list,
        "Desactivar NTSync",
        &cfg.proton_disable_ntsync().to_string(),
        nix_managed,
    );
    add_row(
        &list,
        "Auto-descubrir mods",
        &cfg.auto_discover().to_string(),
        nix_managed,
    );
    add_row(&list, "DXVK HUD", cfg.dxvk_hud(), nix_managed);
    add_row(
        &list,
        "Directorio de mods",
        cfg.mods_dir
            .as_deref()
            .unwrap_or("(por defecto: game_root/mods)"),
        nix_managed,
    );

    if nix_managed {
        let sep = gtk4::Separator::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .margin_top(8)
            .margin_bottom(8)
            .build();
        list.append(&sep_row(&sep));

        let title_row = gtk4::Label::new(Some("Sobrescrituras (config.user.toml):"));
        title_row.set_margin_top(6);
        title_row.set_margin_bottom(6);
        list.append(&label_row(&title_row));

        let user_overrides = config::load_user_overrides().unwrap_or_default();

        let auto_entry = gtk4::Entry::builder()
            .placeholder_text("auto_discover (true/false)")
            .text(
                &user_overrides
                    .auto_discover
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            )
            .build();
        add_override_row(&list, "auto_discover", &auto_entry);

        let wined3d_entry = gtk4::Entry::builder()
            .placeholder_text("proton_use_wined3d (true/false)")
            .text(
                &user_overrides
                    .proton_use_wined3d
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            )
            .build();
        add_override_row(&list, "proton_use_wined3d", &wined3d_entry);

        let dxvk_entry = gtk4::Entry::builder()
            .placeholder_text("dxvk_hud")
            .text(user_overrides.dxvk_hud.as_deref().unwrap_or(""))
            .build();
        add_override_row(&list, "dxvk_hud", &dxvk_entry);

        let save_btn = gtk4::Button::builder()
            .label("Guardar sobrescritura")
            .halign(gtk4::Align::Center)
            .margin_top(12)
            .build();
        save_btn.add_css_class("suggested-action");

        let list_save = list.clone();
        save_btn.connect_clicked(move |_| {
            let overrides = collect_overrides(&list_save);
            if let Err(e) = config::save_user_overrides(&overrides) {
                log::error(format!("Error guardando overrides: {e}"));
            } else {
                log::info("Sobrescrituras guardadas en config.user.toml");
            }
        });
        container.append(&save_btn);
    }

    container
}

fn add_row(list: &gtk4::ListBox, label: &str, value: &str, locked: bool) {
    let row = gtk4::ListBoxRow::builder().build();
    let hbox = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(12)
        .margin_start(6)
        .margin_end(6)
        .margin_top(3)
        .margin_bottom(3)
        .build();

    let label_w = gtk4::Label::builder()
        .label(label)
        .width_chars(25)
        .halign(gtk4::Align::Start)
        .xalign(0.0)
        .build();

    let value_w = gtk4::Label::builder()
        .label(value)
        .halign(gtk4::Align::Start)
        .hexpand(true)
        .xalign(0.0)
        .selectable(true)
        .single_line_mode(true)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .build();

    hbox.append(&label_w);
    hbox.append(&value_w);

    if locked {
        let lock = gtk4::Label::new(Some("(gestionado por Nix)"));
        lock.add_css_class("dim-label");
        lock.add_css_class("caption");
        hbox.append(&lock);
    }

    row.set_child(Some(&hbox));
    list.append(&row);
}

fn error_row(msg: &str) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::builder().build();
    let label = gtk4::Label::new(Some(msg));
    label.add_css_class("error");
    row.set_child(Some(&label));
    row
}

fn sep_row(sep: &gtk4::Separator) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::builder().build();
    row.set_child(Some(sep));
    row
}

fn label_row(label: &gtk4::Label) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::builder().build();
    row.set_child(Some(label));
    row
}

fn add_override_row(list: &gtk4::ListBox, label: &str, entry: &gtk4::Entry) {
    let row = gtk4::ListBoxRow::builder().build();
    let hbox = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(12)
        .margin_start(6)
        .margin_end(6)
        .margin_top(3)
        .margin_bottom(3)
        .build();

    let label_w = gtk4::Label::builder()
        .label(label)
        .width_chars(25)
        .halign(gtk4::Align::Start)
        .xalign(0.0)
        .build();

    entry.set_hexpand(true);
    hbox.append(&label_w);
    hbox.append(entry);
    row.set_child(Some(&hbox));
    list.append(&row);
}

fn collect_overrides(list: &gtk4::ListBox) -> UserOverrides {
    let mut overrides = UserOverrides::default();
    let mut idx = 0;

    while let Some(row) = list.row_at_index(idx) {
        if let Some(child) = row.child() {
            if let Some(hbox) = child.downcast_ref::<gtk4::Box>() {
                if let Some(label) = hbox.first_child().and_downcast::<gtk4::Label>() {
                    if let Some(entry) = hbox.last_child().and_downcast::<gtk4::Entry>() {
                        let text = entry.text().to_string();
                        if !text.is_empty() {
                            match label.label().as_str() {
                                "auto_discover" => {
                                    overrides.auto_discover = Some(text == "true");
                                }
                                "proton_use_wined3d" => {
                                    overrides.proton_use_wined3d = Some(text == "true");
                                }
                                "dxvk_hud" => {
                                    overrides.dxvk_hud = Some(text);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        idx += 1;
    }

    overrides
}
