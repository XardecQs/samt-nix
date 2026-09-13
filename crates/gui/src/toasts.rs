//! Transient toast notifications (success/error/info), shown bottom-right and
//! fading out automatically.

use crate::theme;
use eframe::egui;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Error,
    Info,
}

struct Toast {
    text: String,
    kind: ToastKind,
    born: f64,
}

#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
}

const LIFETIME: f64 = 3.5;
const MAX: usize = 4;

impl Toasts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, ctx: &egui::Context, kind: ToastKind, text: impl Into<String>) {
        let text = text.into();
        if text.trim().is_empty() {
            return;
        }
        self.items.push(Toast {
            text,
            kind,
            born: ctx.input(|i| i.time),
        });
        if self.items.len() > MAX {
            self.items.remove(0);
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        self.items.retain(|t| now - t.born < LIFETIME);
        if self.items.is_empty() {
            return;
        }
        // Keep animating/fading while visible.
        ctx.request_repaint_after(std::time::Duration::from_millis(80));

        let palette = theme::active(ctx);
        egui::Area::new(egui::Id::new("gta_mo_toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -40.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                for t in &self.items {
                    let age = (now - t.born) as f32;
                    let alpha = fade(age, LIFETIME as f32);
                    let accent = match t.kind {
                        ToastKind::Success => palette.success,
                        ToastKind::Error => palette.danger,
                        ToastKind::Info => palette.accent,
                    }
                    .gamma_multiply(alpha);
                    let icon = match t.kind {
                        ToastKind::Success => crate::icons::CHECK,
                        ToastKind::Error => crate::icons::X,
                        ToastKind::Info => crate::icons::INFO,
                    };
                    egui::Frame::new()
                        .fill(palette.surface_raised.gamma_multiply(alpha))
                        .stroke(egui::Stroke::new(1.0, accent))
                        .corner_radius(egui::CornerRadius::same(8))
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(icon).color(accent).size(14.0));
                                ui.label(
                                    egui::RichText::new(&t.text)
                                        .color(palette.text.gamma_multiply(alpha)),
                                );
                            });
                        });
                    ui.add_space(4.0);
                }
            });
    }
}

/// Fades in over the first 15% and out over the last 30% of the lifetime.
fn fade(age: f32, lifetime: f32) -> f32 {
    let fade_in = (age / (lifetime * 0.15)).clamp(0.0, 1.0);
    let fade_out = ((lifetime - age) / (lifetime * 0.30)).clamp(0.0, 1.0);
    fade_in.min(fade_out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_in_out_bounds() {
        assert_eq!(fade(0.0, 4.0), 0.0);
        assert!(fade(2.0, 4.0) > 0.9, "a mitad de vida debe ser casi opaco");
        assert_eq!(fade(4.0, 4.0), 0.0);
    }
}
