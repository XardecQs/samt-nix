//! Preferences dialog (appearance, behavior and advanced info).

use crate::settings::{Density, GuiSettings};
use crate::theme::{Accent, ThemePref};
use eframe::egui;

/// Shows the preferences modal. `open` is set to `false` when it should close.
/// Returns `true` when a preference changed (the caller persists + re-applies).
pub fn show(ctx: &egui::Context, settings: &mut GuiSettings, open: &mut bool) -> bool {
    let mut changed = false;
    let mut close = false;
    let resp = egui::Modal::new(egui::Id::new("gta_mo_preferences"))
        .frame(egui::Frame::popup(ctx.style().as_ref()))
        .show(ctx, |ui| {
            ui.set_min_width(480.0);
            ui.horizontal(|ui| {
                ui.heading("Preferencias");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cerrar").clicked() {
                        close = true;
                    }
                });
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .max_height(460.0)
                .show(ui, |ui| {
                    changed |= appearance(ui, settings);
                    ui.add_space(10.0);
                    changed |= behavior(ui, settings);
                    ui.add_space(10.0);
                    advanced(ui);
                });
        });
    if resp.should_close() {
        close = true;
    }
    if close {
        *open = false;
    }
    changed
}

fn section(ui: &mut egui::Ui, icon: &str, title: &str) {
    ui.add_space(2.0);
    ui.label(egui::RichText::new(format!("{icon}  {title}")).strong());
    ui.add_space(4.0);
}

fn appearance(ui: &mut egui::Ui, s: &mut GuiSettings) -> bool {
    let mut changed = false;
    section(ui, crate::icons::PALETTE, "Apariencia");
    egui::Grid::new("prefs_appearance")
        .num_columns(2)
        .spacing([18.0, 10.0])
        .show(ui, |ui| {
            ui.label("Tema");
            egui::ComboBox::from_id_salt("prefs_theme")
                .selected_text(s.theme.label())
                .show_ui(ui, |ui| {
                    for t in [ThemePref::System, ThemePref::Light, ThemePref::Dark] {
                        changed |= ui.selectable_value(&mut s.theme, t, t.label()).changed();
                    }
                });
            ui.end_row();

            ui.label("Color de acento");
            ui.horizontal(|ui| {
                for a in Accent::ALL {
                    let selected = s.accent == a;
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
                    let painter = ui.painter();
                    painter.circle_filled(rect.center(), 10.0, a.swatch());
                    if selected {
                        painter.circle_stroke(
                            rect.center(),
                            11.5,
                            egui::Stroke::new(2.0, ui.visuals().text_color()),
                        );
                    }
                    if resp.clicked() {
                        s.accent = a;
                        changed = true;
                    }
                    resp.on_hover_text(a.label());
                }
            });
            ui.end_row();

            ui.label("Densidad");
            egui::ComboBox::from_id_salt("prefs_density")
                .selected_text(s.density.label())
                .show_ui(ui, |ui| {
                    for d in [Density::Cozy, Density::Compact] {
                        changed |= ui.selectable_value(&mut s.density, d, d.label()).changed();
                    }
                });
            ui.end_row();

            ui.label("Escala de la interfaz");
            changed |= ui
                .add(egui::Slider::new(&mut s.ui_scale, 0.7..=2.5).fixed_decimals(2))
                .changed();
            ui.end_row();
        });

    ui.add_space(6.0);
    changed |= ui
        .checkbox(&mut s.high_contrast, "Alto contraste")
        .on_hover_text("Refuerza bordes y texto para mejorar la legibilidad")
        .changed();
    changed |= ui
        .checkbox(&mut s.reduce_motion, "Reducir movimiento")
        .on_hover_text("Desactiva animaciones y transiciones")
        .changed();
    changed
}

fn behavior(ui: &mut egui::Ui, s: &mut GuiSettings) -> bool {
    let mut changed = false;
    section(ui, crate::icons::SETTINGS, "Comportamiento");
    changed |= ui
        .checkbox(&mut s.show_covers, "Mostrar portadas en la lista")
        .changed();
    changed |= ui
        .checkbox(&mut s.confirm_deletes, "Pedir confirmación al eliminar")
        .changed();
    changed
}

fn advanced(ui: &mut egui::Ui) {
    section(ui, crate::icons::FOLDER_COG, "Avanzado");
    let bin = crate::backend::find_gta_mo_bin();
    ui.horizontal(|ui| {
        ui.label("Binario gta-mo:");
        ui.monospace(bin);
    });
    match gta_mo_core::config::find_config_file() {
        Some(p) => {
            ui.horizontal(|ui| {
                ui.label("Configuración:");
                ui.monospace(p.display().to_string());
            });
        }
        None => {
            ui.colored_label(
                crate::theme::active(ui.ctx()).warning,
                "No se encontró config.toml",
            );
        }
    }
    ui.horizontal(|ui| {
        ui.label("Base de datos:");
        ui.monospace(gta_mo_core::config::db_path().display().to_string());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_runs_one_frame() {
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx, GuiSettings::default().theme_config());
        let mut settings = GuiSettings::default();
        let mut open = true;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let _ = show(ctx, &mut settings, &mut open);
        });
    }
}
