//! Preferences dialog (appearance, behavior and advanced info).

use crate::settings::{Density, GraphLayout, GraphSettings, GuiSettings, HierOrientation};
use crate::theme::{Accent, ThemePref};
use eframe::egui;

/// Shows the preferences overlay page. `open` is set to `false` when it should
/// close. Returns `true` when a preference changed (caller persists + applies).
pub fn show(ctx: &egui::Context, settings: &mut GuiSettings, open: &mut bool) -> bool {
    let mut changed = false;
    let mut close = false;
    crate::app::overlay_page(ctx, "gta_mo_preferences", 560.0, |ui| {
        if crate::app::overlay_header(ui, "Preferencias", "Cerrar") {
            close = true;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .show(ui, |ui| {
                changed |= appearance(ui, settings);
                ui.add_space(10.0);
                changed |= behavior(ui, settings);
                ui.add_space(10.0);
                changed |= graph(ui, &mut settings.graph);
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

fn graph(ui: &mut egui::Ui, g: &mut GraphSettings) -> bool {
    let mut changed = false;
    section(ui, crate::icons::LINK, "Grafo");

    egui::Grid::new("prefs_graph")
        .num_columns(2)
        .spacing([18.0, 10.0])
        .show(ui, |ui| {
            ui.label("Layout por defecto");
            egui::ComboBox::from_id_salt("prefs_graph_layout")
                .selected_text(g.layout.label())
                .show_ui(ui, |ui| {
                    for l in [GraphLayout::Force, GraphLayout::Hierarchical] {
                        changed |= ui.selectable_value(&mut g.layout, l, l.label()).changed();
                    }
                });
            ui.end_row();

            ui.label("Velocidad de zoom");
            changed |= ui
                .add(egui::Slider::new(&mut g.zoom_speed, 0.02..=0.5).fixed_decimals(2))
                .on_hover_text("Velocidad del zoom con ctrl/rueda y pinch")
                .changed();
            ui.end_row();

            ui.label("Margen al ajustar");
            changed |= ui
                .add(egui::Slider::new(&mut g.fit_padding, 0.0..=0.3).fixed_decimals(2))
                .on_hover_text("Espacio libre alrededor del grafo al pulsar «Ajustar»")
                .changed();
            ui.end_row();
        });

    ui.add_space(4.0);
    ui.label("Aristas:");
    changed |= ui
        .checkbox(&mut g.show_optional, "Mostrar dependencias opcionales")
        .changed();
    changed |= ui
        .checkbox(&mut g.show_conflicts, "Mostrar conflictos")
        .changed();
    changed |= ui
        .checkbox(&mut g.show_variants, "Mostrar familias de variantes")
        .changed();

    ui.add_space(4.0);
    ui.label("Interacción:");
    changed |= ui
        .checkbox(&mut g.labels_always, "Mostrar etiquetas siempre")
        .changed();
    changed |= ui
        .checkbox(&mut g.dragging, "Permitir arrastrar nodos")
        .changed();
    changed |= ui
        .checkbox(&mut g.node_selection, "Permitir seleccionar nodos")
        .changed();
    changed |= ui
        .checkbox(
            &mut g.auto_settle,
            "El layout de Fuerza se detiene al asentarse",
        )
        .changed();

    ui.add_space(4.0);
    egui::CollapsingHeader::new("Fuerza (avanzado)")
        .id_salt("prefs_graph_force")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("prefs_graph_force_grid")
                .num_columns(2)
                .spacing([18.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Gravedad central");
                    changed |= ui
                        .add(egui::Slider::new(&mut g.force.gravity, 0.0..=2.0).fixed_decimals(2))
                        .changed();
                    ui.end_row();

                    ui.label("Atracción");
                    changed |= ui
                        .add(egui::Slider::new(&mut g.force.c_attract, 0.0..=2.0).fixed_decimals(2))
                        .changed();
                    ui.end_row();

                    ui.label("Repulsión");
                    changed |= ui
                        .add(egui::Slider::new(&mut g.force.c_repulse, 0.0..=2.0).fixed_decimals(2))
                        .changed();
                    ui.end_row();

                    ui.label("Amortiguación");
                    changed |= ui
                        .add(egui::Slider::new(&mut g.force.damping, 0.0..=1.0).fixed_decimals(2))
                        .changed();
                    ui.end_row();

                    ui.label("Escala k");
                    changed |= ui
                        .add(egui::Slider::new(&mut g.force.k_scale, 0.1..=2.0).fixed_decimals(2))
                        .changed();
                    ui.end_row();

                    ui.label("Paso de tiempo (dt)");
                    changed |= ui
                        .add(egui::Slider::new(&mut g.force.dt, 0.01..=0.2).fixed_decimals(3))
                        .changed();
                    ui.end_row();

                    ui.label("Paso máximo");
                    changed |= ui
                        .add(egui::Slider::new(&mut g.force.max_step, 1.0..=50.0).fixed_decimals(1))
                        .changed();
                    ui.end_row();

                    ui.label("Épsilon");
                    changed |= ui
                        .add(egui::Slider::new(&mut g.force.epsilon, 1e-4..=1e-2).fixed_decimals(4))
                        .changed();
                    ui.end_row();
                });
        });

    ui.add_space(4.0);
    egui::CollapsingHeader::new("Jerárquico (avanzado)")
        .id_salt("prefs_graph_hier")
        .default_open(false)
        .show(ui, |ui| {
            egui::Grid::new("prefs_graph_hier_grid")
                .num_columns(2)
                .spacing([18.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Separación de filas");
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut g.hierarchical.row_dist, 10.0..=200.0)
                                .fixed_decimals(0),
                        )
                        .changed();
                    ui.end_row();

                    ui.label("Separación de columnas");
                    changed |= ui
                        .add(
                            egui::Slider::new(&mut g.hierarchical.col_dist, 10.0..=200.0)
                                .fixed_decimals(0),
                        )
                        .changed();
                    ui.end_row();

                    ui.label("Orientación");
                    egui::ComboBox::from_id_salt("prefs_graph_orientation")
                        .selected_text(g.hierarchical.orientation.label())
                        .show_ui(ui, |ui| {
                            for o in [HierOrientation::TopDown, HierOrientation::LeftRight] {
                                changed |= ui
                                    .selectable_value(&mut g.hierarchical.orientation, o, o.label())
                                    .changed();
                            }
                        });
                    ui.end_row();
                });
            changed |= ui
                .checkbox(
                    &mut g.hierarchical.center_parent,
                    "Centrar los padres sobre sus hijos",
                )
                .changed();
        });

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
