use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{channel, Receiver, Sender};

use eframe::egui;

use crate::backend::{Backend, GuiEvent};
use crate::model::{filter_and_sort, Filters, Snapshot, SortField, StatusFilter};
use crate::settings::{Density, GuiSettings};
use crate::theme::{self, ThemePref};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Mods,
    Profiles,
    Groups,
    Dependencies,
    Conflicts,
    Log,
}

/// Below this logical width the layout switches to the single-column
/// (phone-like) mode: bottom navigation + detail as an overlay page.
const NARROW_BREAKPOINT: f32 = 760.0;

enum InputAction {
    Create,
    Rename(String),
    Copy(String),
    NewMod,
    RenameMod(String),
    RenameModFolder(String),
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
    settings: GuiSettings,
    base_ppp: f32,
    show_preferences: bool,
    show_about: bool,
    show_shortcuts: bool,
    focus_search: bool,
    toasts: crate::toasts::Toasts,
    lightbox: Option<crate::lightbox::Lightbox>,
}

impl GtaMoApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings = GuiSettings::load();
        let base_ppp = cc.egui_ctx.pixels_per_point();
        cc.egui_ctx
            .set_pixels_per_point(base_ppp * settings.ui_scale);
        crate::fonts::install(&cc.egui_ctx);
        cc.egui_ctx.set_theme(settings.theme.to_egui());
        theme::apply(&cc.egui_ctx, settings.theme_config());
        apply_style(&cc.egui_ctx, &settings);

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
            settings,
            base_ppp,
            show_preferences: false,
            show_about: false,
            show_shortcuts: false,
            focus_search: false,
            toasts: crate::toasts::Toasts::new(),
            lightbox: None,
        };
        app.refresh();
        app
    }

    /// Re-applies everything derived from [`GuiSettings`] after a change.
    fn apply_settings(&mut self, ctx: &egui::Context) {
        ctx.set_pixels_per_point(self.base_ppp * self.settings.ui_scale);
        ctx.set_theme(self.settings.theme.to_egui());
        theme::apply(ctx, self.settings.theme_config());
        apply_style(ctx, &self.settings);
    }

    fn set_theme(&mut self, ctx: &egui::Context, pref: ThemePref) {
        if self.settings.theme != pref {
            self.settings.theme = pref;
            self.settings.save();
            self.apply_settings(ctx);
        }
    }

    fn refresh(&mut self) {
        match self.backend.snapshot() {
            Ok(s) => {
                // Keep the selection only if it still exists (a profile may
                // have been renamed/deleted); otherwise fall back to active.
                let valid = self
                    .selected_profile
                    .as_deref()
                    .map(|sel| s.profiles.iter().any(|p| p.slug == sel))
                    .unwrap_or(false);
                if !valid {
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

    fn poll_events(&mut self, ctx: &egui::Context) {
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
                    let launched = self.playing;
                    self.busy = false;
                    self.playing = false;
                    if !ok {
                        // Abort any follow-up jobs: a failed write may have left
                        // the DB in an unknown state, so don't keep mutating.
                        self.pending.clear();
                        let text = if msg.is_empty() {
                            "La operación falló (ver Log)".to_string()
                        } else {
                            msg.clone()
                        };
                        self.toasts.push(ctx, crate::toasts::ToastKind::Error, text);
                    } else if launched {
                        self.toasts.push(
                            ctx,
                            crate::toasts::ToastKind::Success,
                            "El juego terminó",
                        );
                    }
                    self.pump();
                    if !self.busy {
                        // Refresh even on failure: some commands (e.g. rename)
                        // can mutate the DB before reporting an error.
                        self.refresh();
                        if !ok {
                            self.status = Some(if msg.is_empty() {
                                "La operación falló (ver Log)".to_string()
                            } else {
                                msg
                            });
                        }
                    }
                }
            }
        }
    }

    fn set_enabled(&mut self, id: i64, enabled: bool) {
        // Optimistic local update: reflect the click immediately; the CLI
        // persists it and the next refresh confirms (or reverts on failure).
        if let Some(m) = self.snapshot.mods.iter_mut().find(|m| m.id == id) {
            m.enabled = enabled;
        }
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

    /// Whether the list is in the default, priority-sorted view where drag&drop
    /// reordering is meaningful. Independent of whether a command is running, so
    /// the handles and the hint never shift the layout mid-operation.
    fn reorder_layout(&self) -> bool {
        self.filters.search.is_empty()
            && self.filters.tag.is_none()
            && self.filters.group.is_none()
            && self.filters.status == StatusFilter::All
            && self.filters.sort == SortField::Order
            && self.filters.desc
    }

    /// Whether reordering is actually allowed right now (nothing running).
    fn can_reorder(&self) -> bool {
        !(self.busy || self.playing) && self.reorder_layout()
    }

    /// Applies a drag&drop move: `dragged` is placed at index `index` of the
    /// full priority-sorted list (0 = top). One bulk `ctl reorder` call
    /// persists the whole new order; the local snapshot is updated immediately
    /// so the list does not snap back while the command runs.
    fn reorder_drop_at(&mut self, dragged: &str, index: usize) {
        if !self.can_reorder() {
            return;
        }
        let mut full = self.snapshot.mods.clone();
        full.sort_by_key(|m| std::cmp::Reverse(m.order));
        let Some(orig) = full.iter().position(|m| m.folder == dragged) else {
            return;
        };
        let full_folders: Vec<String> = full.iter().map(|m| m.folder.clone()).collect();
        let mut seq: Vec<String> = full_folders
            .iter()
            .filter(|f| **f != dragged)
            .cloned()
            .collect();
        // `index` is measured on the full list; account for the removed item so
        // downward drags land exactly where the insertion bar was.
        let insert_at = if orig < index { index - 1 } else { index };
        let insert_at = insert_at.min(seq.len());
        seq.insert(insert_at, dragged.to_string());
        if seq == full_folders {
            return;
        }

        // Optimistic local update.
        let n = seq.len() as i64;
        let mut order_of: HashMap<&str, i64> = HashMap::with_capacity(seq.len());
        for (i, folder) in seq.iter().enumerate() {
            order_of.insert(folder.as_str(), n - i as i64);
        }
        for m in &mut self.snapshot.mods {
            if let Some(o) = order_of.get(m.folder.as_str()) {
                m.order = *o;
            }
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
        let path = self.backend.cover_path(folder, cover)?;
        self.load_image(ctx, key, &path)
    }

    /// Loads (and LRU-caches) one image from disk as a texture.
    fn load_image(
        &mut self,
        ctx: &egui::Context,
        key: String,
        path: &std::path::Path,
    ) -> Option<egui::TextureHandle> {
        if let Some(t) = self.covers.get(&key) {
            return Some(t.clone());
        }
        if self.cover_missing.contains(&key) {
            return None;
        }
        let Some(img) = image::ImageReader::open(path)
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
        show_handle: bool,
    ) -> egui::Response {
        let idle = !(self.busy || self.playing);
        let palette = theme::active(ui.ctx());
        let fill = if m.enabled {
            palette.row_active_fill
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
                if show_handle {
                    // Kept visible (only disabled) while a command runs so the
                    // row layout never changes.
                    ui.add_enabled_ui(idle, |ui| {
                        let handle = egui::Id::new(("mod_handle", m.id));
                        ui.dnd_drag_source(handle, m.folder.clone(), |ui| {
                            ui.add(
                                egui::Button::new(crate::icons::MENU)
                                    .frame(false)
                                    .small()
                                    .min_size(egui::vec2(22.0, 24.0)),
                            )
                            .on_hover_text("Arrastra para reordenar");
                        });
                    });
                }
                if Self::toggle_indicator(ui, m.id, m.enabled, idle) {
                    self.set_enabled(m.id, !m.enabled);
                }
                if self.settings.show_covers {
                    if let Some(cover) = m.meta.cover.clone() {
                        if let Some(tex) = self.load_cover(ui.ctx(), &m.folder, &cover) {
                            ui.add(
                                egui::Image::new(&tex)
                                    .fit_to_exact_size(egui::vec2(40.0, 40.0))
                                    .corner_radius(4),
                            );
                        }
                    }
                }
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        let name_color = if m.enabled {
                            palette.text
                        } else {
                            palette.text_muted
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
                                        .color(palette.accent),
                                );
                            }
                            for g in &m.groups {
                                ui.label(egui::RichText::new(format!("[{g}]")).small().weak());
                            }
                        });
                    }
                    if let Some(dep) = self.snapshot.dep_status.get(&m.id) {
                        if dep.required > 0 || dep.optional > 0 {
                            let problems = !dep.disabled.is_empty() || !dep.missing.is_empty();
                            let color = if problems {
                                palette.danger
                            } else {
                                palette.text_muted
                            };
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} {} req · {} opt",
                                        crate::icons::SWAP_H,
                                        dep.required,
                                        dep.optional
                                    ))
                                    .small()
                                    .color(color),
                                );
                                if problems {
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{} requeridas sin resolver",
                                            crate::icons::WARN
                                        ))
                                        .small()
                                        .color(palette.danger),
                                    );
                                }
                            });
                        }
                    }
                });
                if ui
                    .button(format!("{} Detalle", crate::icons::PANEL_RIGHT))
                    .clicked()
                {
                    self.selected_mod = Some(m.id);
                }
            });
        });
        row.response
    }

    /// Indicador de estado propio: cuadrado redondeado de color acento cuando
    /// el mod está activo, gris cuando no. Devuelve `true` cuando se hace clic.
    fn toggle_indicator(ui: &mut egui::Ui, id: i64, active: bool, interactive: bool) -> bool {
        let palette = theme::active(ui.ctx());
        let anim = ui.style().animation_time;
        let t = if anim <= 0.0 {
            if active {
                1.0
            } else {
                0.0
            }
        } else {
            ui.ctx()
                .animate_bool_with_time(egui::Id::new(("toggle", id)), active, anim)
        };
        let (rect, response) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::click());
        let off = ui.visuals().widgets.inactive.weak_bg_fill;
        let fill = lerp_color(off, palette.accent, t);
        let stroke_color = lerp_color(palette.border, palette.on_accent, t);
        let inner = egui::Rect::from_center_size(rect.center(), egui::vec2(14.0, 14.0));
        ui.painter().rect(
            inner,
            egui::CornerRadius::same(4),
            fill,
            egui::Stroke::new(1.5, stroke_color),
            egui::StrokeKind::Inside,
        );
        if t > 0.01 {
            ui.painter().text(
                inner.center(),
                egui::Align2::CENTER_CENTER,
                crate::icons::CHECK,
                egui::FontId::proportional(12.0),
                palette.on_accent.gamma_multiply(t),
            );
        }
        let _ = response
            .clone()
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if !interactive {
            return false;
        }
        response.clicked()
    }
}

