//! Font setup: tries to use the system UI font (via fontconfig / GNOME
//! settings) and always keeps egui's bundled fonts as a fallback, plus the
//! Lucide icon font. Everything is best-effort — a failure must never stop the
//! application from starting.

use eframe::egui;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

/// Reads the GNOME interface font name (`Family Size`). Returns just the
/// family, dropping the trailing point size.
fn system_family() -> Option<String> {
    let out = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "font-name"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_font_name(&String::from_utf8_lossy(&out.stdout))
}

fn parse_font_name(raw: &str) -> Option<String> {
    let s = raw.trim().trim_matches('\'').trim_matches('"').trim();
    if s.is_empty() {
        return None;
    }
    let mut parts: Vec<&str> = s.split_whitespace().collect();
    if let Some(last) = parts.last() {
        if last.parse::<f32>().is_ok() {
            parts.pop();
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn resolve_font_file(family: &str) -> Option<PathBuf> {
    let fc = which::which("fc-match").ok()?;
    let out = Command::new(fc)
        .args(["-f", "%{file}", family])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if path.is_empty() {
        return None;
    }
    let p = PathBuf::from(path);
    p.is_file().then_some(p)
}

/// Resolves and reads the system UI font. `GTA_MO_NO_SYSTEM_FONT=1` disables
/// it (useful for debugging).
fn system_font() -> Option<Vec<u8>> {
    if std::env::var_os("GTA_MO_NO_SYSTEM_FONT").is_some() {
        return None;
    }
    let family = system_family().unwrap_or_else(|| "sans-serif".to_string());
    let path = resolve_font_file(&family).or_else(|| resolve_font_file("sans-serif"))?;
    std::fs::read(&path).ok()
}

/// Installs the fonts: system font first (when available), egui's defaults as
/// fallback for missing glyphs/emoji, and Lucide last for UI icons.
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    if let Some(bytes) = system_font() {
        let name = "system".to_owned();
        fonts
            .font_data
            .insert(name.clone(), Arc::new(egui::FontData::from_owned(bytes)));
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, name.clone());
        }
    }

    fonts.font_data.insert(
        "lucide".to_owned(),
        Arc::new(egui::FontData::from_static(crate::icons::FONT)),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gnome_font_name() {
        assert_eq!(
            parse_font_name("'SF Pro Display 11'").as_deref(),
            Some("SF Pro Display")
        );
        assert_eq!(
            parse_font_name("'Noto Sans 10.5'").as_deref(),
            Some("Noto Sans")
        );
        assert_eq!(
            parse_font_name("Cantarell 11").as_deref(),
            Some("Cantarell")
        );
        assert_eq!(parse_font_name("''").as_deref(), None);
        assert_eq!(parse_font_name("").as_deref(), None);
    }
}
