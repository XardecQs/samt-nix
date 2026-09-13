use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

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
    Data,
    Dependencies,
    Conflicts,
    Log,
}

/// Below this logical width the layout switches to the single-column
/// (phone-like) mode: bottom navigation + detail as an overlay page.
const NARROW_BREAKPOINT: f32 = 760.0;

/// Which header controls have overflowed into the ⋮ menu, so they are drawn
/// there instead of the header (each control appears in exactly one place).
#[derive(Clone, Copy, Default)]
struct HeaderOverflow {
    launch_opts: bool,
    clean: bool,
    discover: bool,
}

/// Límites al decodificar imágenes de mods (no confiables): evita bombas de
/// descompresión. 8192px de lado y 64 MiB de asignación por imagen.
const MAX_IMAGE_DIM: u32 = 8192;
const MAX_IMAGE_ALLOC: u64 = 64 * 1024 * 1024;

enum InputAction {
    Create,
    Rename(String),
    Copy(String),
    NewMod,
    RenameMod(String),
    RenameModFolder(String),
    SetTags(String),
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

/// State of the raw `mod.toml` editor dialog.
struct ManifestEditor {
    folder: String,
    content: String,
    error: Option<String>,
}

/// One row of the Data tab (owned so `self` can be used while rendering).
struct UserDataRow {
    rel: String,
    name: String,
    size: u64,
}

#[allow(clippy::enum_variant_names)]
enum ConfirmAction {
    DeleteProfile(String),
    DeleteMod(String),
    DeleteGroup(String),
    DeleteUserData(String),
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
    /// Optional text to send to the CLI's stdin (e.g. a manifest to save).
    stdin: Option<String>,
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
    manifest_editor: Option<ManifestEditor>,
    /// Profile user data (saves/tracks/screenshots), refreshed on `refresh`.
    userdata: Vec<gta_mo_core::userdata::Entry>,
    /// Whether an enabled mod provides PortableGTA.
    portablegta: bool,
    userdata_error: Option<String>,
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
    list_epoch: u64,
    last_folders: Vec<String>,
    detail_opened: Option<std::time::Instant>,
    last_enabled: usize,
    enabled_pulse: Option<std::time::Instant>,
    last_screen_size: egui::Vec2,
    /// Pid of the running `gta-mo launch` process group, for the Stop button.
    child_pid: Arc<Mutex<Option<u32>>>,
    /// True while the user asked to stop the game (so a non-zero exit is not an
    /// error).
    stopping: bool,
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
            manifest_editor: None,
            userdata: Vec::new(),
            portablegta: false,
            userdata_error: None,
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
            list_epoch: 0,
            last_folders: Vec::new(),
            detail_opened: None,
            last_enabled: 0,
            enabled_pulse: None,
            last_screen_size: egui::Vec2::ZERO,
            child_pid: Arc::new(Mutex::new(None)),
            stopping: false,
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
                let folders: Vec<String> = self
                    .snapshot
                    .mods
                    .iter()
                    .map(|m| m.folder.clone())
                    .collect();
                if folders != self.last_folders {
                    self.last_folders = folders;
                    self.list_epoch += 1;
                }
                self.reload_relations();
                self.reload_groups();
                self.reload_userdata();
                self.start_conflict_scan();
            }
            Err(e) => self.status = Some(e),
        }
    }

    /// Rescans the active profile's user data (saves/tracks/screenshots) and
    /// checks whether PortableGTA is enabled.
    fn reload_userdata(&mut self) {
        let cfg = match gta_mo_core::config::load_config() {
            Ok(c) => c,
            Err(e) => {
                self.userdata.clear();
                self.portablegta = false;
                self.userdata_error = Some(format!("Error de config: {e}"));
                return;
            }
        };
        let Some(subdir) = cfg.game_spec().user_data_dir else {
            self.userdata.clear();
            self.portablegta = false;
            self.userdata_error = Some("El juego no define datos de usuario.".into());
            return;
        };
        let upper = gta_mo_core::config::RuntimePaths::from_config(&cfg)
            .profile_paths(&self.snapshot.active_slug)
            .upper;
        self.userdata = gta_mo_core::userdata::scan(&upper, subdir);
        self.portablegta = self
            .backend
            .mods_dir_path()
            .map(|md| gta_mo_core::userdata::portablegta_installed(&md, &self.snapshot.resolved))
            .unwrap_or(false);
        self.userdata_error = None;
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
                let spec = self.backend.game_spec();
                crate::backend::Backend::scan_conflicts_async(gen, mdir, resolved, spec, tx);
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
        self.pending.push_back(Job {
            args,
            launch,
            stdin: None,
        });
        self.pump();
    }

    /// Queues a command whose stdin is the given text (e.g. `ctl manifest set`).
    fn exec_stdin(&mut self, args: Vec<String>, stdin: String) {
        self.pending.push_back(Job {
            args,
            launch: false,
            stdin: Some(stdin),
        });
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
            self.backend
                .run_cli_async(job.args, job.launch, job.stdin, self.child_pid.clone(), tx);
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
                    let was_stopping = self.stopping;
                    self.busy = false;
                    self.playing = false;
                    self.stopping = false;
                    if let Ok(mut slot) = self.child_pid.lock() {
                        *slot = None;
                    }
                    if was_stopping {
                        self.toasts
                            .push(ctx, crate::toasts::ToastKind::Info, "Juego detenido");
                    } else if !ok {
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
        // Portadas/capturas provienen de mod.toml no confiable: acota dimensiones
        // y memoria para que una imagen maliciosa (bomba de descompresión) no
        // agote la RAM del proceso.
        let decoded = image::ImageReader::open(path).ok().and_then(|mut reader| {
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(MAX_IMAGE_DIM);
            limits.max_image_height = Some(MAX_IMAGE_DIM);
            limits.max_alloc = Some(MAX_IMAGE_ALLOC);
            reader.limits(limits);
            reader.decode().ok()
        });
        let Some(img) = decoded else {
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
            palette.mod_row_active_fill
        } else {
            egui::Color32::TRANSPARENT
        };
        let frame = egui::Frame::new()
            .fill(fill)
            .corner_radius(egui::CornerRadius::same(5))
            .inner_margin(egui::Margin::symmetric(6, 2));
        let row = frame.show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let row_w = ui.available_width();
            // Responsive row: drop the cover/author and the button caption when
            // there is little horizontal room, and truncate/wrap the text.
            let compact = row_w < 440.0;
            let show_cover = self.settings.show_covers && row_w >= 360.0;
            let details_label = if row_w >= 300.0 {
                format!("{} Detalles", crate::icons::PANEL_RIGHT)
            } else {
                crate::icons::PANEL_RIGHT.to_string()
            };
            let details_w = button_width(ui, &details_label);

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
                if show_cover {
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

                // Text column, constrained so it never pushes the button out.
                let spacing = ui.spacing().item_spacing.x;
                let text_w = (ui.available_width() - details_w - spacing).max(48.0);
                ui.scope(|ui| {
                    ui.set_max_width(text_w);
                    ui.vertical(|ui| {
                        let name_color = if m.enabled {
                            palette.text
                        } else {
                            palette.text_muted
                        };
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(&m.name).strong().color(name_color),
                            )
                            .truncate(),
                        )
                        .on_hover_text(&m.name);

                        let mut meta: Vec<String> = Vec::new();
                        if let Some(v) = &m.meta.version {
                            meta.push(format!("v{v}"));
                        }
                        if !compact && !m.meta.author.is_empty() {
                            meta.push(m.meta.author.join(", "));
                        }
                        if !meta.is_empty() {
                            let line = meta.join(" · ");
                            ui.add(egui::Label::new(egui::RichText::new(&line).weak()).truncate())
                                .on_hover_text(&line);
                        }

                        if !m.meta.tags.is_empty() || !m.groups.is_empty() {
                            ui.horizontal_wrapped(|ui| {
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
                                let mut line = format!(
                                    "{} {} req · {} opt",
                                    crate::icons::SWAP_H,
                                    dep.required,
                                    dep.optional
                                );
                                if problems {
                                    line.push_str(" · requeridas sin resolver");
                                }
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(line).small().color(color),
                                    )
                                    .truncate(),
                                );
                            }
                        }
                    });
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(egui::RichText::new(&details_label)).clicked() {
                        self.selected_mod = Some(m.id);
                    }
                });
            });
        });
        row.response
    }

    /// Indicador de estado propio: cuadrado redondeado de color acento cuando
    /// el mod está activo, gris cuando no. Devuelve `true` cuando se hace clic.
    fn toggle_indicator(ui: &mut egui::Ui, id: i64, active: bool, interactive: bool) -> bool {
        let palette = theme::active(ui.ctx());
        let t = crate::motion::spring(
            ui.ctx(),
            egui::Id::new(("toggle", id)),
            if active { 1.0 } else { 0.0 },
        );
        let (rect, response) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
        let off = ui.visuals().widgets.inactive.weak_bg_fill;
        let fill = lerp_color(off, palette.accent, t);
        let stroke_color = lerp_color(palette.border, palette.on_accent, t);
        let inner = egui::Rect::from_center_size(rect.center(), egui::vec2(17.0, 17.0));
        ui.painter().rect(
            inner,
            egui::CornerRadius::same(5),
            fill,
            egui::Stroke::new(1.5_f32, stroke_color),
            egui::StrokeKind::Inside,
        );
        if t > 0.01 {
            ui.painter().text(
                inner.center(),
                egui::Align2::CENTER_CENTER,
                crate::icons::CHECK,
                egui::FontId::proportional(13.5),
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
        self.handle_lightbox_input(ctx);

        // Force a reflow after a window resize so every layout (including the
        // image viewer) is recomputed with the new size instead of reusing a
        // stale one.
        let screen_size = ctx.screen_rect().size();
        if screen_size != self.last_screen_size {
            self.last_screen_size = screen_size;
            ctx.request_repaint();
        }
        let narrow = ctx.screen_rect().width() < NARROW_BREAKPOINT;

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let spacing = ui.spacing().item_spacing.x.max(4.0);
                let avail = ui.available_width();
                let active = self.snapshot.active_slug.clone();
                let profiles = self.snapshot.profiles.clone();

                // Measure the controls so the header degrades gradually: the
                // essentials stay (profile, play/stop, menu) and the rest drop
                // into the ⋮ menu as the window narrows.
                let w_play = button_width(ui, &format!("{} Jugar", crate::icons::PLAY));
                let w_stop = button_width(ui, &format!("{} Detener", crate::icons::SQUARE));
                let w_play_eff = if self.playing { w_stop } else { w_play };
                let w_menu = 34.0;
                let w_clean = button_width(ui, &format!("{} Limpiar", crate::icons::ERASER));
                let w_disc = button_width(ui, &format!("{} Descubrir", crate::icons::SEARCH));
                let w_combo = text_width(ui, &active, egui::TextStyle::Button) + 40.0;
                let w_debug = checkbox_width(ui, "Debug");
                let w_prev = checkbox_width(ui, "Previsualizar");
                let w_plabel = text_width(ui, "Perfil:", egui::TextStyle::Body);
                let w_title = text_width(ui, "GTA SA Mod Organizer", egui::TextStyle::Heading);

                let show_menu = !narrow;
                let mut used = w_play_eff;
                if show_menu {
                    used += spacing + w_menu;
                }
                let show_clean = !narrow && used + spacing + w_clean <= avail;
                if show_clean {
                    used += spacing + w_clean;
                }
                let show_disc = !narrow && used + spacing + w_disc <= avail;
                if show_disc {
                    used += spacing + w_disc;
                }

                let mut left_budget = (avail - used - spacing - w_combo).max(0.0);
                let show_opts = !narrow && left_budget >= w_debug + spacing + w_prev + spacing;
                if show_opts {
                    left_budget -= w_debug + spacing + w_prev + spacing;
                }
                let show_label = !narrow && left_budget >= w_plabel + spacing;
                if show_label {
                    left_budget -= w_plabel + spacing;
                }
                let show_title = !narrow && left_budget >= w_title + spacing;

                let overflow = HeaderOverflow {
                    launch_opts: !show_opts,
                    clean: !show_clean,
                    discover: !show_disc,
                };

                if show_title {
                    ui.heading("GTA SA Mod Organizer");
                    ui.separator();
                }
                if show_label {
                    ui.label("Perfil:");
                }
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
                if show_opts {
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
                    if show_menu {
                        ui.menu_button(crate::icons::ELLIPSIS_V, |ui| {
                            ui.set_min_width(200.0);
                            app_menu_items(self, ui, &mut new_theme, overflow);
                        })
                        .response
                        .on_hover_text("Menú");
                    }

                    if self.playing {
                        let stop = egui::Button::new(
                            egui::RichText::new(format!("{} Detener", crate::icons::SQUARE))
                                .color(egui::Color32::WHITE)
                                .strong(),
                        )
                        .fill(palette.danger);
                        if ui.add(stop).on_hover_text("Detener el juego").clicked() {
                            let pid = self.child_pid.lock().ok().and_then(|s| *s);
                            if let Some(pid) = pid {
                                crate::backend::stop_child_group(pid);
                                self.stopping = true;
                                self.toasts.push(
                                    ctx,
                                    crate::toasts::ToastKind::Info,
                                    "Deteniendo el juego…",
                                );
                            }
                        }
                    } else {
                        let play = egui::Button::new(
                            egui::RichText::new(format!("{} Jugar", crate::icons::PLAY))
                                .color(palette.on_accent)
                                .strong(),
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
                            let mut args: Vec<String> =
                                vec!["launch".into(), "--deps-enable".into()];
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
                    }
                    if show_clean
                        && ui
                            .add_enabled(
                                idle,
                                egui::Button::new(format!("{} Limpiar", crate::icons::ERASER)),
                            )
                            .on_hover_text("Eliminar mods huérfanos (carpetas desaparecidas)")
                            .clicked()
                    {
                        self.exec(vec!["ctl".into(), "clean".into()], false);
                    }
                    if show_disc
                        && ui
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
                Tab::Data => self.ui_data(ui),
                Tab::Dependencies => self.ui_dependencies(ui),
                Tab::Conflicts => self.ui_conflicts(ui),
                Tab::Log => self.ui_log(ui),
            });

        // Mod detail: always a full-screen overlay page with minimal margins and
        // a pinned header/back button.
        if self.selected_mod.is_some() {
            let opened = *self
                .detail_opened
                .get_or_insert_with(std::time::Instant::now);
            let reveal =
                crate::motion::ease_out((opened.elapsed().as_secs_f32() / 0.22).clamp(0.0, 1.0));
            if reveal < 1.0 {
                ctx.request_repaint();
            }
            let screen = ctx.screen_rect();
            let palette = theme::active(ctx);
            let frame = egui::Frame::new()
                .fill(palette.surface)
                .stroke(egui::Stroke::new(1.0_f32, palette.border))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(egui::Margin::same(12));
            // Full width when narrow, a centered card (max 900) otherwise; the
            // size is derived from the current window (minus the frame margins)
            // so it can never overflow the resized window.
            let card_w = (screen.width() - 40.0)
                .min(if narrow { f32::INFINITY } else { 900.0 })
                .max(220.0);
            let card_h = (screen.height() - 40.0).max(220.0);
            let resp = egui::Modal::new(egui::Id::new("detail_overlay"))
                .frame(frame)
                .show(ctx, |ui| {
                    ui.set_min_size(egui::vec2(card_w, card_h));
                    ui.set_opacity(0.2 + 0.8 * reveal);
                    ui.add_space((1.0 - reveal) * 8.0);
                    self.ui_detail(ui);
                });
            if resp.should_close() && self.lightbox.is_none() {
                self.selected_mod = None;
            }
        } else {
            self.detail_opened = None;
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
                    if enabled != self.last_enabled {
                        self.last_enabled = enabled;
                        self.enabled_pulse = Some(std::time::Instant::now());
                    }
                    let scale = match self.enabled_pulse {
                        Some(t) => {
                            let p = crate::motion::ease_out(
                                (t.elapsed().as_secs_f32() / 0.25).clamp(0.0, 1.0),
                            );
                            if p < 1.0 {
                                ui.ctx().request_repaint();
                            }
                            1.0 + 0.18 * (1.0 - p)
                        }
                        None => 1.0,
                    };
                    ui.label(format!("{} mods ·", self.snapshot.mods.len()));
                    ui.label(
                        egui::RichText::new(format!("{enabled} activos"))
                            .size(13.0 * scale)
                            .color(palette.accent),
                    );
                    ui.label(format!("· Perfil: {}", self.snapshot.active_slug));
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

        // Gradual bar: the three primary destinations always stay; Grupos,
        // Dependencias and Conflictos appear when their measured width fits, and
        // labels are dropped at very small widths. Everything else lives in
        // "más".
        let all: [(Tab, &str, &str, Option<usize>); 7] = [
            (Tab::Mods, crate::icons::LIST, "Mods", None),
            (Tab::Profiles, crate::icons::USERS, "Perfiles", None),
            (Tab::Log, crate::icons::INFO, "Log", None),
            (Tab::Data, crate::icons::SAVE, "Datos", None),
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
        let total_w = ui.available_width();
        let spacing = ui.spacing().item_spacing.x.max(2.0);
        let more_w = 46.0;
        let icon_w = 48.0;
        let labeled_w: Vec<f32> = (0..all.len())
            .map(|i| bottom_nav_text_width(ui, all[i].1, all[i].2, all[i].3))
            .collect();
        let fixed_labels = labeled_w[0..3].iter().sum::<f32>() + more_w + 4.0 * spacing;
        let labeled = fixed_labels <= total_w;
        let width_of = |i: usize| if labeled { labeled_w[i] } else { icon_w };

        let mut show = [false; 6];
        let mut used = more_w + spacing;
        for (i, s) in show.iter_mut().enumerate().take(3) {
            *s = true;
            used += width_of(i) + spacing;
        }
        for (i, s) in show.iter_mut().enumerate().skip(3) {
            let w = width_of(i);
            if used + w + spacing <= total_w {
                *s = true;
                used += w + spacing;
            }
        }

        let mut new_theme: Option<ThemePref> = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = spacing;
            for i in 0..all.len() {
                if !show[i] {
                    continue;
                }
                let (tab, icon, label, badge) = all[i];
                let text_label = if labeled { label } else { "" };
                if bottom_nav_item(ui, self.tab == tab, icon, text_label, badge, width_of(i)) {
                    self.tab = tab;
                }
            }
            ui.allocate_ui_with_layout(
                egui::vec2(more_w, 52.0),
                egui::Layout::centered_and_justified(egui::Direction::TopDown),
                |ui| {
                    ui.menu_button(crate::icons::ELLIPSIS_V, |ui| {
                        ui.set_min_width(200.0);
                        for (i, (tab, icon, label, badge)) in all.iter().enumerate() {
                            if show[i] {
                                continue;
                            }
                            let text = match badge {
                                Some(n) => format!("{icon}  {label} ({n})"),
                                None => format!("{icon}  {label}"),
                            };
                            if ui.button(text).clicked() {
                                self.tab = *tab;
                                ui.close();
                            }
                        }
                        ui.separator();
                        app_menu_items(
                            self,
                            ui,
                            &mut new_theme,
                            HeaderOverflow {
                                launch_opts: true,
                                clean: true,
                                discover: true,
                            },
                        );
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

        let items: [(Tab, &str, &str, Option<usize>); 7] = [
            (Tab::Mods, crate::icons::LIST, "Mods", None),
            (Tab::Profiles, crate::icons::USERS, "Perfiles", None),
            (Tab::Groups, crate::icons::FOLDER, "Grupos", None),
            (Tab::Data, crate::icons::SAVE, "Datos", None),
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
                let reduce_motion = crate::motion::reduce(ui.ctx());
                let now = ui.ctx().input(|i| i.time);
                let anim_start = crate::motion::epoch_start(ui.ctx(), self.list_epoch);
                if !reduce_motion && now - anim_start < 2.0 {
                    ui.ctx().request_repaint();
                }
                for (i, m) in filtered.iter().enumerate() {
                    let p = crate::motion::stagger(now, anim_start, i, reduce_motion);
                    let row = ui
                        .scope(|ui| {
                            ui.set_opacity(0.12 + 0.88 * p);
                            self.draw_mod_row(ui, m, show_handles)
                        })
                        .inner;
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
                            egui::Stroke::new(2.0_f32, theme::active(ui.ctx()).accent),
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

        // Pinned header: the back button stays visible even when the content is
        // taller than the window, and the title truncates on narrow windows.
        egui::Sides::new()
            .height(30.0)
            .shrink_left()
            .truncate()
            .show(
                ui,
                |ui| {
                    ui.add(egui::Label::new(egui::RichText::new(&m.name).heading()).truncate());
                },
                |ui| {
                    if ui
                        .button(format!("{} Atrás", crate::icons::CHEVRON_LEFT))
                        .clicked()
                    {
                        self.selected_mod = None;
                    }
                },
            );
        ui.separator();

        let rel = self.relations.clone();
        let mut go_to: Option<String> = None;
        let viewport_h = ui.available_height();
        egui::ScrollArea::vertical()
            .id_salt("detail_scroll")
            .auto_shrink(false)
            .max_height(viewport_h)
            .show(ui, |ui| {
                self.ui_detail_images(ui, &m);
                let max_col = (ui.available_width() - 90.0).max(120.0);
                egui::Grid::new("detail_meta")
                    .num_columns(2)
                    .max_col_width(max_col)
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
                            let resp = ui.add(
                                egui::Label::new(
                                    egui::RichText::new(u)
                                        .underline()
                                        .color(theme::active(ui.ctx()).accent),
                                )
                                .truncate()
                                .sense(egui::Sense::click()),
                            );
                            if resp.clicked() {
                                let folder = m.folder.clone();
                                self.exec(
                                    vec!["ctl".into(), "open".into(), folder, "--url".into()],
                                    false,
                                );
                            }
                            resp.on_hover_text(u);
                            ui.end_row();
                        }
                        ui.label("Tags:");
                        ui.horizontal(|ui| {
                            let text = if m.meta.tags.is_empty() {
                                "(sin tags)".to_string()
                            } else {
                                m.meta.tags.join(", ")
                            };
                            ui.add(egui::Label::new(text).truncate());
                            if ui
                                .add_enabled(
                                    !(self.busy || self.playing) && m.has_manifest,
                                    egui::Button::new(format!("{} Editar", crate::icons::PENCIL))
                                        .small(),
                                )
                                .on_hover_text(if m.has_manifest {
                                    "Editar las etiquetas en mod.toml"
                                } else {
                                    "El mod no tiene mod.toml (créalo primero)"
                                })
                                .clicked()
                            {
                                let mut st = InputState::new(
                                    "Editar tags",
                                    "Tags (separados por comas o espacios):",
                                    InputAction::SetTags(m.folder.clone()),
                                );
                                st.value = m.meta.tags.join(", ");
                                self.input = Some(st);
                            }
                        });
                        ui.end_row();
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

                ui.horizontal_wrapped(|ui| {
                    let folder = m.folder.clone();
                    if !m.has_manifest
                        && ui
                            .add_enabled(
                                !(self.busy || self.playing),
                                egui::Button::new(format!("{} Crear mod.toml", crate::icons::SAVE)),
                            )
                            .on_hover_text(
                                "Crear una plantilla mod.toml para este mod (sin manifiesto)",
                            )
                            .clicked()
                    {
                        self.exec(vec!["ctl".into(), "init".into(), folder.clone()], false);
                    }
                    if m.has_manifest
                        && ui
                            .add_enabled(
                                !(self.busy || self.playing),
                                egui::Button::new(format!(
                                    "{} Editar mod.toml",
                                    crate::icons::PENCIL
                                )),
                            )
                            .on_hover_text("Editar el manifiesto completo (raw)")
                            .clicked()
                    {
                        match self.backend.mods_dir_path() {
                            Some(dir) => {
                                let path = dir.join(&folder).join("mod.toml");
                                match std::fs::read_to_string(&path) {
                                    Ok(content) => {
                                        self.manifest_editor = Some(ManifestEditor {
                                            folder: folder.clone(),
                                            content,
                                            error: None,
                                        });
                                    }
                                    Err(e) => self.toasts.push(
                                        ui.ctx(),
                                        crate::toasts::ToastKind::Error,
                                        format!("No se pudo leer mod.toml: {e}"),
                                    ),
                                }
                            }
                            None => self.toasts.push(
                                ui.ctx(),
                                crate::toasts::ToastKind::Error,
                                "No se pudo resolver el directorio de mods",
                            ),
                        }
                    }
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

    /// Cover and screenshot gallery for the detail panel. Clicking either opens
    /// the lightbox.
    fn ui_detail_images(&mut self, ui: &mut egui::Ui, m: &crate::model::ModView) {
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
                    let items = self.gallery_items(m);
                    if !items.is_empty() {
                        self.lightbox =
                            Some(crate::lightbox::Lightbox::from_rect(items, 0, resp.rect));
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
                                        let items = self.gallery_items(m);
                                        let base = items.len().saturating_sub(shots.len());
                                        let idx = (base + i).min(items.len().saturating_sub(1));
                                        if !items.is_empty() {
                                            self.lightbox =
                                                Some(crate::lightbox::Lightbox::from_rect(
                                                    items, idx, resp.rect,
                                                ));
                                        }
                                    }
                                }
                            }
                        }
                    });
                });
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

    /// Profile user data: saves, settings, user tracks and screenshots written
    /// inside `userfiles/` (with PortableGTA).
    fn ui_data(&mut self, ui: &mut egui::Ui) {
        use gta_mo_core::userdata::Category;
        ui.add_space(6.0);
        let palette = theme::active(ui.ctx());
        let slug = self.snapshot.active_slug.clone();

        if !self.portablegta {
            ui.group(|ui| {
                ui.label(egui::RichText::new("Recomendado: PortableGTA").strong());
                ui.label(
                    "Con PortableGTA cada perfil guarda sus partidas, ajustes, User Tracks y \
                     capturas en su propia carpeta (userfiles/), aisladas por el overlay.",
                );
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Abrir MixMods (descarga)").clicked() {
                        self.exec(
                            vec![
                                "ctl".into(),
                                "open-url".into(),
                                "https://www.mixmods.com.br/2021/06/iii-vc-sa-portablegta-change-saves-folder-mudar-pasta-user-files/".into(),
                            ],
                            false,
                        );
                    }
                    if ui.button("Código fuente (MIT)").clicked() {
                        self.exec(
                            vec![
                                "ctl".into(),
                                "open-url".into(),
                                "https://github.com/GTAmodding/miscmods/blob/master/portablegta.cpp".into(),
                            ],
                            false,
                        );
                    }
                });
                ui.label(
                    egui::RichText::new(
                        "Instálalo como un mod (contenido dentro de mods/) y actívalo. PortableGTA \
                         es de terceros (MIT, GTA modding); el launcher solo lo recomienda.",
                    )
                    .small()
                    .weak(),
                );
            });
            ui.add_space(8.0);
        }

        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(format!("Datos del perfil '{slug}'")).strong());
            if ui
                .button(format!("{} Abrir carpeta", crate::icons::FOLDER_OPEN))
                .clicked()
            {
                self.exec(
                    vec![
                        "ctl".into(),
                        "data".into(),
                        "dir".into(),
                        "--profile".into(),
                        slug.clone(),
                    ],
                    false,
                );
            }
            if ui
                .button(format!("{} Actualizar", crate::icons::REFRESH))
                .clicked()
            {
                self.reload_userdata();
            }
        });
        ui.separator();

        if let Some(err) = self.userdata_error.clone() {
            ui.colored_label(palette.danger, err);
            return;
        }
        if self.userdata.is_empty() {
            let msg = if self.portablegta {
                "Aún no hay partidas, capturas ni tracks en este perfil."
            } else {
                "No hay datos. Instala y activa PortableGTA para que el juego guarde aquí."
            };
            ui.label(msg);
            return;
        }

        // Own the rows so `self.exec` can be called while rendering.
        let cats = [
            Category::Saves,
            Category::Settings,
            Category::UserTracks,
            Category::Screenshots,
            Category::Other,
        ];
        let grouped: Vec<(Category, Vec<UserDataRow>)> = cats
            .iter()
            .map(|c| {
                let items = self
                    .userdata
                    .iter()
                    .filter(|e| e.category == *c)
                    .map(|e| UserDataRow {
                        rel: e.rel.clone(),
                        name: e.name.clone(),
                        size: e.size,
                    })
                    .collect();
                (*c, items)
            })
            .collect();

        let mut pending_remove: Option<String> = None;
        egui::ScrollArea::vertical()
            .id_salt("data_scroll")
            .auto_shrink(false)
            .show(ui, |ui| {
                for (cat, items) in &grouped {
                    if items.is_empty() {
                        continue;
                    }
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(format!("{} ({})", cat.label(), items.len())).strong(),
                    );
                    for row in items {
                        ui.horizontal(|ui| {
                            ui.add(egui::Label::new(egui::RichText::new(&row.name)).truncate())
                                .on_hover_text(&row.rel);
                            ui.label(
                                egui::RichText::new(gta_mo_core::userdata::human_size(row.size))
                                    .small()
                                    .weak(),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .small_button(format!("{} Eliminar", crate::icons::TRASH))
                                        .clicked()
                                    {
                                        pending_remove = Some(row.rel.clone());
                                    }
                                    if ui
                                        .small_button(format!(
                                            "{} Abrir",
                                            crate::icons::FOLDER_OPEN
                                        ))
                                        .clicked()
                                    {
                                        self.exec(
                                            vec![
                                                "ctl".into(),
                                                "data".into(),
                                                "open".into(),
                                                row.rel.clone(),
                                                "--profile".into(),
                                                slug.clone(),
                                            ],
                                            false,
                                        );
                                    }
                                },
                            );
                        });
                    }
                }
            });
        if let Some(rel) = pending_remove {
            self.confirm = Some(ConfirmState {
                title: "Eliminar dato".into(),
                message: format!("¿Eliminar '{rel}' del perfil '{slug}'?"),
                action: ConfirmAction::DeleteUserData(rel),
            });
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
    /// Handles lightbox keyboard input *before* any other modal, so Escape
    /// closes the viewer first (not the detail overlay behind it).
    fn handle_lightbox_input(&mut self, ctx: &egui::Context) {
        if self.lightbox.is_none() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.lightbox = None;
            return;
        }
        let total = self.lightbox.as_ref().map(|l| l.len()).unwrap_or(0);
        if total > 1 {
            let mut go = 0i32;
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight)) {
                go += 1;
            }
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft)) {
                go -= 1;
            }
            if go != 0 {
                if let Some(lb) = self.lightbox.as_mut() {
                    lb.index = (lb.index as i32 + go).rem_euclid(total as i32) as usize;
                }
            }
        }
    }

    fn ui_lightbox(&mut self, ctx: &egui::Context) {
        let Some(mut lb) = self.lightbox.take() else {
            return;
        };
        let total = lb.len();
        if total == 0 {
            return;
        }
        let index = lb.index;
        let (key, path, label) = {
            let it = &lb.items[index];
            (it.key.clone(), it.path.clone(), it.label.clone())
        };
        let tex = self.load_image(ctx, key, &path);
        let palette = theme::active(ctx);
        let screen = ctx.screen_rect();
        let source = lb.source;
        let hero =
            crate::motion::ease_out((lb.started.elapsed().as_secs_f32() / 0.28).clamp(0.0, 1.0));
        if hero < 1.0 {
            ctx.request_repaint();
        }

        // "Compact" (small window / phone-like): the image takes the whole
        // window and the controls float over it. Otherwise a header row leaves
        // room for the title. Everything is derived from the *current* window
        // each frame, so resizing always re-centers correctly.
        let compact = screen.width() < 600.0 || screen.height() < 520.0;
        let margin = if compact { 6.0 } else { 16.0 };
        let header_h = if compact { 0.0 } else { 34.0 };
        let view = egui::Rect::from_min_max(screen.min + egui::vec2(0.0, header_h), screen.max);

        let mut close = false;
        let mut nav = 0i32;
        let mut pick: Option<usize> = None;
        // Manual full-screen overlay: unlike a `Modal`, its geometry is derived
        // from the current window every frame, so it always re-centers on resize
        // and can go edge-to-edge in compact mode.
        egui::Area::new(egui::Id::new("gta_mo_lightbox"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .constrain_to(screen)
            .show(ctx, |ui| {
                ui.set_min_size(screen.size());

                // Backdrop. Clicking outside the image/controls closes.
                let backdrop =
                    ui.interact(screen, egui::Id::new("lb_backdrop"), egui::Sense::click());
                ui.painter()
                    .rect_filled(screen, 0.0, egui::Color32::from_black_alpha(235));
                if backdrop.clicked() {
                    close = true;
                }

                // Image: fit "contain" inside the current view, centered.
                let area = view.shrink(margin);
                match &tex {
                    Some(tex) => {
                        let size = tex.size_vec2();
                        let target = contain_rect(area, size);
                        let rect = match source {
                            Some(s) if hero < 1.0 => egui::Rect::from_min_max(
                                crate::motion::lerp_pos(s.left_top(), target.left_top(), hero),
                                crate::motion::lerp_pos(
                                    s.right_bottom(),
                                    target.right_bottom(),
                                    hero,
                                ),
                            ),
                            _ => target,
                        };
                        ui.painter().image(
                            tex.id(),
                            rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                        // Clicks on the image itself do nothing (only the margins
                        // close), matching the previous modal behaviour.
                        let _ = ui.interact(rect, egui::Id::new("lb_image"), egui::Sense::click());
                    }
                    None => {
                        ui.painter().text(
                            area.center(),
                            egui::Align2::CENTER_CENTER,
                            "No se pudo cargar la imagen",
                            egui::FontId::proportional(15.0),
                            palette.danger,
                        );
                    }
                }

                // Side navigation zones (big tap targets, arrows overlaid). The
                // hover feedback is a small circle around the arrow instead of a
                // fill of the whole zone.
                if total > 1 {
                    let left_rect =
                        egui::Rect::from_min_max(view.min, egui::pos2(view.center().x, view.max.y));
                    let right_rect =
                        egui::Rect::from_min_max(egui::pos2(view.center().x, view.min.y), view.max);
                    for (rect, id, icon, delta) in [
                        (left_rect, "lb_prev", crate::icons::CHEVRON_LEFT, -1i32),
                        (right_rect, "lb_next", crate::icons::CHEVRON_RIGHT, 1i32),
                    ] {
                        let resp = ui.interact(rect, egui::Id::new(id), egui::Sense::click());
                        let cx = if delta < 0 {
                            rect.left() + 26.0
                        } else {
                            rect.right() - 26.0
                        };
                        let cy = rect.center().y;
                        if resp.hovered() {
                            ui.painter().circle_filled(
                                egui::pos2(cx, cy),
                                if compact { 24.0 } else { 20.0 },
                                egui::Color32::from_black_alpha(130),
                            );
                        }
                        ui.painter().text(
                            egui::pos2(cx, cy),
                            egui::Align2::CENTER_CENTER,
                            icon,
                            egui::FontId::proportional(if compact { 26.0 } else { 22.0 }),
                            egui::Color32::WHITE,
                        );
                        if resp.clicked() {
                            nav = delta;
                        }
                    }
                }

                // Title (top-left) and close button (top-right).
                let top = screen.top() + if compact { 4.0 } else { 8.0 };
                let title_rect = egui::Rect::from_min_size(
                    egui::pos2(screen.left() + margin, top),
                    egui::vec2((screen.width() - 2.0 * margin - 48.0).max(40.0), 26.0),
                );
                // Subtle pill so the title stays legible over a bright image.
                ui.painter().rect_filled(
                    title_rect.expand(4.0),
                    egui::CornerRadius::same(6),
                    egui::Color32::from_black_alpha(90),
                );
                ui.put(
                    title_rect,
                    egui::Label::new(
                        egui::RichText::new(&label)
                            .color(egui::Color32::WHITE)
                            .strong(),
                    )
                    .truncate(),
                );
                if ui
                    .put(
                        egui::Rect::from_min_size(
                            egui::pos2(screen.right() - margin - 30.0, top - 1.0),
                            egui::vec2(30.0, 28.0),
                        ),
                        egui::Button::new(crate::icons::X),
                    )
                    .on_hover_text("Cerrar (Esc)")
                    .clicked()
                {
                    close = true;
                }

                // Counter + dots, floating over the bottom.
                if total > 1 {
                    let dot_slot = 14.0;
                    let per_row = (((screen.width() - 24.0) / dot_slot).floor() as usize).max(1);
                    let dot_rows = total.div_ceil(per_row).min(3);
                    let footer_h = 18.0 + dot_rows as f32 * dot_slot;
                    let footer = egui::Rect::from_min_max(
                        egui::pos2(screen.left() + 8.0, screen.bottom() - footer_h - 6.0),
                        egui::pos2(screen.right() - 8.0, screen.bottom() - 6.0),
                    );
                    ui.painter().rect_filled(
                        footer,
                        egui::CornerRadius::same(10),
                        egui::Color32::from_black_alpha(120),
                    );
                    ui.painter().text(
                        egui::pos2(footer.center().x, footer.top() + 10.0),
                        egui::Align2::CENTER_CENTER,
                        format!("{} / {}", index + 1, total),
                        egui::FontId::proportional(12.0),
                        egui::Color32::WHITE,
                    );
                    let mut y = footer.top() + 22.0;
                    let mut x = footer.center().x - (per_row.min(total) as f32 * dot_slot) / 2.0
                        + dot_slot / 2.0;
                    for i in 0..total {
                        if i > 0 && i % per_row == 0 {
                            let remain = (total - i).min(per_row) as f32;
                            x = footer.center().x - (remain * dot_slot) / 2.0 + dot_slot / 2.0;
                            y += dot_slot;
                        }
                        if y > footer.bottom() {
                            break;
                        }
                        let center = egui::pos2(x, y);
                        let dot =
                            egui::Rect::from_center_size(center, egui::vec2(dot_slot, dot_slot));
                        let resp =
                            ui.interact(dot, egui::Id::new(("lb_dot", i)), egui::Sense::click());
                        let selected = i == index;
                        ui.painter().circle_filled(
                            center,
                            if selected { 5.0 } else { 3.0 },
                            if selected {
                                palette.accent
                            } else {
                                egui::Color32::from_white_alpha(150)
                            },
                        );
                        if resp.clicked() {
                            pick = Some(i);
                        }
                        x += dot_slot;
                    }
                }
            });
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
            if submit
                && (!input.value.trim().is_empty()
                    || matches!(input.action, InputAction::SetTags(_)))
            {
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

        if self.manifest_editor.is_some() {
            let mut ed = self.manifest_editor.take().expect("checked is_some");
            let mut open = true;
            let mut save = false;
            let mut reload = false;
            egui::Window::new(format!("mod.toml — {}", ed.folder))
                .open(&mut open)
                .collapsible(false)
                .resizable(true)
                .default_size([640.0, 480.0])
                .show(ctx, |ui| {
                    if let Some(err) = &ed.error {
                        ui.colored_label(theme::active(ui.ctx()).danger, err);
                        ui.add_space(4.0);
                    }
                    egui::ScrollArea::both().show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut ed.content)
                                .code_editor()
                                .desired_width(f32::INFINITY)
                                .desired_rows(18),
                        );
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Guardar").clicked() {
                            save = true;
                        }
                        if ui.button("Recargar").clicked() {
                            reload = true;
                        }
                    });
                });
            if reload {
                if let Some(dir) = self.backend.mods_dir_path() {
                    match std::fs::read_to_string(dir.join(&ed.folder).join("mod.toml")) {
                        Ok(c) => {
                            ed.content = c;
                            ed.error = None;
                        }
                        Err(e) => ed.error = Some(format!("No se pudo leer: {e}")),
                    }
                }
            }
            let mut saved = false;
            if save {
                // Validate locally for immediate feedback (the CLI re-validates).
                match toml::from_str::<gta_mo_core::meta::ModMeta>(&ed.content) {
                    Ok(_) => {
                        self.exec_stdin(
                            vec![
                                "ctl".into(),
                                "manifest".into(),
                                "set".into(),
                                ed.folder.clone(),
                            ],
                            ed.content.clone(),
                        );
                        saved = true;
                    }
                    Err(e) => ed.error = Some(format!("TOML inválido: {e}")),
                }
            }
            if !open || saved {
                self.manifest_editor = None;
            } else {
                self.manifest_editor = Some(ed);
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
            InputAction::SetTags(folder) => {
                let tags: Vec<String> = value
                    .split([',', ' ', '\t', '\n'])
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                let mut args = vec!["ctl".into(), "tag".into(), "set".into(), folder];
                args.extend(tags);
                self.exec(args, false);
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
            ConfirmAction::DeleteUserData(rel) => {
                let slug = self.snapshot.active_slug.clone();
                self.exec(
                    vec![
                        "ctl".into(),
                        "data".into(),
                        "remove".into(),
                        rel,
                        "--yes".into(),
                        "--profile".into(),
                        slug,
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
    crate::motion::set_reduce(ctx, settings.reduce_motion);
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
    let hover = crate::motion::spring(
        ui.ctx(),
        egui::Id::new(("nav_hover", label)),
        if resp.hovered() { 1.0 } else { 0.0 },
    );
    if !selected && hover > 0.01 {
        ui.painter()
            .rect_filled(rect, radius, palette.surface_raised.gamma_multiply(hover));
    }
    if selected {
        ui.painter()
            .rect_filled(rect, radius, palette.row_active_fill);
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

/// Largest rectangle with the `size` aspect ratio that fits centered inside
/// `bounds` (the "contain" fit used by the image viewer).
fn contain_rect(bounds: egui::Rect, size: egui::Vec2) -> egui::Rect {
    let scale = (bounds.width() / size.x.max(1.0)).min(bounds.height() / size.y.max(1.0));
    egui::Rect::from_center_size(bounds.center(), size * scale)
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

/// Approximate width of a text run in the given style.
fn text_width(ui: &egui::Ui, text: &str, style: egui::TextStyle) -> f32 {
    let font = style.resolve(ui.style());
    ui.fonts(|f| {
        f.layout_no_wrap(text.to_owned(), font, egui::Color32::WHITE)
            .size()
            .x
    })
}

/// Approximate width of a button with the given label.
fn button_width(ui: &egui::Ui, text: &str) -> f32 {
    text_width(ui, text, egui::TextStyle::Button) + ui.spacing().button_padding.x * 2.0 + 2.0
}

/// Approximate width of a checkbox with the given label.
fn checkbox_width(ui: &egui::Ui, text: &str) -> f32 {
    text_width(ui, text, egui::TextStyle::Button)
        + ui.spacing().icon_width
        + ui.spacing().icon_spacing
}

/// Approximate width a bottom-nav button needs for its icon + caption (+badge).
fn bottom_nav_text_width(ui: &egui::Ui, icon: &str, label: &str, badge: Option<usize>) -> f32 {
    let text = match badge {
        Some(n) => format!("{icon} {label} {n}"),
        None => format!("{icon} {label}"),
    };
    let font = egui::TextStyle::Button.resolve(ui.style());
    let text_w = ui.fonts(|f| f.layout_no_wrap(text, font, egui::Color32::WHITE).size().x);
    text_w + ui.spacing().button_padding.x * 2.0 + 12.0
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
    let hover = crate::motion::spring(
        ui.ctx(),
        egui::Id::new(("bottom_nav_hover", icon)),
        if resp.hovered() { 1.0 } else { 0.0 },
    );
    if !selected && hover > 0.01 {
        ui.painter().rect_filled(
            rect.shrink2(egui::vec2(4.0, 4.0)),
            egui::CornerRadius::same(8),
            palette.surface_raised.gamma_multiply(hover),
        );
    }
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
    let icon_y = if label.is_empty() {
        rect.center().y
    } else {
        rect.top() + 18.0
    };
    ui.painter().text(
        egui::pos2(rect.center().x, icon_y),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(18.0),
        color,
    );
    if !label.is_empty() {
        ui.painter().text(
            egui::pos2(rect.center().x, rect.bottom() - 12.0),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(10.0),
            color,
        );
    }
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
fn app_menu_items(
    app: &mut GtaMoApp,
    ui: &mut egui::Ui,
    new_theme: &mut Option<ThemePref>,
    overflow: HeaderOverflow,
) {
    // Controls that did not fit in the header (empty in wide layouts).
    if overflow.discover
        && ui
            .button(format!("{} Descubrir", crate::icons::SEARCH))
            .clicked()
    {
        app.exec(vec!["ctl".into(), "discover".into()], false);
        ui.close();
    }
    if overflow.clean
        && ui
            .button(format!("{} Limpiar", crate::icons::ERASER))
            .clicked()
    {
        app.exec(vec!["ctl".into(), "clean".into()], false);
        ui.close();
    }
    if overflow.launch_opts {
        ui.checkbox(&mut app.launch_debug, "Debug");
        ui.checkbox(&mut app.launch_dry_run, "Previsualizar (dry-run)");
    }
    if overflow.discover || overflow.clean || overflow.launch_opts {
        ui.separator();
    }
    if ui
        .button(format!("{} Preferencias…", crate::icons::SETTINGS))
        .clicked()
    {
        app.show_preferences = true;
        ui.close();
    }
    if ui
        .button(format!("{} Acerca de", crate::icons::INFO))
        .clicked()
    {
        app.show_about = true;
        ui.close();
    }
    if ui
        .button(format!("{} Atajos de teclado", crate::icons::KEYBOARD))
        .clicked()
    {
        app.show_shortcuts = true;
        ui.close();
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
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::contain_rect;
    use eframe::egui;

    #[test]
    fn contain_rect_fits_and_centers() {
        let bounds = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(200.0, 100.0));

        // Wide image: limited by width, fills it exactly and stays centered.
        let r = contain_rect(bounds, egui::vec2(400.0, 100.0));
        assert!((r.width() - 200.0).abs() < 0.01);
        assert!((r.height() - 50.0).abs() < 0.01);
        assert!((r.center().x - bounds.center().x).abs() < 0.01);
        assert!((r.center().y - bounds.center().y).abs() < 0.01);

        // Tall image: limited by height.
        let r = contain_rect(bounds, egui::vec2(100.0, 400.0));
        assert!((r.height() - 100.0).abs() < 0.01);
        assert!((r.width() - 25.0).abs() < 0.01);

        // Never exceeds the bounds.
        for size in [
            egui::vec2(1.0, 1.0),
            egui::vec2(1000.0, 3.0),
            egui::vec2(3.0, 1000.0),
        ] {
            let r = contain_rect(bounds, size);
            assert!(r.width() <= bounds.width() + 0.01);
            assert!(r.height() <= bounds.height() + 0.01);
        }
    }
}
