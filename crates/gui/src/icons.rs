//! Lucide icon glyphs (subset of the official Lucide font).
//!
//! Codepoints come from the official `lucide-static` `codepoints.json`
//! (<https://lucide.dev>). The embedded font is a subset of the official
//! Lucide font, licensed under ISC — see `assets/lucide/LICENSE`.

use eframe::egui;

pub const CHECK: &str = "\u{E06C}";
pub const X: &str = "\u{E1B2}";
pub const MENU: &str = "\u{E115}";
pub const FOLDER_OPEN: &str = "\u{E247}";
pub const PLUS: &str = "\u{E13D}";
pub const REFRESH: &str = "\u{E145}";
pub const TRASH: &str = "\u{E18E}";
pub const PENCIL: &str = "\u{E1F9}";
pub const SEARCH: &str = "\u{E151}";
pub const WARN: &str = "\u{E193}";
pub const TAG: &str = "\u{E17F}";
pub const EXTERNAL_LINK: &str = "\u{E0B9}";
pub const LINK: &str = "\u{E102}";
pub const USERS: &str = "\u{E46E}";
pub const PLAY: &str = "\u{E13C}";
pub const INFO: &str = "\u{E0F9}";
pub const IMAGE: &str = "\u{E0F6}";
pub const PANEL_RIGHT: &str = "\u{E431}";
pub const CIRCLE_CHECK: &str = "\u{E226}";
pub const ERASER: &str = "\u{E28F}";
pub const SAVE: &str = "\u{E14D}";
pub const LIST: &str = "\u{E106}";
pub const FOLDER: &str = "\u{E0D7}";
pub const SWAP_H: &str = "\u{E24A}";

/// Subset of the official Lucide font, embedded at compile time.
pub const FONT: &[u8] = include_bytes!("../assets/lucide/lucide.ttf");

/// Installs the Lucide icon font as a fallback for the default families, so
/// the [`CHECK`], [`X`], … glyphs render inline with regular text.
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "lucide".to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(FONT)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("lucide".to_owned());
    }
    ctx.set_fonts(fonts);
}
