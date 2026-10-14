use crate::file_browser::BrowserPrefs;
use crate::theme::{Accent, ThemePref};
use gta_mo_core::config::config_dir_path;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

/// Force-directed layout parameters (advanced).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForcePrefs {
    #[serde(default = "f_dt")]
    pub dt: f32,
    #[serde(default = "f_epsilon")]
    pub epsilon: f32,
    #[serde(default = "f_damping")]
    pub damping: f32,
    #[serde(default = "f_max_step")]
    pub max_step: f32,
    #[serde(default = "f_k_scale")]
    pub k_scale: f32,
    #[serde(default = "f_c_attract")]
    pub c_attract: f32,
    #[serde(default = "f_c_repulse")]
    pub c_repulse: f32,
    /// Center gravity (attraction towards the viewport center).
    #[serde(default = "f_gravity")]
    pub gravity: f32,
}

impl Default for ForcePrefs {
    fn default() -> Self {
        Self {
            dt: f_dt(),
            epsilon: f_epsilon(),
            damping: f_damping(),
            max_step: f_max_step(),
            k_scale: f_k_scale(),
            c_attract: f_c_attract(),
            c_repulse: f_c_repulse(),
            gravity: f_gravity(),
        }
    }
}

fn f_dt() -> f32 {
    0.05
}
fn f_epsilon() -> f32 {
    1e-3
}
fn f_damping() -> f32 {
    0.3
}
fn f_max_step() -> f32 {
    10.0
}
fn f_k_scale() -> f32 {
    0.6
}
fn f_c_attract() -> f32 {
    1.0
}
fn f_c_repulse() -> f32 {
    1.0
}
fn f_gravity() -> f32 {
    0.8
}

/// Dependency graph preferences (persisted under `[graph]` in `gui.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSettings {
    #[serde(default = "default_true")]
    pub show_optional: bool,
    #[serde(default = "default_true")]
    pub show_conflicts: bool,
    #[serde(default = "default_true")]
    pub show_families: bool,
    #[serde(default = "default_true")]
    pub labels_always: bool,
    #[serde(default = "default_true")]
    pub dragging: bool,
    #[serde(default = "default_true")]
    pub node_selection: bool,
    #[serde(default = "default_zoom_speed")]
    pub zoom_speed: f32,
    #[serde(default = "default_fit_padding")]
    pub fit_padding: f32,
    /// Edge stroke width in screen pixels (kept constant across zoom levels).
    #[serde(default = "default_edge_width")]
    pub edge_width: f32,
    /// Arrow tip size in screen pixels (kept constant across zoom levels).
    #[serde(default = "default_arrow_size")]
    pub arrow_size: f32,
    #[serde(default = "default_true")]
    pub show_arrows: bool,
    #[serde(default = "default_node_radius")]
    pub node_radius: f32,
    /// Label font size in screen pixels.
    #[serde(default = "default_label_font")]
    pub label_font: f32,
    #[serde(default = "default_true")]
    pub label_bg: bool,
    #[serde(default = "default_true")]
    pub highlight_selected: bool,
    /// Show only enabled mods in the graph.
    #[serde(default)]
    pub only_enabled: bool,
    #[serde(default = "default_true")]
    pub auto_settle: bool,
    #[serde(default)]
    pub force: ForcePrefs,
}

impl Default for GraphSettings {
    fn default() -> Self {
        Self {
            show_optional: true,
            show_conflicts: true,
            show_families: true,
            labels_always: true,
            dragging: true,
            node_selection: true,
            zoom_speed: default_zoom_speed(),
            fit_padding: default_fit_padding(),
            edge_width: default_edge_width(),
            arrow_size: default_arrow_size(),
            show_arrows: true,
            node_radius: default_node_radius(),
            label_font: default_label_font(),
            label_bg: true,
            highlight_selected: true,
            only_enabled: false,
            auto_settle: true,
            force: ForcePrefs::default(),
        }
    }
}

fn default_node_radius() -> f32 {
    5.0
}

fn default_label_font() -> f32 {
    12.0
}

fn default_edge_width() -> f32 {
    2.0
}

fn default_arrow_size() -> f32 {
    15.0
}

fn default_zoom_speed() -> f32 {
    0.1
}

