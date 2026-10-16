//! Lightweight access to the system (freedesktop) icon theme: a cached index of
//! icon names and their files, used by the `.desktop` editor and future ones.
//!
//! Best-effort and offline: reads `index.theme`/directory listings from the
//! active theme's `Inherits` chain (plus `hicolor`). SVGs are indexed but only
//! raster files can be previewed today.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// `(name, file)` pairs for every icon in the active theme chain, deduplicated
/// by name (a raster file wins over an SVG for previewing).
pub fn index() -> &'static [(String, PathBuf)] {
    static INDEX: OnceLock<Vec<(String, PathBuf)>> = OnceLock::new();
    INDEX.get_or_init(build_index)
}

/// Resolves an icon name (or an absolute path) to a file.
pub fn resolve(name: &str) -> Option<PathBuf> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    if let Some((_, p)) = index().iter().find(|(n, _)| n == name) {
        return Some(p.clone());
    }
    let p = PathBuf::from(name);
    p.is_file().then_some(p)
}

fn build_index() -> Vec<(String, PathBuf)> {
    let mut map: BTreeMap<String, PathBuf> = BTreeMap::new();
    for theme in theme_chain() {
        for base in icon_dirs().into_iter().map(|d| d.join(&theme)) {
            if !base.is_dir() {
                continue;
            }
            walk(&base, &mut |path| {
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.to_ascii_lowercase());
                let is_raster = matches!(
                    ext.as_deref(),
                    Some("png" | "jpg" | "jpeg" | "webp" | "bmp")
                );
                let is_svg = matches!(ext.as_deref(), Some("svg"));
                if !is_raster && !is_svg {
                    return;
                }
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    return;
                };
                match map.get(stem) {
                    None => {
                        map.insert(stem.to_string(), path.to_path_buf());
                    }
                    Some(existing) => {
                        // Prefer a raster over an SVG for previewing.
                        let existing_svg = existing
                            .extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.eq_ignore_ascii_case("svg"))
                            .unwrap_or(false);
                        if is_raster && existing_svg {
                            map.insert(stem.to_string(), path.to_path_buf());
                        }
                    }
                }
            });
        }
    }
    map.into_iter().collect()
}

fn walk(root: &Path, f: &mut impl FnMut(&Path)) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                f(&path);
            }
        }
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".local/share")))
}

fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home().map(|h| h.join(".config")))
}

/// Candidate icon directories (XDG data dirs + the NixOS system profile).
fn icon_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(data) = data_home() {
        dirs.push(data.join("icons"));
    }
    let xdg = std::env::var("XDG_DATA_DIRS")
        .unwrap_or_else(|_| "/usr/local/share:/usr/share".to_string());
    for d in xdg.split(':').filter(|s| !s.is_empty()) {
        dirs.push(PathBuf::from(d).join("icons"));
    }
    dirs.push(PathBuf::from("/run/current-system/sw/share/icons"));
    dirs
}

fn push_unique(chain: &mut Vec<String>, name: &str) {
    let name = name.trim().trim_matches('"').trim_matches('\'');
    if !name.is_empty() && !chain.iter().any(|c| c == name) {
        chain.push(name.to_string());
    }
}

/// The active theme plus its `Inherits` chain and `hicolor`.
fn theme_chain() -> Vec<String> {
    let mut chain: Vec<String> = Vec::new();
    push_unique(&mut chain, &active_theme());
    // Follow `Inherits=` in each theme's `index.theme`.
    let mut i = 0;
    while i < chain.len() {
        let name = chain[i].clone();
        for base in icon_dirs().into_iter().map(|d| d.join(&name)) {
            if let Ok(text) = std::fs::read_to_string(base.join("index.theme")) {
                for line in text.lines() {
                    if let Some(v) = line.trim().strip_prefix("Inherits=") {
                        for parent in v.split(',') {
                            push_unique(&mut chain, parent);
                        }
                    }
                }
            }
        }
        i += 1;
    }
    push_unique(&mut chain, "hicolor");
    chain
}

fn active_theme() -> String {
    if let Ok(out) = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout)
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    // GTK fallback.
    if let Some(home) = config_home() {
        if let Ok(text) = std::fs::read_to_string(home.join("gtk-3.0/settings.ini")) {
            for line in text.lines() {
                if let Some(v) = line.trim().strip_prefix("gtk-icon-theme-name=") {
                    return v.trim().to_string();
                }
            }
        }
    }
    "hicolor".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_empty_is_none() {
        assert!(resolve("").is_none());
        assert!(resolve("   ").is_none());
    }

    #[test]
    fn resolve_absolute_path() {
        let p = std::env::temp_dir().join(format!("gta-mo-icon-{}.png", std::process::id()));
        std::fs::write(&p, b"x").unwrap();
        assert_eq!(resolve(p.to_str().unwrap()), Some(p.clone()));
        let _ = std::fs::remove_file(&p);
    }
}
