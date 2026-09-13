//! Design tokens and theming for the GUI.
//!
//! All UI colors come from [`Palette`] (semantic roles), never from raw RGB
//! literals, so light/dark mode and accessibility contrast are handled in one
//! place. Two palettes are installed per egui theme (`Light`/`Dark`) with
//! [`egui::Context::set_visuals_of`], and [`active`] returns the palette that
//! matches the currently resolved theme.

use eframe::egui;
use egui::{Color32, Id, Stroke};
use serde::{Deserialize, Serialize};

const CFG_ID: &str = "gta_mo_theme_config";

/// Theme preference, persisted in `gui.toml`. `System` follows the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePref {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePref {
    pub fn label(self) -> &'static str {
        match self {
            ThemePref::System => "Sistema",
            ThemePref::Light => "Claro",
            ThemePref::Dark => "Oscuro",
        }
    }

    pub fn to_egui(self) -> egui::ThemePreference {
        match self {
            ThemePref::System => egui::ThemePreference::System,
            ThemePref::Light => egui::ThemePreference::Light,
            ThemePref::Dark => egui::ThemePreference::Dark,
        }
    }
}

/// Accent color, inspired by GNOME 47 and macOS system colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accent {
    #[default]
    Blue,
    Violet,
    Green,
    Orange,
    Pink,
    Graphite,
}

impl Accent {
    pub fn label(self) -> &'static str {
        match self {
            Accent::Blue => "Azul",
            Accent::Violet => "Violeta",
            Accent::Green => "Verde",
            Accent::Orange => "Naranja",
            Accent::Pink => "Rosa",
            Accent::Graphite => "Grafito",
        }
    }

    pub const ALL: [Accent; 6] = [
        Accent::Blue,
        Accent::Violet,
        Accent::Green,
        Accent::Orange,
        Accent::Pink,
        Accent::Graphite,
    ];

    /// Base accent for a theme: brighter on dark, deeper on light, so the
    /// accent keeps a usable contrast against both backgrounds.
    fn base(self, dark: bool) -> Color32 {
        let (d, l) = match self {
            Accent::Blue => ((88, 162, 255), (21, 101, 192)),
            Accent::Violet => ((167, 139, 250), (109, 40, 217)),
            Accent::Green => ((74, 222, 128), (21, 128, 61)),
            Accent::Orange => ((251, 176, 64), (194, 100, 12)),
            Accent::Pink => ((244, 114, 182), (190, 24, 93)),
            Accent::Graphite => ((176, 176, 186), (71, 71, 82)),
        };
        let c = if dark { d } else { l };
        Color32::from_rgb(c.0, c.1, c.2)
    }

    /// Representative color for a settings swatch.
    pub fn swatch(self) -> Color32 {
        self.base(false)
    }
}

/// User-adjustable theme options (accent + high contrast).
#[derive(Debug, Clone, Copy, Default)]
pub struct ThemeConfig {
    pub accent: Accent,
    pub high_contrast: bool,
}

/// Semantic color roles. Every UI color is expressed through one of these.
///
/// This is an intentionally complete design-token set: some roles are reserved
/// for states that are not drawn yet, so unused fields are allowed here.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Palette {
    pub dark_mode: bool,
    pub bg: Color32,
    pub surface: Color32,
    pub surface_raised: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub text_disabled: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub accent_hover: Color32,
    pub accent_pressed: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub info: Color32,
    pub selection: Color32,
    pub focus_ring: Color32,
    /// Translucent fill for the active/enabled row.
    pub row_active_fill: Color32,
}

