use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashMap;
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
    /// License or usage terms stated by the author (e.g. `MIT`), if any.
    pub license: Option<String>,
    pub cover: Option<String>,
    pub guides: Option<Vec<String>>,
    /// Directory (or list of paths) with extra images for the detail gallery,
    /// relative to the mod folder. Directories are expanded to their images.
    #[serde(default, deserialize_with = "de_opt_string_or_list")]
    pub screenshots: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    /// Subdirectories (relative to the mod folder) to overlay onto the game
    /// root. Absent or empty means "mount the whole folder" (legacy behavior).
    pub mount: Option<Vec<String>>,
    #[serde(default)]
    pub dependencies: Option<ModDeps>,
    /// Bundled sub-mods of a composite pack (metadata only).
    #[serde(default)]
    pub components: Option<Vec<MetaComponent>>,
    /// This folder is one variant of a family; only one member of `group` may
    /// be enabled in a profile.
    #[serde(default)]
    pub variant: Option<ModVariant>,
    /// Mods incompatible with this one (by `author:slug` or folder name).
    #[serde(default)]
    pub conflicts: Vec<String>,
    /// At least one of these mods (by `author:slug` or folder) must be enabled
    /// for this mod to work. Unlike `[dependencies].required` (which is an edge
    /// that auto-enables and orders), this is a validation constraint.
    #[serde(default)]
    pub requires_any: Vec<String>,
    /// Mod Loader (`modloader.ini`) integration.
    #[serde(default)]
    pub modloader: Option<ModLoaderMeta>,
    /// Selectable option groups (one value per group per profile). When absent,
    /// groups are auto-discovered from `options/<group>/<value>/`.
    #[serde(default)]
    pub options: Option<Vec<ModOption>>,
}

/// One selectable option group of a mod. The value chosen per profile mounts
/// the matching `options/<group>/<value>/` folder on top of the base `mount`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ModOption {
    /// Stable group id (lowercase `[a-z0-9_-]`).
    pub group: String,
    /// Human-readable label (optional).
    pub name: Option<String>,
    /// Pre-selected value when the profile has not chosen one (optional).
    pub default: Option<String>,
    #[serde(default, deserialize_with = "de_option_choices")]
    pub choices: Vec<OptionChoice>,
}

/// One value of a [`ModOption`] group.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct OptionChoice {
    /// Stable value id (lowercase `[a-z0-9_-]`).
    pub key: String,
    /// Human-readable label (optional).
    pub name: Option<String>,
    /// Relative folder treated as the game root for this value. Defaults to
    /// `options/<group>/<key>`.
    pub path: Option<String>,
    /// Mods incompatible with this mod when this value is selected.
    #[serde(default)]
    pub conflicts: Vec<String>,
    /// At least one of these must be enabled when this value is selected.
    #[serde(default)]
    pub requires_any: Vec<String>,
}

/// Accepts `choices = ["4k", "lite"]` or `choices = [{ key = "4k", ... }]`.
fn de_option_choices<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<OptionChoice>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum ChoiceSpec {
        Key(String),
        Full(OptionChoice),
    }
    let raw = Vec::<ChoiceSpec>::deserialize(d)?;
    Ok(raw
        .into_iter()
        .map(|c| match c {
            ChoiceSpec::Key(key) => OptionChoice {
                key,
                ..Default::default()
            },
            ChoiceSpec::Full(full) => full,
        })
        .collect())
}

/// One variant of a mutually exclusive family.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ModVariant {
    /// Stable family id (lowercase `[a-z0-9_-]`).
    pub group: String,
    /// Human-readable label for this variant (optional).
    pub name: Option<String>,
}

/// Mod Loader priority for the folders a mod adds under `modloader/`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ModLoaderMeta {
    /// Priority 1..=100 (higher wins). Absent = do not touch `modloader.ini`.
    pub priority: Option<i64>,
    /// Explicit folder names under `modloader/`; auto-detected when absent.
    #[serde(default)]
    pub folders: Option<Vec<String>>,
}

/// One bundled component of a composite pack.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct MetaComponent {
    pub name: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub url: Option<String>,
    /// Optional relative path to the component inside the mod folder.
    pub path: Option<String>,
}

