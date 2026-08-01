use adw::prelude::*;
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

    let header = adw::HeaderBar::builder()
        .title_widget(&gtk4::Label::new(Some("SAMT — GTA Mod Organizer")))
        .build();

    let content = adw::ToolbarView::builder().add_top_bar(&header).build();

    let stack = gtk4::Stack::builder()
        .transition_type(gtk4::StackTransitionType::SlideLeftRight)
        .build();

    let view_switcher = adw::ViewSwitcher::builder().stack(&stack).build();
    let view_switcher_title = adw::ViewSwitcherTitle::builder()
        .stack(&stack)
        .title("SAMT")
        .build();
    header.set_title_widget(Some(&view_switcher));

    let db_path = config::db_path();
    log::info(format!("DB: {}", db_path.display()));

    let mods_page = mods_page::create();
    let config_page = config_page::create();
    let launch_page = launch_page::create();

    stack.add_titled_with_icon(
        &mods_page,
        Some("mods"),
        "Mods",
        Some("applications-games-symbolic"),
    );
    stack.add_titled_with_icon(
        &config_page,
        Some("config"),
        "Configuración",
        Some("preferences-system-symbolic"),
    );
    stack.add_titled_with_icon(
        &launch_page,
        Some("launch"),
        "Jugar",
        Some("media-playback-start-symbolic"),
    );

    view_switcher_title.set_stack(&stack);
    view_switcher_title.set_title("SAMT");

    content.set_content(Some(&stack));
    window.set_content(Some(&content));

    let _gamepad = crate::gamepad::GamepadHandler::start(&window);

    window.present();
}
