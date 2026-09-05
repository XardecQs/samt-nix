use gta_mo_core::config::config_dir_path;
use serde::{Deserialize, Serialize};

/// GUI preferences stored in `~/.config/gta-mo/gui.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiSettings {
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,
}

impl Default for GuiSettings {
    fn default() -> Self {
        Self {
            ui_scale: default_ui_scale(),
        }
    }
}

fn default_ui_scale() -> f32 {
    1.2
}

impl GuiSettings {
    fn path() -> Option<std::path::PathBuf> {
        config_dir_path().map(|d| d.join("gui.toml"))
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
        let settings = parsed.unwrap_or_else(|| {
            let defaults = Self::default();
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(text) = toml::to_string(&defaults) {
                let _ = std::fs::write(&path, text);
            }
            defaults
        });
        // Sanity clamp so a typo cannot make the UI unusable.
        let scale = settings.ui_scale.clamp(0.7, 2.5);
        GuiSettings { ui_scale: scale }
    }
}
