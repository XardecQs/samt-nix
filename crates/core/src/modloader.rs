//! Mod Loader (`modloader.ini`) integration.
//!
//! Mod Loader orders the mods under `<game root>/modloader/` with its own
//! priority list in `modloader.ini`, independently of the fuse-overlayfs layer
//! order. This module detects which modloader folders a mod contributes and
//! merges priorities into a profile's `modloader.ini`, preserving the rest of
//! the file (it is an INI with `;` comments, not TOML).

use std::path::{Path, PathBuf};

/// Mod Loader's default profile name.
const DEFAULT_PROFILE: &str = "Default";

/// `<upper>/<...>/modloader/modloader.ini` for a profile's upper directory.
pub fn ini_path(upper: &Path) -> PathBuf {
    upper.join("modloader").join("modloader.ini")
}

/// Detects the `<layer>/modloader/<Name>` folders a mod contributes.
pub fn detect_folders(mods_dir: &Path, folder: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for layer in crate::meta::mod_layers(mods_dir, folder) {
        let Ok(entries) = std::fs::read_dir(layer.join("modloader")) else {
            continue;
        };
        for e in entries.flatten() {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.starts_with('.') && !out.contains(&name) {
                    out.push(name);
                }
            }
        }
    }
    out.sort();
    out
}

/// Priority entries for the enabled mods that declare `[modloader] priority`.
/// Folder names are taken from `[modloader] folders` or auto-detected.
pub fn entries_for(mods_dir: &Path, enabled: &[String]) -> Vec<(String, i64)> {
    let mut entries: Vec<(String, i64)> = Vec::new();
    for folder in enabled {
        let Ok(Some(meta)) = crate::meta::read_mod_meta(mods_dir, folder) else {
            continue;
        };
        let Some(ml) = meta.modloader else { continue };
        let Some(priority) = ml.priority else {
            continue;
        };
        let folders = match ml.folders {
            Some(f) if !f.is_empty() => f,
            _ => detect_folders(mods_dir, folder),
        };
        for name in folders {
            if crate::meta::valid_modloader_folder(&name) {
                entries.push((name, priority));
            } else {
                crate::log::warn(format!(
                    "'{folder}': nombre de carpeta de modloader inválido '{name}' (se ignora)."
                ));
            }
        }
    }
    entries.sort();
    entries.dedup_by(|a, b| a.0 == b.0);
    entries
}

/// Text before a Mod Loader `;` comment.
fn strip_comment(line: &str) -> &str {
    match line.find(';') {
        Some(i) => &line[..i],
        None => line,
    }
}

