//! Reusable file browser for flat listings (overlay `upper/`, PortableGTA
//! `userfiles/`, …): a folder tree sidebar plus a list or thumbnail grid, with
//! search, kind filter and multi-selection.
//!
//! It only renders and reports actions; the caller decides how to open, delete
//! or move the selected paths (and owns any confirmation dialog).

use eframe::egui;
use std::collections::HashSet;
use std::path::PathBuf;

use crate::thumbs::{ThumbCache, ThumbSize, ThumbState};

/// How the content area is rendered.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    List,
    Grid,
}

/// Sort key for the content list.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SortField {
    Name,
    Size,
    Date,
    Path,
}

impl SortField {
    fn label(self) -> &'static str {
        match self {
            SortField::Name => "Nombre",
            SortField::Size => "Tamaño",
            SortField::Date => "Fecha",
            SortField::Path => "Ruta",
        }
    }
}

/// One file (or directory row) to display.
#[derive(Clone)]
pub struct FileEntry {
    /// Path relative to the browser root, with `/` separators.
    pub rel: String,
    pub name: String,
    pub size: u64,
    /// Last modification time, when known (used by the date sort).
    pub modified: Option<std::time::SystemTime>,
    /// Filter key (e.g. `image`, `save`, `game`, `userdata`).
    pub kind: String,
    /// Human-readable kind for the badge.
    pub kind_label: String,
    /// Extra status text (e.g. `huérfano` or the providing mods).
    pub secondary: String,
    pub is_image: bool,
}

/// Action requested by the user from the browser UI.
#[derive(Debug, Clone)]
pub enum BrowserAction {
    Open(Vec<String>),
    Delete(Vec<String>),
    MoveToMod(Vec<String>),
}

/// App-level requests that are not per-file actions.
#[derive(Default)]
pub struct BrowserResponse {
    pub open_folder: bool,
    pub refresh: bool,
    pub actions: Vec<BrowserAction>,
}

/// Static configuration for one browser instance.
pub struct BrowserConfig<'a> {
    /// Stable id (for egui state); use one per tab.
    pub id: &'a str,
    pub title: &'a str,
    pub root: PathBuf,
    /// Prefix for texture cache keys (namespaced per surface/profile).
    pub key_prefix: String,
    pub can_move: bool,
    pub can_delete: bool,
    pub move_targets: Vec<(String, String)>,
    pub empty_msg: &'a str,
}

/// Persisted UI state of one browser (owned by the tab, not rebuilt per frame).
pub struct FileBrowserState {
    initialized: bool,
    search: String,
    kind_filter: Option<String>,
    /// Quick "images only" filter.
    images_only: bool,
    /// Show dotfiles/dot-directories.
    show_hidden: bool,
    pub view: ViewMode,
    sort: SortField,
    desc: bool,
    expanded: HashSet<String>,
    current: String,
    selected: HashSet<String>,
    last_clicked: Option<String>,
    pub move_target: Option<String>,
    /// Grid thumbnail width in px (height follows the aspect ratio).
    pub thumb_px: f32,
}

impl FileBrowserState {
    pub fn new(view: ViewMode) -> Self {
        Self {
            initialized: false,
            search: String::new(),
            kind_filter: None,
            images_only: false,
            show_hidden: false,
            view,
            sort: SortField::Name,
            desc: false,
            expanded: HashSet::new(),
            current: String::new(),
            selected: HashSet::new(),
            last_clicked: None,
            move_target: None,
            thumb_px: 116.0,
        }
    }
}

/// A directory in the tree built from `FileEntry::rel`.
struct Node {
    name: String,
    rel: String,
    dirs: Vec<Node>,
    files: Vec<usize>,
    /// Recursive file count (for the sidebar badge).
    count: usize,
}

impl Node {
    fn new(name: String, rel: String) -> Self {
        Self {
            name,
            rel,
            dirs: Vec::new(),
            files: Vec::new(),
            count: 0,
        }
    }
}

