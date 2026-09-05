use gta_mo_core::db::ModMetaCache;

#[derive(Debug, Clone)]
pub struct ProfileView {
    pub name: String,
    pub slug: String,
    pub is_active: bool,
    pub total: i64,
    pub enabled: i64,
}

#[derive(Debug, Clone)]
pub struct ModView {
    pub id: i64,
    pub folder: String,
    pub name: String,
    pub enabled: bool,
    pub order: i64,
    pub meta: ModMetaCache,
    pub groups: Vec<String>,
}

impl ModView {
    /// Lowercased haystack used by the free-text search.
    pub fn searchable(&self) -> String {
        let mut s = self.name.to_lowercase();
        s.push(' ');
        s.push_str(&self.folder.to_lowercase());
        s.push(' ');
        s.push_str(&self.meta.author.join(" ").to_lowercase());
        if let Some(id) = &self.meta.mod_id {
            s.push(' ');
            s.push_str(&id.to_lowercase());
        }
        if let Some(d) = &self.meta.description {
            s.push(' ');
            s.push_str(&d.to_lowercase());
        }
        s.push(' ');
        s.push_str(&self.meta.tags.join(" ").to_lowercase());
        s
    }
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub profiles: Vec<ProfileView>,
    pub mods: Vec<ModView>,
    pub active_slug: String,
    pub all_tags: Vec<String>,
    pub all_groups: Vec<String>,
    /// Enabled mods of the active profile in overlay priority order (top first).
    pub resolved: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    Order,
    Name,
    Folder,
    Author,
    Version,
    ModId,
    Status,
}

impl SortField {
    pub fn label(&self) -> &'static str {
        match self {
            SortField::Order => "Prioridad",
            SortField::Name => "Nombre",
            SortField::Folder => "Carpeta",
            SortField::Author => "Autor",
            SortField::Version => "Versión",
            SortField::ModId => "Mod ID",
            SortField::Status => "Estado",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Filters {
    pub search: String,
    pub tag: Option<String>,
    pub group: Option<String>,
    pub sort: SortField,
    pub desc: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            search: String::new(),
            tag: None,
            group: None,
            sort: SortField::Order,
            desc: true,
        }
    }
}

pub fn filter_and_sort(mods: &mut Vec<ModView>, filters: &Filters) {
    mods.retain(|m| {
        if let Some(tag) = &filters.tag {
            if !m.meta.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)) {
                return false;
            }
        }
        if let Some(group) = &filters.group {
            if !m.groups.iter().any(|g| g.eq_ignore_ascii_case(group)) {
                return false;
            }
        }
        let q = filters.search.trim().to_lowercase();
        if !q.is_empty() && !m.searchable().contains(&q) {
            return false;
        }
        true
    });

    let key = |m: &ModView| -> String {
        match filters.sort {
            SortField::Order => format!("{:010}", m.order),
            SortField::Name => m.name.to_lowercase(),
            SortField::Folder => m.folder.to_lowercase(),
            SortField::Author => m.meta.author.join(" ").to_lowercase(),
            SortField::Version => m.meta.version.clone().unwrap_or_default(),
            SortField::ModId => m.meta.mod_id.clone().unwrap_or_default().to_lowercase(),
            SortField::Status => format!("{}", m.enabled as u8),
        }
    };
    let desc = filters.desc;
    mods.sort_by(|a, b| {
        let ord = key(a).cmp(&key(b));
        if desc {
            ord.reverse()
        } else {
            ord
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mod_view(id: i64, name: &str, folder: &str, enabled: bool, order: i64) -> ModView {
        let mut meta = ModMetaCache::default();
        meta.author = vec!["a".into()];
        meta.tags = vec!["essential".into()];
        ModView {
            id,
            folder: folder.into(),
            name: name.into(),
            enabled,
            order,
            meta,
            groups: vec!["Graphics".into()],
        }
    }

    #[test]
    fn filters_by_search_tag_and_group() {
        let mut mods = vec![
            mod_view(1, "Alpha", "alpha", true, 10),
            mod_view(2, "Beta", "beta", false, 20),
        ];
        let mut f = Filters::default();
        f.search = "ALPHA".into();
        filter_and_sort(&mut mods, &f);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].id, 1);

        let mut f = Filters::default();
        f.tag = Some("essential".into());
        filter_and_sort(&mut mods, &f);
        assert_eq!(mods.len(), 1);

        let mut f = Filters::default();
        f.group = Some("graphics".into());
        filter_and_sort(&mut mods, &f);
        assert_eq!(mods.len(), 1);
    }

    #[test]
    fn sorts_by_name_ascending() {
        let mut mods = vec![
            mod_view(1, "Beta", "b", true, 10),
            mod_view(2, "Alpha", "a", true, 20),
        ];
        let mut f = Filters::default();
        f.sort = SortField::Name;
        f.desc = false;
        filter_and_sort(&mut mods, &f);
        assert_eq!(mods[0].name, "Alpha");
        assert_eq!(mods[1].name, "Beta");
    }
}
