//! "About" and "Keyboard shortcuts" dialogs.

use crate::settings::GuiSettings;
use eframe::egui;

const REPO_URL: &str = "https://github.com/XardecQs/samt-nix";
const LICENSE: &str = "GPL-3.0-or-later";

pub fn show_about(ctx: &egui::Context, settings: &GuiSettings, open: &mut bool) {
    let mut close = false;
    let resp = egui::Modal::new(egui::Id::new("gta_mo_about"))
        .frame(egui::Frame::popup(ctx.style().as_ref()))
        .show(ctx, |ui| {
            ui.set_min_width(420.0);
            ui.vertical_centered(|ui| {
                ui.add_space(4.0);
                ui.heading("GTA SA Mod Organizer");
                ui.label(
                    egui::RichText::new(format!("Versión {}", env!("CARGO_PKG_VERSION"))).weak(),
                );
                ui.add_space(8.0);
            });
            ui.separator();
            ui.label("Organizador de mods para GTA San Andreas en Linux, con fuse-overlayfs.");
            ui.add_space(6.0);
            egui::Grid::new("about_info")
                .num_columns(2)
                .spacing([14.0, 6.0])
                .show(ui, |ui| {
                    ui.label("Licencia");
                    ui.label(LICENSE);
                    ui.end_row();
                    ui.label("Proyecto");
                    ui.hyperlink_to("GitHub", REPO_URL);
                    ui.end_row();
                    ui.label("Iconos");
                    ui.hyperlink_to("Lucide (ISC)", "https://lucide.dev");
                    ui.end_row();
                });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button("Copiar diagnóstico").clicked() {
                    ui.ctx().copy_text(diagnostics(settings));
                }
                if ui.button("Cerrar").clicked() {
                    close = true;
                }
            });
        });
    if resp.should_close() {
        close = true;
    }
    if close {
        *open = false;
    }
}

fn diagnostics(settings: &GuiSettings) -> String {
    let cfg = gta_mo_core::config::find_config_file()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "(no encontrado)".to_string());
    format!(
        "gta-mo-gui {}\n\
         tema: {:?}\n\
         acento: {:?}\n\
         alto contraste: {}\n\
         reducir movimiento: {}\n\
         escala ui: {}\n\
         binario gta-mo: {}\n\
         config: {}\n\
         base de datos: {}",
        env!("CARGO_PKG_VERSION"),
        settings.theme,
        settings.accent,
        settings.high_contrast,
        settings.reduce_motion,
        settings.ui_scale,
        crate::backend::find_gta_mo_bin(),
        cfg,
        gta_mo_core::config::db_path().display(),
    )
}

pub fn show_shortcuts(ctx: &egui::Context, open: &mut bool) {
    let mut close = false;
    let resp = egui::Modal::new(egui::Id::new("gta_mo_shortcuts"))
        .frame(egui::Frame::popup(ctx.style().as_ref()))
        .show(ctx, |ui| {
            ui.set_min_width(360.0);
            ui.heading("Atajos de teclado");
            ui.separator();
            egui::Grid::new("shortcuts")
                .num_columns(2)
                .spacing([18.0, 6.0])
                .show(ui, |ui| {
                    for (keys, action) in SHORTCUTS {
                        ui.monospace(*keys);
                        ui.label(*action);
                        ui.end_row();
                    }
                });
            ui.add_space(8.0);
            if ui.button("Cerrar").clicked() {
                close = true;
            }
        });
    if resp.should_close() {
        close = true;
    }
    if close {
        *open = false;
    }
}

const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+F", "Buscar mods"),
    ("Ctrl+R", "Refrescar"),
    ("Ctrl+,", "Abrir Preferencias"),
    ("Enter", "Confirmar diálogo"),
    ("Esc", "Cerrar diálogo / modal"),
];
