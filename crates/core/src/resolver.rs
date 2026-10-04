use crate::db::{DepRef, ModEntry};
use std::collections::{BinaryHeap, HashMap, HashSet};

/// Maximum dependency-chain depth walked recursively. Guards against a stack
/// overflow from a pathological or maliciously deep dependency graph.
const MAX_DEP_DEPTH: usize = 4096;

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
            if !self.dfs_cycle_check(mid, &mut state, &mut Vec::new()) {
                ok = false;
            }
        }

        ok
    }

    fn dfs_cycle_check(
        &self,
        mid: i64,
        state: &mut HashMap<i64, CycleState>,
        path: &mut Vec<i64>,
    ) -> bool {
        if path.len() > MAX_DEP_DEPTH {
            if path.len() == MAX_DEP_DEPTH + 1 {
                crate::log::warn(
                    "Grafo de dependencias demasiado profundo; se omite la detección de ciclos.",
                );
            }
            return true;
        }
        state.insert(mid, CycleState::Visiting);
        path.push(mid);

        if let Some(dep_ids) = self.deps.get(&mid) {
            for did in dep_ids {
                match state.get(did).copied() {
                    Some(CycleState::Visiting) => {
                        let mut chain: Vec<&str> =
                            path.iter().map(|id| self.mod_folder(*id)).collect();
                        chain.push(self.mod_folder(*did));
                        crate::log::error(format!(
                            "Ciclo de dependencias detectado: {}",
                            chain.join(" -> ")
                        ));
                        return false;
                    }
                    None if !self.dfs_cycle_check(*did, state, path) => return false,
                    _ => {}
                }
            }
        }

        path.pop();
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
        self.enable_recursive_depth(did, 0);
    }

    fn enable_recursive_depth(&mut self, did: i64, depth: usize) {
        if depth > MAX_DEP_DEPTH {
            return;
        }
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
                self.enable_recursive_depth(sub, depth + 1);
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

    /// Topological order (top priority first) biased by `load_order`.
    ///
    /// A mod is only emitted once every enabled mod that depends on it has
    /// already been emitted (so a dependency is always layered *below* its
    /// dependents and the dependent's files win). Among the mods that are
    /// ready, the one with the highest `load_order` goes first, so unrelated
    /// dependencies are never hoisted above higher-priority mods just because
    /// a low-priority mod happens to require them.
    pub fn resolve(&self) -> Vec<String> {
        let enabled: HashSet<i64> = self.enabled_ids.iter().copied().collect();

        // dep -> enabled parents that depend on it; plus how many of those
        // parents are still pending (the dep cannot be emitted until 0).
        let mut dependents_of: HashMap<i64, Vec<i64>> = HashMap::new();
        let mut pending_dependents: HashMap<i64, usize> = HashMap::new();
        for mid in &self.enabled_ids {
            if self.skip_ids.contains(mid) {
                continue;
            }
            let required = self.deps.get(mid).into_iter().flatten().copied();
            let optional = self
                .optional_deps
                .get(mid)
                .into_iter()
                .flatten()
                .copied()
                .filter(|d| enabled.contains(d));
            for dep in required.chain(optional) {
                if self.skip_ids.contains(&dep) {
                    continue;
                }
                dependents_of.entry(dep).or_default().push(*mid);
                *pending_dependents.entry(dep).or_default() += 1;
            }
        }

        // Max-heap by (load_order, id): highest priority first.
        let mut ready: BinaryHeap<(i64, i64)> = BinaryHeap::new();
        for mid in &self.enabled_ids {
            if self.skip_ids.contains(mid) {
                continue;
            }
            if pending_dependents.get(mid).copied().unwrap_or(0) == 0 {
                if let Some(m) = self.mods.get(mid) {
                    ready.push((m.load_order, *mid));
                }
            }
        }

        let mut visited: HashSet<i64> = HashSet::new();
        let mut resolved: Vec<String> = Vec::new();

        while let Some((_, mid)) = ready.pop() {
            if !visited.insert(mid) {
                continue;
            }
            if let Some(m) = self.mods.get(&mid) {
                resolved.push(m.folder_name.clone());
            }
            if let Some(parents) = dependents_of.get(&mid) {
                for parent in parents {
                    let Some(count) = pending_dependents.get_mut(parent) else {
                        continue;
                    };
                    *count = count.saturating_sub(1);
                    if *count == 0 && !visited.contains(parent) {
                        if let Some(m) = self.mods.get(parent) {
                            ready.push((m.load_order, *parent));
                        }
                    }
                }
            }
        }

        // A dependency cycle would leave nodes unreachable: emit whatever is
        // left by priority so nothing is silently dropped (cycles are reported
        // separately by `detect_cycles`).
        let mut leftover: Vec<(i64, i64)> = self
            .enabled_ids
            .iter()
            .filter(|id| !visited.contains(id) && !self.skip_ids.contains(id))
            .filter_map(|id| self.mods.get(id).map(|m| (m.load_order, *id)))
            .collect();
        leftover.sort_by_key(|b| std::cmp::Reverse(*b));
        for (_, id) in leftover {
            if visited.insert(id) {
                if let Some(m) = self.mods.get(&id) {
                    resolved.push(m.folder_name.clone());
                }
            }
        }

        resolved
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

/// Resolves the overlay priority order (top first) of the enabled mods of a
/// profile, ignoring disabled/missing dependencies. Shared by the CLI and the
/// GUI so both agree on the effective order.
pub fn enabled_order_for_profile(
    conn: &rusqlite::Connection,
    profile_id: i64,
) -> anyhow::Result<Vec<String>> {
    let all_mods = crate::db::load_all_mods_for_profile(conn, profile_id)?;
    let mods_map = all_mods.into_iter().map(|m| (m.id, m)).collect();
    let deps = crate::db::load_dependencies(conn)?;
    let enabled_ids = crate::db::load_enabled_mod_ids_for_profile(conn, profile_id)?;
    let mut graph = DepGraph::new(mods_map, deps, enabled_ids);
    graph.prompt = DepPrompt::Ignore;
    let _ = graph.validate_dependencies();
    let _ = graph.detect_cycles();
    Ok(graph.resolve())
}

impl DepGraph {
    /// Returns each required-dependency cycle as a closed chain of mod ids
    /// (first == last). Used for diagnostics in `ctl health` and the GUI.
    pub fn find_cycles(&self) -> Vec<Vec<i64>> {
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Mark {
            InStack,
            Done,
        }

        fn dfs(
            graph: &HashMap<i64, Vec<i64>>,
            node: i64,
            marks: &mut HashMap<i64, Mark>,
            stack: &mut Vec<i64>,
            index: &mut HashMap<i64, usize>,
            out: &mut Vec<Vec<i64>>,
        ) {
            if stack.len() > MAX_DEP_DEPTH {
                return;
            }
            marks.insert(node, Mark::InStack);
            index.insert(node, stack.len());
            stack.push(node);
            if let Some(neighbors) = graph.get(&node) {
                for n in neighbors {
                    match marks.get(n) {
                        Some(Mark::InStack) => {
                            let start = index.get(n).copied().unwrap_or(0);
                            let mut chain = stack[start..].to_vec();
                            chain.push(*n);
                            out.push(chain);
                        }
                        Some(Mark::Done) => {}
                        None => dfs(graph, *n, marks, stack, index, out),
                    }
                }
            }
            stack.pop();
            index.remove(&node);
            marks.insert(node, Mark::Done);
        }

        let mut marks: HashMap<i64, Mark> = HashMap::new();
        let mut stack: Vec<i64> = Vec::new();
        let mut index: HashMap<i64, usize> = HashMap::new();
        let mut out: Vec<Vec<i64>> = Vec::new();
        let mut nodes: Vec<i64> = self.deps.keys().copied().collect();
        nodes.sort_unstable();
        for n in nodes {
            if !marks.contains_key(&n) {
                dfs(&self.deps, n, &mut marks, &mut stack, &mut index, &mut out);
            }
        }
        out
    }
}

/// Folder-name chains of every dependency cycle in a profile (empty when none).
pub fn dependency_cycles(
    conn: &rusqlite::Connection,
    profile_id: i64,
) -> anyhow::Result<Vec<Vec<String>>> {
    let deps = crate::db::load_dependencies(conn)?;
    dependency_cycles_with(conn, profile_id, deps)
}

/// Like [`dependency_cycles`] but with an explicit (e.g. live-read) dependency
/// map, so the GUI reflects `mod.toml` edits without a discover pass.
pub fn dependency_cycles_with(
    conn: &rusqlite::Connection,
    profile_id: i64,
    deps: HashMap<i64, Vec<crate::db::DepRef>>,
) -> anyhow::Result<Vec<Vec<String>>> {
    let all_mods = crate::db::load_all_mods_for_profile(conn, profile_id)?;
    let folder_of: HashMap<i64, String> = all_mods
        .iter()
        .map(|m| (m.id, m.folder_name.clone()))
        .collect();
    let mods_map = all_mods.into_iter().map(|m| (m.id, m)).collect();
    let enabled_ids = crate::db::load_enabled_mod_ids_for_profile(conn, profile_id)?;
    let graph = DepGraph::new(mods_map, deps, enabled_ids);
    Ok(graph
        .find_cycles()
        .into_iter()
        .map(|chain| {
            chain
                .into_iter()
                .map(|id| {
                    folder_of
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| format!("id={id}"))
                })
                .collect()
        })
        .collect())
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
                hidden_global: false,
                hidden_profile: None,
                notes: None,
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
    fn unrelated_low_priority_dep_is_not_hoisted() {
        // "a" (order 20) depends on "dep" (order 1); "b" (order 18) is
        // independent. The dependency must stay below its dependent but must
        // not jump above the higher-priority "b".
        let m = mods(&[(1, "dep", true, 1), (2, "b", true, 18), (3, "a", true, 20)]);
        let d = deps(&[(3, 1, true)]);
        let g = DepGraph::new(m, d, vec![3, 2, 1]);
        assert_eq!(
            g.resolve(),
            vec!["a".to_string(), "b".to_string(), "dep".to_string()]
        );
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
        // `enabled_ids` is the authoritative enabled set: opt_off is enabled in
        // its entry but disabled in the profile, so it must stay out.
        let g = DepGraph::new(m, d, vec![1, 2]);
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
        let m = mods(&[
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