impl Palette {
    pub fn build(theme: egui::Theme, cfg: ThemeConfig) -> Self {
        let dark = theme == egui::Theme::Dark;
        let hc = cfg.high_contrast;
        let accent = cfg.accent.base(dark);
        let on_accent = if contrast_ratio(accent, Color32::WHITE) >= 4.5 {
            Color32::WHITE
        } else {
            Color32::from_rgb(12, 12, 14)
        };

        if dark {
            Palette {
                dark_mode: true,
                bg: Color32::from_rgb(20, 20, 24),
                surface: Color32::from_rgb(29, 29, 34),
                surface_raised: Color32::from_rgb(38, 38, 45),
                border: Color32::from_rgb(
                    if hc { 96 } else { 58 },
                    if hc { 96 } else { 58 },
                    if hc { 108 } else { 68 },
                ),
                text: Color32::from_rgb(235, 235, 240),
                text_muted: if hc {
                    Color32::from_rgb(205, 205, 212)
                } else {
                    Color32::from_rgb(170, 170, 180)
                },
                text_disabled: Color32::from_rgb(120, 120, 130),
                accent,
                on_accent,
                accent_hover: lighten(accent, 0.10),
                accent_pressed: darken(accent, 0.12),
                success: Color32::from_rgb(112, 204, 132),
                warning: Color32::from_rgb(236, 177, 96),
                danger: Color32::from_rgb(242, 128, 128),
                info: accent,
                selection: accent,
                focus_ring: lighten(accent, 0.25),
                row_active_fill: Color32::from_rgba_unmultiplied(
                    accent.r(),
                    accent.g(),
                    accent.b(),
                    26,
                ),
            }
        } else {
            Palette {
                dark_mode: false,
                bg: Color32::from_rgb(244, 244, 248),
                surface: Color32::from_rgb(255, 255, 255),
                surface_raised: Color32::from_rgb(255, 255, 255),
                border: Color32::from_rgb(
                    if hc { 140 } else { 212 },
                    if hc { 140 } else { 212 },
                    if hc { 150 } else { 222 },
                ),
                text: Color32::from_rgb(26, 26, 30),
                text_muted: if hc {
                    Color32::from_rgb(60, 60, 68)
                } else {
                    Color32::from_rgb(96, 96, 106)
                },
                text_disabled: Color32::from_rgb(150, 150, 158),
                accent,
                on_accent,
                accent_hover: darken(accent, 0.08),
                accent_pressed: darken(accent, 0.16),
                success: Color32::from_rgb(21, 128, 61),
                warning: Color32::from_rgb(154, 92, 8),
                danger: Color32::from_rgb(185, 28, 28),
                info: accent,
                selection: accent,
                focus_ring: darken(accent, 0.10),
                row_active_fill: Color32::from_rgba_unmultiplied(
                    accent.r(),
                    accent.g(),
                    accent.b(),
                    30,
                ),
            }
        }
    }

