use gta_mo_core::config;
use gta_mo_core::db;
use gta_mo_core::db::log;
use gtk4::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub fn create() -> gtk4::Box {
    let container = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(6)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let scroll = gtk4::ScrolledWindow::builder()
        .vexpand(true)
        .hexpand(true)
        .build();

    let list_box = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::Single)
        .build();

    scroll.set_child(Some(&list_box));
    container.append(&scroll);

    let btn_box = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .spacing(6)
        .halign(gtk4::Align::Start)
        .build();

    let add_btn = gtk4::Button::builder().label("+ Añadir").build();
    let remove_btn = gtk4::Button::builder().label("− Quitar").build();
    let up_btn = gtk4::Button::builder().label("↑").build();
    let down_btn = gtk4::Button::builder().label("↓").build();

    btn_box.append(&add_btn);
    btn_box.append(&remove_btn);
    btn_box.append(&up_btn);
    btn_box.append(&down_btn);
    container.append(&btn_box);

    let mods_data: Rc<RefCell<Vec<db::ModEntry>>> = Rc::new(RefCell::new(Vec::new()));

    refresh_list(&list_box, &mods_data);

    let mods_add = mods_data.clone();
    let list_add = list_box.clone();
    add_btn.connect_clicked(move |_| {
        show_add_dialog(&list_add, &mods_add);
    });

    let mods_rem = mods_data.clone();
    let list_rem = list_box.clone();
    remove_btn.connect_clicked(move |_| {
        if let Some(row) = list_rem.selected_row() {
            let idx = row.index() as usize;
            let mods = mods_rem.borrow();
            if let Some(m) = mods.get(idx) {
                let id = m.id;
                drop(mods);
                show_remove_dialog(&list_rem, &mods_rem, id);
            }
        }
    });

    let mods_up = mods_data.clone();
    let list_up = list_box.clone();
    up_btn.connect_clicked(move |_| {
        if let Some(row) = list_up.selected_row() {
            let idx = row.index() as usize;
            let mods = mods_up.borrow();
            if let Some(m) = mods.get(idx) {
                adjust_order(&list_up, &mods_up, m.id, 5);
            }
        }
    });

    let mods_down = mods_data;
    let list_down = list_box;
    down_btn.connect_clicked(move |_| {
        if let Some(row) = list_down.selected_row() {
            let idx = row.index() as usize;
            let mods = mods_down.borrow();
            if let Some(m) = mods.get(idx) {
                adjust_order(&list_down, &mods_down, m.id, -5);
            }
        }
    });

    container
}

