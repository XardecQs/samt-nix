use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{channel, Receiver, Sender};

use eframe::egui;

use crate::backend::{Backend, GuiEvent};
use crate::model::{filter_and_sort, Filters, Snapshot, SortField};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Mods,
    Profiles,
    Groups,
    Conflicts,
    Log,
}

enum InputAction {
    Create,
    Rename(String),
    Copy(String),
    NewMod,
    RenameMod(String),
    NewGroup,
    RenameGroup(String),
}

struct InputState {
    title: String,
    label: String,
    value: String,
    action: InputAction,
}

impl InputState {
    fn new(title: &str, label: &str, action: InputAction) -> Self {
        Self {
            title: title.into(),
            label: label.into(),
            value: String::new(),
            action,
        }
    }
}

#[allow(clippy::enum_variant_names)]
enum ConfirmAction {
    DeleteProfile(String),
    DeleteMod(String),
    DeleteGroup(String),
}

struct ConfirmState {
    title: String,
    message: String,
    action: ConfirmAction,
}

/// A CLI invocation queued for execution. Commands run strictly one at a time
/// so concurrent `gta-mo ctl` writes never race on the SQLite database.
struct Job {
    args: Vec<String>,
    launch: bool,
}

pub struct GtaMoApp {
    backend: Backend,
    snapshot: Snapshot,
    rx: Receiver<GuiEvent>,
    tx: Sender<GuiEvent>,
    log: Vec<String>,
    tab: Tab,
    filters: Filters,
    selected_mod: Option<i64>,
    selected_profile: Option<String>,
    busy: bool,
    playing: bool,
    launch_debug: bool,
    launch_dry_run: bool,
    pending: VecDeque<Job>,
    input: Option<InputState>,
    confirm: Option<ConfirmState>,
    relations: Option<crate::backend::ModRelations>,
    relations_for: Option<i64>,
    conflicts: Vec<crate::backend::ConflictView>,
    conflicts_pending: bool,
    scan_gen: u64,
    selected_group: Option<String>,
    group_members: Option<Vec<String>>,
    group_pick: Option<String>,
    drag_folder: Option<String>,
    drop_index: usize,
    covers: HashMap<String, egui::TextureHandle>,
    covers_order: VecDeque<String>,
    cover_missing: HashSet<String>,
    status: Option<String>,
}

