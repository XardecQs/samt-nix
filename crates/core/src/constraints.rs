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
    /// A mod's `requires_any` alternatives are all missing/disabled.
    Requires,
}

/// One violated constraint.
#[derive(Debug, Clone)]
pub struct Violation {
    pub kind: Kind,
    pub message: String,
}

/// Resolves a manifest reference (by `author:slug` id or by folder name).
fn resolve_ref(
    reference: &str,
    by_id: &HashMap<String, String>,
    folders: &[String],
) -> Option<String> {
    let normalized = crate::meta::normalize_mod_id(reference);
    if crate::meta::valid_mod_id(&normalized) {
        if let Some(folder) = by_id.get(&normalized) {
            return Some(folder.clone());
        }
    }
    let trimmed = reference.trim();
    folders.iter().find(|f| f.as_str() == trimmed).cloned()
}

/// Checks the enabled mods for variant/conflict/requires-any violations.
///
/// `all_folders` is used to resolve references by `author:slug` id; `enabled`
/// is the set of folders enabled in the profile.
pub fn check(mods_dir: &Path, all_folders: &[String], enabled: &[String]) -> Vec<Violation> {
    let enabled_set: HashSet<&str> = enabled.iter().map(|s| s.as_str()).collect();

    // id -> folder, so references can use the stable id.
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

        // Conflicts: report a pair once (one-way declaration is enough).
        for reference in &meta.conflicts {
            let Some(target) = resolve_ref(reference, &by_id, all_folders) else {
                continue;
            };
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

        // requires_any: at least one alternative must be enabled.
        if !meta.requires_any.is_empty() {
            let any = meta.requires_any.iter().any(|reference| {
                resolve_ref(reference, &by_id, all_folders)
                    .map(|f| enabled_set.contains(f.as_str()))
                    .unwrap_or(false)
            });
            if !any {
                violations.push(Violation {
                    kind: Kind::Requires,
                    message: format!(
                        "'{folder}' requiere al menos uno de: {} (ninguno activado).",
                        meta.requires_any.join(", ")
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

    #[test]
    fn requires_any_needs_one_alternative() {
        let dir = tmp_dir("reqany");
        for folder in ["Needs", "Alt1", "Alt2"] {
            std::fs::create_dir_all(dir.join(folder)).unwrap();
        }
        std::fs::write(
            dir.join("Needs/mod.toml"),
            "id = \"x:needs\"\nrequires_any = [\"x:alt1\", \"Alt2\"]\n",
        )
        .unwrap();
        std::fs::write(dir.join("Alt1/mod.toml"), "id = \"x:alt1\"\n").unwrap();
        std::fs::write(dir.join("Alt2/mod.toml"), "id = \"x:alt2\"\n").unwrap();

        let all = vec!["Needs".to_string(), "Alt1".to_string(), "Alt2".to_string()];
        // Nothing enabled -> violation.
        let v = check(&dir, &all, &["Needs".to_string()]);
        assert_eq!(v.iter().filter(|x| x.kind == Kind::Requires).count(), 1);

        // Any one alternative satisfies it.
        assert!(check(&dir, &all, &["Needs".to_string(), "Alt1".to_string()]).is_empty());
        assert!(check(&dir, &all, &["Needs".to_string(), "Alt2".to_string()]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
