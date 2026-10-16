//! Small reusable form widgets shared by the in-app editors (`.desktop` now,
//! the structured `mod.toml` editor later). Each returns `true` when the value
//! changed.

// Widget kit: not every helper is used yet.
#![allow(dead_code)]

use eframe::egui;

/// Label + single-line text field.
pub fn text(ui: &mut egui::Ui, label: &str, value: &mut String) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        changed = ui
            .add(egui::TextEdit::singleline(value).desired_width(f32::INFINITY))
            .changed();
    });
    changed
}

/// Label + multiline text area.
pub fn multiline(ui: &mut egui::Ui, label: &str, value: &mut String, rows: usize) -> bool {
    ui.label(label);
    ui.add(
        egui::TextEdit::multiline(value)
            .desired_width(f32::INFINITY)
            .desired_rows(rows),
    )
    .changed()
}

/// Checkbox with a label.
pub fn check(ui: &mut egui::Ui, label: &str, value: &mut bool) -> bool {
    ui.checkbox(value, label).changed()
}

/// Editable list of strings (one field per entry + add/remove).
pub fn list(ui: &mut egui::Ui, label: &str, values: &mut Vec<String>, hint: &str) -> bool {
    let mut changed = false;
    ui.label(label);
    let mut remove: Option<usize> = None;
    for (i, v) in values.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::TextEdit::singleline(v)
                        .hint_text(hint)
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                changed = true;
            }
            if ui.button("✕").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        values.remove(i);
        changed = true;
    }
    if ui.button("+ Añadir").clicked() {
        values.push(String::new());
        changed = true;
    }
    changed
}