impl ModMeta {
    /// A manifest with `[[components]]` is a composite pack.
    pub fn is_pack(&self) -> bool {
        self.components
            .as_ref()
            .map(|c| !c.is_empty())
            .unwrap_or(false)
    }
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

/// Accepts `screenshots = "dir"` or `screenshots = ["a.png", "dir"]`.
fn de_opt_string_or_list<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Option::<OneOrMany>::deserialize(d)? {
        None => None,
        Some(OneOrMany::One(s)) => Some(vec![s]),
        Some(OneOrMany::Many(v)) => Some(v),
    })
}

/// True when `path` looks like an image the gallery can render.
pub fn is_image_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif")
    )
}

/// Resolves the `screenshots` manifest entries to relative image paths inside
/// the mod folder. Directory entries are expanded to their images (sorted).
pub fn mod_screenshots(mods_dir: &Path, folder: &str) -> Vec<String> {
    let Some(meta) = read_mod_meta(mods_dir, folder).ok().flatten() else {
        return Vec::new();
    };
    let Some(entries) = meta.screenshots else {
        return Vec::new();
    };
    let base = mods_dir.join(folder);
    let mut out: Vec<String> = Vec::new();
    for entry in entries {
        if !valid_relative_path(&entry) {
            continue;
        }
        let p = base.join(&entry);
        if p.is_dir() {
            let mut files: Vec<String> = std::fs::read_dir(&p)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .filter(|e| is_image_file(&e.path()))
                .map(|e| {
                    format!(
                        "{}/{}",
                        entry.trim_end_matches('/'),
                        e.file_name().to_string_lossy()
                    )
                })
                .collect();
            files.sort();
            out.extend(files);
        } else if is_image_file(&p) {
            out.push(entry);
        }
    }
    out
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

/// Normalizes a stable mod id (`author:slug`) to its canonical lowercase form.
pub fn normalize_mod_id(id: &str) -> String {
    id.trim().to_lowercase()
}

/// A tag is a lowercase `[a-z0-9_-]` word.
pub fn valid_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// A variant group id has the same shape as a tag.
pub fn valid_variant_group(group: &str) -> bool {
    valid_tag(group)
}

/// A `[modloader] folders` entry is a single path segment under `modloader/`:
/// no separators and no overlay-breaking characters.
pub fn valid_modloader_folder(folder: &str) -> bool {
    !folder.is_empty()
        && folder != "."
        && folder != ".."
        && !folder.contains('/')
        && !folder.contains('\\')
        && !folder.contains(',')
        && !folder.contains(':')
        // `=` starts a value in `modloader.ini`; control characters (newlines,
        // tabs…) would inject extra lines/sections.
        && !folder.contains('=')
        && !folder.chars().any(char::is_control)
        && folder.trim() == folder
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
/// components, and must not contain characters that would corrupt the
/// `fuse-overlayfs` option string: `:` separates `lowerdir` layers and `,`
/// separates mount options. Rejecting them here means a manifest can never
/// escape the mod folder nor inject overlay options.
pub fn valid_mount_entry(entry: &str) -> bool {
    !entry.is_empty()
        && !entry.contains(',')
        && !entry.contains(':')
        && entry
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != "..")
}

/// True when a mod folder name is safe to use as an overlay layer path. Folder
/// names ultimately land in the `lowerdir` option, so a bare `,` or `:` would
/// split options/layers even though the folder itself is a valid filename.
pub fn overlay_safe_folder(folder: &str) -> bool {
    !folder.contains(',') && !folder.contains(':')
}

/// True when a filesystem path is safe to embed in the `fuse-overlayfs`
/// option string (`lowerdir`/`upperdir`/`workdir`). Covers `mods_dir`,
/// `game_root` and the per-profile dirs, which come from the user config and
/// are not otherwise validated.
pub fn overlay_safe_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    !s.contains(',') && !s.contains(':')
}

