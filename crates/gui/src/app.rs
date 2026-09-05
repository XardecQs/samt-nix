use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{channel, Receiver, Sender};

use eframe::egui;

use crate::backend::{Backend, GuiEvent};
use crate::model::{filter_and_sort, Filters, Snapshot, SortField};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Mods,
    Profiles,
    Log,
}

enum InputAction {
    Create,
    Rename(String),
    Copy(String),
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

enum ConfirmAction {
    DeleteProfile(String),
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
    pending: VecDeque<Job>,
    input: Option<InputState>,
    confirm: Option<ConfirmState>,
    covers: HashMap<String, egui::TextureHandle>,
    status: Option<String>,
}

impl GtaMoApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
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
            pending: VecDeque::new(),
            input: None,
            confirm: None,
            covers: HashMap::new(),
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
            }
            Err(e) => self.status = Some(e),
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

    fn move_mod(&mut self, id: i64, dir: i32) {
        let mut sorted = self.snapshot.mods.clone();
        sorted.sort_by_key(|m| std::cmp::Reverse(m.order));
        let Some(pos) = sorted.iter().position(|m| m.id == id) else {
            return;
        };
        let other = if dir < 0 {
            pos.checked_sub(1)
        } else {
            (pos + 1 < sorted.len()).then_some(pos + 1)
        };
        let Some(other) = other else { return };
        let a = sorted[pos].clone();
        let b = sorted[other].clone();
        let slug = self.snapshot.active_slug.clone();
        self.exec(
            vec![
                "ctl",
                "order",
                &a.id.to_string(),
                &b.order.to_string(),
                "--profile",
                &slug,
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            false,
        );
        self.exec(
            vec![
                "ctl",
                "order",
                &b.id.to_string(),
                &a.order.to_string(),
                "--profile",
                &slug,
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            false,
        );
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
        let path = self.backend.cover_path(folder, cover)?;
        let img = image::ImageReader::open(&path)
            .ok()?
            .decode()
            .ok()?
            .to_rgba8();
        let (w, h) = (img.width(), img.height());
        let color =
            egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
        let tex = ctx.load_texture(&key, color, egui::TextureOptions::LINEAR);
        if self.covers.len() >= 64 {
            self.covers.clear();
        }
        self.covers.insert(key, tex.clone());
        Some(tex)
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
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if self.playing { "Jugando…" } else { "Jugar" };
                    if ui
                        .add_enabled(!self.busy, egui::Button::new(label))
                        .clicked()
                    {
                        let slug = self.snapshot.active_slug.clone();
                        self.log.clear();
                        self.log.push(format!("--- Lanzando perfil '{slug}' ---"));
                        self.exec(
                            vec![
                                "launch".into(),
                                "--deps-enable".into(),
                                "--profile".into(),
                                slug,
                            ],
                            true,
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
            Tab::Log => self.ui_log(ui),
        });

        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if let Some(e) = &self.status {
                    ui.colored_label(egui::Color32::YELLOW, e);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("Conflictos: {}", self.snapshot.conflicts));
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
        });
        ui.separator();

        let mut filtered = self.snapshot.mods.clone();
        filter_and_sort(&mut filtered, &self.filters);

        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for m in &filtered {
                    ui.horizontal(|ui| {
                        let mut enabled = m.enabled;
                        if ui
                            .add_enabled(
                                !(self.busy || self.playing),
                                egui::Checkbox::new(&mut enabled, ""),
                            )
                            .on_hover_text("Activar/desactivar")
                            .changed()
                        {
                            self.set_enabled(m.id, enabled);
                        }
                        if ui
                            .add_enabled(
                                !(self.busy || self.playing),
                                egui::Button::new("▲").small(),
                            )
                            .on_hover_text("Subir prioridad")
                            .clicked()
                        {
                            self.move_mod(m.id, -1);
                        }
                        if ui
                            .add_enabled(
                                !(self.busy || self.playing),
                                egui::Button::new("▼").small(),
                            )
                            .on_hover_text("Bajar prioridad")
                            .clicked()
                        {
                            self.move_mod(m.id, 1);
                        }
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(&m.name).strong());
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
                    ui.separator();
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

                ui.horizontal(|ui| {
                    let folder = m.folder.clone();
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
            });
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
        }
    }
}
