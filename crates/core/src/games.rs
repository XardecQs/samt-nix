//! Game-specific defaults, kept apart from the engine.
//!
//! Right now only GTA San Andreas is built in and the app is not multi-game,
//! but every GTA-SA-specific constant (UMU game id, executable name, layer
//! conventions, user-data folder) lives here instead of being hardcoded across
//! the launcher, the conflict scanner and the config defaults.
//!
//! Adding a game later is a matter of adding another [`GameSpec`] and selecting
//! it from `config.toml` with `game = "<key>"`.

/// Description of one supported game and its conventions.
///
/// All fields are `'static`, so a spec is cheap to copy by reference.
#[derive(Debug, Clone, Copy)]
pub struct GameSpec {
    /// Stable key used in `config.toml` (`game = "<key>"`).
    pub key: &'static str,
    /// Human-readable name.
    pub name: &'static str,
    /// UMU/Proton game id (`GAMEID`).
    pub umu_game_id: &'static str,
    /// Default game executable.
    pub exe: &'static str,
    /// Default `DXVK_HUD` value used with `--debug`.
    pub dxvk_hud: &'static str,
    /// Extra executable names treated as high-severity conflicts.
    pub exe_names: &'static [&'static str],
    /// Path prefix of the mod-loader tree (lower-severity conflicts).
    pub loader_prefix: &'static str,
    /// Per-profile user-data folder written inside the game dir (PortableGTA
    /// redirects saves/settings/screenshots there), relative to the game root.
    pub user_data_dir: Option<&'static str>,
}

impl GameSpec {
    /// True when a game-relative path is a game executable (high severity).
    pub fn is_executable(&self, path: &str) -> bool {
        let p = path.to_lowercase();
        p.ends_with(".exe") || self.exe_names.iter().any(|n| p == *n)
    }

    /// True when a game-relative path lives under the mod-loader tree.
    pub fn is_loader_path(&self, path: &str) -> bool {
        path.to_lowercase().starts_with(self.loader_prefix)
    }
}

/// The built-in game specs (only GTA SA for now).
pub const ALL: &[&GameSpec] = &[&GTA_SA];

/// GTA San Andreas.
pub const GTA_SA: GameSpec = GameSpec {
    key: "gta_sa",
    name: "GTA San Andreas",
    umu_game_id: "umu-gtasa",
    exe: "gta_sa.exe",
    dxvk_hud:
        "devinfo,fps,frametimes,submissions,compiler,version,api,pipelines,memory,gpuload,drawcalls",
    exe_names: &["gta_sa.exe", "gta_sa.pdb"],
    loader_prefix: "modloader/",
    // PortableGTA redirects the user files to `<game root>/userfiles`.
    user_data_dir: Some("userfiles"),
};

/// The default game, used when the config does not select one.
pub fn default_spec() -> &'static GameSpec {
    &GTA_SA
}

/// Looks up a game spec by its `key`.
pub fn spec_for(key: &str) -> Option<&'static GameSpec> {
    ALL.iter().copied().find(|s| s.key == key)
}

/// Resolves a configured game key to a spec, falling back to the default when
/// it is missing or unknown.
pub fn resolve(key: Option<&str>) -> &'static GameSpec {
    key.and_then(spec_for).unwrap_or_else(default_spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_known_and_falls_back() {
        assert_eq!(resolve(Some("gta_sa")).key, "gta_sa");
        assert_eq!(resolve(Some("nope")).key, "gta_sa");
        assert_eq!(resolve(None).key, "gta_sa");
    }

    #[test]
    fn executable_and_loader_predicates() {
        let g = default_spec();
        assert!(g.is_executable("gta_sa.exe"));
        assert!(g.is_executable("GTA_SA.PDB"));
        assert!(g.is_executable("models/other.EXE"));
        assert!(!g.is_executable("data/handling.cfg"));
        assert!(g.is_loader_path("modloader/foo.asi"));
        assert!(g.is_loader_path("ModLoader/bar"));
        assert!(!g.is_loader_path("models/x.dff"));
    }
}
