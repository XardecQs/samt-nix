use crate::db::{DepRef, ModEntry};
use std::collections::{HashMap, HashSet};

pub struct DepGraph {
    pub mods: HashMap<i64, ModEntry>,
    pub deps: HashMap<i64, Vec<i64>>,
    pub optional_deps: HashMap<i64, Vec<i64>>,
    pub enabled_ids: Vec<i64>,
    skip_ids: HashSet<i64>,
    has_errors: bool,
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
            skip_ids: HashSet::new(),
            has_errors: false,
        }
    }

    pub fn validate_dependencies(&mut self) -> bool {
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
                    crate::db::log::error(format!(
                        "'{mod_name}' depende del mod con id={did}, que no existe en la base de datos."
                    ));
                    ok = false;
                }
            }
        }

        if !ok {
            self.has_errors = true;
        }

        for (mid, dep_ids) in &self.optional_deps {
            for did in dep_ids {
                if !mod_ids.contains(did) {
                    let mod_name = self
                        .mods
                        .get(mid)
                        .map(|m| m.folder_name.as_str())
                        .unwrap_or("?");
                    crate::db::log::warn(format!(
                        "'{mod_name}' recomienda el mod con id={did}, que no existe en la base de datos."
                    ));
                }
            }
        }

        ok
    }

    pub fn detect_cycles(&mut self) -> bool {
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

        if !ok {
            self.has_errors = true;
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
                        crate::db::log::error(format!(
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
            crate::db::log::info(format!("    [+] Activado: {}", m.folder_name));

            let sub_deps: Vec<i64> = self.deps.get(&did).cloned().unwrap_or_default();
            for sub in sub_deps {
                self.enable_recursive(sub);
            }
        }
    }

    pub fn enable_mods_for_deps(&mut self) {
        let disabled_deps = self.check_disabled_deps();
        if disabled_deps.is_empty() {
            return;
        }

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
        std::io::Write::flush(&mut std::io::stdout()).ok();
        std::io::stdin().read_line(&mut input).ok();
        let choice = input.trim();

        match choice {
            "1" => {
                for (_, did) in &disabled_deps {
                    self.enable_recursive(*did);
                }
                eprintln!();
            }
            "2" => {
                crate::db::log::warn("Continuando sin las dependencias. Puede que el juego falle.");
                for (_, did) in &disabled_deps {
                    self.skip_ids.insert(*did);
                }
                eprintln!();
            }
            "3" => {
                crate::db::log::die("Cancelado.");
            }
            _ => {
                crate::db::log::die("Opción inválida. Cancelando.");
            }
        }
    }

    pub fn warn_optional_deps(&self) {
        for mid in &self.enabled_ids {
            if let Some(dep_ids) = self.optional_deps.get(mid) {
                let mod_name = self.mod_folder(*mid);
                for did in dep_ids {
                    match self.mods.get(did) {
                        Some(m) if m.enabled => {}
                        Some(m) => crate::db::log::warn(format!(
                            "'{mod_name}' recomienda '{}', pero está desactivado.",
                            m.folder_name
                        )),
                        None => crate::db::log::warn(format!(
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

    pub fn sync_enabled_to_db(&self, conn: &rusqlite::Connection) -> anyhow::Result<()> {
        for m in self.mods.values() {
            crate::db::set_mod_enabled(conn, m.id, m.enabled)?;
        }
        Ok(())
    }
}
