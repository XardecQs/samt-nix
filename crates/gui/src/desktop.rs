//! Model for a per-profile freedesktop `.desktop` entry.

/// Editable fields of a `.desktop` entry (only the common `[Desktop Entry]`
/// keys; unknown keys are not preserved because we regenerate the whole file).
#[derive(Debug, Clone, Default)]
pub struct DesktopEntry {
    pub name: String,
    pub generic_name: String,
    pub comment: String,
    pub exec: String,
    pub icon: String,
    pub working_dir: String,
    pub terminal: bool,
    pub categories: String,
    pub startup_wm_class: String,
    pub no_display: bool,
}

impl DesktopEntry {
    /// Renders the `[Desktop Entry]` file. Empty optional fields are omitted.
    pub fn to_desktop_string(&self) -> String {
        fn clean(v: &str) -> String {
            v.replace(['\n', '\r'], " ").trim().to_string()
        }
        fn kv(out: &mut String, key: &str, value: String) {
            if !value.is_empty() {
                out.push_str(&format!("{key}={value}\n"));
            }
        }
        let mut out = String::from("[Desktop Entry]\nType=Application\nVersion=1.0\n");
        kv(&mut out, "Name", clean(&self.name));
        kv(&mut out, "GenericName", clean(&self.generic_name));
        kv(&mut out, "Comment", clean(&self.comment));
        kv(&mut out, "Exec", clean(&self.exec));
        kv(&mut out, "Icon", clean(&self.icon));
        kv(&mut out, "Path", clean(&self.working_dir));
        out.push_str(&format!("Terminal={}\n", self.terminal));
        kv(&mut out, "Categories", clean(&self.categories));
        kv(&mut out, "StartupWMClass", clean(&self.startup_wm_class));
        if self.no_display {
            out.push_str("NoDisplay=true\n");
        }
        out
    }

    /// Parses the keys we know from a `.desktop` file.
    pub fn parse(text: &str) -> Self {
        let mut e = DesktopEntry::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let k = k.trim();
            let v = v.trim().to_string();
            match k {
                "Name" => e.name = v,
                "GenericName" => e.generic_name = v,
                "Comment" => e.comment = v,
                "Exec" => e.exec = v,
                "Icon" => e.icon = v,
                "Path" => e.working_dir = v,
                "Terminal" => e.terminal = v.eq_ignore_ascii_case("true"),
                "Categories" => e.categories = v,
                "StartupWMClass" => e.startup_wm_class = v,
                "NoDisplay" => e.no_display = v.eq_ignore_ascii_case("true"),
                _ => {}
            }
        }
        e
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_known_fields() {
        let e = DesktopEntry {
            name: "Principal (GTA: SA)".into(),
            comment: "Lanzar".into(),
            exec: "gta-mo-gui --launch principal".into(),
            icon: "gta-mo-gui".into(),
            terminal: false,
            categories: "Game;".into(),
            startup_wm_class: "gta_sa.exe".into(),
            ..Default::default()
        };
        let text = e.to_desktop_string();
        assert!(text.starts_with("[Desktop Entry]\nType=Application\n"));
        assert!(text.contains("Name=Principal (GTA: SA)\n"));
        assert!(text.contains("Exec=gta-mo-gui --launch principal\n"));
        assert!(text.contains("Terminal=false\n"));
        assert!(!text.contains("GenericName")); // empty omitted
        let back = DesktopEntry::parse(&text);
        assert_eq!(back.name, e.name);
        assert_eq!(back.exec, e.exec);
        assert_eq!(back.startup_wm_class, e.startup_wm_class);
        assert!(!back.terminal);
    }

    #[test]
    fn parse_ignores_unknown_and_sections() {
        let e = DesktopEntry::parse("[Desktop Entry]\nFoo=bar\nIcon=x\nNoDisplay=true\n");
        assert_eq!(e.icon, "x");
        assert!(e.no_display);
    }
}