/// True when `entry` is a relative path confined to its base directory: it is
/// not absolute and has no `.`/`..` component. Used for manifest paths that are
/// read from (but must never escape) the mod folder: `cover`, `guides`,
/// `screenshots` and `components[].path`. A trailing `/` is tolerated.
pub fn valid_relative_path(entry: &str) -> bool {
    let e = entry.trim_end_matches('/');
    !e.is_empty()
        && !e.starts_with('/')
        && !e.starts_with('\\')
        && e.split(['/', '\\'])
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

/// An option group/value id has the same shape as a tag.
pub fn valid_option_key(key: &str) -> bool {
    valid_tag(key)
}

/// Names of the immediate subdirectories of `dir` (empty when it does not
/// exist or cannot be read).
fn read_subdirs(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .collect()
}

/// Relative path of a choice's game-root folder (its `path`, or the default
/// `options/<group>/<key>`).
pub fn option_choice_path(group: &ModOption, choice: &OptionChoice) -> String {
    choice
        .path
        .clone()
        .unwrap_or_else(|| format!("options/{}/{}", group.group, choice.key))
}

/// Resolves a mod's option groups: the `[[options]]` declarations merged with
/// the folders auto-discovered under `options/<group>/<value>/`.
///
/// Declared groups keep their order/labels; discovered groups/values not
/// declared are appended (alphabetically). Invalid keys are skipped.
pub fn mod_option_groups(mods_dir: &Path, folder: &str) -> Vec<ModOption> {
    let base = mods_dir.join(folder);
    let mut groups: Vec<ModOption> = Vec::new();

    if let Ok(Some(meta)) = read_mod_meta(mods_dir, folder) {
        if let Some(opts) = meta.options {
            for o in opts {
                if !valid_option_key(&o.group) {
                    continue;
                }
                let mut group = o;
                group.choices.retain(|c| valid_option_key(&c.key));
                groups.push(group);
            }
        }
    }

    let opts_root = base.join("options");
    if opts_root.is_dir() {
        let mut group_dirs = read_subdirs(&opts_root);
        group_dirs.sort();
        for g in group_dirs {
            if !valid_option_key(&g) {
                continue;
            }
            let mut values: Vec<String> = read_subdirs(&opts_root.join(&g))
                .into_iter()
                .filter(|v| valid_option_key(v))
                .collect();
            values.sort();
            if let Some(existing) = groups.iter_mut().find(|x| x.group == g) {
                for v in values {
                    if !existing.choices.iter().any(|c| c.key == v) {
                        existing.choices.push(OptionChoice {
                            key: v,
                            ..Default::default()
                        });
                    }
                }
            } else {
                let choices = values
                    .into_iter()
                    .map(|key| OptionChoice {
                        key,
                        ..Default::default()
                    })
                    .collect();
                groups.push(ModOption {
                    group: g,
                    name: None,
                    default: None,
                    choices,
                });
            }
        }
    }

    groups
}

/// Returns `selected` with the manifest `default` filled in for every group the
/// profile has not chosen. A default that does not name a declared choice is
/// ignored, so it can never mount a non-existent folder.
pub fn fill_option_defaults(
    mods_dir: &Path,
    folder: &str,
    selected: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut out = selected.clone();
    for group in mod_option_groups(mods_dir, folder) {
        if out.contains_key(&group.group) {
            continue;
        }
        if let Some(default) = &group.default {
            if group.choices.iter().any(|c| &c.key == default) {
                out.insert(group.group.clone(), default.clone());
            }
        }
    }
    out
}

/// Resolves the overlay layer(s) contributed by a mod's **selected** options.
/// One layer per selected group (in group order), above the base mount.
pub fn mod_option_layers(
    mods_dir: &Path,
    folder: &str,
    selected: &HashMap<String, String>,
) -> Vec<PathBuf> {
    if selected.is_empty() {
        return Vec::new();
    }
    let base = mods_dir.join(folder);
    let mut out = Vec::new();
    for group in mod_option_groups(mods_dir, folder) {
        let Some(value) = selected.get(&group.group) else {
            continue;
        };
        let Some(choice) = group.choices.iter().find(|c| &c.key == value) else {
            continue;
        };
        let rel = option_choice_path(&group, choice);
        if valid_mount_entry(&rel) {
            out.push(base.join(&rel));
        }
    }
    out
}

/// Like [`mod_layers`], but prepends the layers of the mod's selected options
/// so they override the base `mount`.
pub fn mod_layers_with_options(
    mods_dir: &Path,
    folder: &str,
    selected: &HashMap<String, String>,
) -> Vec<PathBuf> {
    let mut layers = mod_option_layers(mods_dir, folder, selected);
    layers.extend(mod_layers(mods_dir, folder));
    layers
}

/// Atomically writes `content` to `path` (temp file + rename) so a crash or
/// power loss can never leave a truncated `mod.toml`.
fn write_manifest(path: &Path, content: &str) -> anyhow::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("No hay directorio padre para {}", path.display()))?;
    let tmp = dir.join(format!(".mod.toml.tmp-{}", std::process::id()));
    std::fs::write(&tmp, content)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
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
    write_manifest(&path, &doc.to_string())
}