/// Name of the profile configured in `[Folder.Config]` (`Profile = X`).
pub fn configured_profile(content: &str) -> Option<String> {
    let mut in_section = false;
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_section = t.eq_ignore_ascii_case("[Folder.Config]");
            continue;
        }
        if in_section {
            if let Some((k, v)) = strip_comment(t).split_once('=') {
                if k.trim().eq_ignore_ascii_case("Profile") {
                    let v = v.trim().trim_matches('"').trim();
                    if !v.is_empty() {
                        return Some(v.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Reads the `[Profiles.<profile>.Priority]` entries from an ini.
pub fn read_priorities(content: &str, profile: &str) -> Vec<(String, i64)> {
    let target = format!("[Profiles.{profile}.Priority]");
    let mut out = Vec::new();
    let mut in_target = false;
    for line in content.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_target = t.eq_ignore_ascii_case(&target);
            continue;
        }
        if in_target {
            if let Some((k, v)) = strip_comment(t).split_once('=') {
                if let Ok(p) = v.trim().parse::<i64>() {
                    out.push((k.trim().to_string(), p));
                }
            }
        }
    }
    out
}

/// Returns `content` with the priority entries of `profile` replaced by
/// `entries`, keeping comments and every other section intact.
pub fn merge_priority(content: &str, profile: &str, entries: &[(String, i64)]) -> String {
    let target = format!("[Profiles.{profile}.Priority]");
    let lines: Vec<&str> = content.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    let mut found = false;
    while i < lines.len() {
        let t = lines[i].trim();
        if t.starts_with('[') && t.eq_ignore_ascii_case(&target) {
            found = true;
            out.push(lines[i].to_string());
            i += 1;
            // Keep comments/blank lines; drop the previous entries.
            let mut comments: Vec<String> = Vec::new();
            while i < lines.len() && !lines[i].trim().starts_with('[') {
                let nt = lines[i].trim();
                if nt.is_empty() || nt.starts_with(';') || nt.starts_with('#') {
                    comments.push(lines[i].to_string());
                }
                i += 1;
            }
            out.extend(comments);
            for (name, prio) in entries {
                out.push(format!("{name}={prio}"));
            }
            continue;
        }
        out.push(lines[i].to_string());
        i += 1;
    }
    if !found {
        if out.last().map(|l| !l.trim().is_empty()).unwrap_or(false) {
            out.push(String::new());
        }
        out.push(target);
        out.push("; Higher values override lower ones (default 50).".to_string());
        for (name, prio) in entries {
            out.push(format!("{name}={prio}"));
        }
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// Merges `entries` into `ini_path` (creating it if needed), atomically.
pub fn apply(ini_path: &Path, entries: &[(String, i64)]) -> anyhow::Result<()> {
    if let Some(parent) = ini_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = std::fs::read_to_string(ini_path).unwrap_or_default();
    // A missing/empty file gets a minimal, valid Mod Loader config.
    let content = if content.trim().is_empty() {
        ";\n;  Mod Loader Folder Config File\n;\n[Folder.Config]\nProfile = Default\nPriorityLimit = 100\n"
            .to_string()
    } else {
        content
    };
    let profile = configured_profile(&content).unwrap_or_else(|| DEFAULT_PROFILE.to_string());
    let merged = merge_priority(&content, &profile, entries);
    let tmp = ini_path.with_extension("ini.tmp");
    std::fs::write(&tmp, &merged)?;
    std::fs::rename(&tmp, ini_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_modloader_folders() {
        let dir = std::env::temp_dir().join(format!("gta-mo-ml-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("M/modloader/Proper Fixes")).unwrap();
        std::fs::create_dir_all(dir.join("M/modloader/.data")).unwrap();
        assert_eq!(detect_folders(&dir, "M"), vec!["Proper Fixes".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_preserves_comments_and_replaces_entries() {
        let sample = "\
;
;  Mod Loader Folder Config File
;
[Folder.Config]
Profile = Default                 ; Profile to be used
PriorityLimit = 100

[Profiles.Default.Priority]
; higher wins
OldMod=30

[Profiles.Default.IgnoreMods]
_ignore
";
        let entries = vec![
            ("Proper Fixes".to_string(), 60),
            ("RoSA Project Evolved".to_string(), 50),
        ];
        let merged = merge_priority(sample, "Default", &entries);
        assert!(merged.contains("; higher wins"));
        assert!(merged.contains(";  Mod Loader Folder Config File"));
        assert!(merged.contains("Proper Fixes=60"));
        assert!(merged.contains("RoSA Project Evolved=50"));
        assert!(!merged.contains("OldMod=30"));
        assert!(merged.contains("[Profiles.Default.IgnoreMods]"));
        assert_eq!(
            read_priorities(&merged, "Default"),
            vec![
                ("Proper Fixes".to_string(), 60),
                ("RoSA Project Evolved".to_string(), 50),
            ]
        );
        // The configured profile is detected.
        assert_eq!(configured_profile(sample).as_deref(), Some("Default"));
    }

    #[test]
    fn merge_creates_missing_section() {
        let sample = "[Folder.Config]\nProfile = Custom\n";
        let merged = merge_priority(sample, "Custom", &[("X".to_string(), 55)]);
        assert!(merged.contains("[Profiles.Custom.Priority]"));
        assert_eq!(
            read_priorities(&merged, "Custom"),
            vec![("X".to_string(), 55)]
        );
    }

    #[test]
    fn apply_roundtrip() {
        let dir = std::env::temp_dir().join(format!("gta-mo-mlapply-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = ini_path(&dir);
        apply(&path, &[("Proper Fixes".to_string(), 60)]).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(configured_profile(&content).as_deref(), Some("Default"));
        assert_eq!(
            read_priorities(&content, "Default"),
            vec![("Proper Fixes".to_string(), 60)]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
