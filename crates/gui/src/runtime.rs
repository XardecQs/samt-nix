//! "Runtime de umu" dialog: shows the steamrt install used by `umu-run` and
//! lets the user clean the download cache or force a reinstall.

use eframe::egui;

/// Action requested from the dialog; the caller performs it (so it can show a
/// toast and reload the status).
pub enum RuntimeAction {
    Clean,
    Reinstall,
}

pub fn show_runtime(ctx: &egui::Context, open: &mut bool) -> Option<RuntimeAction> {
    let st = gta_mo_core::umu::status();
    let mut close = false;
    let mut action = None;

    crate::app::overlay_page(ctx, "gta_mo_runtime", 560.0, |ui| {
        if crate::app::overlay_header(ui, "Runtime de umu", "Cerrar") {
            close = true;
        }

        egui::Grid::new("runtime_info")
            .num_columns(2)
            .spacing([14.0, 6.0])
            .show(ui, |ui| {
                ui.label("Estado");
                if st.installed {
                    ui.label(
                        egui::RichText::new("instalado").color(crate::theme::active(ctx).success),
                    );
                } else {
                    ui.label(egui::RichText::new("no instalado").weak());
                }
                ui.end_row();

                ui.label("Directorio");
                ui.label(st.runtime_dir.display().to_string());
                ui.end_row();

                ui.label("Versión");
                ui.label(
                    st.version
                        .clone()
                        .unwrap_or_else(|| "(desconocida)".to_string()),
                );
                ui.end_row();

                ui.label("Caché de descargas");
                ui.label(format!(
                    "{} ({} archivo(s), {})",
                    st.cache_dir.display(),
                    st.cache_files,
                    human_bytes(st.cache_bytes)
                ));
                ui.end_row();
            });

        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(
                "gta-mo desactiva la actualización automática del runtime de umu \
                 (umu_runtime_update = false) para no reintentar la descarga en cada \
                 lanzamiento. Actívala en la configuración o fuerza una reinstalación \
                 desde aquí.",
            )
            .small()
            .weak(),
        );

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui
                .button(format!("{} Limpiar caché", crate::icons::ERASER))
                .on_hover_text("Borra las descargas parciales de umu")
                .clicked()
            {
                action = Some(RuntimeAction::Clean);
            }
            if ui
                .button(format!("{} Reinstalar runtime", crate::icons::REFRESH))
                .on_hover_text("Borra el runtime instalado; umu lo reinstalará al lanzar")
                .clicked()
            {
                action = Some(RuntimeAction::Reinstall);
            }
        });
    });

    if close {
        *open = false;
    }
    action
}

/// Formats a byte count in a human-friendly way.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