fn insert(node: &mut Node, parts: &[&str], idx: usize) {
    if parts.len() == 1 {
        node.files.push(idx);
        return;
    }
    let dir = parts[0];
    let pos = match node.dirs.iter().position(|d| d.name == dir) {
        Some(p) => p,
        None => {
            let rel = if node.rel.is_empty() {
                dir.to_string()
            } else {
                format!("{}/{}", node.rel, dir)
            };
            node.dirs.push(Node::new(dir.to_string(), rel));
            node.dirs.len() - 1
        }
    };
    insert(&mut node.dirs[pos], &parts[1..], idx);
}

fn build_tree(entries: &[FileEntry], show_hidden: bool) -> Node {
    let mut root = Node::new(String::new(), String::new());
    for (i, e) in entries.iter().enumerate() {
        if !show_hidden && is_hidden(&e.rel) {
            continue;
        }
        let parts: Vec<&str> = e.rel.split('/').collect();
        insert(&mut root, &parts, i);
    }
    finalize(&mut root);
    root
}

/// True when any path component is a dotfile/dot-directory.
fn is_hidden(rel: &str) -> bool {
    rel.split('/').any(|p| p.starts_with('.'))
}

fn finalize(node: &mut Node) -> usize {
    node.dirs.sort_by(|a, b| a.name.cmp(&b.name));
    let mut count = node.files.len();
    for d in &mut node.dirs {
        count += finalize(d);
    }
    node.count = count;
    count
}

fn find<'a>(node: &'a Node, rel: &str) -> Option<&'a Node> {
    if node.rel == rel {
        return Some(node);
    }
    for d in &node.dirs {
        if rel == d.rel || rel.starts_with(&format!("{}/", d.rel)) {
            return find(d, rel);
        }
    }
    None
}

/// Whether `e` passes the search text and the kind filter.
fn matches(e: &FileEntry, search: &str, kind: Option<&str>) -> bool {
    if let Some(k) = kind {
        if e.kind != k {
            return false;
        }
    }
    if search.is_empty() {
        return true;
    }
    let needle = search.to_lowercase();
    e.name.to_lowercase().contains(&needle) || e.rel.to_lowercase().contains(&needle)
}

/// Inclusive range of rels between two anchors in display order.
fn range_between(order: &[String], a: &str, b: &str) -> Vec<String> {
    let ia = order.iter().position(|x| x == a);
    let ib = order.iter().position(|x| x == b);
    match (ia, ib) {
        (Some(ia), Some(ib)) => {
            let (lo, hi) = if ia <= ib { (ia, ib) } else { (ib, ia) };
            order[lo..=hi].to_vec()
        }
        _ => vec![b.to_string()],
    }
}

/// Parent directory of a root-relative path (`""` for top-level entries).
fn parent_of(rel: &str) -> String {
    match rel.rsplit_once('/') {
        Some((dir, _)) => dir.to_string(),
        None => String::new(),
    }
}

