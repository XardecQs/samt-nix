use gta_mo_core::db::{ModDepStatus, ModMetaCache};
use std::collections::HashMap;

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
    /// Expanded screenshot paths (relative to the mod folder), for the gallery.
    pub screenshots: Vec<String>,
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
    /// Effective overlay priority order of the enabled mods (top first).
    pub resolved: Vec<String>,
    /// (group name, members in the active profile).
    pub group_counts: Vec<(String, usize)>,
    /// Per-mod dependency health, keyed by mod id.
    pub dep_status: HashMap<i64, ModDepStatus>,
    /// Folder chains of every required-dependency cycle in the profile.
    pub dep_cycles: Vec<Vec<String>>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusFilter {
    All,
    Enabled,
    Disabled,
}

impl StatusFilter {
    pub fn label(&self) -> &'static str {
        match self {
            StatusFilter::All => "Todos",
            StatusFilter::Enabled => "Activados",
            StatusFilter::Disabled => "Desactivados",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Filters {
    pub search: String,
    pub tag: Option<String>,
    pub group: Option<String>,
    pub status: StatusFilter,
    pub sort: SortField,
    pub desc: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            search: String::new(),
            tag: None,
            group: None,
            status: StatusFilter::All,
            sort: SortField::Order,
            desc: true,
        }
    }
}

pub fn filter_and_sort(mods: &mut Vec<ModView>, filters: &Filters) {
    mods.retain(|m| {
        let status_ok = match filters.status {
            StatusFilter::All => true,
            StatusFilter::Enabled => m.enabled,
            StatusFilter::Disabled => !m.enabled,
        };
        if !status_ok {
            return false;
        }
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
        let meta = ModMetaCache {
            author: vec!["a".into()],
            tags: vec!["essential".into()],
            ..Default::default()
        };
        ModView {
            id,
            folder: folder.into(),
            name: name.into(),
            enabled,
            order,
            meta,
            groups: vec!["Graphics".into()],
            screenshots: vec![],
        }
    }

    #[test]
    fn filters_by_search_tag_and_group() {
        let mut mods = vec![
            mod_view(1, "Alpha", "alpha", true, 10),
            mod_view(2, "Beta", "beta", false, 20),
        ];
        let f = Filters {
            search: "ALPHA".into(),
            ..Default::default()
        };
        filter_and_sort(&mut mods, &f);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].id, 1);

        let f = Filters {
            tag: Some("essential".into()),
            ..Default::default()
        };
        filter_and_sort(&mut mods, &f);
        assert_eq!(mods.len(), 1);

        let f = Filters {
            group: Some("graphics".into()),
            ..Default::default()
        };
        filter_and_sort(&mut mods, &f);
        assert_eq!(mods.len(), 1);
    }

    #[test]
    fn sorts_by_name_ascending() {
        let mut mods = vec![
            mod_view(1, "Beta", "b", true, 10),
            mod_view(2, "Alpha", "a", true, 20),
        ];
        let f = Filters {
            sort: SortField::Name,
            desc: false,
            ..Default::default()
        };
        filter_and_sort(&mut mods, &f);
        assert_eq!(mods[0].name, "Alpha");
        assert_eq!(mods[1].name, "Beta");
    }
}
