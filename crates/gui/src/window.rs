use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
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

    let content_box = adw::ToolbarView::builder().build();
    content_box.add_top_bar(&header);

    let stack = gtk4::Stack::builder()
        .transition_type(gtk4::StackTransitionType::SlideLeftRight)
        .build();

    let stack_switcher = gtk4::StackSwitcher::builder()
        .stack(&stack)
        .halign(gtk4::Align::Center)
        .build();
    header.set_title_widget(Some(&stack_switcher));

    let db_path = config::db_path();
    log::info(format!("DB: {}", db_path.display()));

    let mods_page = mods_page::create();
    let config_page = config_page::create();
    let launch_page = launch_page::create();

    stack.add_titled(&mods_page, Some("mods"), "Mods");
    stack.add_titled(&config_page, Some("config"), "Configuración");
    stack.add_titled(&launch_page, Some("launch"), "Jugar");

    content_box.set_content(Some(&stack));
    window.set_content(Some(&content_box));

    let gtk_window: &gtk4::ApplicationWindow = window.upcast_ref();
    let _gamepad = crate::gamepad::GamepadHandler::start(gtk_window);

    window.present();
}