/// Draws the browser and returns the requested app-level actions.
pub fn show(
    ui: &mut egui::Ui,
    state: &mut FileBrowserState,
    config: &BrowserConfig<'_>,
    entries: &[FileEntry],
    thumbs: &mut ThumbCache,
) -> BrowserResponse {
    let mut resp = BrowserResponse::default();
    let tree = build_tree(entries, state.show_hidden);

    // Keep selection/expansion/current consistent with the current entries.
    let rels: HashSet<&str> = entries.iter().map(|e| e.rel.as_str()).collect();
    state.selected.retain(|r| rels.contains(r.as_str()));
    state.expanded.retain(|r| tree_has_dir(&tree, r));
    if !state.current.is_empty() && find(&tree, &state.current).is_none() {
        state.current.clear();
    }
    if !state.initialized {
        state.initialized = true;
        if let Some(root) = find(&tree, "") {
            for d in &root.dirs {
                state.expanded.insert(d.rel.clone());
            }
        }
    }

    let searching = !state.search.is_empty() || state.kind_filter.is_some() || state.images_only;
    let visible = visible_entries(&tree, state, entries, searching);
    let ordered: Vec<String> = visible.iter().map(|&i| entries[i].rel.clone()).collect();

    // Keyboard shortcuts. Ctrl+F focuses the search box from anywhere.
    if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::F)) {
        ui.memory_mut(|m| m.request_focus(egui::Id::new((config.id, "search"))));
    }
    // The rest only act when no widget has focus (so a focused search field
    // keeps its caret/selection editing).
    if ui.memory(|m| m.focused().is_none()) {
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::A)) {
            state.selected = ordered.iter().cloned().collect();
            state.last_clicked = ordered.last().cloned();
        }
        if !ordered.is_empty() {
            let cur = state
                .last_clicked
                .as_ref()
                .and_then(|r| ordered.iter().position(|x| x == r));
            let step = ui.input(|i| {
                if i.key_pressed(egui::Key::ArrowDown) {
                    1i32
                } else if i.key_pressed(egui::Key::ArrowUp) {
                    -1
                } else {
                    0
                }
            });
            if step != 0 {
                let n = ordered.len() as i32;
                let next = match cur {
                    Some(p) => (p as i32 + step).rem_euclid(n) as usize,
                    None => {
                        if step > 0 {
                            0
                        } else {
                            ordered.len() - 1
                        }
                    }
                };
                state.selected.clear();
                state.selected.insert(ordered[next].clone());
                state.last_clicked = Some(ordered[next].clone());
            }
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Some(rel) = state.last_clicked.clone() {
                    resp.actions.push(BrowserAction::Open(vec![rel]));
                }
            }
            if config.can_delete && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                let sel: Vec<String> = state.selected.iter().cloned().collect();
                if !sel.is_empty() {
                    resp.actions.push(BrowserAction::Delete(sel));
                }
            }
        }
    }

    let narrow = ui.available_width() < 640.0;

    let visible_total: u64 = visible.iter().map(|&i| entries[i].size).sum();
    header(
        ui,
        state,
        config,
        entries,
        &ordered,
        visible.len(),
        visible_total,
        &mut resp,
    );

    // Folder tree (hidden on narrow layouts; a breadcrumb handles navigation).
    if !narrow {
        egui::SidePanel::left(egui::Id::new((config.id, "tree")))
            .resizable(true)
            .default_width(220.0)
            .width_range(150.0..=360.0)
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt((config.id, "tree_scroll"))
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        let root_label = format!("{}  {}", crate::icons::FOLDER_OPEN, "Raíz");
                        if ui
                            .selectable_label(
                                state.current.is_empty(),
                                format!("{root_label} ({})", tree.count),
                            )
                            .clicked()
                        {
                            state.current.clear();
                        }
                        tree_rows(ui, &tree, state, 0);
                    });
            });
    }

    content(
        ui, state, config, entries, &visible, &ordered, searching, narrow, thumbs, &mut resp,
    );
    resp
}

fn tree_has_dir(node: &Node, rel: &str) -> bool {
    find(node, rel).is_some()
}

fn tree_rows(ui: &mut egui::Ui, node: &Node, state: &mut FileBrowserState, depth: usize) {
    for d in &node.dirs {
        let expanded = state.expanded.contains(&d.rel);
        ui.horizontal(|ui| {
            ui.add_space(depth as f32 * 10.0);
            let arrow = if expanded { "▾" } else { "▸" };
            if ui
                .add(egui::Button::new(arrow).frame(false).small())
                .clicked()
            {
                if expanded {
                    state.expanded.remove(&d.rel);
                } else {
                    state.expanded.insert(d.rel.clone());
                }
            }
            if ui
                .selectable_label(state.current == d.rel, format!("{} ({})", d.name, d.count))
                .clicked()
            {
                state.current = d.rel.clone();
            }
        });
        if expanded {
            tree_rows(ui, d, state, depth + 1);
        }
    }
}

/// Filters and sorts the entries shown in the content area.
fn visible_entries(
    tree: &Node,
    state: &FileBrowserState,
    entries: &[FileEntry],
    searching: bool,
) -> Vec<usize> {
    let mut visible: Vec<usize> = if searching {
        entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                (state.show_hidden || !is_hidden(&e.rel))
                    && matches(e, &state.search, state.kind_filter.as_deref())
            })
            .map(|(i, _)| i)
            .collect()
    } else {
        find(tree, &state.current)
            .map(|n| n.files.clone())
            .unwrap_or_default()
    };
    if state.images_only {
        visible.retain(|&i| entries[i].is_image);
    }
    visible.sort_by(|&a, &b| {
        let (ea, eb) = (&entries[a], &entries[b]);
        let ord = match state.sort {
            SortField::Name => ea.name.to_lowercase().cmp(&eb.name.to_lowercase()),
            SortField::Size => ea.size.cmp(&eb.size),
            SortField::Date => ea.modified.cmp(&eb.modified),
            SortField::Path => ea.rel.cmp(&eb.rel),
        };
        if state.desc {
            ord.reverse()
        } else {
            ord
        }
    });
    visible
}