/// Replaces the `tags` array of a `mod.toml` (keeping comments and other
/// fields). An empty list removes the key. Errors when the mod has no manifest.
pub fn set_meta_tags(mods_dir: &Path, folder: &str, tags: &[String]) -> anyhow::Result<()> {
    let path = mods_dir.join(folder).join("mod.toml");
    if !path.exists() {
        anyhow::bail!(
            "El mod '{folder}' no tiene mod.toml; créalo primero (botón Crear mod.toml)."
        );
    }
    let content = std::fs::read_to_string(&path)?;
    let mut doc: toml_edit::DocumentMut = content.parse()?;
    if tags.is_empty() {
        doc.as_table_mut().remove("tags");
    } else {
        let mut arr = toml_edit::Array::new();
        for t in tags {
            arr.push(t.as_str());
        }
        doc["tags"] = toml_edit::value(arr);
    }
    write_manifest(&path, &doc.to_string())
}

/// Replaces a mod's whole `mod.toml` with `content`, after validating that it
/// parses as a manifest. Errors when the mod has no manifest to replace.
pub fn write_manifest_validated(
    mods_dir: &Path,
    folder: &str,
    content: &str,
) -> anyhow::Result<()> {
    let path = mods_dir.join(folder).join("mod.toml");
    if !path.exists() {
        anyhow::bail!("El mod '{folder}' no tiene mod.toml.");
    }
    // Validate before touching the file.
    toml::from_str::<ModMeta>(content).map_err(|e| anyhow::anyhow!("mod.toml inválido: {e}"))?;
    write_manifest(&path, content)
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
    let deps = doc["dependencies"]
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("'dependencies' en {} no es una tabla", path.display()))?;
    if add {
        let key = if optional { "optional" } else { "required" };
        if !deps.contains_key(key) {
            deps.insert(
                key,
                toml_edit::Item::Value(toml_edit::Value::Array(toml_edit::Array::new())),
            );
        }
        let arr = deps[key]
            .as_array_mut()
            .ok_or_else(|| anyhow::anyhow!("'{key}' en {} no es una lista", path.display()))?;
        if !arr.iter().any(|v| v.as_str() == Some(dep_ref)) {
            arr.push(dep_ref.to_string());
        }
    } else {
        // Removal is key-agnostic: the dep may live under `required` or
        // `optional` (we don't track that flag in the DB at remove time).
        for key in ["required", "optional"] {
            if deps.contains_key(key) {
                if let Some(arr) = deps[key].as_array_mut() {
                    arr.retain(|v| v.as_str() != Some(dep_ref));
                }
            }
        }
    }
    write_manifest(&path, &doc.to_string())?;
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
    fn parses_components_and_flags_pack() {
        let dir = tmp_mods_dir("comp");
        fs::create_dir_all(dir.join("P")).unwrap();
        fs::create_dir_all(dir.join("S")).unwrap();
        fs::write(
            dir.join("P/mod.toml"),
            "id = \"mix:pack\"\n[[components]]\nname = \"SilentPatch\"\nversion = \"1.0.1\"\nauthor = \"Silent\"\nurl = \"http://x/\"\npath = \"content/silentpatch\"\n",
        )
        .unwrap();
        fs::write(dir.join("S/mod.toml"), "name = \"Simple\"\n").unwrap();

        let p = read_mod_meta(&dir, "P").unwrap().unwrap();
        assert!(p.is_pack());
        let comps = p.components.unwrap();
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0].name.as_deref(), Some("SilentPatch"));
        assert_eq!(comps[0].version.as_deref(), Some("1.0.1"));
        assert_eq!(comps[0].author.as_deref(), Some("Silent"));
        assert_eq!(comps[0].url.as_deref(), Some("http://x/"));
        assert_eq!(comps[0].path.as_deref(), Some("content/silentpatch"));

        let s = read_mod_meta(&dir, "S").unwrap().unwrap();
        assert!(!s.is_pack());
        let _ = fs::remove_dir_all(&dir);
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
        // overlay option injection
        assert!(!valid_mount_entry("content,upperdir=/etc"));
        assert!(!valid_mount_entry("a:b"));
        assert!(!overlay_safe_folder("evil,upperdir=/etc"));
        assert!(!overlay_safe_folder("a:b"));
        assert!(overlay_safe_folder("My Mod"));
    }

    #[test]
    fn relative_path_is_confined_to_base() {
        assert!(valid_relative_path("cover.png"));
        assert!(valid_relative_path("guides/instalacion.md"));
        assert!(valid_relative_path("capturas/"));
        // escapes
        assert!(!valid_relative_path("/etc/passwd"));
        assert!(!valid_relative_path("../secret"));
        assert!(!valid_relative_path("a/../../b"));
        assert!(!valid_relative_path(""));
        assert!(!valid_relative_path("a//b"));
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
    fn screenshots_dir_is_expanded_to_images() {
        let dir = tmp_mods_dir("shots");
        fs::create_dir_all(dir.join("M/capturas")).unwrap();
        fs::write(dir.join("M/capturas/b.png"), "x").unwrap();
        fs::write(dir.join("M/capturas/a.jpg"), "x").unwrap();
        fs::write(dir.join("M/capturas/nota.txt"), "x").unwrap();
        fs::write(dir.join("M/mod.toml"), "screenshots = [\"capturas\"]\n").unwrap();

        assert_eq!(
            mod_screenshots(&dir, "M"),
            vec!["capturas/a.jpg".to_string(), "capturas/b.png".to_string()]
        );
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

    #[test]
    fn set_meta_tags_replaces_and_clears() {
        let dir = tmp_mods_dir("tags");
        fs::create_dir_all(dir.join("M")).unwrap();
        fs::write(
            dir.join("M/mod.toml"),
            "# c\nname = \"M\"\ntags = [\"old\"]\nversion = \"1.0\"\n",
        )
        .unwrap();

        set_meta_tags(&dir, "M", &["essential".into(), "bugfix".into()]).unwrap();
        let meta = read_mod_meta(&dir, "M").unwrap().unwrap();
        assert_eq!(
            meta.tags.as_deref(),
            Some(&["essential".to_string(), "bugfix".to_string()][..])
        );
        let content = fs::read_to_string(dir.join("M/mod.toml")).unwrap();
        assert!(content.contains("# c") && content.contains("version = \"1.0\""));

        // empty -> key removed
        set_meta_tags(&dir, "M", &[]).unwrap();
        assert!(read_mod_meta(&dir, "M").unwrap().unwrap().tags.is_none());

        // no manifest -> error
        assert!(set_meta_tags(&dir, "NoSuchMod", &["x".into()]).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_manifest_validated_rejects_bad_toml() {
        let dir = tmp_mods_dir("manifest");
        fs::create_dir_all(dir.join("M")).unwrap();
        fs::write(dir.join("M/mod.toml"), "name = \"M\"\n").unwrap();

        // Malformed TOML is rejected and the file is untouched.
        assert!(write_manifest_validated(&dir, "M", "name = [").is_err());
        assert_eq!(
            fs::read_to_string(dir.join("M/mod.toml")).unwrap(),
            "name = \"M\"\n"
        );

        write_manifest_validated(&dir, "M", "name = \"New\"\nversion = \"2.0\"\n").unwrap();
        let meta = read_mod_meta(&dir, "M").unwrap().unwrap();
        assert_eq!(meta.name.as_deref(), Some("New"));
        assert_eq!(meta.version.as_deref(), Some("2.0"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn option_groups_merge_manifest_and_folders() {
        let dir = tmp_mods_dir("opts");
        fs::create_dir_all(dir.join("M/options/variant/4k")).unwrap();
        fs::create_dir_all(dir.join("M/options/variant/lite")).unwrap();
        fs::create_dir_all(dir.join("M/options/preset/high")).unwrap();
        fs::write(
            dir.join("M/mod.toml"),
            r#"
mount = ["content"]

[[options]]
group = "variant"
name = "Variante"
default = "4k"

  [[options.choices]]
  key = "4k"

  [[options.choices]]
  key = "lite"
  name = "LITE"
  conflicts = ["x:other"]
"#,
        )
        .unwrap();

        let groups = mod_option_groups(&dir, "M");
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].group, "variant");
        assert_eq!(groups[0].name.as_deref(), Some("Variante"));
        assert_eq!(groups[0].default.as_deref(), Some("4k"));
        // Declared order kept (4k, lite); discovered "preset" appended.
        let keys: Vec<&str> = groups[0].choices.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, vec!["4k", "lite"]);
        assert_eq!(
            groups[0]
                .choices
                .iter()
                .find(|c| c.key == "lite")
                .unwrap()
                .conflicts,
            vec!["x:other"]
        );
        assert_eq!(groups[1].group, "preset");
        assert_eq!(groups[1].choices.len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn option_layers_go_above_the_base_mount() {
        let dir = tmp_mods_dir("optlayers");
        fs::create_dir_all(dir.join("M/content")).unwrap();
        fs::create_dir_all(dir.join("M/options/variant/4k")).unwrap();
        fs::write(
            dir.join("M/mod.toml"),
            "mount = [\"content\"]\n[[options]]\ngroup = \"variant\"\nchoices = [\"4k\"]\n",
        )
        .unwrap();

        let selected: HashMap<String, String> = [("variant".to_string(), "4k".to_string())].into();
        let layers = mod_layers_with_options(&dir, "M", &selected);
        assert_eq!(
            layers,
            vec![dir.join("M/options/variant/4k"), dir.join("M/content")]
        );

        // No selection -> only the base mount.
        assert_eq!(
            mod_layers_with_options(&dir, "M", &HashMap::new()),
            vec![dir.join("M/content")]
        );

        // Unknown value -> only the base mount.
        let unknown: HashMap<String, String> = [("variant".to_string(), "nope".to_string())].into();
        assert_eq!(
            mod_layers_with_options(&dir, "M", &unknown),
            vec![dir.join("M/content")]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn option_default_fills_when_unselected() {
        let dir = tmp_mods_dir("optdefault");
        fs::create_dir_all(dir.join("M/content")).unwrap();
        fs::create_dir_all(dir.join("M/options/variant/4k")).unwrap();
        fs::create_dir_all(dir.join("M/options/variant/lite")).unwrap();
        fs::write(
            dir.join("M/mod.toml"),
            "mount = [\"content\"]\n[[options]]\ngroup = \"variant\"\ndefault = \"4k\"\nchoices = [\"4k\", \"lite\"]\n",
        )
        .unwrap();

        // No selection -> the declared default is filled and mounted.
        let effective = fill_option_defaults(&dir, "M", &HashMap::new());
        assert_eq!(effective.get("variant").map(String::as_str), Some("4k"));
        assert_eq!(
            mod_layers_with_options(&dir, "M", &effective),
            vec![dir.join("M/options/variant/4k"), dir.join("M/content")]
        );

        // An explicit choice wins over the default.
        let chosen: HashMap<String, String> = [("variant".to_string(), "lite".to_string())].into();
        let effective = fill_option_defaults(&dir, "M", &chosen);
        assert_eq!(effective.get("variant").map(String::as_str), Some("lite"));

        // An invalid default is ignored (never mounts a missing folder).
        fs::write(
            dir.join("M/mod.toml"),
            "mount = [\"content\"]\n[[options]]\ngroup = \"variant\"\ndefault = \"ghost\"\nchoices = [\"4k\"]\n",
        )
        .unwrap();
        assert!(fill_option_defaults(&dir, "M", &HashMap::new()).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn modloader_folder_rejects_ini_injection() {
        assert!(valid_modloader_folder("MyFolder"));
        assert!(valid_modloader_folder("my-folder_1"));
        // Characters that could break or inject lines in `modloader.ini`.
        assert!(!valid_modloader_folder(""));
        assert!(!valid_modloader_folder("."));
        assert!(!valid_modloader_folder(".."));
        assert!(!valid_modloader_folder("a/b"));
        assert!(!valid_modloader_folder("a:b"));
        assert!(!valid_modloader_folder("a,b"));
        assert!(!valid_modloader_folder("a=b"));
        assert!(!valid_modloader_folder("a\n[Profiles.Evil]\nevil=100"));
        assert!(!valid_modloader_folder("a\tb"));
        assert!(!valid_modloader_folder(" leading"));
        assert!(!valid_modloader_folder("trailing "));
    }
}
