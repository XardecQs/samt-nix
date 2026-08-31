use serde::{Deserialize, Deserializer};
use std::path::{Path, PathBuf};

/// Metadata manifest (`mod.toml`) read from inside each mod folder.
///
/// All fields are optional. It is the canonical source of mod metadata; the
/// database caches the key fields on every `discover`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ModMeta {
    /// Stable `author:slug` identifier (optional).
    pub id: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    /// Accepts a single string or a list: `author = "x"` / `author = ["x", "y"]`.
    #[serde(default, deserialize_with = "de_author_list")]
    pub author: Vec<String>,
    pub url: Option<String>,
    pub description: Option<String>,
    pub cover: Option<String>,
    pub guides: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    /// Subdirectories (relative to the mod folder) to overlay onto the game
    /// root. Absent or empty means "mount the whole folder" (legacy behavior).
    pub mount: Option<Vec<String>>,
    #[serde(default)]
    pub dependencies: Option<ModDeps>,
}

/// `[dependencies]` section of a manifest. Entries reference a mod by its
/// `id` (`author:slug`) or, as a fallback, by its folder name.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ModDeps {
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
}

fn de_author_list<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Option::<OneOrMany>::deserialize(d)? {
        None => vec![],
        Some(OneOrMany::One(s)) => vec![s],
        Some(OneOrMany::Many(v)) => v,
    })
}

/// A stable mod id must be `author:slug`, both parts lowercase
/// `[a-z0-9_-]` (non-empty).
pub fn valid_mod_id(id: &str) -> bool {
    let Some((author, slug)) = id.split_once(':') else {
        return false;
    };
    let ok_part = |p: &str| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    };
    ok_part(author) && ok_part(slug)
}

/// A tag is a lowercase `[a-z0-9_-]` word.
pub fn valid_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
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

/// Resolves the overlay layer(s) that a mod contributes.
///
/// Each `mount` entry names a folder that is treated as the *game root*: its
/// CONTENTS are laid over the game (e.g. `mount = ["content"]` maps
/// `content/d3d9.dll` to `<game root>/d3d9.dll`). When there is no mount list
/// the whole mod folder is treated as the game root (legacy behavior).
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

/// Rewrites the `name` field of a `mod.toml` (keeping comments and the rest of
/// the file intact). No-op when the mod has no manifest.
pub fn set_meta_name(mods_dir: &Path, folder: &str, name: &str) -> anyhow::Result<()> {
    let path = mods_dir.join(folder).join("mod.toml");
    if !path.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(&path)?;
    let mut doc: toml_edit::DocumentMut = content.parse()?;
    doc["name"] = toml_edit::value(name);
    std::fs::write(&path, doc.to_string())?;
    Ok(())
}