#[allow(clippy::too_many_arguments)]
fn header(
    ui: &mut egui::Ui,
    state: &mut FileBrowserState,
    config: &BrowserConfig<'_>,
    entries: &[FileEntry],
    ordered: &[String],
    visible_count: usize,
    visible_total: u64,
    resp: &mut BrowserResponse,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(config.title).strong());
        if ui
            .button(format!("{} Abrir carpeta", crate::icons::FOLDER_OPEN))
            .clicked()
        {
            resp.open_folder = true;
        }
        if ui
            .button(format!("{} Refrescar", crate::icons::REFRESH))
            .clicked()
        {
            resp.refresh = true;
        }
        ui.separator();
        ui.add(
            egui::TextEdit::singleline(&mut state.search)
                .id(egui::Id::new((config.id, "search")))
                .hint_text("Buscar…")
                .desired_width(160.0),
        );
        if ui
            .selectable_label(state.images_only, crate::icons::IMAGE)
            .on_hover_text("Solo imágenes")
            .clicked()
        {
            state.images_only = !state.images_only;
        }
        if ui
            .selectable_label(state.show_hidden, "Ocultos")
            .on_hover_text("Mostrar archivos ocultos")
            .clicked()
        {
            state.show_hidden = !state.show_hidden;
        }
        // Kind filter dropdown, built from the entries actually present.
        let mut kinds: Vec<(String, String)> = Vec::new();
        for e in entries {
            if !kinds.iter().any(|(k, _)| k == &e.kind) {
                kinds.push((e.kind.clone(), e.kind_label.clone()));
            }
        }
        kinds.sort_by(|a, b| a.1.cmp(&b.1));
        let selected = state
            .kind_filter
            .as_ref()
            .and_then(|k| kinds.iter().find(|(key, _)| key == k))
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| "Todos los tipos".to_string());
        egui::ComboBox::from_id_salt((config.id, "kind"))
            .selected_text(selected)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(state.kind_filter.is_none(), "Todos los tipos")
                    .clicked()
                {
                    state.kind_filter = None;
                }
                for (key, label) in &kinds {
                    if ui
                        .selectable_label(state.kind_filter.as_deref() == Some(key), label)
                        .clicked()
                    {
                        state.kind_filter = Some(key.clone());
                    }
                }
            });
        egui::ComboBox::from_id_salt((config.id, "sort"))
            .selected_text(state.sort.label())
            .show_ui(ui, |ui| {
                for f in [
                    SortField::Name,
                    SortField::Size,
                    SortField::Date,
                    SortField::Path,
                ] {
                    ui.selectable_value(&mut state.sort, f, f.label());
                }
            });
        if ui
            .small_button(if state.desc { "↓" } else { "↑" })
            .on_hover_text("Invertir orden")
            .clicked()
        {
            state.desc = !state.desc;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .selectable_label(state.view == ViewMode::Grid, crate::icons::IMAGE)
                .on_hover_text("Rejilla")
                .clicked()
            {
                state.view = ViewMode::Grid;
            }
            if ui
                .selectable_label(state.view == ViewMode::List, crate::icons::LIST)
                .on_hover_text("Lista")
                .clicked()
            {
                state.view = ViewMode::List;
            }
            if state.view == ViewMode::Grid {
                ui.add(egui::Slider::new(&mut state.thumb_px, 80.0..=220.0).show_value(false))
                    .on_hover_text("Tamaño de miniatura");
            }
        });
    });
    ui.label(
        egui::RichText::new(format!(
            "{visible_count} archivo(s) · {}",
            gta_mo_core::userdata::human_size(visible_total)
        ))
        .small()
        .weak(),
    );
    let sel_total: u64 = entries
        .iter()
        .filter(|e| state.selected.contains(&e.rel))
        .map(|e| e.size)
        .sum();
    action_bar(ui, state, config, ordered, sel_total, resp);
    ui.separator();
}

