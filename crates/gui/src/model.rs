use gta_mo_core::db::{ModDepStatus, ModMetaCache};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct ProfileView {
    pub name: String,
    pub slug: String,
    pub is_active: bool,
    pub total: i64,
    pub enabled: i64,
    /// Total playtime recorded for this profile (seconds).
    pub played_seconds: i64,
}

/// Human-readable label for a mod origin (`external`, `own`, `generated`, `legacy`).
pub fn origin_label(origin: &str) -> &'static str {
    match origin {
        "external" => "Externo",
        "own" => "Propio",
        "generated" => "Generado",
        _ => "Legacy",
    }
}

#[derive(Debug, Clone)]
pub struct ModView {
    pub id: i64,
    pub folder: String,
    /// Path relative to the mods directory (`external/foo`).
    pub rel_path: String,
    /// `external`, `own`, `generated` or `legacy`.
    pub origin: String,
    pub name: String,
    pub enabled: bool,
    pub order: i64,
    pub meta: ModMetaCache,
    pub groups: Vec<String>,
    /// Expanded screenshot paths (relative to the mod folder), for the gallery.
    pub screenshots: Vec<String>,
    /// Whether the mod folder has a `mod.toml` manifest on disk.
    pub has_manifest: bool,
    /// Mutually exclusive family of alternatives (`[family]`).
    pub family_id: Option<String>,
    pub family_name: Option<String>,
    /// Incompatible mods (by id or folder).
    pub conflicts: Vec<String>,
    /// Mod Loader priority, when declared.
    pub modloader_priority: Option<i64>,
    /// Effective hidden state (profile override, else global flag).
    pub hidden: bool,
    /// Global hidden flag (`mods.hidden`).
    pub hidden_global: bool,
    /// Notes for this mod in the active profile.
    pub notes: Option<String>,
    /// Selectable option groups (manifest + `options/<group>/<value>/`).
    pub option_groups: Vec<gta_mo_core::meta::ModOption>,
    /// Selected option values in the active profile (`{group: value}`).
    pub selected_options: HashMap<String, String>,
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
    pub all_origins: Vec<String>,
    /// Logical folder name -> relative path on disk.
    pub mod_paths: gta_mo_core::layout::ModPaths,
    /// Effective overlay priority order of the enabled mods (top first).
    pub resolved: Vec<String>,
    /// (group name, members in the active profile).
    pub group_counts: Vec<(String, usize)>,
    /// Per-mod dependency health, keyed by mod id.
    pub dep_status: HashMap<i64, ModDepStatus>,
    /// Folder chains of every required-dependency cycle in the profile.
    pub dep_cycles: Vec<Vec<String>>,
    /// Family (`id`) -> members `(mod id, folder, name)`, for navigation.
    pub family_members: HashMap<String, Vec<(i64, String, String)>>,
    /// Option selections keyed by mod folder (`folder -> {group: value}`).
    pub options_by_folder: HashMap<String, HashMap<String, String>>,
    /// Dependency/relation edges for the graph view (`from` depends on `to`).
    pub edges: Vec<GEdge>,
    /// Global playtime across every profile (seconds).
    pub global_played_seconds: i64,
}

/// Kind of relation drawn by the dependency graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GEdgeKind {
    Required,
    Optional,
    Conflict,
    RequiresAny,
    Family,
}

/// One edge of the dependency graph (`from` -> `to`).
#[derive(Debug, Clone)]
pub struct GEdge {
    pub from: i64,
    pub to: i64,
    pub kind: GEdgeKind,
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
    pub author: Option<String>,
    pub origin: Option<String>,
    pub status: StatusFilter,
    pub sort: SortField,
    pub desc: bool,
    /// Include mods hidden in the active profile.
    pub show_hidden: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            search: String::new(),
            tag: None,
            group: None,
            author: None,
            origin: None,
            status: StatusFilter::All,
            sort: SortField::Order,
            desc: true,
            show_hidden: false,
        }
    }
}

