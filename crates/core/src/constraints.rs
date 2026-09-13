//! Cross-mod constraints that the per-mod dependency graph cannot express:
//! mutually exclusive variant families and declared incompatibilities.
//!
//! The manifest is the source of truth, so this reads `mod.toml` live.

use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Severity/kind of a constraint violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// More than one variant of the same family is enabled.
    Variant,
    /// Two mods declared incompatible are enabled.
    Conflict,
}

/// One violated constraint.
#[derive(Debug, Clone)]
pub struct Violation {
    pub kind: Kind,
    pub message: String,
}

/// Checks the enabled mods for variant/conflict violations.
///
/// `all_folders` is used to resolve conflict references by `author:slug` id;
/// `enabled` is the set of folders enabled in the profile.
pub fn check(mods_dir: &Path, all_folders: &[String], enabled: &[String]) -> Vec<Violation> {
    let enabled_set: HashSet<&str> = enabled.iter().map(|s| s.as_str()).collect();

    // id -> folder, so conflicts can reference a mod by its stable id.
    let mut by_id: HashMap<String, String> = HashMap::new();
    for folder in all_folders {
        if let Ok(Some(meta)) = crate::meta::read_mod_meta(mods_dir, folder) {
            if let Some(id) = meta.id {
                by_id.insert(crate::meta::normalize_mod_id(&id), folder.clone());
            }
        }
    }

    let mut violations = Vec::new();
    let mut groups: HashMap<String, Vec<String>> = HashMap::new();
    let mut seen: HashSet<(String, String)> = HashSet::new();

    for folder in enabled {
        let Ok(Some(meta)) = crate::meta::read_mod_meta(mods_dir, folder) else {
            continue;
        };
        if let Some(variant) = &meta.variant {
            groups
                .entry(variant.group.clone())
                .or_default()
                .push(folder.clone());
        }
        for reference in &meta.conflicts {
            let normalized = crate::meta::normalize_mod_id(reference);
            let target = if crate::meta::valid_mod_id(&normalized) {
                by_id.get(&normalized).cloned()
            } else {
                None
            }
            .or_else(|| {
                all_folders
                    .iter()
                    .find(|f| f.as_str() == reference.trim())
                    .cloned()
            });
            let Some(target) = target else { continue };
            if &target == folder || !enabled_set.contains(target.as_str()) {
                continue;
            }
            let mut pair = [folder.clone(), target];
            pair.sort();
            if seen.insert((pair[0].clone(), pair[1].clone())) {
                violations.push(Violation {
                    kind: Kind::Conflict,
                    message: format!(
                        "'{}' es incompatible con '{}' (ambos activados).",
                        pair[0], pair[1]
                    ),
                });
            }
        }
    }

    for (group, mut members) in groups {
        if members.len() > 1 {
            members.sort();
            violations.push(Violation {
                kind: Kind::Variant,
                message: format!(
                    "Varias variantes de '{}' activadas a la vez: {}.",
                    group,
                    members.join(", ")
                ),
            });
        }
    }

    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gta-mo-constr-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_variant_and_conflict() {
        let dir = tmp_dir("main");
        for folder in ["A", "B", "C"] {
            std::fs::create_dir_all(dir.join(folder)).unwrap();
        }
        std::fs::write(
            dir.join("A/mod.toml"),
            "id = \"x:a\"\n[variant]\ngroup = \"fam\"\nname = \"one\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("B/mod.toml"),
            "id = \"x:b\"\n[variant]\ngroup = \"fam\"\nname = \"two\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("C/mod.toml"),
            "id = \"x:c\"\nconflicts = [\"x:a\"]\n",
        )
        .unwrap();

        let all = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let v = check(&dir, &all, &["A".to_string(), "B".to_string()]);
        assert_eq!(v.iter().filter(|x| x.kind == Kind::Variant).count(), 1);

        let v = check(&dir, &all, &["A".to_string(), "C".to_string()]);
        assert_eq!(v.iter().filter(|x| x.kind == Kind::Conflict).count(), 1);
        // one-way declaration still detected
        assert!(v[0].message.contains("A") && v[0].message.contains("C"));

        // Not co-enabled -> no violation.
        assert!(check(&dir, &all, &["A".to_string()]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
