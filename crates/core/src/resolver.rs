use crate::db::{DepRef, ModEntry};
use std::collections::{HashMap, HashSet};

pub struct DepGraph {
    pub mods: HashMap<i64, ModEntry>,
    pub deps: HashMap<i64, Vec<i64>>,
    pub optional_deps: HashMap<i64, Vec<i64>>,
    pub enabled_ids: Vec<i64>,
    pub prompt: DepPrompt,
    skip_ids: HashSet<i64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DepPrompt {
    Prompt,
    AutoEnable,
    Ignore,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CycleState {
    Visiting,
    Visited,
}

impl DepGraph {
    pub fn new(
        mods: HashMap<i64, ModEntry>,
        deps: HashMap<i64, Vec<DepRef>>,
        enabled_ids: Vec<i64>,
    ) -> Self {
        let mut required: HashMap<i64, Vec<i64>> = HashMap::new();
        let mut optional: HashMap<i64, Vec<i64>> = HashMap::new();
        for (mid, refs) in deps {
            for r in refs {
                if r.required {
                    required.entry(mid).or_default().push(r.id);
                } else {
                    optional.entry(mid).or_default().push(r.id);
                }
            }
        }
        Self {
            mods,
            deps: required,
            optional_deps: optional,
            enabled_ids,
            prompt: DepPrompt::Prompt,
            skip_ids: HashSet::new(),
        }
    }

    pub fn validate_dependencies(&self) -> bool {
        let mut ok = true;
        let mod_ids: HashSet<i64> = self.mods.keys().copied().collect();

        for (mid, dep_ids) in &self.deps {
            for did in dep_ids {
                if !mod_ids.contains(did) {
                    let mod_name = self
                        .mods
                        .get(mid)
                        .map(|m| m.folder_name.as_str())
                        .unwrap_or("?");
                    crate::log::error(format!(
                        "'{mod_name}' depende del mod con id={did}, que no existe en la base de datos."
                    ));
                    ok = false;
                }
            }
        }

        for (mid, dep_ids) in &self.optional_deps {
            for did in dep_ids {
                if !mod_ids.contains(did) {
                    let mod_name = self
                        .mods
                        .get(mid)
                        .map(|m| m.folder_name.as_str())
                        .unwrap_or("?");
                    crate::log::warn(format!(
                        "'{mod_name}' recomienda el mod con id={did}, que no existe en la base de datos."
                    ));
                }
            }
        }

        ok
    }

    pub fn detect_cycles(&self) -> bool {
        let mut state: HashMap<i64, CycleState> = HashMap::new();
        let mut ok = true;

        let all_ids: Vec<i64> = self.deps.keys().copied().collect();
        for mid in all_ids {
            if state.contains_key(&mid) {
                continue;
            }
            if !self.dfs_cycle_check(mid, &mut state, &mut String::new()) {
                ok = false;
            }
        }

        ok
    }

    fn dfs_cycle_check(
        &self,
        mid: i64,
        state: &mut HashMap<i64, CycleState>,
        path: &mut String,
    ) -> bool {
        state.insert(mid, CycleState::Visiting);

        if let Some(dep_ids) = self.deps.get(&mid) {
            for did in dep_ids {
                match state.get(did) {
                    Some(CycleState::Visiting) => {
                        let folder = self
                            .mods
                            .get(did)
                            .map(|m| m.folder_name.as_str())
                            .unwrap_or("?");
                        crate::log::error(format!(
                            "Ciclo de dependencias detectado: {path}{folder} -> {folder}"
                        ));
                        return false;
                    }
                    None => {
                        let folder = self
                            .mods
                            .get(did)
                            .map(|m| m.folder_name.as_str())
                            .unwrap_or("?");
                        path.push_str(folder);
                        path.push_str(" -> ");
                        if !self.dfs_cycle_check(*did, state, path) {
                            return false;
                        }
                    }
                    _ => {}
                }
            }
        }

        state.insert(mid, CycleState::Visited);
        true
    }

    pub fn check_disabled_deps(&self) -> Vec<(i64, i64)> {
        let mut disabled: Vec<(i64, i64)> = Vec::new();

        for mid in &self.enabled_ids {
            if let Some(dep_ids) = self.deps.get(mid) {
                for did in dep_ids {
                    if let Some(m) = self.mods.get(did) {
                        if !m.enabled {
                            disabled.push((*mid, *did));
                        }
                    }
                }
            }
        }

        disabled
    }

    pub fn enable_recursive(&mut self, did: i64) {
        if let Some(m) = self.mods.get_mut(&did) {
            if m.enabled {
                return;
            }
            m.enabled = true;
            if !self.enabled_ids.contains(&did) {
                self.enabled_ids.push(did);
            }
            crate::log::info(format!("    [+] Activado: {}", m.folder_name));

            let sub_deps: Vec<i64> = self.deps.get(&did).cloned().unwrap_or_default();
            for sub in sub_deps {
                self.enable_recursive(sub);
            }
        }
    }

    pub fn enable_mods_for_deps(&mut self) -> anyhow::Result<()> {
        let disabled_deps = self.check_disabled_deps();
        if disabled_deps.is_empty() {
            return Ok(());
        }

        if self.prompt == DepPrompt::Prompt {
            eprintln!();
            eprintln!("[!] Se detectaron dependencias deshabilitadas:");
            for (mid, did) in &disabled_deps {
                let mod_name = self.mod_folder(*mid);
                let dep_name = self.mod_folder(*did);
                eprintln!("    - '{mod_name}' requiere '{dep_name}' (deshabilitado)");
            }
            eprintln!();
            eprintln!("Opciones:");
            eprintln!("  1) Activar dependencias (incluyendo transitivas) y continuar");
            eprintln!("  2) Continuar sin las dependencias (ignorar)");
            eprintln!("  3) Cancelar");
            eprint!("Elige una opción [1-3]: ");

            let mut input = String::new();
            std::io::Write::flush(&mut std::io::stderr()).ok();
            std::io::stdin().read_line(&mut input).ok();
            let choice = input.trim().to_string();
            self.apply_dep_choice(&disabled_deps, choice.as_str())?;
        } else if self.prompt == DepPrompt::AutoEnable {
            crate::log::info(
                "Activando dependencias deshabilitadas automáticamente (--deps-enable)...",
            );
            self.apply_dep_choice(&disabled_deps, "1")?;
        } else {
            crate::log::warn(
                "Ignorando dependencias deshabilitadas (--deps-ignore). Puede que el juego falle.",
            );
            self.apply_dep_choice(&disabled_deps, "2")?;
        }
        Ok(())
    }

    fn apply_dep_choice(
        &mut self,
        disabled_deps: &[(i64, i64)],
        choice: &str,
    ) -> anyhow::Result<()> {
        match choice {
            "1" => {
                for (_, did) in disabled_deps {
                    self.enable_recursive(*did);
                }
                eprintln!();
                Ok(())
            }
            "2" => {
                for (_, did) in disabled_deps {
                    self.skip_ids.insert(*did);
                }
                eprintln!();
                Ok(())
            }
            "3" => anyhow::bail!("Cancelado."),
            _ => anyhow::bail!("Opción inválida. Cancelando."),
        }
    }

    pub fn warn_optional_deps(&self) {
        for mid in &self.enabled_ids {
            if let Some(dep_ids) = self.optional_deps.get(mid) {
                let mod_name = self.mod_folder(*mid);
                for did in dep_ids {
                    match self.mods.get(did) {
                        Some(m) if m.enabled => {}
                        Some(m) => crate::log::warn(format!(
                            "'{mod_name}' recomienda '{}', pero está desactivado.",
                            m.folder_name
                        )),
                        None => crate::log::warn(format!(
                            "'{mod_name}' recomienda el mod con id={did}, que no está instalado."
                        )),
                    }
                }
            }
        }
    }

    pub fn resolve(&self) -> Vec<String> {
        let mut visited: HashSet<i64> = HashSet::new();
        let mut resolved: Vec<String> = Vec::new();

        let dependency_of: HashSet<i64> = self
            .enabled_ids
            .iter()
            .flat_map(|mid| {
                self.deps
                    .get(mid)
                    .into_iter()
                    .chain(self.optional_deps.get(mid))
                    .flatten()
                    .copied()
            })
            .collect();

        for mid in &self.enabled_ids {
            if dependency_of.contains(mid) {
                continue;
            }
            self.dfs_resolve(*mid, &mut visited, &mut resolved);
        }

        resolved
    }

    fn dfs_resolve(&self, mid: i64, visited: &mut HashSet<i64>, resolved: &mut Vec<String>) {
        if visited.contains(&mid) {
            return;
        }
        if self.skip_ids.contains(&mid) {
            return;
        }

        visited.insert(mid);

        if let Some(m) = self.mods.get(&mid) {
            resolved.push(m.folder_name.clone());
        }

        if let Some(dep_ids) = self.deps.get(&mid) {
            let mut sorted_deps: Vec<(i64, i64)> = dep_ids
                .iter()
                .filter_map(|did| self.mods.get(did).map(|m| (m.load_order, *did)))
                .collect();
            sorted_deps.sort_by_key(|b| std::cmp::Reverse(b.0));

            for (_, did) in sorted_deps {
                self.dfs_resolve(did, visited, resolved);
            }
        }

        if let Some(opt_ids) = self.optional_deps.get(&mid) {
            let mut sorted_opt: Vec<(i64, i64)> = opt_ids
                .iter()
                .filter_map(|did| {
                    self.mods
                        .get(did)
                        .filter(|m| m.enabled)
                        .map(|m| (m.load_order, *did))
                })
                .collect();
            sorted_opt.sort_by_key(|b| std::cmp::Reverse(b.0));

            for (_, did) in sorted_opt {
                self.dfs_resolve(did, visited, resolved);
            }
        }
    }

    fn mod_folder(&self, id: i64) -> &str {
        self.mods
            .get(&id)
            .map(|m| m.folder_name.as_str())
            .unwrap_or("?")
    }

    pub fn sync_enabled_to_db(
        &self,
        conn: &rusqlite::Connection,
        profile_id: i64,
    ) -> anyhow::Result<()> {
        for m in self.mods.values() {
            crate::db::set_mod_enabled(conn, profile_id, m.id, m.enabled)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{DepRef, ModEntry};

    fn mods(entries: &[(i64, &str, bool, i64)]) -> HashMap<i64, ModEntry> {
        entries
            .iter()
            .map(|&(id, folder, enabled, order)| ModEntry {
                id,
                folder_name: folder.to_string(),
                name: folder.to_string(),
                enabled,
                load_order: order,
            })
            .map(|m| (m.id, m))
            .collect()
    }

    fn deps(entries: &[(i64, i64, bool)]) -> HashMap<i64, Vec<DepRef>> {
        let mut map: HashMap<i64, Vec<DepRef>> = HashMap::new();
        for &(a, b, req) in entries {
            map.entry(a).or_default().push(DepRef {
                id: b,
                required: req,
            });
        }
        map
    }

    #[test]
    fn resolve_puts_mod_above_its_deps() {
        let m = mods(&[(1, "mod", true, 30), (2, "dep", true, 10)]);
        let d = deps(&[(1, 2, true)]);
        let g = DepGraph::new(m, d, vec![1, 2]);
        assert_eq!(g.resolve(), vec!["mod".to_string(), "dep".to_string()]);
    }

    #[test]
    fn detect_cycles_finds_cycles() {
        let m = mods(&[(1, "a", true, 10), (2, "b", true, 20)]);
        let d = deps(&[(1, 2, true), (2, 1, true)]);
        let g = DepGraph::new(m, d, vec![1, 2]);
        assert!(!g.detect_cycles());
    }

    #[test]
    fn no_cycle_is_ok() {
        let m = mods(&[(1, "a", true, 10), (2, "b", true, 20), (3, "c", true, 30)]);
        let d = deps(&[(1, 2, true), (2, 3, true)]);
        let g = DepGraph::new(m, d, vec![1, 2, 3]);
        assert!(g.detect_cycles());
    }

    #[test]
    fn optional_deps_only_included_if_enabled() {
        let m = mods(&[
            (1, "mod", true, 30),
            (2, "opt_on", true, 20),
            (3, "opt_off", false, 10),
        ]);
        let d = deps(&[(1, 2, false), (1, 3, false)]);
        let g = DepGraph::new(m, d, vec![1]);
        let out = g.resolve();
        assert!(out.contains(&"opt_on".to_string()));
        assert!(!out.contains(&"opt_off".to_string()));
    }

    #[test]
    fn disabled_deps_are_reported() {
        let m = mods(&[(1, "mod", true, 30), (2, "needed", false, 10)]);
        let d = deps(&[(1, 2, true)]);
        let g = DepGraph::new(m, d, vec![1]);
        let disabled = g.check_disabled_deps();
        assert_eq!(disabled, vec![(1, 2)]);
    }

    #[test]
    fn enable_recursive_activates_transitive_deps() {
        let mut m = mods(&[
            (1, "mod", true, 30),
            (2, "mid", false, 20),
            (3, "base", false, 10),
        ]);
        let d = deps(&[(1, 2, true), (2, 3, true)]);
        let mut g = DepGraph::new(m, d, vec![1]);
        g.enable_recursive(2);
        assert!(g.mods.get(&2).unwrap().enabled);
        assert!(g.mods.get(&3).unwrap().enabled);
        assert!(g.enabled_ids.contains(&2));
        assert!(g.enabled_ids.contains(&3));
    }

    #[test]
    fn skip_ids_are_excluded_from_resolve() {
        let m = mods(&[(1, "mod", true, 30), (2, "broken", true, 10)]);
        let d = deps(&[(1, 2, true)]);
        let mut g = DepGraph::new(m, d, vec![1, 2]);
        g.skip_ids.insert(2);
        assert_eq!(g.resolve(), vec!["mod".to_string()]);
    }
}