fn refresh_list(list_box: &gtk4::ListBox, mods_data: &Rc<RefCell<Vec<db::ModEntry>>>) {
    while let Some(row) = list_box.first_child() {
        list_box.remove(&row);
    }

    let db_path = config::db_path();
    let conn = match db::open_db(&db_path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let mods = db::load_all_mods(&conn).unwrap_or_default();
    *mods_data.borrow_mut() = mods.clone();

    for m in &mods {
        let row = gtk4::ListBoxRow::builder().activatable(true).build();

        let hbox = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(12)
            .margin_start(6)
            .margin_end(6)
            .margin_top(4)
            .margin_bottom(4)
            .build();

        let toggle = gtk4::CheckButton::builder()
            .active(m.enabled)
            .tooltip_text("Activar / Desactivar")
            .build();

        let folder_label = gtk4::Label::builder()
            .label(&m.folder_name)
            .width_chars(28)
            .halign(gtk4::Align::Start)
            .xalign(0.0)
            .single_line_mode(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();

        let name_label = gtk4::Label::builder()
            .label(&m.name)
            .width_chars(28)
            .halign(gtk4::Align::Start)
            .xalign(0.0)
            .single_line_mode(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();

        let order_label = gtk4::Label::builder()
            .label(&format!("{}", m.load_order))
            .width_chars(6)
            .halign(gtk4::Align::End)
            .build();

        hbox.append(&toggle);
        hbox.append(&folder_label);
        hbox.append(&name_label);
        hbox.append(&order_label);

        let mod_id = m.id;
        let db_path = db_path.clone();
        let mods_clone = mods_data.clone();
        let list_clone = list_box.clone();
        toggle.connect_toggled(move |toggle| {
            if let Ok(conn) = db::open_db(&db_path) {
                let _ = db::set_mod_enabled(&conn, mod_id, toggle.is_active());
                refresh_list(&list_clone, &mods_clone);
            }
        });

        row.set_child(Some(&hbox));
        list_box.append(&row);
    }
}

fn show_add_dialog(list_box: &gtk4::ListBox, mods_data: &Rc<RefCell<Vec<db::ModEntry>>>) {
    let dialog = gtk4::Dialog::builder()
        .title("Añadir mod")
        .modal(true)
        .build();

    let content = gtk4::Box::builder()
        .orientation(gtk4::Orientation::Vertical)
        .spacing(6)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let entry = gtk4::Entry::builder()
        .placeholder_text("Nombre de la carpeta del mod")
        .build();
    let name_entry = gtk4::Entry::builder()
        .placeholder_text("Nombre visible (opcional)")
        .build();

    content.append(&gtk4::Label::new(Some("Carpeta del mod:")));
    content.append(&entry);
    content.append(&gtk4::Label::new(Some("Nombre visible:")));
    content.append(&name_entry);

    dialog.content_area().unwrap().append(&content);
    dialog.add_button("Cancelar", gtk4::ResponseType::Cancel);
    dialog.add_button("Añadir", gtk4::ResponseType::Ok);

    let list = list_box.clone();
    let data = mods_data.clone();
    let db_path = config::db_path();

    dialog.connect_response(move |dialog, response| {
        if response == gtk4::ResponseType::Ok {
            let folder = entry.text().to_string();
            let name = name_entry.text().to_string();
            let name_opt = if name.is_empty() {
                None
            } else {
                Some(name.as_str())
            };
            if !folder.is_empty() {
                if let Ok(conn) = db::open_db(&db_path) {
                    match db::add_mod(&conn, folder.trim(), name_opt, None) {
                        Ok(_) => {
                            log::info(format!("Mod '{}' añadido.", folder.trim()));
                            refresh_list(&list, &data);
                        }
                        Err(e) => log::error(format!("{e}")),
                    }
                }
            }
        }
        dialog.close();
    });

    dialog.present(None::<&gtk4::Window>);
}

fn show_remove_dialog(
    list_box: &gtk4::ListBox,
    mods_data: &Rc<RefCell<Vec<db::ModEntry>>>,
    mod_id: i64,
) {
    let folder = mods_data
        .borrow()
        .iter()
        .find(|m| m.id == mod_id)
        .map(|m| m.folder_name.clone())
        .unwrap_or_default();

    let dialog = gtk4::MessageDialog::builder()
        .message_type(gtk4::MessageType::Question)
        .text(format!("¿Eliminar el mod '{}'?", folder))
        .secondary_text("Esta acción no se puede deshacer.")
        .modal(true)
        .build();

    dialog.add_button("Cancelar", gtk4::ResponseType::Cancel);
    dialog.add_button("Eliminar", gtk4::ResponseType::Ok);

    let list = list_box.clone();
    let data = mods_data.clone();
    let db_path = config::db_path();

    dialog.connect_response(move |dialog, response| {
        if response == gtk4::ResponseType::Ok {
            if let Ok(conn) = db::open_db(&db_path) {
                let _ = db::remove_mod(&conn, mod_id);
                refresh_list(&list, &data);
            }
        }
        dialog.close();
    });

    dialog.present(None::<&gtk4::Window>);
}

fn adjust_order(
    list_box: &gtk4::ListBox,
    mods_data: &Rc<RefCell<Vec<db::ModEntry>>>,
    mod_id: i64,
    delta: i64,
) {
    let db_path = config::db_path();
    if let Ok(conn) = db::open_db(&db_path) {
        let current = mods_data
            .borrow()
            .iter()
            .find(|m| m.id == mod_id)
            .map(|m| m.load_order)
            .unwrap_or(0);
        let new_order = (current + delta).max(0);
        let _ = db::set_mod_order(&conn, mod_id, new_order);
        refresh_list(list_box, mods_data);
    }
}
