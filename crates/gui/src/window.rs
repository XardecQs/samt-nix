use gtk4::prelude::*;
use libadwaita as adw;
use gta_mo_core::config;
use gta_mo_core::db::log;

use crate::pages::{config_page, launch_page, mods_page};

pub fn build_ui(app: &adw::Application) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("SAMT — GTA Mod Organizer")
        .default_width(900)
        .default_height(600)
        .build();

    let header = adw::HeaderBar::builder().build();

    let content = adw::ToolbarView::builder()
        .add_top_bar(&header)
        .build();

    let stack = gtk4::Stack::builder()
        .transition_type(gtk4::StackTransitionType::SlideLeftRight)
        .build();

    let view_switcher = adw::ViewSwitcher::builder().stack(&stack).build();
    let view_switcher_title = adw::ViewSwitcherTitle::builder()
        .stack(&stack)
        .title("SAMT")
        .build();
    header.set_title_widget(Some(&view_switcher_title));

    let db_path = config::db_path();
    log::info(format!("DB: {}", db_path.display()));

    let mods_page = mods_page::create();
    let config_page = config_page::create();
    let launch_page = launch_page::create();

    stack.add_titled(&mods_page, Some("mods"), "Mods");
    stack.add_titled(&config_page, Some("config"), "Configuración");
    stack.add_titled(&launch_page, Some("launch"), "Jugar");

    content.set_content(Some(&stack));
    window.set_content(Some(&content));

    let _gamepad = crate::gamepad::GamepadHandler::start(&window);

    window.present();
}
