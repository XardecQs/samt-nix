use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Metadata manifest (`mod.toml`) read from inside each mod folder.
///
/// All fields are optional. It is the canonical source of mod metadata; the
/// database caches the key fields on every `discover`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ModMeta {
    pub name: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub url: Option<String>,
    pub description: Option<String>,
    pub cover: Option<String>,
    pub guides: Option<Vec<String>>,
    /// Subdirectories (relative to the mod folder) to overlay onto the game
    /// root. Absent or empty means "mount the whole folder" (legacy behavior).
    pub mount: Option<Vec<String>>,
}

/// Reads `mod.toml` from `mods_dir/<folder>`. Returns `None` when there is no
/// manifest (legacy mod), `Err` when the file exists but is malformed.
pub fn read_mod_meta(mods_dir: &Path, folder: &str) -> anyhow::Result<Option<ModMeta>> {
    let path = mods_dir.join(folder).join("mod.toml");
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)?;
    let meta: ModMeta = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("mod.toml inválido en '{}': {e}", path.display()))?;
    Ok(Some(meta))
}

/// A mount entry must be a relative path (using `/`) with no `.`/`..`
/// components, so it can never escape the mod folder.
pub fn valid_mount_entry(entry: &str) -> bool {
    !entry.is_empty()
        && entry
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != "..")
}

/// Resolves the overlay layer(s) that a mod contributes: the `mount` list from
/// `mod.toml`, or the whole folder when there is no mount list.
pub fn mod_layers(mods_dir: &Path, folder: &str) -> Vec<PathBuf> {
    let base = mods_dir.join(folder);
    if let Ok(Some(meta)) = read_mod_meta(mods_dir, folder) {
        if let Some(mount) = meta.mount {
            let valid: Vec<String> = mount.into_iter().filter(|e| valid_mount_entry(e)).collect();
            if !valid.is_empty() {
                return valid.into_iter().map(|e| base.join(e)).collect();
            }
        }
    }
    vec![base]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_mods_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("gta-mo-meta-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn parses_valid_manifest() {
        let dir = tmp_mods_dir("ok");
        fs::create_dir_all(dir.join("MyMod/models")).unwrap();
        fs::write(
            dir.join("MyMod/mod.toml"),
            r#"
name = "My Mod"
version = "1.2.0"
author = "Author"
url = "https://example.org"
description = "Desc."
cover = "cover.png"
guides = ["guide.md"]
mount = ["models"]
"#,
        )
        .unwrap();

        let meta = read_mod_meta(&dir, "MyMod").unwrap().unwrap();
        assert_eq!(meta.name.as_deref(), Some("My Mod"));
        assert_eq!(meta.version.as_deref(), Some("1.2.0"));
        assert_eq!(meta.author.as_deref(), Some("Author"));
        assert_eq!(meta.mount.as_deref(), Some(&["models".to_string()][..]));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_manifest_is_legacy() {
        let dir = tmp_mods_dir("none");
        fs::create_dir_all(dir.join("Legacy")).unwrap();
        assert!(read_mod_meta(&dir, "Legacy").unwrap().is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_manifest_errors() {
        let dir = tmp_mods_dir("bad");
        fs::create_dir_all(dir.join("Broken")).unwrap();
        fs::write(dir.join("Broken/mod.toml"), "name = [").unwrap();
        assert!(read_mod_meta(&dir, "Broken").is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mount_entry_validation() {
        assert!(valid_mount_entry("models"));
        assert!(valid_mount_entry("a/b/c"));
        assert!(!valid_mount_entry(""));
        assert!(!valid_mount_entry("."));
        assert!(!valid_mount_entry(".."));
        assert!(!valid_mount_entry("../etc"));
        assert!(!valid_mount_entry("a/../b"));
        assert!(!valid_mount_entry("/abs"));
        assert!(!valid_mount_entry("a//b"));
        assert!(!valid_mount_entry("a/"));
    }

    #[test]
    fn layers_respect_mount_list() {
        let dir = tmp_mods_dir("layers");
        fs::create_dir_all(dir.join("M1/models")).unwrap();
        fs::create_dir_all(dir.join("M1/data")).unwrap();
        fs::write(dir.join("M1/mod.toml"), "mount = [\"models\", \"data\"]\n").unwrap();
        fs::create_dir_all(dir.join("M2")).unwrap();

        let m1 = mod_layers(&dir, "M1");
        assert_eq!(m1, vec![dir.join("M1/models"), dir.join("M1/data")]);

        let m2 = mod_layers(&dir, "M2");
        assert_eq!(m2, vec![dir.join("M2")]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalid_mount_entries_fall_back_to_folder() {
        let dir = tmp_mods_dir("invalid");
        fs::create_dir_all(dir.join("M")).unwrap();
        fs::write(dir.join("M/mod.toml"), "mount = [\"../..\", \"/abs\"]\n").unwrap();
        assert_eq!(mod_layers(&dir, "M"), vec![dir.join("M")]);
        let _ = fs::remove_dir_all(&dir);
    }
}
