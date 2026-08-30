use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Deserialize, Serialize, Clone)]
pub struct Config {
    pub game_root: String,
    pub proton_path: String,
    pub game_id: Option<String>,
    pub game_exe: Option<String>,
    pub proton_use_wined3d: Option<bool>,
    pub proton_disable_ntsync: Option<bool>,
    pub proton_disable_upscalers: Option<bool>,
    pub dxvk_hud: Option<String>,
    pub auto_discover: Option<bool>,
    pub mods_dir: Option<String>,
    pub default_profile: Option<String>,
}

impl Config {
    pub fn game_id(&self) -> &str {
        self.game_id.as_deref().unwrap_or("umu-gtasa")
    }

    pub fn game_exe(&self) -> &str {
        self.game_exe.as_deref().unwrap_or("gta_sa.exe")
    }

    pub fn proton_use_wined3d(&self) -> bool {
        self.proton_use_wined3d.unwrap_or(true)
    }

    pub fn proton_disable_ntsync(&self) -> bool {
        self.proton_disable_ntsync.unwrap_or(false)
    }

    pub fn proton_disable_upscalers(&self) -> bool {
        self.proton_disable_upscalers.unwrap_or(false)
    }

    pub fn auto_discover(&self) -> bool {
        self.auto_discover.unwrap_or(false)
    }

    pub fn dxvk_hud(&self) -> &str {
        self.dxvk_hud
            .as_deref()
            .unwrap_or("devinfo,fps,frametimes,submissions,compiler,version,api,pipelines,memory,gpuload,drawcalls")
    }

    pub fn default_profile(&self) -> Option<&str> {
        self.default_profile.as_deref()
    }
}

pub struct RuntimePaths {
    pub base_game: PathBuf,
    pub mods_dir: PathBuf,
    pub wine_prefix: PathBuf,
    pub profiles_root: PathBuf,
    pub merged: PathBuf,
}

pub struct ProfilePaths {
    pub upper: PathBuf,
    pub work: PathBuf,
    pub log_dir: PathBuf,
}

impl RuntimePaths {
    pub fn from_config(config: &Config) -> Self {
        let game_root = PathBuf::from(&config.game_root);
        let mods_dir = config
            .mods_dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| game_root.join("mods"));

        Self {
            profiles_root: game_root.join("run/profiles"),
            merged: game_root.join("run/merged"),
            base_game: game_root.join("base"),
            mods_dir,
            wine_prefix: game_root.join("pfx"),
        }
    }

    pub fn profile_paths(&self, slug: &str) -> ProfilePaths {
        let base = self.profiles_root.join(slug);
        ProfilePaths {
            upper: base.join("upper"),
            work: base.join("work"),
            log_dir: base.join("logs"),
        }
    }
}

pub fn find_config_file() -> Option<PathBuf> {
    if let Ok(val) = std::env::var("GTA_MO_CONFIG") {
        let p = PathBuf::from(val);
        if p.exists() {
            return Some(p);
        }
        crate::log::warn(format!(
            "GTA_MO_CONFIG apunta a un archivo inexistente: {}",
            p.display()
        ));
    }

    let local = PathBuf::from("config.toml");
    if local.exists() {
        return Some(local);
    }

    let xdg = dirs::config_dir()?.join("gta-mo/config.toml");
    if xdg.exists() {
        return Some(xdg);
    }

    None
}

pub fn db_path() -> PathBuf {
    if let Ok(val) = std::env::var("GTA_MO_DB") {
        return PathBuf::from(val);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gta-mo/organizer.db")
}

pub fn lockfile_path() -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));
    dir.join("gta-mo-launcher.lock")
}

pub fn load_config() -> anyhow::Result<Config> {
    let path = find_config_file()
        .ok_or_else(|| anyhow::anyhow!("Archivo de configuración no encontrado. Buscado en:\n  - $GTA_MO_CONFIG\n  - ./config.toml\n  - $XDG_CONFIG_HOME/gta-mo/config.toml"))?;
    let content = std::fs::read_to_string(&path)?;
    let config: Config = toml::from_str(&content)?;

    Ok(config)
}
