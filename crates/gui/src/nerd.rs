//! Nerd Font v3 glyphs used as decorative prefixes in the Log tab.
//!
//! These live in the private-use area; the glyphs render thanks to the Nerd
//! Font registered as a fallback in [`crate::fonts::install`] (any installed
//! Nerd Font found via fontconfig). When no Nerd Font is available the
//! decorations are disabled and the plain text is shown instead.

// This is a glyph palette: not every glyph is used yet.
#![allow(dead_code)]

pub const INFO: &str = "\u{f05a}"; // info-circle
pub const WARN: &str = "\u{f071}"; // exclamation-triangle
pub const ERROR: &str = "\u{f057}"; // times-circle
pub const TIMING: &str = "\u{f017}"; // clock
pub const HEADER: &str = "\u{f120}"; // terminal
pub const SEPARATOR: &str = "\u{f11e}"; // flag-checkered
pub const GTA_MO: &str = "\u{f085}"; // cogs
pub const OUTPUT: &str = "\u{f105}"; // angle-right
pub const PROTON: &str = "\u{f0c3}"; // flask
pub const DXVK: &str = "\u{f108}"; // desktop
pub const UMU: &str = "\u{e73c}"; // python (devicon)