impl GtaMoApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let ui_scale = crate::settings::GuiSettings::load().ui_scale;
        cc.egui_ctx
            .set_pixels_per_point(cc.egui_ctx.pixels_per_point() * ui_scale);
        let backend = Backend::new();
        let (tx, rx) = channel();
        let mut app = Self {
            backend,
            snapshot: Snapshot::default(),
            rx,
            tx,
            log: vec!["--- gta-mo-gui iniciado ---".to_string()],
            tab: Tab::Mods,
            filters: Filters::default(),
            selected_mod: None,
            selected_profile: None,
            busy: false,
            playing: false,
            launch_debug: false,
            launch_dry_run: false,
            pending: VecDeque::new(),
            input: None,
            confirm: None,
            relations: None,
            relations_for: None,
            conflicts: Vec::new(),
            conflicts_pending: false,
            scan_gen: 0,
            selected_group: None,
            group_members: None,
            group_pick: None,
            drag_folder: None,
            drop_index: 0,
            covers: HashMap::new(),
            covers_order: VecDeque::new(),
            cover_missing: HashSet::new(),
            status: None,
        };
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        match self.backend.snapshot() {
            Ok(s) => {
                if self.selected_profile.is_none() {
                    self.selected_profile = Some(s.active_slug.clone());
                }
                self.snapshot = s;
                self.status = None;
                self.reload_relations();
                self.reload_groups();
                self.start_conflict_scan();
            }
            Err(e) => self.status = Some(e),
        }
    }

    /// Kicks off a background conflict scan. Results are discarded if they
    /// arrive after a newer scan started (`scan_gen`).
    fn start_conflict_scan(&mut self) {
        self.scan_gen += 1;
        self.conflicts_pending = true;
        match self.backend.mods_dir_path() {
            Some(mdir) => {
                let resolved = self.snapshot.resolved.clone();
                let gen = self.scan_gen;
                let tx = self.tx.clone();
                crate::backend::Backend::scan_conflicts_async(gen, mdir, resolved, tx);
            }
            None => self.conflicts_pending = false,
        }
    }

    fn reload_relations(&mut self) {
        match self.selected_mod {
            Some(id) => {
                self.relations = self.backend.mod_relations(id).ok();
                self.relations_for = Some(id);
            }
            None => {
                self.relations = None;
                self.relations_for = None;
            }
        }
    }

    fn reload_groups(&mut self) {
        if let Some(group) = self.selected_group.clone() {
            self.group_members = self.backend.group_members(&group).ok();
        }
    }

    /// Queues a CLI invocation and starts one if none is running. Commands are
    /// executed strictly one at a time to avoid racing SQLite writes between
    /// concurrent `gta-mo ctl` processes.
    fn exec(&mut self, args: Vec<String>, launch: bool) {
        self.pending.push_back(Job { args, launch });
        self.pump();
    }

    fn pump(&mut self) {
        if self.busy {
            return;
        }
        if let Some(job) = self.pending.pop_front() {
            self.busy = true;
            self.playing = job.launch;
            let tx = self.tx.clone();
            self.backend.run_cli_async(job.args, tx);
        } else {
            self.playing = false;
        }
    }

    fn poll_events(&mut self) {
        let events: Vec<GuiEvent> = self.rx.try_iter().collect();
        for ev in events {
            match ev {
                GuiEvent::LogLine(l) => {
                    self.log.push(l);
                    if self.log.len() > 2000 {
                        self.log.drain(..self.log.len() - 2000);
                    }
                }
                GuiEvent::ConflictScan(gen, list) => {
                    if gen == self.scan_gen {
                        self.conflicts = list;
                        self.conflicts_pending = false;
                    }
                }
                GuiEvent::CommandDone(ok, msg) => {
                    self.busy = false;
                    self.playing = false;
                    if !ok {
                        // Abort any follow-up jobs: a failed write may have left
                        // the DB in an unknown state, so don't keep mutating.
                        self.pending.clear();
                        self.status = Some(if msg.is_empty() {
                            "La operación falló (ver Log)".to_string()
                        } else {
                            msg
                        });
                        self.pump();
                    } else {
                        self.pump();
                        if !self.busy {
                            self.refresh();
                        }
                    }
                }
            }
        }
    }

    fn set_enabled(&mut self, id: i64, enabled: bool) {
        let slug = self.snapshot.active_slug.clone();
        let id_s = id.to_string();
        let mut args = vec!["ctl".to_string()];
        if enabled {
            args.push("enable".into());
        } else {
            args.push("disable".into());
        }
        args.push(id_s);
        if !enabled {
            args.push("--yes".into());
        }
        args.push("--profile".into());
        args.push(slug);
        self.exec(args, false);
    }

    /// Drag&drop reordering is only meaningful on the full, priority-sorted
    /// list (no search/tag/group filters) and while nothing is running.
    fn can_reorder(&self) -> bool {
        !(self.busy || self.playing)
            && self.filters.search.is_empty()
            && self.filters.tag.is_none()
            && self.filters.group.is_none()
            && self.filters.sort == SortField::Order
            && self.filters.desc
    }

    /// Applies a drag&drop move: `dragged` is placed at index `index` of the
    /// full priority-sorted list (0 = top). One bulk `ctl reorder` call
    /// persists the whole new order.
    fn reorder_drop_at(&mut self, dragged: &str, index: usize) {
        if !self.can_reorder() {
            return;
        }
        let mut full = self.snapshot.mods.clone();
        full.sort_by_key(|m| std::cmp::Reverse(m.order));
        let full_folders: Vec<String> = full.iter().map(|m| m.folder.clone()).collect();
        if !full_folders.iter().any(|f| f == dragged) {
            return;
        }
        let mut seq: Vec<String> = full_folders.iter().filter(|f| **f != dragged).cloned().collect();
        let insert_at = index.min(seq.len());
        seq.insert(insert_at, dragged.to_string());
        if seq == full_folders {
            return;
        }

        let mut args: Vec<String> = vec!["ctl".into(), "reorder".into()];
        args.extend(seq);
        args.push("--profile".into());
        args.push(self.snapshot.active_slug.clone());
        self.exec(args, false);
    }

    fn load_cover(
        &mut self,
        ctx: &egui::Context,
        folder: &str,
        cover: &str,
    ) -> Option<egui::TextureHandle> {
        let key = format!("{folder}/{cover}");
        if let Some(t) = self.covers.get(&key) {
            return Some(t.clone());
        }
        if self.cover_missing.contains(&key) {
            return None;
        }
        let Some(path) = self.backend.cover_path(folder, cover) else {
            self.cover_missing.insert(key);
            return None;
        };
        let Some(img) = image::ImageReader::open(&path)
            .ok()
            .and_then(|reader| reader.decode().ok())
        else {
            self.cover_missing.insert(key);
            return None;
        };
        let img = img.to_rgba8();
        let (w, h) = (img.width(), img.height());
        let color =
            egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
        let tex = ctx.load_texture(&key, color, egui::TextureOptions::LINEAR);

        // LRU: capa el número de texturas y descarta solo la más antigua.
        if self.covers.len() >= 128 {
            if let Some(old) = self.covers_order.pop_front() {
                self.covers.remove(&old);
            }
        }
        self.covers.insert(key.clone(), tex.clone());
        self.covers_order.retain(|k| k != &key);
        self.covers_order.push_back(key);
        Some(tex)
    }

    /// Draws one row of the mods list: optional cover thumbnail, enable
    /// checkbox and the name/author/tags block plus a "Detalle" button.
    fn draw_mod_row(
        &mut self,
        ui: &mut egui::Ui,
        m: &crate::model::ModView,
        reorderable: bool,
    ) -> egui::Response {
        let idle = !(self.busy || self.playing);
        let fill = if m.enabled {
            egui::Color32::from_rgba_unmultiplied(70, 130, 230, 22)
        } else {
            egui::Color32::TRANSPARENT
        };
        let frame = egui::Frame::new()
            .fill(fill)
            .corner_radius(egui::CornerRadius::same(5))
            .inner_margin(egui::Margin::symmetric(6, 2));
        let row = frame.show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                if reorderable {
                    let handle = egui::Id::new(("mod_handle", m.id));
                    ui.dnd_drag_source(handle, m.folder.clone(), |ui| {
                        ui.add(
                            egui::Button::new("⠿")
                                .frame(false)
                                .small()
                                .min_size(egui::vec2(22.0, 24.0)),
                        )
                        .on_hover_text("Arrastra para reordenar");
                    });
                }
                if Self::toggle_indicator(ui, m.enabled, idle) {
                    self.set_enabled(m.id, !m.enabled);
                }
                if let Some(cover) = m.meta.cover.clone() {
                    if let Some(tex) = self.load_cover(ui.ctx(), &m.folder, &cover) {
                        ui.add(
                            egui::Image::new(&tex)
                                .fit_to_exact_size(egui::vec2(40.0, 40.0))
                                .corner_radius(4),
                        );
                    }
                }
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        let name_color = if m.enabled {
                            egui::Color32::WHITE
                        } else {
                            ui.visuals().text_color()
                        };
                        ui.label(egui::RichText::new(&m.name).strong().color(name_color));
                        if let Some(v) = &m.meta.version {
                            ui.label(egui::RichText::new(format!("v{v}")).weak());
                        }
                        if !m.meta.author.is_empty() {
                            ui.label(egui::RichText::new(m.meta.author.join(", ")).weak());
                        }
                    });
                    if !m.meta.tags.is_empty() || !m.groups.is_empty() {
                        ui.horizontal(|ui| {
                            for t in &m.meta.tags {
                                ui.label(
                                    egui::RichText::new(format!("#{t}"))
                                        .small()
                                        .color(egui::Color32::from_rgb(120, 160, 255)),
                                );
                            }
                            for g in &m.groups {
                                ui.label(
                                    egui::RichText::new(format!("[{g}]")).small().weak(),
                                );
                            }
                        });
                    }
                });
                if ui.button("Detalle").clicked() {
                    self.selected_mod = Some(m.id);
                }
            });
        });
        row.response
    }

    /// Indicador de estado propio: cuadrado redondeado de color acento cuando
    /// el mod está activo, gris cuando no. Devuelve `true` cuando se hace clic.
    fn toggle_indicator(ui: &mut egui::Ui, active: bool, interactive: bool) -> bool {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
        let fill = if active {
            egui::Color32::from_rgb(90, 160, 255)
        } else {
            egui::Color32::from_rgb(58, 58, 66)
        };
        let stroke_color = if active {
            egui::Color32::from_rgb(140, 195, 255)
        } else {
            egui::Color32::from_rgb(110, 110, 120)
        };
        let inner = egui::Rect::from_center_size(rect.center(), egui::vec2(14.0, 14.0));
        ui.painter()
            .rect(inner, egui::CornerRadius::same(4), fill, egui::Stroke::new(1.5, stroke_color), egui::StrokeKind::Inside);
        if active {
            ui.painter().text(
                inner.center(),
                egui::Align2::CENTER_CENTER,
                "✓",
                egui::FontId::proportional(12.0),
                egui::Color32::WHITE,
            );
        }
        let _ = response.clone().on_hover_cursor(egui::CursorIcon::PointingHand);
        if !interactive {
            return false;
        }
        response.clicked()
    }
}