    /// egui [`egui::Visuals`] derived from this palette.
    pub fn visuals(&self) -> egui::Visuals {
        let mut v = if self.dark_mode {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        let radius = egui::CornerRadius::same(6);
        let small_radius = egui::CornerRadius::same(4);

        v.override_text_color = Some(self.text);
        v.panel_fill = self.surface;
        v.window_fill = self.surface_raised;
        v.extreme_bg_color = if self.dark_mode {
            Color32::from_rgb(16, 16, 20)
        } else {
            Color32::from_rgb(236, 236, 241)
        };
        v.code_bg_color = if self.dark_mode {
            Color32::from_rgb(42, 42, 50)
        } else {
            Color32::from_rgb(240, 240, 245)
        };
        v.faint_bg_color = if self.dark_mode {
            Color32::from_rgb(34, 34, 40)
        } else {
            Color32::from_rgb(238, 238, 243)
        };
        v.hyperlink_color = self.accent;
        v.warn_fg_color = self.warning;
        v.error_fg_color = self.danger;
        v.window_stroke = Stroke::new(1.0, self.border);
        v.window_corner_radius = egui::CornerRadius::same(10);
        v.menu_corner_radius = egui::CornerRadius::same(8);
        v.window_shadow = subtle_shadow(self.dark_mode);
        v.popup_shadow = subtle_shadow(self.dark_mode);
        v.striped = true;
        v.button_frame = true;

        v.selection.bg_fill = self.selection;
        v.selection.stroke = Stroke::new(1.0, self.on_accent);

        let w = &mut v.widgets;
        w.noninteractive.bg_fill = self.surface_raised;
        w.noninteractive.weak_bg_fill = self.surface;
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        w.noninteractive.corner_radius = radius;

        w.inactive.bg_fill = self.surface_raised;
        w.inactive.weak_bg_fill = if self.dark_mode {
            Color32::from_rgb(46, 46, 54)
        } else {
            Color32::from_rgb(232, 232, 238)
        };
        w.inactive.bg_stroke = Stroke::new(1.0, self.border);
        w.inactive.fg_stroke = Stroke::new(1.0, self.text);
        w.inactive.corner_radius = small_radius;

        w.hovered.bg_fill = lighten(
            w.inactive.bg_fill,
            if self.dark_mode { 0.10 } else { -0.05 },
        );
        w.hovered.weak_bg_fill = lighten(
            w.inactive.weak_bg_fill,
            if self.dark_mode { 0.10 } else { -0.05 },
        );
        w.hovered.bg_stroke = Stroke::new(1.0, self.focus_ring);
        w.hovered.fg_stroke = Stroke::new(1.0, self.text);
        w.hovered.corner_radius = small_radius;
        w.hovered.expansion = 1.0;

        w.active.bg_fill = self.accent_pressed;
        w.active.weak_bg_fill = self.accent_pressed;
        w.active.bg_stroke = Stroke::new(1.0, self.accent_pressed);
        w.active.fg_stroke = Stroke::new(1.0, self.on_accent);
        w.active.corner_radius = small_radius;

        w.open.bg_fill = w.inactive.bg_fill;
        w.open.weak_bg_fill = w.hovered.weak_bg_fill;
        w.open.bg_stroke = Stroke::new(1.0, self.border);
        w.open.fg_stroke = Stroke::new(1.0, self.text);

        v
    }
}

fn subtle_shadow(dark: bool) -> egui::epaint::Shadow {
    egui::epaint::Shadow {
        offset: [0, 2],
        blur: 8,
        spread: 0,
        color: if dark {
            Color32::from_black_alpha(120)
        } else {
            Color32::from_black_alpha(28)
        },
    }
}

/// Installs the light and dark visuals and remembers the config, so
/// [`active`] can rebuild the palette from the resolved theme each frame.
pub fn apply(ctx: &egui::Context, cfg: ThemeConfig) {
    ctx.set_visuals_of(
        egui::Theme::Light,
        Palette::build(egui::Theme::Light, cfg).visuals(),
    );
    ctx.set_visuals_of(
        egui::Theme::Dark,
        Palette::build(egui::Theme::Dark, cfg).visuals(),
    );
    ctx.data_mut(|d| d.insert_temp(Id::new(CFG_ID), cfg));
}

/// The palette matching the theme egui is currently rendering with.
pub fn active(ctx: &egui::Context) -> Palette {
    let cfg = ctx
        .data(|d| d.get_temp::<ThemeConfig>(Id::new(CFG_ID)))
        .unwrap_or_default();
    Palette::build(ctx.theme(), cfg)
}

// ---- color math (WCAG 2.1) ----

fn srgb_to_linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance of an opaque color.
pub fn relative_luminance(c: Color32) -> f32 {
    0.2126 * srgb_to_linear(c.r()) + 0.7152 * srgb_to_linear(c.g()) + 0.0722 * srgb_to_linear(c.b())
}

/// WCAG contrast ratio between two opaque colors (1.0 .. 21.0).
pub fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Approximates mixing `c` towards white (`amount > 0`) or black.
fn lighten(c: Color32, amount: f32) -> Color32 {
    let mix = |v: u8| -> u8 {
        let v = v as f32;
        (v + (255.0 - v) * amount).clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(mix(c.r()), mix(c.g()), mix(c.b()))
}

fn darken(c: Color32, amount: f32) -> Color32 {
    lighten(c, -amount)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn for_config(cfg: ThemeConfig) -> Vec<Palette> {
        vec![
            Palette::build(egui::Theme::Light, cfg),
            Palette::build(egui::Theme::Dark, cfg),
        ]
    }

    /// Body text must meet WCAG AA (>= 4.5:1) on every surface, in both themes
    /// and for every accent / contrast setting.
    #[test]
    fn text_meets_wcag_aa() {
        for accent in Accent::ALL {
            for hc in [false, true] {
                let cfg = ThemeConfig {
                    accent,
                    high_contrast: hc,
                };
                for p in for_config(cfg) {
                    for (name, fg) in [("text", p.text), ("muted", p.text_muted)] {
                        for (bg_name, bg) in [
                            ("bg", p.bg),
                            ("surface", p.surface),
                            ("raised", p.surface_raised),
                        ] {
                            let r = contrast_ratio(fg, bg);
                            assert!(
                                r >= 4.5,
                                "{accent:?} hc={hc} dark={} {name} on {bg_name}: {r:.2} < 4.5",
                                p.dark_mode
                            );
                        }
                    }
                }
            }
        }
    }

    /// Semantic colors and the accent must be legible on the panel background
    /// (>= 3.0:1, the AA threshold for UI components / large text).
    #[test]
    fn semantic_colors_meet_ui_aa() {
        for accent in Accent::ALL {
            for p in for_config(ThemeConfig {
                accent,
                high_contrast: false,
            }) {
                for (name, fg) in [
                    ("accent", p.accent),
                    ("success", p.success),
                    ("warning", p.warning),
                    ("danger", p.danger),
                ] {
                    for (bg_name, bg) in [("bg", p.bg), ("surface", p.surface)] {
                        let r = contrast_ratio(fg, bg);
                        assert!(
                            r >= 3.0,
                            "{accent:?} dark={} {name} on {bg_name}: {r:.2} < 3.0",
                            p.dark_mode
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn on_accent_contrasts_with_accent() {
        for accent in Accent::ALL {
            for p in for_config(ThemeConfig {
                accent,
                high_contrast: false,
            }) {
                let r = contrast_ratio(p.on_accent, p.accent);
                assert!(
                    r >= 4.0,
                    "{accent:?} dark={} on_accent: {r:.2}",
                    p.dark_mode
                );
            }
        }
    }

    #[test]
    fn apply_and_active_run_in_a_frame() {
        let ctx = egui::Context::default();
        apply(&ctx, ThemeConfig::default());
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let p = active(ctx);
            assert_ne!(p.accent, Color32::TRANSPARENT);
        });
    }
}
