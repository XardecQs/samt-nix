//! Structured (form) view of a `mod.toml` manifest.
//!
//! The form extracts the known fields from the manifest and, on save, applies
//! them onto the *original* `toml_edit::DocumentMut`, so comments, ordering and
//! unknown keys are preserved. `options`/`components` are edited with nested
//! sub-editors.

use gta_mo_core::meta::ModMeta;
use toml_edit::{value, Array, ArrayOfTables, DocumentMut, Item, Table};

#[derive(Debug, Clone, Default)]
pub struct FormChoice {
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, Default)]
pub struct FormOption {
    pub group: String,
    pub name: String,
    pub default: String,
    pub choices: Vec<FormChoice>,
}

#[derive(Debug, Clone, Default)]
pub struct FormComponent {
    pub name: String,
    pub version: String,
    pub author: String,
    pub url: String,
    pub path: String,
}

/// Editable view of the manifest's known fields.
#[derive(Debug, Clone, Default)]
pub struct ManifestForm {
    pub id: String,
    pub name: String,
    pub version: String,
    pub url: String,
    pub description: String,
    pub license: String,
    pub cover: String,
    pub authors: Vec<String>,
    pub urls: Vec<String>,
    pub tags: Vec<String>,
    pub guides: Vec<String>,
    pub screenshots: Vec<String>,
    pub mount: Vec<String>,
    pub conflicts: Vec<String>,
    pub requires_any: Vec<String>,
    pub family_id: String,
    pub family_name: String,
    pub dep_required: Vec<String>,
    pub dep_optional: Vec<String>,
    /// Kept as text so an empty/invalid value does not force a number.
    pub modloader_priority: String,
    pub modloader_folders: Vec<String>,
    pub options: Vec<FormOption>,
    pub components: Vec<FormComponent>,
}

impl ManifestForm {
    pub fn from_meta(m: &ModMeta) -> Self {
        let deps = m.dependencies.clone().unwrap_or_default();
        Self {
            id: m.id.clone().unwrap_or_default(),
            name: m.name.clone().unwrap_or_default(),
            version: m.version.clone().unwrap_or_default(),
            url: m.url.clone().unwrap_or_default(),
            description: m.description.clone().unwrap_or_default(),
            license: m.license.clone().unwrap_or_default(),
            cover: m.cover.clone().unwrap_or_default(),
            authors: m.author.clone(),
            urls: m.urls.clone(),
            tags: m.tags.clone().unwrap_or_default(),
            guides: m.guides.clone().unwrap_or_default(),
            screenshots: m.screenshots.clone().unwrap_or_default(),
            mount: m.mount.clone().unwrap_or_default(),
            conflicts: m.conflicts.clone(),
            requires_any: m.requires_any.clone(),
            family_id: m.family.as_ref().map(|f| f.id.clone()).unwrap_or_default(),
            family_name: m
                .family
                .as_ref()
                .and_then(|f| f.name.clone())
                .unwrap_or_default(),
            dep_required: deps.required,
            dep_optional: deps.optional,
            modloader_priority: m
                .modloader
                .as_ref()
                .and_then(|ml| ml.priority)
                .map(|p| p.to_string())
                .unwrap_or_default(),
            modloader_folders: m
                .modloader
                .as_ref()
                .and_then(|ml| ml.folders.clone())
                .unwrap_or_default(),
            options: m
                .options
                .iter()
                .flatten()
                .map(|o| FormOption {
                    group: o.group.clone(),
                    name: o.name.clone().unwrap_or_default(),
                    default: o.default.clone().unwrap_or_default(),
                    choices: o
                        .choices
                        .iter()
                        .map(|c| FormChoice {
                            key: c.key.clone(),
                            name: c.name.clone().unwrap_or_default(),
                        })
                        .collect(),
                })
                .collect(),
            components: m
                .components
                .iter()
                .flatten()
                .map(|c| FormComponent {
                    name: c.name.clone().unwrap_or_default(),
                    version: c.version.clone().unwrap_or_default(),
                    author: c.author.clone().unwrap_or_default(),
                    url: c.url.clone().unwrap_or_default(),
                    path: c.path.clone().unwrap_or_default(),
                })
                .collect(),
        }
    }