impl eframe::App for GtaMoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events();

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.heading("GTA SA Mod Organizer");
                ui.separator();
                ui.label("Perfil:");
                let active = self.snapshot.active_slug.clone();
                let profiles = self.snapshot.profiles.clone();
                egui::ComboBox::from_id_salt("profile")
                    .selected_text(active.clone())
                    .show_ui(ui, |ui| {
                        for p in &profiles {
                            if ui.selectable_label(p.slug == active, &p.name).clicked()
                                && p.slug != active
                                && !(self.busy || self.playing)
                            {
                                self.exec(
                                    vec![
                                        "ctl".into(),
                                        "profile".into(),
                                        "use".into(),
                                        p.slug.clone(),
                                    ],
                                    false,
                                );
                            }
                        }
                    });
                ui.separator();
                ui.checkbox(&mut self.launch_debug, "Debug")
                    .on_hover_text("Habilitar log de Proton/DXVK (--debug)");
                ui.checkbox(&mut self.launch_dry_run, "Previsualizar")
                    .on_hover_text("Mostrar el orden de capas sin montar ni lanzar (--dry-run)");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let idle = !self.busy && !self.playing;
                    let label = if self.playing { "Jugando…" } else { "Jugar" };
                    if ui.add_enabled(idle, egui::Button::new(label)).clicked() {
                        let slug = self.snapshot.active_slug.clone();
                        self.log.clear();
                        let mode = if self.launch_dry_run {
                            "previsualizando (dry-run)"
                        } else {
                            "lanzando"
                        };
                        self.log.push(format!("--- {mode} perfil '{slug}' ---"));
                        let mut args: Vec<String> = vec!["launch".into(), "--deps-enable".into()];
                        if self.launch_debug {
                            args.push("--debug".into());
                        }
                        if self.launch_dry_run {
                            args.push("--dry-run".into());
                        }
                        args.push("--profile".into());
                        args.push(slug.clone());
                        self.exec(args, !self.launch_dry_run);
                    }
                    if ui
                        .add_enabled(idle, egui::Button::new("Limpiar"))
                        .on_hover_text("Eliminar mods huérfanos (carpetas desaparecidas)")
                        .clicked()
                    {
                        let slug = self.snapshot.active_slug.clone();
                        self.exec(
                            vec![
                                "launch".into(),
                                "--clean".into(),
                                "--profile".into(),
                                slug,
                            ],
                            false,
                        );
                    }
                    if ui
                        .add_enabled(idle, egui::Button::new("Descubrir"))
                        .on_hover_text("Escanear mods/ y registrar mods nuevos")
                        .clicked()
                    {
                        let slug = self.snapshot.active_slug.clone();
                        self.exec(
                            vec![
                                "launch".into(),
                                "--discover".into(),
                                "--profile".into(),
                                slug,
                            ],
                            false,
                        );
                    }
                });
            });
            ui.add_space(6.0);
        });

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.selectable_label(self.tab == Tab::Mods, "Mods").clicked() {
                    self.tab = Tab::Mods;
                }
                if ui
                    .selectable_label(self.tab == Tab::Profiles, "Perfiles")
                    .clicked()
                {
                    self.tab = Tab::Profiles;
                }
                if ui
                    .selectable_label(self.tab == Tab::Groups, "Grupos")
                    .clicked()
                {
                    self.tab = Tab::Groups;
                }
                let conflict_label = format!(
                    "Conflictos ({})",
                    self.conflicts.iter().filter(|c| !c.duplicate).count()
                );
                if ui
                    .selectable_label(self.tab == Tab::Conflicts, conflict_label)
                    .clicked()
                {
                    self.tab = Tab::Conflicts;
                }
                if ui.selectable_label(self.tab == Tab::Log, "Log").clicked() {
                    self.tab = Tab::Log;
                }
                if ui.button("↻").on_hover_text("Refrescar").clicked() {
                    self.refresh();
                }
            });
        });

        if self.selected_mod.is_some() {
            egui::SidePanel::right("detail")
                .resizable(true)
                .default_width(360.0)
                .show(ctx, |ui| self.ui_detail(ui));
        }

        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Mods => self.ui_mods(ui),
            Tab::Profiles => self.ui_profiles(ui),
            Tab::Groups => self.ui_groups(ui),
            Tab::Conflicts => self.ui_conflicts(ui),
            Tab::Log => self.ui_log(ui),
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(e) = self.backend.error_str() {
                    ui.colored_label(egui::Color32::RED, e);
                    if ui.button("Reintentar").clicked() {
                        self.backend.retry();
                        self.refresh();
                    }
                } else if let Some(e) = &self.status {
                    ui.colored_label(egui::Color32::YELLOW, e);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let n = self.conflicts.iter().filter(|c| !c.duplicate).count();
                    let conflicts_label = if self.conflicts_pending {
                        format!("Conflictos: {n}…")
                    } else {
                        format!("Conflictos: {n}")
                    };
                    ui.label(conflicts_label);
                    ui.separator();
                    let enabled = self.snapshot.mods.iter().filter(|m| m.enabled).count();
                    ui.label(format!(
                        "{} mods · {} activos · Perfil: {}",
                        self.snapshot.mods.len(),
                        enabled,
                        self.snapshot.active_slug
                    ));
                });
            });
        });

        self.ui_dialogs(ctx);
    }
}