impl eframe::App for GtaMoApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events(ctx);
        let narrow = ctx.screen_rect().width() < NARROW_BREAKPOINT;

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if !narrow {
                    ui.heading("GTA SA Mod Organizer");
                    ui.separator();
                    ui.label("Perfil:");
                }
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
                if !narrow {
                    ui.separator();
                    ui.checkbox(&mut self.launch_debug, "Debug")
                        .on_hover_text("Habilitar log de Proton/DXVK (--debug)");
                    ui.checkbox(&mut self.launch_dry_run, "Previsualizar")
                        .on_hover_text(
                            "Mostrar el orden de capas sin montar ni lanzar (--dry-run)",
                        );
                }

                let mut new_theme: Option<ThemePref> = None;
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let idle = !self.busy && !self.playing;
                    let palette = theme::active(ui.ctx());

                    // Wide layout: the ⋮ menu is the single place for the app
                    // menu (narrow uses the bottom bar's "more" menu instead).
                    if !narrow {
                        ui.menu_button(crate::icons::ELLIPSIS_V, |ui| {
                            ui.set_min_width(200.0);
                            app_menu_items(self, ui, &mut new_theme);
                        })
                        .response
                        .on_hover_text("Menú");
                    }

                    let label = if self.playing {
                        format!("{} Jugando…", crate::icons::PLAY)
                    } else {
                        format!("{} Jugar", crate::icons::PLAY)
                    };
                    let play = egui::Button::new(
                        egui::RichText::new(label).color(palette.on_accent).strong(),
                    )
                    .fill(palette.accent);
                    if ui.add_enabled(idle, play).clicked() {
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
                    if !narrow {
                        if ui
                            .add_enabled(
                                idle,
                                egui::Button::new(format!("{} Limpiar", crate::icons::ERASER)),
                            )
                            .on_hover_text("Eliminar mods huérfanos (carpetas desaparecidas)")
                            .clicked()
                        {
                            self.exec(vec!["ctl".into(), "clean".into()], false);
                        }
                        if ui
                            .add_enabled(
                                idle,
                                egui::Button::new(format!("{} Descubrir", crate::icons::SEARCH)),
                            )
                            .on_hover_text(
                                "Escanear mods/ y registrar/actualizar mods y dependencias",
                            )
                            .clicked()
                        {
                            self.exec(vec!["ctl".into(), "discover".into()], false);
                        }
                    }
                });
                if let Some(pref) = new_theme {
                    self.set_theme(ctx, pref);
                }
            });
            ui.add_space(6.0);
        });

        if !narrow {
            egui::SidePanel::left("nav")
                .resizable(false)
                .exact_width(200.0)
                .show(ctx, |ui| self.ui_sidebar(ui));
        }

        // Status bar is declared before the central panel so it is not
        // overlapped; on narrow screens the nav bar sits just above it.
        self.ui_status_bar(ctx);
        if narrow {
            egui::TopBottomPanel::bottom("bottom_nav").show(ctx, |ui| self.ui_bottom_nav(ui));
        }

        let content_frame = egui::Frame::central_panel(&ctx.style()).fill(theme::active(ctx).bg);
        egui::CentralPanel::default()
            .frame(content_frame)
            .show(ctx, |ui| match self.tab {
                Tab::Mods => self.ui_mods(ui),
                Tab::Profiles => self.ui_profiles(ui),
                Tab::Groups => self.ui_groups(ui),
                Tab::Dependencies => self.ui_dependencies(ui),
                Tab::Conflicts => self.ui_conflicts(ui),
                Tab::Log => self.ui_log(ui),
            });

        // Mod detail: a floating window when wide, a full-screen overlay page
        // when narrow (libadwaita `NavigationSplitView` style).
        if self.selected_mod.is_some() {
            if narrow {
                let screen = ctx.screen_rect();
                let palette = theme::active(ctx);
                let frame = egui::Frame::new()
                    .fill(palette.surface)
                    .stroke(egui::Stroke::new(1.0, palette.border))
                    .corner_radius(egui::CornerRadius::same(12))
                    .inner_margin(egui::Margin::same(12));
                let resp = egui::Modal::new(egui::Id::new("detail_overlay"))
                    .frame(frame)
                    .show(ctx, |ui| {
                        ui.set_min_size(egui::vec2(
                            (screen.width() - 40.0).max(200.0),
                            (screen.height() - 120.0).max(200.0),
                        ));
                        self.ui_detail(ui, true);
                    });
                if resp.should_close() {
                    self.selected_mod = None;
                }
            } else {
                self.ui_detail_window(ctx);
            }
        }

        self.handle_shortcuts(ctx);
        self.ui_dialogs(ctx);

        if self.show_preferences {
            let changed =
                crate::preferences::show(ctx, &mut self.settings, &mut self.show_preferences);
            if changed {
                self.settings.save();
                self.apply_settings(ctx);
                self.toasts.push(
                    ctx,
                    crate::toasts::ToastKind::Info,
                    "Preferencias guardadas",
                );
            }
        }
        if self.show_about {
            crate::about::show_about(ctx, &self.settings, &mut self.show_about);
        }
        if self.show_shortcuts {
            crate::about::show_shortcuts(ctx, &mut self.show_shortcuts);
        }

        self.ui_lightbox(ctx);
        self.toasts.show(ctx);
    }
}