/// Bulk bar shown while there is a selection.
fn action_bar(
    ui: &mut egui::Ui,
    state: &mut FileBrowserState,
    config: &BrowserConfig<'_>,
    ordered: &[String],
    sel_total: u64,
    resp: &mut BrowserResponse,
) {
    if state.selected.is_empty() {
        return;
    }
    let selected: Vec<String> = state.selected.iter().cloned().collect();
    ui.horizontal_wrapped(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{} seleccionado(s) · {}",
                selected.len(),
                gta_mo_core::userdata::human_size(sel_total)
            ))
            .small()
            .strong(),
        );
        if ui
            .button(format!("{} Abrir", crate::icons::FOLDER_OPEN))
            .clicked()
        {
            resp.actions.push(BrowserAction::Open(selected.clone()));
        }
        if config.can_move {
            ui.label("Mover a:");
            if state.move_target.is_none() {
                state.move_target = config.move_targets.first().map(|(f, _)| f.clone());
            }
            let text = state.move_target.clone().unwrap_or_else(|| "—".into());
            egui::ComboBox::from_id_salt((config.id, "move_target"))
                .selected_text(text)
                .show_ui(ui, |ui| {
                    for (folder, name) in &config.move_targets {
                        if ui
                            .selectable_label(
                                state.move_target.as_deref() == Some(folder.as_str()),
                                name,
                            )
                            .clicked()
                        {
                            state.move_target = Some(folder.clone());
                        }
                    }
                });
            if ui
                .button(format!("{} Mover", crate::icons::ARROW_LEFT))
                .clicked()
            {
                resp.actions
                    .push(BrowserAction::MoveToMod(selected.clone()));
            }
        }
        if config.can_delete && ui.button(crate::icons::TRASH).clicked() {
            resp.actions.push(BrowserAction::Delete(selected.clone()));
        }
        if ui.small_button("Limpiar").clicked() {
            state.selected.clear();
            state.last_clicked = None;
        }
        if ui
            .small_button("Invertir")
            .on_hover_text("Invertir la selección entre los visibles")
            .clicked()
        {
            let sel = state.selected.clone();
            state.selected = ordered
                .iter()
                .filter(|r| !sel.contains(*r))
                .cloned()
                .collect();
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn content(
    ui: &mut egui::Ui,
    state: &mut FileBrowserState,
    config: &BrowserConfig<'_>,
    entries: &[FileEntry],
    visible: &[usize],
    ordered: &[String],
    searching: bool,
    narrow: bool,
    thumbs: &mut ThumbCache,
    resp: &mut BrowserResponse,
) {
    // Breadcrumb / "up" navigation (always visible, clickable segments).
    ui.horizontal_wrapped(|ui| {
        if !state.current.is_empty() {
            let up = if narrow {
                crate::icons::ARROW_LEFT.to_string()
            } else {
                format!("{} Subir", crate::icons::ARROW_LEFT)
            };
            if ui
                .small_button(up)
                .on_hover_text("Subir un nivel")
                .clicked()
            {
                state.current = parent_of(&state.current);
            }
        }
        let root_label = format!("{} Raíz", crate::icons::FOLDER_OPEN);
        if ui
            .selectable_label(state.current.is_empty(), root_label)
            .clicked()
        {
            state.current.clear();
        }
        let segments: Vec<String> = state
            .current
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        let mut acc = String::new();
        for seg in segments {
            ui.label("/");
            if acc.is_empty() {
                acc = seg.clone();
            } else {
                acc = format!("{acc}/{seg}");
            }
            if ui.selectable_label(acc == state.current, seg).clicked() {
                state.current = acc.clone();
            }
        }
    });

    if visible.is_empty() {
        let msg = if searching {
            "Ningún archivo coincide con el filtro."
        } else {
            config.empty_msg
        };
        ui.label(msg);
        return;
    }

    egui::ScrollArea::vertical()
        .id_salt((config.id, "content"))
        .auto_shrink(false)
        .show(ui, |ui| match state.view {
            ViewMode::List => {
                for &i in visible {
                    list_row(ui, state, config, &entries[i], ordered, thumbs, resp);
                }
            }
            ViewMode::Grid => {
                ui.horizontal_wrapped(|ui| {
                    for &i in visible {
                        grid_card(ui, state, config, &entries[i], ordered, thumbs, resp);
                    }
                });
            }
        });
}

/// Applies a click with modifiers (ctrl toggles, shift extends the range).
fn click_select(ui: &egui::Ui, state: &mut FileBrowserState, rel: &str, ordered: &[String]) {
    let (shift, command) = ui.input(|i| (i.modifiers.shift, i.modifiers.command));
    if shift {
        if let Some(anchor) = state.last_clicked.clone() {
            let range = range_between(ordered, &anchor, rel);
            for r in range {
                state.selected.insert(r);
            }
        } else {
            state.selected.insert(rel.to_string());
        }
    } else if command {
        if !state.selected.remove(rel) {
            state.selected.insert(rel.to_string());
        }
        state.last_clicked = Some(rel.to_string());
    } else {
        state.selected.clear();
        state.selected.insert(rel.to_string());
        state.last_clicked = Some(rel.to_string());
    }
}

/// Draws the thumbnail inside `rect`; returns false when the entry has no
/// decodable image (caller draws a text placeholder).
fn thumb_in(
    ui: &mut egui::Ui,
    config: &BrowserConfig<'_>,
    e: &FileEntry,
    rect: egui::Rect,
    thumbs: &mut ThumbCache,
) -> bool {
    if !e.is_image {
        return false;
    }
    let key = format!("{}/{}", config.key_prefix, e.rel);
    match thumbs.get(ui.ctx(), &key, &config.root.join(&e.rel), ThumbSize::Small) {
        ThumbState::Ready(tex) => {
            ui.put(
                rect,
                egui::Image::new(&tex)
                    .fit_to_exact_size(rect.size())
                    .corner_radius(4),
            );
            true
        }
        _ => false,
    }
}

fn list_row(
    ui: &mut egui::Ui,
    state: &mut FileBrowserState,
    config: &BrowserConfig<'_>,
    e: &FileEntry,
    ordered: &[String],
    thumbs: &mut ThumbCache,
    resp: &mut BrowserResponse,
) {
    let selected = state.selected.contains(&e.rel);
    ui.horizontal(|ui| {
        let mut checked = selected;
        if ui.checkbox(&mut checked, "").changed() {
            if checked {
                state.selected.insert(e.rel.clone());
                state.last_clicked = Some(e.rel.clone());
            } else {
                state.selected.remove(&e.rel);
            }
        }
        let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
        thumb_in(ui, config, e, rect, thumbs);
        let label = ui.selectable_label(selected, egui::RichText::new(&e.name));
        if label.clicked() {
            click_select(ui, state, &e.rel, ordered);
        }
        if label.double_clicked() {
            state.selected.clear();
            state.selected.insert(e.rel.clone());
            state.last_clicked = Some(e.rel.clone());
            resp.actions.push(BrowserAction::Open(vec![e.rel.clone()]));
        }
        ui.label(egui::RichText::new(&e.kind_label).small().weak());
        if e.is_image {
            if let Some((w, h)) = thumbs.dimensions(&config.root.join(&e.rel), ThumbSize::Small) {
                ui.label(egui::RichText::new(format!("{w}×{h}")).small().weak());
            }
        }
        if !e.secondary.is_empty() {
            ui.label(egui::RichText::new(&e.secondary).small().weak());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(gta_mo_core::userdata::human_size(e.size))
                    .small()
                    .weak(),
            );
            ui.add(egui::Label::new(egui::RichText::new(&e.rel).small().weak()).truncate());
        });
    });
}

