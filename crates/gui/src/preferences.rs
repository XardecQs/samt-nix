//! Preferences dialog (appearance, behavior and advanced info).

use crate::settings::{Density, GuiSettings};
use crate::theme::{Accent, ThemePref};
use eframe::egui;

/// Shows the preferences overlay page. `open` is set to `false` when it should
/// close. Returns `true` when a preference changed (caller persists + applies).
pub fn show(ctx: &egui::Context, settings: &mut GuiSettings, open: &mut bool) -> bool {
    let mut changed = false;
    let mut close = false;
    let narrow = ctx.screen_rect().width() < crate::app::NARROW_BREAKPOINT;
    crate::app::overlay_page(ctx, "gta_mo_preferences", narrow, 560.0, |ui| {
        if crate::app::overlay_header(ui, "Preferencias", "Cerrar") {
            close = true;
        }
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                changed |= appearance(ui, settings);
                ui.add_space(10.0);
                changed |= behavior(ui, settings);
                ui.add_space(10.0);
                advanced(ui);
            });
    });
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
            ui.horizontal_wrapped(|ui| {
                let custom = s.custom_accent.is_some();
                for a in Accent::ALL {
                    let selected = !custom && s.accent == a;
                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
                    let painter = ui.painter();
                    painter.circle_filled(rect.center(), 10.0, a.swatch());
                    if selected {
                        painter.circle_stroke(
                            rect.center(),
                            12.0,
                            egui::Stroke::new(2.0_f32, ui.visuals().text_color()),
                        );
                    }
                    if resp.clicked() {
                        s.custom_accent = None;
                        s.accent = a;
                        changed = true;
                    }
                    resp.on_hover_text(a.label());
                }

                // Botón extra: color personalizado (se usa tal cual en ambos temas).
                let mut rgb = s
                    .custom_accent
                    .as_deref()
                    .and_then(crate::theme::parse_hex)
                    .map(|c| [c.r(), c.g(), c.b()])
                    .unwrap_or_else(|| {
                        let d = s.accent.swatch();
                        [d.r(), d.g(), d.b()]
                    });
                if ui
                    .color_edit_button_srgb(&mut rgb)
                    .on_hover_text("Color personalizado")
                    .changed()
                {
                    s.custom_accent = Some(crate::theme::format_hex(egui::Color32::from_rgb(
                        rgb[0], rgb[1], rgb[2],
                    )));
                    changed = true;
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
