use crate::theme::{Accent, ThemePref};
use gta_mo_core::config::config_dir_path;
use serde::{Deserialize, Serialize};

/// UI density. Affects spacing and control sizes (GNOME HIG: compact vs. cozy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    Compact,
    #[default]
    Cozy,
}

impl Density {
    pub fn label(self) -> &'static str {
        match self {
            Density::Compact => "Compacta",
            Density::Cozy => "Cómoda",
        }
    }
}

/// GUI preferences stored in `~/.config/gta-mo/gui.toml`.
///
/// Every field is `#[serde(default)]` so older files keep working.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiSettings {
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,
    #[serde(default)]
    pub theme: ThemePref,
    #[serde(default)]
    pub accent: Accent,
    /// Custom accent color as `#rrggbb`; when present it overrides `accent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_accent: Option<String>,
    #[serde(default)]
    pub high_contrast: bool,
    #[serde(default)]
    pub reduce_motion: bool,
    #[serde(default)]
    pub density: Density,
    #[serde(default = "default_true")]
    pub show_covers: bool,
    #[serde(default = "default_true")]
    pub confirm_deletes: bool,
}

impl Default for GuiSettings {
    fn default() -> Self {
        Self {
            ui_scale: default_ui_scale(),
            theme: ThemePref::default(),
            accent: Accent::default(),
            custom_accent: None,
            high_contrast: false,
            reduce_motion: false,
            density: Density::default(),
            show_covers: true,
            confirm_deletes: true,
        }
    }
}

fn default_ui_scale() -> f32 {
    1.2
}

fn default_true() -> bool {
    true
}

impl GuiSettings {
    fn path() -> Option<std::path::PathBuf> {
        config_dir_path().map(|d| d.join("gui.toml"))
    }

    /// Maps the persisted appearance settings to the theme config.
    pub fn theme_config(&self) -> crate::theme::ThemeConfig {
        crate::theme::ThemeConfig {
            accent: self.accent,
            custom: self
                .custom_accent
                .as_deref()
                .and_then(crate::theme::parse_hex),
            high_contrast: self.high_contrast,
        }
    }

    /// Loads the settings, creating `gui.toml` with defaults on first run.
    /// Any missing/invalid file falls back to sane defaults.
    pub fn load() -> Self {
        let Some(path) = Self::path() else {
            return Self::default();
        };
        let parsed = std::fs::read_to_string(&path)
            .ok()
            .and_then(|content| toml::from_str::<GuiSettings>(&content).ok());
        let mut settings = parsed.unwrap_or_else(|| {
            let defaults = Self::default();
            defaults.save();
            defaults
        });
        // Sanity clamp so a typo cannot make the UI unusable.
        settings.ui_scale = settings.ui_scale.clamp(0.7, 2.5);
        settings
    }

    /// Persists the settings, best-effort (a read-only home must not crash the
    /// app). Called when a preference changes.
    pub fn save(&self) {
        let Some(path) = Self::path() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = toml::to_string(self) {
            let _ = std::fs::write(&path, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_only_file_still_parses() {
        // An old gui.toml with only ui_scale must keep working.
        let s: GuiSettings = toml::from_str("ui_scale = 1.0\n").unwrap();
        assert_eq!(s.ui_scale, 1.0);
        assert_eq!(s.theme, ThemePref::System);
        assert_eq!(s.accent, Accent::Azure);
        assert!(s.show_covers);
        assert!(s.custom_accent.is_none());
    }

    #[test]
    fn roundtrips() {
        let s = GuiSettings {
            theme: ThemePref::Dark,
            accent: Accent::Purple,
            custom_accent: Some("#a550a7".into()),
            reduce_motion: true,
            density: Density::Compact,
            ..Default::default()
        };
        let text = toml::to_string(&s).unwrap();
        let back: GuiSettings = toml::from_str(&text).unwrap();
        assert_eq!(back.theme, ThemePref::Dark);
        assert_eq!(back.accent, Accent::Purple);
        assert_eq!(back.custom_accent.as_deref(), Some("#a550a7"));
        assert!(back.reduce_motion);
        assert_eq!(back.density, Density::Compact);
    }

    #[test]
    fn legacy_accent_names_still_parse() {
        // Old gui.toml values map to the nearest new accent without resetting
        // the rest of the settings.
        let s: GuiSettings = toml::from_str("accent = \"violet\"\nui_scale = 1.3\n").unwrap();
        assert_eq!(s.accent, Accent::Purple);
        assert_eq!(s.ui_scale, 1.3);
        let s: GuiSettings = toml::from_str("accent = \"graphite\"\n").unwrap();
        assert_eq!(s.accent, Accent::Gray);
    }
}