fn default_fit_padding() -> f32 {
    0.1
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
    /// Run the game against a tmpfs copy of the Wine prefix (faster startup on
    /// slow disks). `None` inherits from `config.toml` (no flag is passed);
    /// `Some(true/false)` forces `--prefix-ram`/`--no-prefix-ram`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix_ram: Option<bool>,
    /// Prefetch the Proton runtime into the page cache. `None` inherits from
    /// `config.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preload_proton: Option<bool>,
    /// Prefetch umu's steamrt runtime into the page cache. `None` inherits from
    /// `config.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preload_steamrt: Option<bool>,
    /// Warm umu's Python environment during the prewarm. `None` inherits from
    /// `config.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preload_umu: Option<bool>,
    #[serde(default)]
    pub graph: GraphSettings,
    /// Per-browser UI preferences (`[browsers.<id>]`).
    #[serde(default)]
    pub browsers: HashMap<String, BrowserPrefs>,
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
            prefix_ram: None,
            preload_proton: None,
            preload_steamrt: None,
            preload_umu: None,
            graph: GraphSettings::default(),
            browsers: HashMap::new(),
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

    /// Persisted UI prefs of a browser instance (defaults when absent).
    pub fn browser_prefs(&self, id: &str) -> BrowserPrefs {
        self.browsers.get(id).cloned().unwrap_or_default()
    }

    pub fn set_browser_prefs(&mut self, id: &str, prefs: BrowserPrefs) {
        self.browsers.insert(id.to_string(), prefs);
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

    #[test]
    fn optimize_flags_are_tristate_and_backward_compatible() {
        // An old gui.toml with plain bools still loads (Some), and the removed
        // `prefix_ram_proton` key is ignored.
        let s: GuiSettings =
            toml::from_str("prefix_ram = true\nprefix_ram_proton = true\n").unwrap();
        assert_eq!(s.prefix_ram, Some(true));
        assert_eq!(s.preload_proton, None);
        // Inherit (`None`) is not serialized.
        let text = toml::to_string(&GuiSettings::default()).unwrap();
        assert!(!text.contains("prefix_ram"));
        assert!(!text.contains("preload_proton"));
    }

    #[test]
    fn graph_settings_defaults_and_roundtrip() {
        // A file without a [graph] table gets the defaults.
        let s: GuiSettings = toml::from_str("ui_scale = 1.0\n").unwrap();
        assert!(s.graph.show_optional && s.graph.show_conflicts && s.graph.show_families);
        assert!(s.graph.auto_settle);
        assert_eq!(s.graph.force.k_scale, 0.6);
        assert_eq!(s.graph.edge_width, 2.0);
        assert_eq!(s.graph.arrow_size, 15.0);
        assert!(s.graph.show_arrows);
        assert_eq!(s.graph.node_radius, 5.0);
        assert_eq!(s.graph.label_font, 12.0);
        assert!(s.graph.label_bg && s.graph.highlight_selected);
        assert!(!s.graph.only_enabled);

        let s = GuiSettings {
            graph: GraphSettings {
                show_conflicts: false,
                zoom_speed: 0.25,
                force: ForcePrefs {
                    gravity: 1.2,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let text = toml::to_string(&s).unwrap();
        assert!(text.contains("[graph]"));
        let back: GuiSettings = toml::from_str(&text).unwrap();
        assert!(!back.graph.show_conflicts);
        assert_eq!(back.graph.zoom_speed, 0.25);
        assert_eq!(back.graph.force.gravity, 1.2);
    }

    #[test]
    fn browser_prefs_roundtrip() {
        let mut s = GuiSettings::default();
        s.set_browser_prefs(
            "data_browser",
            BrowserPrefs {
                view: "grid".into(),
                sort: "date".into(),
                desc: true,
                thumb_px: 180.0,
            },
        );
        let text = toml::to_string(&s).unwrap();
        let back: GuiSettings = toml::from_str(&text).unwrap();
        let p = back.browser_prefs("data_browser");
        assert_eq!(p.view, "grid");
        assert_eq!(p.sort, "date");
        assert!(p.desc);
        assert_eq!(p.thumb_px, 180.0);
        // Unknown id falls back to defaults.
        assert_eq!(back.browser_prefs("nope"), BrowserPrefs::default());
    }
}