    /// Applies the form's values onto `doc`, preserving everything else.
    pub fn apply(&self, doc: &mut DocumentMut) {
        set_str(doc, "id", &self.id);
        set_str(doc, "name", &self.name);
        set_str(doc, "version", &self.version);
        set_str(doc, "url", &self.url);
        set_str(doc, "description", &self.description);
        set_str(doc, "license", &self.license);
        set_str(doc, "cover", &self.cover);
        set_list(doc, "author", &self.authors);
        set_list(doc, "urls", &self.urls);
        set_list(doc, "tags", &self.tags);
        set_list(doc, "guides", &self.guides);
        set_list(doc, "screenshots", &self.screenshots);
        set_list(doc, "mount", &self.mount);
        set_list(doc, "conflicts", &self.conflicts);
        set_list(doc, "requires_any", &self.requires_any);

        // [family]
        if self.family_id.trim().is_empty() {
            doc.as_table_mut().remove("family");
        } else {
            let t = ensure_table(doc, "family");
            t["id"] = value(self.family_id.trim().to_string());
            if self.family_name.trim().is_empty() {
                t.remove("name");
            } else {
                t["name"] = value(self.family_name.trim().to_string());
            }
        }

        // [dependencies]
        let req = clean(&self.dep_required);
        let opt = clean(&self.dep_optional);
        if req.is_empty() && opt.is_empty() {
            doc.as_table_mut().remove("dependencies");
        } else {
            let t = ensure_table(doc, "dependencies");
            set_table_list(t, "required", &req);
            set_table_list(t, "optional", &opt);
        }

        // [modloader]
        let prio = self.modloader_priority.trim();
        let folders = clean(&self.modloader_folders);
        if prio.is_empty() && folders.is_empty() {
            doc.as_table_mut().remove("modloader");
        } else {
            let t = ensure_table(doc, "modloader");
            match prio.parse::<i64>() {
                Ok(n) => {
                    t["priority"] = value(n);
                }
                Err(_) => {
                    t.remove("priority");
                }
            }
            set_table_list(t, "folders", &folders);
        }

        // [[options]]
        if self.options.is_empty() {
            doc.as_table_mut().remove("options");
        } else {
            let mut aot = ArrayOfTables::new();
            for o in &self.options {
                if o.group.trim().is_empty() {
                    continue;
                }
                let mut t = Table::new();
                t["group"] = value(o.group.trim().to_string());
                if !o.name.trim().is_empty() {
                    t["name"] = value(o.name.trim().to_string());
                }
                if !o.default.trim().is_empty() {
                    t["default"] = value(o.default.trim().to_string());
                }
                let choices: Vec<&FormChoice> = o
                    .choices
                    .iter()
                    .filter(|c| !c.key.trim().is_empty())
                    .collect();
                if !choices.is_empty() {
                    let mut caot = ArrayOfTables::new();
                    for c in choices {
                        let mut ct = Table::new();
                        ct["key"] = value(c.key.trim().to_string());
                        if !c.name.trim().is_empty() {
                            ct["name"] = value(c.name.trim().to_string());
                        }
                        caot.push(ct);
                    }
                    t["choices"] = Item::ArrayOfTables(caot);
                }
                aot.push(t);
            }
            doc["options"] = Item::ArrayOfTables(aot);
        }

        // [[components]]
        let comps: Vec<&FormComponent> = self
            .components
            .iter()
            .filter(|c| !c.name.trim().is_empty() || !c.path.trim().is_empty())
            .collect();
        if comps.is_empty() {
            doc.as_table_mut().remove("components");
        } else {
            let mut aot = ArrayOfTables::new();
            for c in comps {
                let mut t = Table::new();
                if !c.name.trim().is_empty() {
                    t["name"] = value(c.name.trim().to_string());
                }
                if !c.version.trim().is_empty() {
                    t["version"] = value(c.version.trim().to_string());
                }
                if !c.author.trim().is_empty() {
                    t["author"] = value(c.author.trim().to_string());
                }
                if !c.url.trim().is_empty() {
                    t["url"] = value(c.url.trim().to_string());
                }
                if !c.path.trim().is_empty() {
                    t["path"] = value(c.path.trim().to_string());
                }
                aot.push(t);
            }
            doc["components"] = Item::ArrayOfTables(aot);
        }
    }
}

