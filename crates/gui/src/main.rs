#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod about;
mod app;
mod backend;
mod desktop;
mod file_browser;
mod fonts;
mod form;
mod graph_edge;
mod graph_node;
mod icons;
mod icons_theme;
mod lightbox;
mod model;
mod motion;
mod nerd;
mod preferences;
mod runtime;
mod settings;
mod theme;
mod thumbs;
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

/// Parses `--launch <slug>` / `--launch=<slug>` from the CLI, so a per-profile
/// `.desktop` entry can open the GUI and start the game right away.
fn parse_launch_arg(args: &[String]) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--launch" {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix("--launch=") {
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn main() -> eframe::Result {
    let launch = parse_launch_arg(&std::env::args().collect::<Vec<_>>());
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1050.0, 700.0])
        .with_min_inner_size([360.0, 420.0])
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
        Box::new(move |cc| Ok(Box::new(app::GtaMoApp::new(cc, launch.clone())))),
    )
}