impl GtaMoApp {
    fn ui_mods(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.filters.search)
                    .hint_text("Buscar mods…")
                    .desired_width(220.0),
            );

            let mut tag = self.filters.tag.clone();
            egui::ComboBox::from_id_salt("tag_filter")
                .selected_text(tag.clone().unwrap_or_else(|| "Todos los tags".into()))
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(tag.is_none(), "Todos los tags")
                        .clicked()
                    {
                        tag = None;
                    }
                    for t in &self.snapshot.all_tags {
                        if ui
                            .selectable_label(tag.as_deref() == Some(t.as_str()), t)
                            .clicked()
                        {
                            tag = Some(t.clone());
                        }
                    }
                });
            self.filters.tag = tag;

            let mut group = self.filters.group.clone();
            egui::ComboBox::from_id_salt("group_filter")
                .selected_text(group.clone().unwrap_or_else(|| "Todos los grupos".into()))
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(group.is_none(), "Todos los grupos")
                        .clicked()
                    {
                        group = None;
                    }
                    for g in &self.snapshot.all_groups {
                        if ui
                            .selectable_label(group.as_deref() == Some(g.as_str()), g)
                            .clicked()
                        {
                            group = Some(g.clone());
                        }
                    }
                });
            self.filters.group = group;

            egui::ComboBox::from_id_salt("sort")
                .selected_text(self.filters.sort.label())
                .show_ui(ui, |ui| {
                    for f in [
                        SortField::Order,
                        SortField::Name,
                        SortField::Folder,
                        SortField::Author,
                        SortField::Version,
                        SortField::ModId,
                        SortField::Status,
                    ] {
                        if ui
                            .selectable_value(&mut self.filters.sort, f, f.label())
                            .clicked()
                        {
                            self.filters.desc = f == SortField::Order;
                        }
                    }
                });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !(self.busy || self.playing),
                        egui::Button::new("+ Nuevo mod"),
                    )
                    .on_hover_text("Crear carpeta con plantilla mod.toml y registrar el mod")
                    .clicked()
                {
                    self.input = Some(InputState::new(
                        "Nuevo mod",
                        "Nombre de carpeta del mod:",
                        InputAction::NewMod,
                    ));
                }
            });
        });
        ui.separator();

        let mut filtered = self.snapshot.mods.clone();
        filter_and_sort(&mut filtered, &self.filters);

        let reorderable = self.can_reorder();
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                if reorderable {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Arrastra el asidero ⠿ para reordenar")
                                .weak()
                                .small(),
                        );
                    });
                    ui.add_space(2.0);
                }

                // (rect, folder) de cada fila, para el indicador de inserción.
                let mut row_rects: Vec<(egui::Rect, String)> = Vec::new();
                for m in &filtered {
                    let row = self.draw_mod_row(ui, m, reorderable);
                    if reorderable {
                        row_rects.push((row.rect, m.folder.clone()));
                    }
                    ui.separator();
                }

                if reorderable && !row_rects.is_empty() {
                    let ctx = ui.ctx();
                    // ¿Hay un arrastre activo desde un asidero?
                    let active: Option<String> = filtered
                        .iter()
                        .find(|m| ctx.is_being_dragged(egui::Id::new(("mod_handle", m.id))))
                        .map(|m| m.folder.clone());

                    let pos = ctx.pointer_hover_pos().or_else(|| ctx.pointer_latest_pos());

                    let prev_dragging = self.drag_folder.take();
                    if let Some(folder) = active {
                        self.drag_folder = Some(folder.clone());
                        // Índice de inserción y barra indicadora.
                        let index = match pos {
                            Some(p) => row_rects
                                .iter()
                                .position(|(r, _)| p.y < r.center().y)
                                .unwrap_or(row_rects.len()),
                            None => row_rects.len(),
                        };
                        self.drop_index = index;
                        let y = if index < row_rects.len() {
                            row_rects[index].0.top()
                        } else {
                            row_rects.last().map(|(r, _)| r.bottom()).unwrap_or_else(|| ui.clip_rect().bottom())
                        };
                        let x0 = ui.clip_rect().left() + 4.0;
                        let x1 = ui.clip_rect().right() - 4.0;
                        ui.painter().line_segment(
                            [egui::pos2(x0, y), egui::pos2(x1, y)],
                            egui::Stroke::new(2.0, egui::Color32::from_rgb(90, 160, 255)),
                        );
                    } else if let Some(folder) = prev_dragging {
                        // Suelta: persiste el orden con la posición señalada.
                        let index = self.drop_index;
                        self.reorder_drop_at(&folder, index);
                    }
                }
            });
    }

    fn ui_detail(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected_mod else {
            return;
        };
        let Some(m) = self.snapshot.mods.iter().find(|m| m.id == id).cloned() else {
            self.selected_mod = None;
            return;
        };
        if self.relations_for != self.selected_mod {
            self.reload_relations();
        }

        ui.horizontal(|ui| {
            ui.heading(&m.name);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Cerrar").clicked() {
                    self.selected_mod = None;
                }
            });
        });
        ui.separator();

        if let Some(cover) = m.meta.cover.clone() {
            if let Some(tex) = self.load_cover(ui.ctx(), &m.folder, &cover) {
                ui.add(egui::Image::new(&tex).max_width(320.0).max_height(200.0));
            }
        }

        let rel = self.relations.clone();
        let mut go_to: Option<String> = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                egui::Grid::new("detail_meta")
                    .num_columns(2)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| {
                        if let Some(id) = &m.meta.mod_id {
                            ui.label("Mod ID:");
                            ui.label(egui::RichText::new(id).monospace());
                            ui.end_row();
                        }
                        if let Some(v) = &m.meta.version {
                            ui.label("Versión:");
                            ui.label(v);
                            ui.end_row();
                        }
                        if !m.meta.author.is_empty() {
                            ui.label("Autor:");
                            ui.label(m.meta.author.join(", "));
                            ui.end_row();
                        }
                        if let Some(u) = &m.meta.url {
                            ui.label("URL:");
                            ui.hyperlink_to(u.clone(), u.clone());
                            ui.end_row();
                        }
                        if !m.meta.tags.is_empty() {
                            ui.label("Tags:");
                            ui.label(m.meta.tags.join(", "));
                            ui.end_row();
                        }
                        if !m.groups.is_empty() {
                            ui.label("Grupos:");
                            ui.label(m.groups.join(", "));
                            ui.end_row();
                        }
                        if !m.meta.mount.is_empty() {
                            ui.label("Mount:");
                            ui.label(m.meta.mount.join(", "));
                            ui.end_row();
                        }
                        if let Some(d) = &m.meta.description {
                            ui.label("Descripción:");
                            ui.add(egui::Label::new(egui::RichText::new(d)).wrap());
                            ui.end_row();
                        }
                    });

                ui.add_space(8.0);
                let deps = m.meta.clone();
                if !deps.guides.is_empty() {
                    ui.label(egui::RichText::new("Guías").strong());
                    for g in &deps.guides {
                        ui.label(egui::RichText::new(g).small().weak());
                    }
                    ui.add_space(6.0);
                }

                if !m.meta.components.is_empty() {
                    ui.label(
                        egui::RichText::new(format!("Componentes ({})", m.meta.components.len()))
                            .strong(),
                    );
                    for c in &m.meta.components {
                        let name = c.name.clone().unwrap_or_default();
                        let extra: Vec<String> =
                            vec![c.version.clone().map(|v| format!("v{v}")), c.author.clone()]
                                .into_iter()
                                .flatten()
                                .collect();
                        ui.label(
                            egui::RichText::new(if extra.is_empty() {
                                name
                            } else {
                                format!("{name} — {}", extra.join(" · "))
                            })
                            .small(),
                        );
                    }
                    ui.add_space(6.0);
                }

                if let Some(r) = &rel {
                    if !r.depends.is_empty() {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("Depende de").strong());
                        for (folder, name, required, enabled) in &r.depends {
                            let title = if name.is_empty() {
                                folder.clone()
                            } else {
                                format!("{name} ({})", if *required { "requerido" } else { "opcional" })
                            };
                            let color = if *enabled {
                                egui::Color32::from_rgb(130, 200, 130)
                            } else {
                                egui::Color32::from_rgb(230, 120, 120)
                            };
                            if ui
                                .add(
                                    egui::Label::new(
                                        egui::RichText::new(&title).color(color).size(13.0),
                                    )
                                    .sense(egui::Sense::click()),
                                )
                                .on_hover_text(format!(
                                    "{folder} · {} — clic para verlo",
                                    if *enabled { "activo" } else { "inactivo" }
                                ))
                                .clicked()
                            {
                                go_to = Some(folder.clone());
                            }
                        }
                    }
                    if !r.dependents.is_empty() {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new("Usado por").strong());
                        for (folder, name, enabled) in &r.dependents {
                            let title = if name.is_empty() {
                                folder.clone()
                            } else {
                                name.clone()
                            };
                            let color = if *enabled {
                                egui::Color32::from_rgb(130, 200, 130)
                            } else {
                                egui::Color32::from_rgb(190, 190, 190)
                            };
                            if ui
                                .add(
                                    egui::Label::new(
                                        egui::RichText::new(&title).color(color).size(13.0),
                                    )
                                    .sense(egui::Sense::click()),
                                )
                                .on_hover_text(format!("{folder} — clic para verlo"))
                                .clicked()
                            {
                                go_to = Some(folder.clone());
                            }
                        }
                    }
                }

                ui.horizontal(|ui| {
                    let folder = m.folder.clone();
                    if ui
                        .add_enabled(
                            !(self.busy || self.playing),
                            egui::Button::new("Renombrar"),
                        )
                        .on_hover_text("Cambiar el nombre visible (y el de mod.toml)")
                        .clicked()
                    {
                        self.input = Some(InputState::new(
                            "Renombrar mod",
                            "Nuevo nombre:",
                            InputAction::RenameMod(folder.clone()),
                        ));
                    }
                    if ui
                        .add_enabled(
                            !(self.busy || self.playing),
                            egui::Button::new("Eliminar"),
                        )
                        .clicked()
                    {
                        self.confirm = Some(ConfirmState {
                            title: "Eliminar mod".into(),
                            message: format!("¿Eliminar el mod '{folder}' y sus estados?"),
                            action: ConfirmAction::DeleteMod(folder.clone()),
                        });
                    }
                    if ui.button("Abrir carpeta").clicked() {
                        self.exec(vec!["ctl".into(), "open".into(), folder], false);
                    }
                    if m.meta.url.is_some() {
                        let folder = m.folder.clone();
                        if ui.button("Abrir URL").clicked() {
                            self.exec(
                                vec!["ctl".into(), "open".into(), folder, "--url".into()],
                                false,
                            );
                        }
                    }
                });

                if !self.snapshot.all_groups.is_empty() {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let groups = self.snapshot.all_groups.clone();
                        let mut pick = self.group_pick.clone();
                        egui::ComboBox::from_id_salt("detail_group_pick")
                            .selected_text(pick.clone().unwrap_or_else(|| "Añadir a grupo…".into()))
                            .show_ui(ui, |ui| {
                                for name in &groups {
                                    if ui
                                        .selectable_label(pick.as_deref() == Some(name.as_str()), name)
                                        .clicked()
                                    {
                                        pick = Some(name.clone());
                                    }
                                }
                            });
                        self.group_pick = pick;
                        if let Some(group) = self.group_pick.clone() {
                            let in_group = m.groups.iter().any(|g| g == &group);
                            let action = if in_group { "remove" } else { "add" };
                            let label = if in_group {
                                "Quitar del grupo"
                            } else {
                                "Añadir al grupo"
                            };
                            if ui
                                .add_enabled(
                                    !(self.busy || self.playing),
                                    egui::Button::new(label),
                                )
                                .on_hover_text("Membresía del perfil activo (no global)")
                                .clicked()
                            {
                                let folder = m.folder.clone();
                                let slug = self.snapshot.active_slug.clone();
                                self.exec(
                                    vec![
                                        "ctl".into(),
                                        "group".into(),
                                        action.into(),
                                        folder,
                                        group,
                                        "--profile".into(),
                                        slug,
                                    ],
                                    false,
                                );
                            }
                        }
                    });
                }
            });
        if let Some(go) = go_to {
            if let Some(tm) = self.snapshot.mods.iter().find(|x| x.folder == go).cloned() {
                self.selected_mod = Some(tm.id);
                self.reload_relations();
            }
        }
    }

    fn ui_profiles(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!(self.busy || self.playing), egui::Button::new("Nuevo"))
                .clicked()
            {
                self.input = Some(InputState::new(
                    "Nuevo perfil",
                    "Nombre:",
                    InputAction::Create,
                ));
            }
            let sel = self.selected_profile.clone();
            if ui
                .add_enabled(
                    sel.is_some() && !(self.busy || self.playing),
                    egui::Button::new("Usar"),
                )
                .clicked()
            {
                if let Some(s) = &sel {
                    self.exec(
                        vec!["ctl".into(), "profile".into(), "use".into(), s.clone()],
                        false,
                    );
                }
            }
            if ui
                .add_enabled(
                    sel.is_some() && !(self.busy || self.playing),
                    egui::Button::new("Renombrar"),
                )
                .clicked()
            {
                if let Some(s) = &sel {
                    self.input = Some(InputState::new(
                        "Renombrar perfil",
                        "Nuevo nombre:",
                        InputAction::Rename(s.clone()),
                    ));
                }
            }
            if ui
                .add_enabled(
                    sel.is_some() && !(self.busy || self.playing),
                    egui::Button::new("Copiar"),
                )
                .clicked()
            {
                if let Some(s) = &sel {
                    self.input = Some(InputState::new(
                        "Copiar perfil",
                        "Nombre del nuevo perfil:",
                        InputAction::Copy(s.clone()),
                    ));
                }
            }
            if ui
                .add_enabled(
                    sel.is_some() && !(self.busy || self.playing),
                    egui::Button::new("Eliminar"),
                )
                .clicked()
            {
                if let Some(s) = &sel {
                    self.confirm = Some(ConfirmState {
                        title: "Eliminar perfil".into(),
                        message: format!("¿Eliminar el perfil '{s}' y sus estados?"),
                        action: ConfirmAction::DeleteProfile(s.clone()),
                    });
                }
            }
        });
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for p in &self.snapshot.profiles {
                let selected = self.selected_profile.as_deref() == Some(p.slug.as_str());
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(selected, &p.name)
                        .on_hover_text("Seleccionar")
                        .clicked()
                    {
                        self.selected_profile = Some(p.slug.clone());
                    }
                    ui.label(
                        egui::RichText::new(format!("{} mods · {} activos", p.total, p.enabled))
                            .weak(),
                    );
                    if p.is_active {
                        ui.label(egui::RichText::new("(activo)").weak());
                    }
                });
            }
        });
    }

    fn ui_groups(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !(self.busy || self.playing),
                    egui::Button::new("Nuevo grupo"),
                )
                .clicked()
            {
                self.input = Some(InputState::new(
                    "Nuevo grupo",
                    "Nombre del grupo:",
                    InputAction::NewGroup,
                ));
            }
            let sel = self.selected_group.clone();
            if ui
                .add_enabled(
                    sel.is_some() && !(self.busy || self.playing),
                    egui::Button::new("Renombrar"),
                )
                .clicked()
            {
                if let Some(g) = &sel {
                    self.input = Some(InputState::new(
                        "Renombrar grupo",
                        "Nuevo nombre:",
                        InputAction::RenameGroup(g.clone()),
                    ));
                }
            }
            if ui
                .add_enabled(
                    sel.is_some() && !(self.busy || self.playing),
                    egui::Button::new("Eliminar"),
                )
                .clicked()
            {
                if let Some(g) = &sel {
                    self.confirm = Some(ConfirmState {
                        title: "Eliminar grupo".into(),
                        message: format!("¿Eliminar el grupo '{g}' y sus membresías?"),
                        action: ConfirmAction::DeleteGroup(g.clone()),
                    });
                }
            }
        });
        ui.separator();

        let mut select: Option<String> = None;
        for (name, count) in &self.snapshot.group_counts {
            let selected = self.selected_group.as_deref() == Some(name.as_str());
            if ui
                .selectable_label(selected, format!("{name} ({count} mods)"))
                .clicked()
            {
                select = Some(name.clone());
            }
        }
        if let Some(g) = select {
            if self.selected_group.as_deref() != Some(g.as_str()) {
                self.selected_group = Some(g.clone());
                self.reload_groups();
            }
        }

        if let Some(g) = self.selected_group.clone() {
            ui.separator();
            ui.label(egui::RichText::new(format!("Grupo: {g}")).strong());
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !(self.busy || self.playing),
                        egui::Button::new("Activar grupo"),
                    )
                    .on_hover_text("Activa todos sus mods en el perfil activo (con deps)")
                    .clicked()
                {
                    let slug = self.snapshot.active_slug.clone();
                    self.exec(
                        vec![
                            "ctl".into(),
                            "group".into(),
                            "enable".into(),
                            g.clone(),
                            "--profile".into(),
                            slug,
                        ],
                        false,
                    );
                }
                if ui
                    .add_enabled(
                        !(self.busy || self.playing),
                        egui::Button::new("Desactivar grupo"),
                    )
                    .clicked()
                {
                    let slug = self.snapshot.active_slug.clone();
                    self.exec(
                        vec![
                            "ctl".into(),
                            "group".into(),
                            "disable".into(),
                            g.clone(),
                            "--profile".into(),
                            slug,
                        ],
                        false,
                    );
                }
            });
            ui.add_space(4.0);
            ui.label("Miembros (perfil activo):");
            match &self.group_members {
                Some(mems) if !mems.is_empty() => {
                    for folder in mems {
                        ui.label(egui::RichText::new(format!("• {folder}")).small());
                    }
                }
                _ => {
                    ui.label(egui::RichText::new("(vacío)").weak().small());
                }
            }
        }
    }

    fn ui_conflicts(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        if self.conflicts.is_empty() {
            if self.conflicts_pending {
                ui.label("Calculando conflictos de archivos…");
            } else {
                ui.label("Sin conflictos de archivo entre los mods activos de este perfil.");
            }
            return;
        }

        let mut open: Option<String> = None;
        let mut open_folder: Option<String> = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for c in &self.conflicts {
                    ui.horizontal(|ui| {
                        let sev_color = match c.severity.as_str() {
                            "alta" => egui::Color32::from_rgb(230, 90, 90),
                            "media" => egui::Color32::from_rgb(230, 170, 90),
                            _ => egui::Color32::from_rgb(150, 150, 150),
                        };
                        ui.label(
                            egui::RichText::new(&c.severity).color(sev_color).strong().small(),
                        );
                        ui.monospace(&c.path);
                        if c.duplicate {
                            ui.label(egui::RichText::new("idéntico").weak().small());
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("provee:").weak().small());
                        ui.label(
                            egui::RichText::new(c.providers.join(" → ")).small(),
                        );
                        if let Some(winner) = c.providers.first() {
                            let w = winner.clone();
                            if ui.button("Ver ganador").clicked() {
                                open = Some(w.clone());
                            }
                            if ui.button("Carpeta").clicked() {
                                open_folder = Some(w.clone());
                            }
                        }
                    });
                    ui.separator();
                }
            });

        if let Some(folder) = open {
            if let Some(tm) = self.snapshot.mods.iter().find(|x| x.folder == folder).cloned() {
                self.selected_mod = Some(tm.id);
                self.reload_relations();
            }
        }
        if let Some(folder) = open_folder {
            self.exec(vec!["ctl".into(), "open".into(), folder], false);
        }
    }

    fn ui_log(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Limpiar").clicked() {
                self.log.clear();
            }
            if ui.button("Copiar").clicked() {
                ui.ctx().copy_text(self.log.join("\n"));
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for l in &self.log {
                    ui.monospace(l);
                }
            });
    }

    fn ui_dialogs(&mut self, ctx: &egui::Context) {
        if self.input.is_some() {
            // Take the state so its `value` buffer persists across frames; it is
            // put back when the dialog stays open.
            let mut input = self.input.take().expect("checked is_some");
            let mut open = true;
            let mut close = false;
            let mut submit = false;
            let title = input.title.clone();
            let label = input.label.clone();
            egui::Window::new(title)
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(label);
                    submit |= ui
                        .add(egui::TextEdit::singleline(&mut input.value).desired_width(220.0))
                        .lost_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            submit = true;
                        }
                        if ui.button("Cancelar").clicked() {
                            close = true;
                        }
                    });
                });
            if submit && !input.value.trim().is_empty() {
                let value = input.value.trim().to_string();
                self.apply_input(input.action, value);
            } else if close || !open {
                // descartar
            } else {
                self.input = Some(input);
            }
        }

        if self.confirm.is_some() {
            let (title, message) = {
                let c = self.confirm.as_ref().unwrap();
                (c.title.clone(), c.message.clone())
            };
            let mut open = true;
            let mut close = false;
            let mut ok = false;
            egui::Window::new(title)
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label(message);
                    ui.horizontal(|ui| {
                        if ui.button("Sí").clicked() {
                            ok = true;
                        }
                        if ui.button("No").clicked() {
                            close = true;
                        }
                    });
                });
            if ok {
                if let Some(c) = self.confirm.take() {
                    self.apply_confirm(c.action);
                }
            } else if close || !open {
                self.confirm = None;
            }
        }
    }

    fn apply_input(&mut self, action: InputAction, value: String) {
        match action {
            InputAction::Create => {
                self.exec(
                    vec!["ctl".into(), "profile".into(), "create".into(), value],
                    false,
                );
            }
            InputAction::Rename(slug) => {
                self.exec(
                    vec!["ctl".into(), "profile".into(), "rename".into(), slug, value],
                    false,
                );
            }
            InputAction::Copy(slug) => {
                self.exec(
                    vec!["ctl".into(), "profile".into(), "copy".into(), slug, value],
                    false,
                );
            }
            InputAction::NewMod => {
                self.exec(vec!["ctl".into(), "init".into(), value], false);
            }
            InputAction::RenameMod(folder) => {
                self.exec(
                    vec!["ctl".into(), "rename".into(), folder, value],
                    false,
                );
            }
            InputAction::NewGroup => {
                self.exec(vec!["ctl".into(), "group".into(), "create".into(), value], false);
            }
            InputAction::RenameGroup(ident) => {
                self.exec(
                    vec!["ctl".into(), "group".into(), "rename".into(), ident, value],
                    false,
                );
            }
        }
    }

    fn apply_confirm(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::DeleteProfile(slug) => {
                self.exec(
                    vec![
                        "ctl".into(),
                        "profile".into(),
                        "delete".into(),
                        slug,
                        "--yes".into(),
                    ],
                    false,
                );
            }
            ConfirmAction::DeleteMod(folder) => {
                self.exec(
                    vec![
                        "ctl".into(),
                        "remove".into(),
                        folder,
                        "--yes".into(),
                        "--profile".into(),
                        self.snapshot.active_slug.clone(),
                    ],
                    false,
                );
            }
            ConfirmAction::DeleteGroup(ident) => {
                self.exec(
                    vec![
                        "ctl".into(),
                        "group".into(),
                        "delete".into(),
                        ident,
                        "--yes".into(),
                    ],
                    false,
                );
            }
        }
    }
}