fn clean(v: &[String]) -> Vec<String> {
    v.iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn array_of(items: &[String]) -> Item {
    let mut a = Array::new();
    for i in items {
        a.push(i.clone());
    }
    Item::Value(toml_edit::Value::Array(a))
}

fn set_str(doc: &mut DocumentMut, key: &str, v: &str) {
    if v.trim().is_empty() {
        doc.as_table_mut().remove(key);
    } else {
        doc[key] = value(v.trim().to_string());
    }
}

fn set_list(doc: &mut DocumentMut, key: &str, v: &[String]) {
    let items = clean(v);
    if items.is_empty() {
        doc.as_table_mut().remove(key);
    } else {
        doc[key] = array_of(&items);
    }
}

fn set_table_list(t: &mut Table, key: &str, v: &[String]) {
    if v.is_empty() {
        t.remove(key);
    } else {
        t[key] = array_of(v);
    }
}

fn ensure_table<'a>(doc: &'a mut DocumentMut, key: &str) -> &'a mut Table {
    let needs = !doc.get(key).map(|i| i.is_table()).unwrap_or(false);
    if needs {
        let mut t = Table::new();
        t.set_implicit(false);
        doc[key] = Item::Table(t);
    }
    doc[key].as_table_mut().expect("just ensured a table")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_preserves_comments_unknown_keys_and_order() {
        let src =
            "# cabecera\nid = \"a:b\"\nfoo = 1\nname = \"Viejo\"\n# nota\nmount = [\"content\"]\n";
        let meta: ModMeta = toml::from_str(src).unwrap();
        let form = ManifestForm::from_meta(&meta);
        let mut doc: DocumentMut = src.parse().unwrap();
        form.apply(&mut doc);
        let out = doc.to_string();
        assert!(out.contains("# cabecera"), "{out}");
        assert!(out.contains("# nota"), "{out}");
        assert!(out.contains("foo = 1"), "{out}");
        assert!(out.contains("name = \"Viejo\""), "{out}");
        // Parses back to the same manifest.
        let back: ModMeta = toml::from_str(&out).unwrap();
        assert_eq!(back.id.as_deref(), Some("a:b"));
        assert_eq!(back.mount.as_deref(), Some(&["content".to_string()][..]));
    }

    #[test]
    fn apply_writes_nested_tables() {
        let mut form = ManifestForm {
            id: "a:b".into(),
            name: "N".into(),
            family_id: "reload".into(),
            dep_required: vec!["x:y".into()],
            modloader_priority: "60".into(),
            ..Default::default()
        };
        form.options.push(FormOption {
            group: "variant".into(),
            default: "4k".into(),
            choices: vec![
                FormChoice {
                    key: "4k".into(),
                    name: String::new(),
                },
                FormChoice {
                    key: "lite".into(),
                    name: "Lite".into(),
                },
            ],
            ..Default::default()
        });
        form.components.push(FormComponent {
            name: "SilentPatch".into(),
            ..Default::default()
        });
        let mut doc: DocumentMut = String::new().parse().unwrap();
        form.apply(&mut doc);
        let out = doc.to_string();
        let back: ModMeta = toml::from_str(&out).unwrap();
        assert_eq!(back.family.as_ref().unwrap().id, "reload");
        assert_eq!(back.dependencies.as_ref().unwrap().required, vec!["x:y"]);
        assert_eq!(back.modloader.as_ref().unwrap().priority, Some(60));
        assert_eq!(back.options.as_ref().unwrap()[0].group, "variant");
        assert_eq!(back.options.as_ref().unwrap()[0].choices.len(), 2);
        assert_eq!(
            back.components.as_ref().unwrap()[0].name.as_deref(),
            Some("SilentPatch")
        );
    }

    #[test]
    fn apply_clears_empty_fields() {
        let src = "name = \"N\"\ntags = [\"a\"]\n[family]\nid = \"f\"\n";
        let mut form = ManifestForm {
            name: "N".into(),
            ..Default::default()
        };
        form.tags.clear();
        let mut doc: DocumentMut = src.parse().unwrap();
        form.apply(&mut doc);
        let out = doc.to_string();
        assert!(!out.contains("tags"));
        assert!(!out.contains("[family]"), "{out}");
    }
}