fn grid_card(
    ui: &mut egui::Ui,
    state: &mut FileBrowserState,
    config: &BrowserConfig<'_>,
    e: &FileEntry,
    ordered: &[String],
    thumbs: &mut ThumbCache,
    resp: &mut BrowserResponse,
) {
    let selected = state.selected.contains(&e.rel);
    let thumb_size = egui::vec2(state.thumb_px, state.thumb_px * 0.72);
    ui.allocate_ui_with_layout(
        egui::vec2(thumb_size.x + 10.0, thumb_size.y + 48.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            let frame = egui::Frame::new()
                .fill(if selected {
                    ui.visuals().selection.bg_fill
                } else {
                    egui::Color32::TRANSPARENT
                })
                .corner_radius(6)
                .inner_margin(egui::Margin::same(4));
            frame.show(ui, |ui| {
                let (rect, click) = ui.allocate_exact_size(thumb_size, egui::Sense::click());
                if !thumb_in(ui, config, e, rect, thumbs) {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        &e.kind_label,
                        egui::FontId::proportional(11.0),
                        ui.visuals().weak_text_color(),
                    );
                }
                if click.clicked() {
                    click_select(ui, state, &e.rel, ordered);
                }
                if click.double_clicked() {
                    state.selected.clear();
                    state.selected.insert(e.rel.clone());
                    state.last_clicked = Some(e.rel.clone());
                    resp.actions.push(BrowserAction::Open(vec![e.rel.clone()]));
                }
                let hover = match thumbs.dimensions(&config.root.join(&e.rel), ThumbSize::Small) {
                    Some((w, h)) => format!("{}\n{w}×{h}", e.rel),
                    None => e.rel.clone(),
                };
                ui.add(egui::Label::new(egui::RichText::new(&e.name).small()).truncate())
                    .on_hover_text(hover);
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(rel: &str, kind: &str, size: u64) -> FileEntry {
        let name = rel.rsplit('/').next().unwrap().to_string();
        FileEntry {
            rel: rel.to_string(),
            name,
            size,
            modified: None,
            kind: kind.to_string(),
            kind_label: kind.to_string(),
            secondary: String::new(),
            is_image: false,
        }
    }

    #[test]
    fn tree_groups_by_directory() {
        let entries = vec![
            entry("models/a.dff", "game", 1),
            entry("models/b.dff", "game", 2),
            entry("data/c.cfg", "game", 3),
        ];
        let tree = build_tree(&entries, true);
        assert_eq!(tree.count, 3);
        assert_eq!(tree.dirs.len(), 2);
        let models = find(&tree, "models").unwrap();
        assert_eq!(models.files.len(), 2);
        assert_eq!(models.count, 2);
    }

    #[test]
    fn matches_search_and_kind() {
        let e = entry("Gallery/shot.png", "image", 10);
        assert!(matches(&e, "", None));
        assert!(matches(&e, "shot", None));
        assert!(!matches(&e, "nope", None));
        assert!(matches(&e, "", Some("image")));
        assert!(!matches(&e, "", Some("save")));
    }

    #[test]
    fn range_between_is_inclusive_both_ways() {
        let order: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        assert_eq!(range_between(&order, "b", "d"), vec!["b", "c", "d"]);
        assert_eq!(range_between(&order, "d", "b"), vec!["b", "c", "d"]);
        assert_eq!(range_between(&order, "x", "c"), vec!["c"]);
    }

    #[test]
    fn parent_of_root_and_nested() {
        assert_eq!(parent_of("models/x.dff"), "models");
        assert_eq!(parent_of("x.dff"), "");
    }

    #[test]
    fn renders_list_and_grid_without_panicking() {
        let ctx = egui::Context::default();
        let entries = vec![
            entry("models/a.dff", "game", 1),
            entry("gallery/shot.png", "image", 2),
            entry("gallery/nested/deep.png", "image", 3),
        ];
        let config = BrowserConfig {
            id: "test",
            title: "Prueba",
            root: std::path::PathBuf::from("/tmp/gta-mo-nope"),
            key_prefix: "test".into(),
            can_move: true,
            can_delete: true,
            move_targets: vec![("mod".into(), "Mod".into())],
            empty_msg: "vacío",
        };
        let mut state = FileBrowserState::new(ViewMode::List);
        let mut thumbs = ThumbCache::new();
        for view in [ViewMode::List, ViewMode::Grid] {
            state.view = view;
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let _ = show(ui, &mut state, &config, &entries, &mut thumbs);
                });
            });
        }
    }
}
