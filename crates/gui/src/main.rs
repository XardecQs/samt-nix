#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod backend;
mod model;

use eframe::egui;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1050.0, 700.0])
            .with_min_inner_size([720.0, 480.0])
            .with_title("GTA SA Mod Organizer"),
        ..Default::default()
    };
    eframe::run_native(
        "gta-mo-gui",
        options,
        Box::new(|cc| Ok(Box::new(app::GtaMoApp::new(cc)))),
    )
}
