//! Lucide icon glyphs (subset of the official Lucide font).
//!
//! Codepoints come from the official `lucide-static` `codepoints.json`
//! (<https://lucide.dev>) and are kept in sync with
//! `assets/lucide/codepoints.json`. The embedded font is a subset of the
//! official Lucide font, licensed under ISC — see `assets/lucide/LICENSE`.
//!
//! To add an icon: add its name to `assets/lucide/icons.txt`, regenerate the
//! font with `assets/lucide/build.sh`, then add the codepoint here.

// This is an icon palette: not every glyph is used yet.
#![allow(dead_code)]

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
pub const USERS: &str = "\u{E1A4}";
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

// Theme / preferences / navigation.
pub const SETTINGS: &str = "\u{E154}";
pub const SUN: &str = "\u{E178}";
pub const MOON: &str = "\u{E11E}";
pub const MONITOR: &str = "\u{E11D}";
pub const KEYBOARD: &str = "\u{E284}";
pub const ACCESSIBILITY: &str = "\u{E297}";
pub const SLIDERS: &str = "\u{E29A}";
pub const CHEVRON_LEFT: &str = "\u{E06E}";
pub const CHEVRON_RIGHT: &str = "\u{E06F}";
pub const ELLIPSIS_V: &str = "\u{E0B7}";
pub const PALETTE: &str = "\u{E1DD}";

// Action extras.
pub const CIRCLE_PLUS: &str = "\u{E081}";
pub const PLAY_CIRCLE: &str = "\u{E080}";
pub const MAXIMIZE: &str = "\u{E112}";
pub const ZOOM_IN: &str = "\u{E1B6}";
pub const ZOOM_OUT: &str = "\u{E1B7}";
pub const FOLDER_COG: &str = "\u{E331}";
pub const WAND: &str = "\u{E357}";
pub const PLUG: &str = "\u{E37F}";
pub const ROTATE_CW: &str = "\u{E149}";
pub const IMAGE_OFF: &str = "\u{E1C0}";
pub const ARROW_LEFT: &str = "\u{E048}";
pub const PANEL_LEFT: &str = "\u{E12A}";
pub const SQUARE: &str = "\u{E167}";

/// Subset of the official Lucide font, embedded at compile time.
pub const FONT: &[u8] = include_bytes!("../assets/lucide/lucide.ttf");
