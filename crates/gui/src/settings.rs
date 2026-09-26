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

/// Default layout of the dependency graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GraphLayout {
    #[default]
    Force,
    Hierarchical,
}

impl GraphLayout {
    pub fn label(self) -> &'static str {
        match self {
            GraphLayout::Force => "Fuerza",
            GraphLayout::Hierarchical => "Jerárquico",
        }
    }
}

/// Orientation of the hierarchical layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HierOrientation {
    #[default]
    TopDown,
    LeftRight,
}

impl HierOrientation {
    pub fn label(self) -> &'static str {
        match self {
            HierOrientation::TopDown => "Arriba → abajo",
            HierOrientation::LeftRight => "Izquierda → derecha",
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

/// Hierarchical layout parameters (advanced).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchicalPrefs {
    #[serde(default = "h_dist")]
    pub row_dist: f32,
    #[serde(default = "h_dist")]
    pub col_dist: f32,
    #[serde(default)]
    pub center_parent: bool,
    #[serde(default)]
    pub orientation: HierOrientation,
}

impl Default for HierarchicalPrefs {
    fn default() -> Self {
        Self {
            row_dist: h_dist(),
            col_dist: h_dist(),
            center_parent: false,
            orientation: HierOrientation::default(),
        }
    }
}

fn h_dist() -> f32 {
    50.0
}

/// Dependency graph preferences (persisted under `[graph]` in `gui.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphSettings {
    #[serde(default)]
    pub layout: GraphLayout,
    #[serde(default = "default_true")]
    pub show_optional: bool,
    #[serde(default = "default_true")]
    pub show_conflicts: bool,
    #[serde(default = "default_true")]
    pub show_variants: bool,
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
    #[serde(default = "default_true")]
    pub auto_settle: bool,
    #[serde(default)]
    pub force: ForcePrefs,
    #[serde(default)]
    pub hierarchical: HierarchicalPrefs,
}

impl Default for GraphSettings {
    fn default() -> Self {
        Self {
            layout: GraphLayout::default(),
            show_optional: true,
            show_conflicts: true,
            show_variants: true,
            labels_always: true,
            dragging: true,
            node_selection: true,
            zoom_speed: default_zoom_speed(),
            fit_padding: default_fit_padding(),
            auto_settle: true,
            force: ForcePrefs::default(),
            hierarchical: HierarchicalPrefs::default(),
        }
    }
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
    #[serde(default)]
    pub graph: GraphSettings,
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
            graph: GraphSettings::default(),
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

    #[test]
    fn graph_settings_defaults_and_roundtrip() {
        // A file without a [graph] table gets the defaults.
        let s: GuiSettings = toml::from_str("ui_scale = 1.0\n").unwrap();
        assert_eq!(s.graph.layout, GraphLayout::Force);
        assert!(s.graph.show_optional && s.graph.show_conflicts && s.graph.show_variants);
        assert!(s.graph.auto_settle);
        assert_eq!(s.graph.force.k_scale, 0.6);
        assert_eq!(s.graph.hierarchical.orientation, HierOrientation::TopDown);

        let s = GuiSettings {
            graph: GraphSettings {
                layout: GraphLayout::Hierarchical,
                show_conflicts: false,
                zoom_speed: 0.25,
                force: ForcePrefs {
                    gravity: 1.2,
                    ..Default::default()
                },
                hierarchical: HierarchicalPrefs {
                    center_parent: true,
                    orientation: HierOrientation::LeftRight,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let text = toml::to_string(&s).unwrap();
        assert!(text.contains("[graph]"));
        let back: GuiSettings = toml::from_str(&text).unwrap();
        assert_eq!(back.graph.layout, GraphLayout::Hierarchical);
        assert!(!back.graph.show_conflicts);
        assert_eq!(back.graph.zoom_speed, 0.25);
        assert_eq!(back.graph.force.gravity, 1.2);
        assert_eq!(
            back.graph.hierarchical.orientation,
            HierOrientation::LeftRight
        );
    }
}
