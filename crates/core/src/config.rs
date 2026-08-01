use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Deserialize, Serialize, Clone)]
pub struct Config {
    #[serde(alias = "game_root")]
    pub game_root: String,
    #[serde(alias = "proton_path")]
    pub proton_path: String,
    #[serde(alias = "game_id")]
    pub game_id: Option<String>,
    #[serde(alias = "game_exe")]
    pub game_exe: Option<String>,
    #[serde(alias = "proton_use_wined3d")]
    pub proton_use_wined3d: Option<bool>,
    #[serde(alias = "proton_disable_ntsync")]
    pub proton_disable_ntsync: Option<bool>,
    #[serde(alias = "dxvk_hud")]
    pub dxvk_hud: Option<String>,
    #[serde(alias = "auto_discover")]
    pub auto_discover: Option<bool>,
    #[serde(alias = "mods_dir")]
    pub mods_dir: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct UserOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proton_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub game_exe: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proton_use_wined3d: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proton_disable_ntsync: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dxvk_hud: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_discover: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mods_dir: Option<String>,
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

    pub fn auto_discover(&self) -> bool {
        self.auto_discover.unwrap_or(false)
    }

    pub fn dxvk_hud(&self) -> &str {
        self.dxvk_hud
            .as_deref()
            .unwrap_or("devinfo,fps,frametimes,submissions,compiler,version,api,pipelines,memory,gpuload,drawcalls")
    }

    pub fn apply(&mut self, overrides: &UserOverrides) {
        if let Some(ref v) = overrides.game_root {
            self.game_root = v.clone();
        }
        if let Some(ref v) = overrides.proton_path {
            self.proton_path = v.clone();
        }
        if let Some(ref v) = overrides.game_id {
            self.game_id = Some(v.clone());
        }
        if let Some(ref v) = overrides.game_exe {
            self.game_exe = Some(v.clone());
        }
        if let Some(v) = overrides.proton_use_wined3d {
            self.proton_use_wined3d = Some(v);
        }
        if let Some(v) = overrides.proton_disable_ntsync {
            self.proton_disable_ntsync = Some(v);
        }
        if let Some(ref v) = overrides.dxvk_hud {
            self.dxvk_hud = Some(v.clone());
        }
        if let Some(v) = overrides.auto_discover {
            self.auto_discover = Some(v);
        }
        if let Some(ref v) = overrides.mods_dir {
            self.mods_dir = Some(v.clone());
        }
    }
}

pub struct RuntimePaths {
    pub base_game: PathBuf,
    pub mods_dir: PathBuf,
    pub wine_prefix: PathBuf,
    pub upper: PathBuf,
    pub work: PathBuf,
    pub merged: PathBuf,
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
            upper: game_root.join("run/upper"),
            work: game_root.join("run/work"),
            merged: game_root.join("run/merged"),
            log_dir: game_root.join("run/logs"),
            base_game: game_root.join("base"),
            mods_dir,
            wine_prefix: game_root.join("pfx"),
        }
    }
}

pub fn find_config_file() -> Option<PathBuf> {
    if let Ok(val) = std::env::var("GTA_MO_CONFIG") {
        let p = PathBuf::from(val);
        if p.exists() {
            return Some(p);
        }
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

pub fn is_nix_managed() -> bool {
    if let Some(path) = find_config_file() {
        if let Ok(meta) = std::fs::symlink_metadata(&path) {
            if meta.file_type().is_symlink() {
                if let Ok(target) = std::fs::read_link(&path) {
                    return target.starts_with("/nix/store/");
                }
            }
        }
    }
    false
}

pub fn user_override_path() -> Option<PathBuf> {
    let p = dirs::config_dir()?.join("gta-mo/config.user.toml");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gta-mo")
}

pub fn user_override_file_path() -> PathBuf {
    config_dir().join("config.user.toml")
}

pub fn load_user_overrides() -> Option<UserOverrides> {
    let path = user_override_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&content).ok()
}

pub fn save_user_overrides(overrides: &UserOverrides) -> anyhow::Result<()> {
    let path = user_override_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(overrides)?;
    std::fs::write(&path, content)?;
    Ok(())
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
    let mut config: Config = toml::from_str(&content)?;

    if let Some(overrides) = load_user_overrides() {
        config.apply(&overrides);
    }

    Ok(config)
}

pub fn load_config_or_die() -> Config {
    match load_config() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[X] Error: {e}");
            std::process::exit(1);
        }
    }
}