/// Adds/removes `dep_ref` from the `[dependencies]` section of a `mod.toml`
/// (creating the table/array if needed, keeping comments and other fields).
/// No-op when the mod has no manifest.
pub fn set_mod_dependency(
    mods_dir: &Path,
    folder: &str,
    dep_ref: &str,
    optional: bool,
    add: bool,
) -> anyhow::Result<()> {
    let path = mods_dir.join(folder).join("mod.toml");
    if !path.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(&path)?;
    let mut doc: toml_edit::DocumentMut = content.parse()?;
    if !doc.contains_key("dependencies") {
        doc.insert(
            "dependencies",
            toml_edit::Item::Table(toml_edit::Table::new()),
        );
    }
    let deps = doc["dependencies"].as_table_mut().unwrap();
    let key = if optional { "optional" } else { "required" };
    if !deps.contains_key(key) {
        deps.insert(
            key,
            toml_edit::Item::Value(toml_edit::Value::Array(toml_edit::Array::new())),
        );
    }
    let arr = deps[key].as_array_mut().unwrap();
    if add {
        if !arr.iter().any(|v| v.as_str() == Some(dep_ref)) {
            arr.push(dep_ref.to_string());
        }
    } else {
        arr.retain(|v| v.as_str() != Some(dep_ref));
    }
    std::fs::write(&path, doc.to_string())?;
    Ok(())
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
id = "xardec:my-mod"
name = "My Mod"
version = "1.2.0"
author = ["Author", "Coauthor"]
url = "https://example.org"
description = "Desc."
cover = "cover.png"
guides = ["guide.md"]
tags = ["essential", "bugfix"]
mount = ["models"]

[dependencies]
required = ["xardec:asi-loader"]
optional = ["xardec:extra"]
"#,
        )
        .unwrap();

        let meta = read_mod_meta(&dir, "MyMod").unwrap().unwrap();
        assert_eq!(meta.id.as_deref(), Some("xardec:my-mod"));
        assert_eq!(meta.name.as_deref(), Some("My Mod"));
        assert_eq!(meta.version.as_deref(), Some("1.2.0"));
        assert_eq!(meta.author, vec!["Author", "Coauthor"]);
        assert_eq!(
            meta.tags.as_deref(),
            Some(&["essential".to_string(), "bugfix".to_string()][..])
        );
        assert_eq!(meta.mount.as_deref(), Some(&["models".to_string()][..]));
        let deps = meta.dependencies.unwrap();
        assert_eq!(deps.required, vec!["xardec:asi-loader"]);
        assert_eq!(deps.optional, vec!["xardec:extra"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn author_accepts_string_or_list() {
        let dir = tmp_mods_dir("author");
        fs::create_dir_all(dir.join("A")).unwrap();
        fs::create_dir_all(dir.join("B")).unwrap();
        fs::write(dir.join("A/mod.toml"), "author = \"solo\"\n").unwrap();
        fs::write(dir.join("B/mod.toml"), "author = [\"a\", \"b\"]\n").unwrap();
        assert_eq!(
            read_mod_meta(&dir, "A").unwrap().unwrap().author,
            vec!["solo"]
        );
        assert_eq!(
            read_mod_meta(&dir, "B").unwrap().unwrap().author,
            vec!["a", "b"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mod_id_and_tag_validation() {
        assert!(valid_mod_id("xardec:hdcars"));
        assert!(valid_mod_id("doitsujin:dxvk-d3d9"));
        assert!(!valid_mod_id("hdcars"));
        assert!(!valid_mod_id(":hdcars"));
        assert!(!valid_mod_id("xardec:"));
        assert!(!valid_mod_id("Xardec:hdcars"));
        assert!(!valid_mod_id("xardec:hdcars/hmm"));

        assert!(valid_tag("essential"));
        assert!(valid_tag("bug-fix"));
        assert!(!valid_tag(""));
        assert!(!valid_tag("Essential"));
        assert!(!valid_tag("a b"));
    }

    #[test]
    fn set_mod_dependency_edits_the_table() {
        let dir = tmp_mods_dir("dep");
        fs::create_dir_all(dir.join("M")).unwrap();
        fs::write(
            dir.join("M/mod.toml"),
            "# comment\nname = \"M\"\n[dependencies]\nrequired = [\"a:mod\"]\n",
        )
        .unwrap();

        set_mod_dependency(&dir, "M", "b:mod", false, true).unwrap();
        set_mod_dependency(&dir, "M", "c:opt", true, true).unwrap();
        set_mod_dependency(&dir, "M", "a:mod", false, false).unwrap();

        let content = fs::read_to_string(dir.join("M/mod.toml")).unwrap();
        assert!(content.contains("# comment"));
        let deps = read_mod_meta(&dir, "M")
            .unwrap()
            .unwrap()
            .dependencies
            .unwrap();
        assert_eq!(deps.required, vec!["b:mod"]);
        assert_eq!(deps.optional, vec!["c:opt"]);

        // adding twice does not duplicate
        set_mod_dependency(&dir, "M", "b:mod", false, true).unwrap();
        let deps = read_mod_meta(&dir, "M")
            .unwrap()
            .unwrap()
            .dependencies
            .unwrap();
        assert_eq!(deps.required, vec!["b:mod"]);
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

    #[test]
    fn set_meta_name_keeps_comments_and_other_fields() {
        let dir = tmp_mods_dir("rename");
        fs::create_dir_all(dir.join("M")).unwrap();
        fs::write(
            dir.join("M/mod.toml"),
            "# comment\nname = \"Old\"\nversion = \"1.0\"\n",
        )
        .unwrap();

        set_meta_name(&dir, "M", "New Name").unwrap();
        let content = fs::read_to_string(dir.join("M/mod.toml")).unwrap();
        assert!(content.contains("# comment"));
        assert!(content.contains("name = \"New Name\""));
        assert!(content.contains("version = \"1.0\""));

        // no manifest -> no-op
        set_meta_name(&dir, "NoSuchMod", "x").unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}
