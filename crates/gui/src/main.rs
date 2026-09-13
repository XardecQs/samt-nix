#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod about;
mod app;
mod backend;
mod fonts;
mod icons;
mod model;
mod preferences;
mod settings;
mod theme;
mod toasts;

use eframe::egui;
use std::sync::Arc;

/// Window icon (placeholder PNG; replace `assets/icon.png` with your design).
fn load_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .ok()?
        .to_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData {
        rgba: img.into_raw(),
        width,
        height,
    })
}

fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1050.0, 700.0])
        .with_min_inner_size([720.0, 480.0])
        .with_title("GTA SA Mod Organizer")
        // Must match StartupWMClass in the .desktop entry.
        .with_app_id("gta-mo-gui");
    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(Arc::new(icon));
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "gta-mo-gui",
        options,
        Box::new(|cc| Ok(Box::new(app::GtaMoApp::new(cc)))),
    )
}
