mod gamepad;
mod pages;
mod window;

use gtk4::prelude::*;
use libadwaita as adw;

fn main() {
    let app = adw::Application::builder()
        .application_id("com.samt.gta-mo-gui")
        .build();

    app.connect_activate(|app| {
        window::build_ui(app);
    });

    app.run();
}