impl GtaMoApp {
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::R)) {
            self.backend.reload();
            self.refresh();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Comma)) {
            self.show_preferences = true;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::F)) {
            self.tab = Tab::Mods;
            self.focus_search = true;
        }
    }
}

impl GtaMoApp {
    fn ui_status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            let palette = theme::active(ui.ctx());
            ui.horizontal(|ui| {
                if let Some(e) = self.backend.error_str() {
                    ui.colored_label(palette.danger, e);
                    if ui.button("Reintentar").clicked() {
                        self.backend.retry();
                        self.refresh();
                    }
                } else if let Some(e) = &self.status {
                    ui.colored_label(palette.warning, e);
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
    }

    /// Bottom navigation bar for the narrow (phone-like) layout.
    fn ui_bottom_nav(&mut self, ui: &mut egui::Ui) {
        let dep_problems: usize = self
            .snapshot
            .dep_status
            .values()
            .filter(|s| !s.disabled.is_empty() || !s.missing.is_empty())
            .count()
            + self.snapshot.dep_cycles.len();
        let conflicts = self.conflicts.iter().filter(|c| !c.duplicate).count();

        // Only the three primary destinations live in the bar; everything else
        // is in "more" so the window can get very narrow.
        let items: [(Tab, &str, &str, Option<usize>); 3] = [
            (Tab::Mods, crate::icons::LIST, "Mods", None),
            (Tab::Profiles, crate::icons::USERS, "Perfiles", None),
            (Tab::Log, crate::icons::INFO, "Log", None),
        ];
        let mut new_theme: Option<ThemePref> = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let w = ui.available_width() / (items.len() + 1) as f32;
            for (tab, icon, label, badge) in items {
                if bottom_nav_item(ui, self.tab == tab, icon, label, badge, w) {
                    self.tab = tab;
                }
            }
            ui.allocate_ui_with_layout(
                egui::vec2(w, 52.0),
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    ui.menu_button(crate::icons::ELLIPSIS_V, |ui| {
                        ui.set_min_width(200.0);
                        let hidden: [(Tab, &str, &str, Option<usize>); 3] = [
                            (Tab::Groups, crate::icons::FOLDER, "Grupos", None),
                            (
                                Tab::Dependencies,
                                crate::icons::SWAP_H,
                                "Dependencias",
                                (dep_problems > 0).then_some(dep_problems),
                            ),
                            (
                                Tab::Conflicts,
                                crate::icons::WARN,
                                "Conflictos",
                                (conflicts > 0).then_some(conflicts),
                            ),
                        ];
                        for (tab, icon, label, badge) in hidden {
                            let text = match badge {
                                Some(n) => format!("{icon}  {label} ({n})"),
                                None => format!("{icon}  {label}"),
                            };
                            if ui.button(text).clicked() {
                                self.tab = tab;
                                ui.close_menu();
                            }
                        }
                        ui.separator();
                        if ui
                            .button(format!("{} Descubrir", crate::icons::SEARCH))
                            .clicked()
                        {
                            self.exec(vec!["ctl".into(), "discover".into()], false);
                            ui.close_menu();
                        }
                        if ui
                            .button(format!("{} Limpiar", crate::icons::ERASER))
                            .clicked()
                        {
                            self.exec(vec!["ctl".into(), "clean".into()], false);
                            ui.close_menu();
                        }
                        ui.separator();
                        ui.checkbox(&mut self.launch_debug, "Debug");
                        ui.checkbox(&mut self.launch_dry_run, "Previsualizar (dry-run)");
                        ui.separator();
                        app_menu_items(self, ui, &mut new_theme);
                    });
                },
            );
        });
        if let Some(pref) = new_theme {
            self.set_theme(ui.ctx(), pref);
        }
    }

    fn ui_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        let dep_problems: usize = self
            .snapshot
            .dep_status
            .values()
            .filter(|s| !s.disabled.is_empty() || !s.missing.is_empty())
            .count()
            + self.snapshot.dep_cycles.len();
        let conflicts = self.conflicts.iter().filter(|c| !c.duplicate).count();

        let items: [(Tab, &str, &str, Option<usize>); 6] = [
            (Tab::Mods, crate::icons::LIST, "Mods", None),
            (Tab::Profiles, crate::icons::USERS, "Perfiles", None),
            (Tab::Groups, crate::icons::FOLDER, "Grupos", None),
            (
                Tab::Dependencies,
                crate::icons::SWAP_H,
                "Dependencias",
                (dep_problems > 0).then_some(dep_problems),
            ),
            (
                Tab::Conflicts,
                crate::icons::WARN,
                "Conflictos",
                (conflicts > 0).then_some(conflicts),
            ),
            (Tab::Log, crate::icons::INFO, "Log", None),
        ];
        for (tab, icon, label, badge) in items {
            if nav_item(ui, self.tab == tab, icon, label, badge) {
                self.tab = tab;
            }
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            let w = ui.available_width();
            if ui
                .add_sized(
                    egui::vec2(w, 28.0),
                    egui::Button::new(format!("{} Refrescar", crate::icons::REFRESH)),
                )
                .clicked()
            {
                self.backend.reload();
                self.refresh();
            }
        });
    }

    /// Number of active mod-list filters (for the "Filtros (n)" badge).
    fn active_filter_count(&self) -> usize {
        self.filters.tag.is_some() as usize
            + self.filters.group.is_some() as usize
            + (self.filters.status != StatusFilter::All) as usize
            + (self.filters.sort != SortField::Order) as usize
            + (!self.filters.search.trim().is_empty()) as usize
    }

    fn ui_filter_controls(&mut self, ui: &mut egui::Ui) {
        let mut tag = self.filters.tag.clone();
        egui::ComboBox::from_id_salt("tag_filter")
            .selected_text(tag.clone().unwrap_or_else(|| "Todos los tags".into()))
            .show_ui(ui, |ui| {
                ui.set_min_width(180.0);
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
                ui.set_min_width(180.0);
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

        egui::ComboBox::from_id_salt("status_filter")
            .selected_text(self.filters.status.label())
            .show_ui(ui, |ui| {
                for s in [
                    StatusFilter::All,
                    StatusFilter::Enabled,
                    StatusFilter::Disabled,
                ] {
                    ui.selectable_value(&mut self.filters.status, s, s.label());
                }
            });

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
    }

    fn ui_new_mod_button(&mut self, ui: &mut egui::Ui) {
        if ui
            .add_enabled(
                !(self.busy || self.playing),
                egui::Button::new(format!("{} Nuevo mod", crate::icons::PLUS)),
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
    }

    fn ui_mods(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        let narrow = ui.ctx().screen_rect().width() < NARROW_BREAKPOINT;
        if narrow {
            // Narrow: search on its own full-width line, then a Filters menu
            // plus the "new mod" action so nothing overflows.
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.filters.search)
                    .hint_text("Buscar mods…")
                    .desired_width(f32::INFINITY),
            );
            if self.focus_search {
                search.request_focus();
                self.focus_search = false;
            }
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let active = self.active_filter_count();
                let label = if active > 0 {
                    format!("{} Filtros ({active})", crate::icons::SLIDERS)
                } else {
                    format!("{} Filtros", crate::icons::SLIDERS)
                };
                ui.menu_button(label, |ui| {
                    ui.set_min_width(220.0);
                    self.ui_filter_controls(ui);
                });
                self.ui_new_mod_button(ui);
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                let search = ui.add(
                    egui::TextEdit::singleline(&mut self.filters.search)
                        .hint_text("Buscar mods…")
                        .desired_width(220.0),
                );
                if self.focus_search {
                    search.request_focus();
                    self.focus_search = false;
                }
                self.ui_filter_controls(ui);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.ui_new_mod_button(ui);
                });
            });
        }
        ui.separator();

        let mut filtered = self.snapshot.mods.clone();
        filter_and_sort(&mut filtered, &self.filters);

        let show_handles = self.reorder_layout();
        let interactive = !(self.busy || self.playing);
        egui::ScrollArea::vertical()
            .id_salt("mods_scroll")
            .auto_shrink(false)
            .show(ui, |ui| {
                if show_handles {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(crate::icons::MENU).weak());
                        ui.label(
                            egui::RichText::new("Arrastra para reordenar")
                                .weak()
                                .small(),
                        );
                    });
                    ui.add_space(2.0);
                }

                // (rect, folder) de cada fila, para el indicador de inserción.
                let mut row_rects: Vec<(egui::Rect, String)> = Vec::new();
                for m in &filtered {
                    let row = self.draw_mod_row(ui, m, show_handles);
                    if show_handles {
                        row_rects.push((row.rect, m.folder.clone()));
                    }
                    ui.separator();
                }

                if interactive && show_handles && !row_rects.is_empty() {
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
                            row_rects
                                .last()
                                .map(|(r, _)| r.bottom())
                                .unwrap_or_else(|| ui.clip_rect().bottom())
                        };
                        let x0 = ui.clip_rect().left() + 4.0;
                        let x1 = ui.clip_rect().right() - 4.0;
                        ui.painter().line_segment(
                            [egui::pos2(x0, y), egui::pos2(x1, y)],
                            egui::Stroke::new(2.0, theme::active(ui.ctx()).accent),
                        );
                    } else if let Some(folder) = prev_dragging {
                        // Suelta: persiste el orden con la posición señalada.
                        let index = self.drop_index;
                        self.reorder_drop_at(&folder, index);
                    }
                }
            });
    }

    /// Floating, movable detail window used in the wide layout.
    fn ui_detail_window(&mut self, ctx: &egui::Context) {
        let Some(id) = self.selected_mod else {
            return;
        };
        let title = self
            .snapshot
            .mods
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "Detalle".to_string());
        let mut open = true;
        let start = ctx.screen_rect();
        egui::Window::new(title)
            .id(egui::Id::new("detail_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_pos(egui::pos2(
                (start.right() - 460.0).max(220.0),
                start.top() + 90.0,
            ))
            .default_size(egui::vec2(420.0, 620.0))
            .show(ctx, |ui| {
                ui.set_min_width(320.0);
                self.ui_detail(ui, false);
            });
        if !open {
            self.selected_mod = None;
        }
    }

    fn ui_detail(&mut self, ui: &mut egui::Ui, narrow: bool) {
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

        // Wide uses a window whose title bar already shows the name and a
        // close button, so only the narrow overlay needs a header/back button.
        if narrow {
            ui.horizontal(|ui| {
                ui.heading(&m.name);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(format!("{} Atrás", crate::icons::CHEVRON_LEFT))
                        .clicked()
                    {
                        self.selected_mod = None;
                    }
                });
            });
            ui.separator();
        }

        if let Some(cover) = m.meta.cover.clone() {
            if let Some(tex) = self.load_cover(ui.ctx(), &m.folder, &cover) {
                let resp = ui
                    .add(
                        egui::Image::new(&tex)
                            .max_width(320.0)
                            .max_height(200.0)
                            .corner_radius(6)
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_text(format!("{} Ampliar", crate::icons::MAXIMIZE));
                if resp.clicked() {
                    let items = self.gallery_items(&m);
                    if !items.is_empty() {
                        self.lightbox = Some(crate::lightbox::Lightbox::new(items, 0));
                    }
                }
            }
        }

        let shots = m.screenshots.clone();
        if !shots.is_empty() {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!(
                    "{} Capturas ({})",
                    crate::icons::IMAGE,
                    shots.len()
                ))
                .strong(),
            );
            egui::ScrollArea::horizontal()
                .id_salt(("shots", m.id))
                .max_height(120.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (i, rel) in shots.iter().take(24).enumerate() {
                            let key = format!("{}/{}", m.folder, rel);
                            if let Some(path) = self.backend.cover_path(&m.folder, rel) {
                                if let Some(tex) = self.load_image(ui.ctx(), key, &path) {
                                    let resp = ui
                                        .add(
                                            egui::Image::new(&tex)
                                                .fit_to_exact_size(egui::vec2(150.0, 90.0))
                                                .corner_radius(4)
                                                .sense(egui::Sense::click()),
                                        )
                                        .on_hover_text("Ampliar");
                                    if resp.clicked() {
                                        let items = self.gallery_items(&m);
                                        let base = items.len().saturating_sub(shots.len());
                                        let idx = (base + i).min(items.len().saturating_sub(1));
                                        if !items.is_empty() {
                                            self.lightbox =
                                                Some(crate::lightbox::Lightbox::new(items, idx));
                                        }
                                    }
                                }
                            }
                        }
                    });
                });
        }

        let rel = self.relations.clone();
        let mut go_to: Option<String> = None;
        egui::ScrollArea::vertical()
            .id_salt("detail_scroll")
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
                                format!(
                                    "{name} ({})",
                                    if *required { "requerido" } else { "opcional" }
                                )
                            };
                            let color = if *enabled {
                                theme::active(ui.ctx()).success
                            } else {
                                theme::active(ui.ctx()).danger
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
                                theme::active(ui.ctx()).success
                            } else {
                                theme::active(ui.ctx()).text_muted
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
                            egui::Button::new(format!("{} Cambiar nombre", crate::icons::PENCIL)),
                        )
                        .on_hover_text("Cambiar el nombre visible (y el de mod.toml)")
                        .clicked()
                    {
                        self.input = Some(InputState::new(
                            "Cambiar nombre",
                            "Nuevo nombre:",
                            InputAction::RenameMod(folder.clone()),
                        ));
                    }
                    if ui
                        .add_enabled(
                            !(self.busy || self.playing),
                            egui::Button::new(format!(
                                "{} Renombrar carpeta",
                                crate::icons::FOLDER_OPEN
                            )),
                        )
                        .on_hover_text("Renombrar la carpeta del mod en disco")
                        .clicked()
                    {
                        self.input = Some(InputState::new(
                            "Renombrar carpeta",
                            "Nueva carpeta:",
                            InputAction::RenameModFolder(folder.clone()),
                        ));
                    }
                    if ui
                        .add_enabled(
                            !(self.busy || self.playing),
                            egui::Button::new(format!("{} Eliminar", crate::icons::TRASH)),
                        )
                        .clicked()
                    {
                        self.confirm = Some(ConfirmState {
                            title: "Eliminar mod".into(),
                            message: format!("¿Eliminar el mod '{folder}' y sus estados?"),
                            action: ConfirmAction::DeleteMod(folder.clone()),
                        });
                    }
                    if ui
                        .button(format!("{} Abrir carpeta", crate::icons::FOLDER_OPEN))
                        .clicked()
                    {
                        self.exec(vec!["ctl".into(), "open".into(), folder], false);
                    }
                    if m.meta.url.is_some() {
                        let folder = m.folder.clone();
                        if ui
                            .button(format!("{} Abrir URL", crate::icons::LINK))
                            .clicked()
                        {
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
                                        .selectable_label(
                                            pick.as_deref() == Some(name.as_str()),
                                            name,
                                        )
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
                                format!("{} Quitar del grupo", crate::icons::X)
                            } else {
                                format!("{} Añadir al grupo", crate::icons::TAG)
                            };
                            if ui
                                .add_enabled(!(self.busy || self.playing), egui::Button::new(label))
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

    /// Builds the browsable gallery (cover first, when present, then every
    /// screenshot) for a mod.
    fn gallery_items(&self, m: &crate::model::ModView) -> Vec<crate::lightbox::GalleryItem> {
        let mut items = Vec::new();
        if let Some(cover) = &m.meta.cover {
            if let Some(path) = self.backend.cover_path(&m.folder, cover) {
                items.push(crate::lightbox::GalleryItem {
                    key: format!("{}/{}", m.folder, cover),
                    path,
                    label: format!("Portada — {}", m.name),
                });
            }
        }
        for rel in &m.screenshots {
            if let Some(path) = self.backend.cover_path(&m.folder, rel) {
                let key = format!("{}/{}", m.folder, rel);
                items.push(crate::lightbox::GalleryItem {
                    key,
                    path,
                    label: rel.clone(),
                });
            }
        }
        items
    }

    fn ui_profiles(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !(self.busy || self.playing),
                    egui::Button::new(format!("{} Nuevo", crate::icons::PLUS)),
                )
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
                    egui::Button::new(format!("{} Usar", crate::icons::CIRCLE_CHECK)),
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
                    egui::Button::new(format!("{} Renombrar", crate::icons::PENCIL)),
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
                    egui::Button::new(format!("{} Eliminar", crate::icons::TRASH)),
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
        egui::ScrollArea::vertical()
            .id_salt("profiles_scroll")
            .show(ui, |ui| {
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
                            egui::RichText::new(format!(
                                "{} mods · {} activos",
                                p.total, p.enabled
                            ))
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
                    egui::Button::new(format!("{} Nuevo grupo", crate::icons::PLUS)),
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
                    egui::Button::new(format!("{} Renombrar", crate::icons::PENCIL)),
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
                    egui::Button::new(format!("{} Eliminar", crate::icons::TRASH)),
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
                        egui::Button::new(format!("{} Activar grupo", crate::icons::CIRCLE_CHECK)),
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
                        egui::Button::new(format!("{} Desactivar grupo", crate::icons::X)),
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

    fn ui_dependencies(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        let mut go_to: Option<String> = None;

        let has_issues = !self.snapshot.dep_cycles.is_empty()
            || self
                .snapshot
                .dep_status
                .values()
                .any(|s| !s.disabled.is_empty() || !s.missing.is_empty());

        if !has_issues {
            ui.label("Sin problemas de dependencias en este perfil.");
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "Las dependencias requeridas de los mods activados están satisfechas.",
                )
                .weak()
                .small(),
            );
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt("deps_scroll")
            .auto_shrink(false)
            .show(ui, |ui| {
                if !self.snapshot.dep_cycles.is_empty() {
                    ui.label(
                        egui::RichText::new("Ciclos de dependencias")
                            .strong()
                            .color(theme::active(ui.ctx()).danger),
                    );
                    for cyc in &self.snapshot.dep_cycles {
                        ui.label(
                            egui::RichText::new(format!("  {}", cyc.join(" → ")))
                                .monospace()
                                .small(),
                        );
                    }
                    ui.add_space(8.0);
                }

                let mods = self.snapshot.mods.clone();
                for m in &mods {
                    let Some(s) = self.snapshot.dep_status.get(&m.id) else {
                        continue;
                    };
                    if s.disabled.is_empty() && s.missing.is_empty() {
                        continue;
                    }
                    ui.label(egui::RichText::new(&m.name).strong());
                    for d in &s.disabled {
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("  requiere '{d}' (desactivado)"))
                                    .color(theme::active(ui.ctx()).warning)
                                    .small(),
                            );
                            if ui
                                .small_button(format!("{} Ver", crate::icons::EXTERNAL_LINK))
                                .clicked()
                            {
                                go_to = Some(d.clone());
                            }
                        });
                    }
                    for d in &s.missing {
                        ui.label(
                            egui::RichText::new(format!("  requiere {d} (no instalado)"))
                                .color(theme::active(ui.ctx()).danger)
                                .small(),
                        );
                    }
                    ui.separator();
                }
            });

        if let Some(folder) = go_to {
            if let Some(tm) = self
                .snapshot
                .mods
                .iter()
                .find(|x| x.folder == folder)
                .cloned()
            {
                self.selected_mod = Some(tm.id);
                self.reload_relations();
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
            .id_salt("conflicts_scroll")
            .auto_shrink(false)
            .show(ui, |ui| {
                for c in &self.conflicts {
                    ui.horizontal(|ui| {
                        let palette = theme::active(ui.ctx());
                        let sev_color = match c.severity.as_str() {
                            "alta" => palette.danger,
                            "media" => palette.warning,
                            _ => palette.text_muted,
                        };
                        ui.label(
                            egui::RichText::new(&c.severity)
                                .color(sev_color)
                                .strong()
                                .small(),
                        );
                        ui.monospace(&c.path);
                        if c.duplicate {
                            ui.label(egui::RichText::new("idéntico").weak().small());
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("provee:").weak().small());
                        ui.label(egui::RichText::new(c.providers.join(" → ")).small());
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
            if let Some(tm) = self
                .snapshot
                .mods
                .iter()
                .find(|x| x.folder == folder)
                .cloned()
            {
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
            if ui
                .button(format!("{} Limpiar", crate::icons::ERASER))
                .clicked()
            {
                self.log.clear();
            }
            if ui
                .button(format!("{} Copiar", crate::icons::SAVE))
                .clicked()
            {
                ui.ctx().copy_text(self.log.join("\n"));
            }
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("log_scroll")
            .auto_shrink(false)
            .show(ui, |ui| {
                for l in &self.log {
                    ui.monospace(l);
                }
            });
    }

    /// Full-screen image viewer with arrows, counter and dots.
    fn ui_lightbox(&mut self, ctx: &egui::Context) {
        let Some(mut lb) = self.lightbox.take() else {
            return;
        };
        let total = lb.len();
        if total == 0 {
            return;
        }
        let mut go = 0i32;
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)) {
            go += 1;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)) {
            go -= 1;
        }
        if go != 0 && total > 1 {
            lb.index = (lb.index as i32 + go).rem_euclid(total as i32) as usize;
        }

        let index = lb.index;
        let (key, path, label) = {
            let it = &lb.items[index];
            (it.key.clone(), it.path.clone(), it.label.clone())
        };
        let tex = self.load_image(ctx, key, &path);
        let palette = theme::active(ctx);
        let screen = ctx.screen_rect();

        // Fixed content box so the modal never resizes/re-centers; the image is
        // scaled to fit (up or down) inside a reserved viewport, preserving its
        // aspect ratio.
        let margin = 24.0;
        let content_w = (screen.width() - margin * 2.0).max(240.0);
        let content_h = (screen.height() - margin * 2.0).max(240.0);
        let footer_h = if total > 1 { 60.0 } else { 0.0 };
        let img_area = egui::vec2(
            (content_w - 8.0).max(64.0),
            (content_h - 34.0 - footer_h).max(64.0),
        );

        let mut close = false;
        let mut nav = 0i32;
        let mut pick: Option<usize> = None;
        let resp = egui::Modal::new(egui::Id::new("gta_mo_lightbox"))
            .backdrop_color(egui::Color32::from_black_alpha(220))
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::TRANSPARENT)
                    .inner_margin(egui::Margin::same(8)),
            )
            .show(ctx, |ui| {
                ui.set_min_size(egui::vec2(content_w, content_h));
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&label).color(palette.text).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(crate::icons::X)
                            .on_hover_text("Cerrar (Esc)")
                            .clicked()
                        {
                            close = true;
                        }
                    });
                });
                ui.add_space(6.0);
                match &tex {
                    Some(tex) => {
                        let size = tex.size_vec2();
                        let scale =
                            (img_area.x / size.x.max(1.0)).min(img_area.y / size.y.max(1.0));
                        ui.allocate_ui(img_area, |ui| {
                            ui.centered_and_justified(|ui| {
                                ui.add(
                                    egui::Image::new(tex)
                                        .fit_to_exact_size(size * scale)
                                        .corner_radius(6),
                                );
                            });
                        });
                        if total > 1 {
                            ui.vertical_centered(|ui| {
                                ui.horizontal(|ui| {
                                    if ui
                                        .button(crate::icons::CHEVRON_LEFT)
                                        .on_hover_text("Anterior (←)")
                                        .clicked()
                                    {
                                        nav = -1;
                                    }
                                    ui.label(format!("{} / {}", index + 1, total));
                                    if ui
                                        .button(crate::icons::CHEVRON_RIGHT)
                                        .on_hover_text("Siguiente (→)")
                                        .clicked()
                                    {
                                        nav = 1;
                                    }
                                });
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    for i in 0..total {
                                        let selected = i == index;
                                        let (r, resp) = ui.allocate_exact_size(
                                            egui::vec2(12.0, 12.0),
                                            egui::Sense::click(),
                                        );
                                        ui.painter().circle_filled(
                                            r.center(),
                                            if selected { 5.0 } else { 3.0 },
                                            if selected {
                                                palette.accent
                                            } else {
                                                palette.text_muted
                                            },
                                        );
                                        if resp.clicked() {
                                            pick = Some(i);
                                        }
                                    }
                                });
                            });
                        }
                    }
                    None => {
                        ui.vertical_centered(|ui| {
                            ui.label(
                                egui::RichText::new("No se pudo cargar la imagen")
                                    .color(palette.danger),
                            );
                        });
                    }
                }
            });
        if resp.should_close() {
            close = true;
        }
        if let Some(i) = pick {
            lb.index = i;
        } else if nav != 0 && total > 1 {
            lb.index = (lb.index as i32 + nav).rem_euclid(total as i32) as usize;
        }
        if !close {
            self.lightbox = Some(lb);
        }
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
                self.exec(vec!["ctl".into(), "rename".into(), folder, value], false);
            }
            InputAction::RenameModFolder(folder) => {
                self.exec(
                    vec![
                        "ctl".into(),
                        "rename".into(),
                        folder,
                        value,
                        "--folder".into(),
                    ],
                    false,
                );
            }
            InputAction::NewGroup => {
                self.exec(
                    vec!["ctl".into(), "group".into(), "create".into(), value],
                    false,
                );
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

/// Applies spacing, control sizes and animation derived from the settings.
/// Colors are handled separately by [`theme::apply`].
fn apply_style(ctx: &egui::Context, settings: &GuiSettings) {
    let mut style = (*ctx.style()).clone();
    style.animation_time = if settings.reduce_motion { 0.0 } else { 0.18 };
    let (item, pad, min_h) = match settings.density {
        Density::Compact => (4.0, egui::vec2(6.0, 3.0), 22.0),
        Density::Cozy => (8.0, egui::vec2(10.0, 5.0), 26.0),
    };
    style.spacing.item_spacing = egui::vec2(item, item);
    style.spacing.button_padding = pad;
    style.spacing.interact_size.y = min_h;

    // Consistent type scale (título / cuerpo / botón / pequeño / mono).
    use egui::{FontFamily, FontId, TextStyle};
    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(20.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(11.5, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(12.5, FontFamily::Monospace),
        ),
    ]
    .into();

    ctx.set_style(style);
}

/// A full-width, left-aligned navigation row for the sidebar, with an optional
/// count badge. Returns `true` when clicked.
fn nav_item(
    ui: &mut egui::Ui,
    selected: bool,
    icon: &str,
    label: &str,
    badge: Option<usize>,
) -> bool {
    let palette = theme::active(ui.ctx());
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::click());
    let radius = egui::CornerRadius::same(6);
    if selected {
        ui.painter()
            .rect_filled(rect, radius, palette.row_active_fill);
    } else if resp.hovered() {
        ui.painter()
            .rect_filled(rect, radius, palette.surface_raised);
    }
    let text_color = if selected {
        palette.text
    } else {
        palette.text_muted
    };
    let font = egui::FontId::proportional(14.0);
    ui.painter().text(
        egui::pos2(rect.left() + 12.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        icon,
        font.clone(),
        text_color,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 40.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        text_color,
    );
    if let Some(n) = badge {
        let badge_rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 18.0, rect.center().y),
            egui::vec2(24.0, 18.0),
        );
        ui.painter()
            .rect_filled(badge_rect, egui::CornerRadius::same(9), palette.accent);
        ui.painter().text(
            badge_rect.center(),
            egui::Align2::CENTER_CENTER,
            n.to_string(),
            egui::FontId::proportional(11.0),
            palette.on_accent,
        );
    }
    resp.clone().on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.clicked()
}

