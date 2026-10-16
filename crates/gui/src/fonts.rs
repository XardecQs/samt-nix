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

/// egui family name used for the Log tab when a custom font is chosen.
pub const LOG_FAMILY: &str = "gta-mo-log";

/// Installed font families, computed once (spawning `fc-list` every frame would
/// be wasteful since the Preferences window is redrawn continuously).
pub fn families_cached() -> &'static [String] {
    static FAMILIES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    FAMILIES.get_or_init(list_families)
}

/// Lists installed font families (via `fc-list`), deduped and sorted.
/// Best-effort: returns an empty list when fontconfig is unavailable.
pub fn list_families() -> Vec<String> {
    let Ok(fc) = which::which("fc-list") else {
        return Vec::new();
    };
    let out = match Command::new(fc).args([":", "family"]).output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let mut families: Vec<String> = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        // `fc-list : family` prints comma-separated aliases per line.
        for fam in line.split(',') {
            let fam = fam.trim();
            if !fam.is_empty() && !families.iter().any(|f| f == fam) {
                families.push(fam.to_string());
            }
        }
    }
    families.sort_by_key(|f| f.to_lowercase());
    families
}

/// Installs the fonts: system font first (when available), egui's defaults as
/// fallback for missing glyphs/emoji, and Lucide last for UI icons. When
/// `log_family` is set, its file is registered under [`LOG_FAMILY`] for the Log
/// tab (falling back to the monospace chain for missing glyphs).
pub fn install(ctx: &egui::Context, log_family: Option<&str>) {
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

    if let Some(family) = log_family {
        if let Some(bytes) = resolve_font_file(family).and_then(|p| std::fs::read(p).ok()) {
            fonts.font_data.insert(
                "log".to_owned(),
                Arc::new(egui::FontData::from_owned(bytes)),
            );
            let mut chain = vec!["log".to_owned()];
            if let Some(mono) = fonts.families.get(&egui::FontFamily::Monospace) {
                chain.extend(mono.iter().cloned());
            }
            fonts
                .families
                .insert(egui::FontFamily::Name(LOG_FAMILY.into()), chain);
        }
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