pub fn filter_and_sort(mods: &mut Vec<ModView>, filters: &Filters) {
    mods.retain(|m| {
        if m.hidden && !filters.show_hidden {
            return false;
        }
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
        if let Some(author) = &filters.author {
            if !m.meta.author.iter().any(|a| a.eq_ignore_ascii_case(author)) {
                return false;
            }
        }
        if let Some(origin) = &filters.origin {
            if !m.origin.eq_ignore_ascii_case(origin) {
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

/// Whether a mod declares the `essential` tag (e.g. DXVK, Essential Pack).
fn is_essential(m: &ModView) -> bool {
    m.meta
        .tags
        .iter()
        .any(|t| t.eq_ignore_ascii_case("essential"))
}

/// Alphabetical key of a mod: visible name, then folder as a tie-breaker.
fn alpha_key(m: &ModView) -> (String, String) {
    (m.name.to_lowercase(), m.folder.to_lowercase())
}

/// Computes the automatic priority order of a profile (top first).
///
/// Disabled mods go on top, alphabetically. Enabled mods go below, also
/// alphabetically, but a dependency is never placed above a mod that requires
/// it, and mods tagged `essential` sink to the bottom (they are the ones other
/// mods depend on most often).
pub fn auto_organize_order(mods: &[ModView], edges: &[GEdge]) -> Vec<String> {
    let mut disabled: Vec<&ModView> = mods.iter().filter(|m| !m.enabled).collect();
    disabled.sort_by_key(|m| alpha_key(m));
    let mut order: Vec<String> = disabled.iter().map(|m| m.folder.clone()).collect();

    let enabled: HashMap<i64, &ModView> = mods
        .iter()
        .filter(|m| m.enabled)
        .map(|m| (m.id, m))
        .collect();

    // A dependency can only be emitted once every enabled mod that depends on
    // it has already been emitted, so it always ends up below its dependents.
    let mut dependents_of: HashMap<i64, Vec<i64>> = HashMap::new();
    let mut pending: HashMap<i64, usize> = HashMap::new();
    for e in edges {
        if !matches!(e.kind, GEdgeKind::Required | GEdgeKind::Optional) || e.from == e.to {
            continue;
        }
        if !enabled.contains_key(&e.from) || !enabled.contains_key(&e.to) {
            continue;
        }
        dependents_of.entry(e.to).or_default().push(e.from);
        *pending.entry(e.to).or_default() += 1;
    }

    // Min-heap: non-essential before essential, then alphabetical.
    let key_of = |m: &ModView| {
        (
            is_essential(m),
            m.name.to_lowercase(),
            m.folder.to_lowercase(),
            m.id,
        )
    };
    let mut ready: BinaryHeap<Reverse<(bool, String, String, i64)>> = BinaryHeap::new();
    for m in enabled.values() {
        if pending.get(&m.id).copied().unwrap_or(0) == 0 {
            ready.push(Reverse(key_of(m)));
        }
    }

    let mut visited: HashSet<i64> = HashSet::new();
    while let Some(Reverse((_, _, _, id))) = ready.pop() {
        if !visited.insert(id) {
            continue;
        }
        if let Some(m) = enabled.get(&id) {
            order.push(m.folder.clone());
        }
        if let Some(parents) = dependents_of.get(&id) {
            for parent in parents {
                let Some(count) = pending.get_mut(parent) else {
                    continue;
                };
                *count = count.saturating_sub(1);
                if *count == 0 && !visited.contains(parent) {
                    if let Some(m) = enabled.get(parent) {
                        ready.push(Reverse(key_of(m)));
                    }
                }
            }
        }
    }

    // A dependency cycle leaves nodes unreachable: append them alphabetically
    // (essential last) so nothing is dropped.
    let mut leftover: Vec<&ModView> = enabled
        .values()
        .copied()
        .filter(|m| !visited.contains(&m.id))
        .collect();
    leftover.sort_by_key(|m| key_of(m));
    order.extend(leftover.into_iter().map(|m| m.folder.clone()));
    order
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
            rel_path: folder.into(),
            origin: "legacy".into(),
            name: name.into(),
            enabled,
            order,
            meta,
            groups: vec!["Graphics".into()],
            screenshots: vec![],
            has_manifest: true,
            family_id: None,
            family_name: None,
            conflicts: vec![],
            modloader_priority: None,
            hidden: false,
            hidden_global: false,
            notes: None,
            option_groups: vec![],
            selected_options: HashMap::new(),
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
    fn filters_by_origin() {
        let mut mods = vec![
            mod_view(1, "Alpha", "alpha", true, 10),
            mod_view(2, "Beta", "beta", true, 20),
        ];
        mods[0].origin = "external".into();
        mods[1].origin = "own".into();
        let f = Filters {
            origin: Some("own".into()),
            ..Default::default()
        };
        filter_and_sort(&mut mods, &f);
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].id, 2);
        assert_eq!(origin_label("generated"), "Generado");
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

    fn plain_mod(id: i64, name: &str, folder: &str, enabled: bool) -> ModView {
        let mut m = mod_view(id, name, folder, enabled, 0);
        m.meta.tags.clear();
        m
    }

    fn dep_edge(from: i64, to: i64) -> GEdge {
        GEdge {
            from,
            to,
            kind: GEdgeKind::Required,
        }
    }

    #[test]
    fn auto_organize_puts_disabled_first_then_enabled_alphabetically() {
        let mods = vec![
            plain_mod(1, "Zeta", "zeta", true),
            plain_mod(2, "Beta", "beta", false),
            plain_mod(3, "Alpha", "alpha", true),
            plain_mod(4, "Gamma", "gamma", false),
        ];
        let order = auto_organize_order(&mods, &[]);
        assert_eq!(order, vec!["beta", "gamma", "alpha", "zeta"]);
    }

    #[test]
    fn auto_organize_places_dependencies_below_their_dependents() {
        let mods = vec![
            plain_mod(1, "Base", "base", true),
            plain_mod(2, "Zeta", "zeta", true),
            plain_mod(3, "Alpha", "alpha", true),
        ];
        // Alpha requires Base.
        let edges = vec![dep_edge(3, 1)];
        let order = auto_organize_order(&mods, &edges);
        assert_eq!(order, vec!["alpha", "zeta", "base"]);
    }

    #[test]
    fn auto_organize_sinks_essential_mods_to_the_bottom() {
        let mut essential = plain_mod(2, "B Essential", "b", true);
        essential.meta.tags = vec!["Essential".into()];
        let mods = vec![
            plain_mod(1, "A Mod", "a", true),
            essential,
            plain_mod(3, "C Mod", "c", true),
        ];
        let order = auto_organize_order(&mods, &[]);
        assert_eq!(order, vec!["a", "c", "b"]);
    }

    #[test]
    fn auto_organize_falls_back_on_dependency_cycles() {
        let mods = vec![
            plain_mod(1, "A", "a", true),
            plain_mod(2, "B", "b", true),
            plain_mod(3, "C", "c", true),
        ];
        // A <-> B cycle; C is unrelated.
        let edges = vec![dep_edge(1, 2), dep_edge(2, 1)];
        let order = auto_organize_order(&mods, &edges);
        assert_eq!(order, vec!["c", "a", "b"]);
    }
}