/// Linear interpolation between two colors (per-channel, sRGB).
fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    egui::Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

/// Icon for a theme preference.
fn theme_icon(pref: ThemePref) -> &'static str {
    match pref {
        ThemePref::System => crate::icons::MONITOR,
        ThemePref::Light => crate::icons::SUN,
        ThemePref::Dark => crate::icons::MOON,
    }
}

/// A bottom-navigation item (icon + caption), used in the narrow layout.
fn bottom_nav_item(
    ui: &mut egui::Ui,
    selected: bool,
    icon: &str,
    label: &str,
    badge: Option<usize>,
    width: f32,
) -> bool {
    let palette = theme::active(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 52.0), egui::Sense::click());
    if selected {
        ui.painter().rect_filled(
            rect.shrink2(egui::vec2(4.0, 4.0)),
            egui::CornerRadius::same(8),
            palette.row_active_fill,
        );
    }
    let color = if selected {
        palette.accent
    } else {
        palette.text_muted
    };
    ui.painter().text(
        egui::pos2(rect.center().x, rect.top() + 18.0),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(18.0),
        color,
    );
    ui.painter().text(
        egui::pos2(rect.center().x, rect.bottom() - 12.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(10.0),
        color,
    );
    if let Some(n) = badge {
        let badge_rect = egui::Rect::from_center_size(
            egui::pos2(rect.center().x + 16.0, rect.top() + 12.0),
            egui::vec2(16.0, 14.0),
        );
        ui.painter()
            .rect_filled(badge_rect, egui::CornerRadius::same(7), palette.accent);
        ui.painter().text(
            badge_rect.center(),
            egui::Align2::CENTER_CENTER,
            n.to_string(),
            egui::FontId::proportional(9.0),
            palette.on_accent,
        );
    }
    resp.clone().on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.clicked()
}

/// Shared "app" part of the ⋮ / "more" menus: preferences, about, shortcuts and
/// the theme selector. Writes the chosen theme into `new_theme`.
fn app_menu_items(app: &mut GtaMoApp, ui: &mut egui::Ui, new_theme: &mut Option<ThemePref>) {
    if ui
        .button(format!("{} Preferencias…", crate::icons::SETTINGS))
        .clicked()
    {
        app.show_preferences = true;
        ui.close_menu();
    }
    if ui
        .button(format!("{} Acerca de", crate::icons::INFO))
        .clicked()
    {
        app.show_about = true;
        ui.close_menu();
    }
    if ui
        .button(format!("{} Atajos de teclado", crate::icons::KEYBOARD))
        .clicked()
    {
        app.show_shortcuts = true;
        ui.close_menu();
    }
    ui.separator();
    ui.label(egui::RichText::new("Tema").small().weak());
    for pref in [ThemePref::System, ThemePref::Light, ThemePref::Dark] {
        if ui
            .radio(
                app.settings.theme == pref,
                format!("{}  {}", theme_icon(pref), pref.label()),
            )
            .clicked()
        {
            *new_theme = Some(pref);
            ui.close_menu();
        }
    }
}
