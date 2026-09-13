use anyhow::Result;
use duckie_app::{ExecutionService, RunEvent};
use duckie_model::*;
use duckie_storage::{Collection, SecretsFile, StoredRequest};
use eframe::egui;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::Instant,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub const EXAMPLE: &str = "test(\"returns a successful JSON response\", () => {\n  expect(response.status).toBe(200);\n  expect(response.header(\"content-type\")).toContain(\"application/json\");\n  expect(response.json()).toBeType(\"object\");\n});\n";
#[derive(PartialEq, Clone, Copy)]
pub enum RequestTab {
    Params,
    Headers,
    Auth,
    Body,
    Tests,
    Settings,
}
#[derive(PartialEq, Clone, Copy)]
pub enum ResponseTab {
    Body,
    Headers,
    Tests,
    Details,
}
#[derive(Default)]
pub struct Draft {
    pub request: RequestDefinition,
    pub source: String,
    pub revision: u64,
    pub dirty: bool,
    pub error: String,
}
pub struct ResponseView {
    pub result: ExecutionResult,
    pub preview: String,
    pub pretty: Option<String>,
    pub binary: bool,
    pub offset: u64,
    pub report: Option<TestReport>,
    pub test_revision: u64,
    pub environment: Values,
    pub viewed: u64,
}
pub enum IoEvent {
    Opened(Result<Collection>),
    Saved(Result<Collection>),
    Imported(Result<duckie_openapi::ImportDraft>),
    Preview {
        id: String,
        run_id: String,
        offset: u64,
        result: Result<(String, Option<String>, bool)>,
    },
    Message(Result<String>),
    Secrets(Result<SecretsFile>),
}
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct Preferences {
    pub last_collection: Option<PathBuf>,
    pub sidebar: bool,
    pub appearance: u8,
}
#[derive(Clone, Copy)]
pub enum Pending {
    Open,
    NewCollection,
    Import,
    Close,
    Reload,
}
#[derive(Default)]
pub struct ImportUi {
    pub url_mode: bool,
    pub source: String,
    pub bearer: String,
    pub api_header: String,
    pub api_value: String,
    pub error: String,
    pub draft: Option<duckie_openapi::ImportDraft>,
    pub server: String,
    pub filter: String,
    pub selected: usize,
    pub cancel: Option<CancellationToken>,
}
pub struct Duckie {
    #[cfg(feature = "bench")]
    pub bench: Option<crate::bench::Bench>,
    #[cfg(feature = "screenshot")]
    pub capture_frames: usize,
    pub ctx: egui::Context,
    pub service: ExecutionService,
    pub events: mpsc::Receiver<RunEvent>,
    pub io_tx: mpsc::Sender<IoEvent>,
    pub io_rx: mpsc::Receiver<IoEvent>,
    pub collection: Option<Collection>,
    pub drafts: Vec<Draft>,
    pub selected: usize,
    pub responses: BTreeMap<String, ResponseView>,
    pub clock: u64,
    pub envs: Vec<Environment>,
    pub env_index: usize,
    pub secrets: BTreeMap<String, Values>,
    pub remember: BTreeSet<(String, String)>,
    pub env_dirty: bool,
    pub request_tab: RequestTab,
    pub response_tab: ResponseTab,
    pub search: String,
    pub response_find: crate::editor::Find,
    pub editor_find: crate::editor::Find,
    /// Last frame's focus, so Ctrl+F can route to whichever editor the caret is in.
    pub editor_focused: bool,
    /// A one-based line in the Tests editor to select and scroll to on the next frame.
    pub goto_line: Option<u32>,
    pub pretty: bool,
    pub prefs: Preferences,
    pub status: String,
    pub io_busy: bool,
    pub active: Option<(String, CancellationToken, Instant)>,
    pub active_environment: Values,
    pub active_testing: bool,
    pub env_dialog: bool,
    pub import: Option<ImportUi>,
    pub pending: Option<Pending>,
    pub after_save: Option<Pending>,
    pub allow_close: bool,
    pub delete: Option<usize>,
    pub about: bool,
    pub focus_url: bool,
    pub reveal: bool,
    pub new_env: String,
}
impl Duckie {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let prefs = cc
            .storage
            .and_then(|s| eframe::get_value(s, "duckie-preferences"))
            .unwrap_or(Preferences {
                sidebar: true,
                ..Default::default()
            });
        let ctx = cc.egui_ctx.clone();
        let wake = ctx.clone();
        let (service, events) = ExecutionService::new(move || wake.request_repaint())
            .expect("Cannot initialize request runtime");
        let (io_tx, io_rx) = mpsc::channel(16);
        let mut app = Self {
            #[cfg(feature = "bench")]
            bench: crate::bench::Bench::from_env(),
            #[cfg(feature = "screenshot")]
            capture_frames: 0,
            ctx,
            service,
            events,
            io_tx,
            io_rx,
            collection: None,
            drafts: vec![Draft::default()],
            selected: 0,
            responses: BTreeMap::new(),
            clock: 0,
            envs: vec![Environment::default()],
            env_index: 0,
            secrets: BTreeMap::new(),
            remember: BTreeSet::new(),
            env_dirty: false,
            request_tab: RequestTab::Params,
            response_tab: ResponseTab::Body,
            search: String::new(),
            response_find: Default::default(),
            editor_find: Default::default(),
            editor_focused: false,
            goto_line: None,
            pretty: true,
            prefs,
            status: "Ready · Local files. No account required.".into(),
            io_busy: false,
            active: None,
            active_environment: Values::new(),
            active_testing: false,
            env_dialog: false,
            import: None,
            pending: None,
            after_save: None,
            allow_close: false,
            delete: None,
            about: false,
            focus_url: true,
            reveal: false,
            new_env: String::new(),
        };
        #[cfg(feature = "screenshot")]
        if std::env::var_os("DUCKIE_CAPTURE_PATH").is_some() {
            app.prefs.last_collection = None;
            app.prefs.appearance =
                if std::env::var("DUCKIE_CAPTURE_THEME").as_deref() == Ok("light") {
                    1
                } else {
                    2
                };
            // DUCKIE_CAPTURE_VIEW=editor seeds a draft so a capture shows the code editor
            // chrome — the gutter and find highlighting — instead of the empty scratch screen.
            if std::env::var("DUCKIE_CAPTURE_VIEW").as_deref() == Ok("editor") {
                app.drafts[0].request.url = "https://api.example.test/ducks".into();
                app.drafts[0].request.body = Body::Json {
                    text: "{\n  \"name\": \"Duckie\",\n  \"colours\": [\n    \"yellow\",\n    \"orange\"\n  ],\n  \"count\": 3\n}"
                        .into(),
                };
                app.request_tab = RequestTab::Body;
                app.editor_find.query = "\"".into();
                app.editor_find.open = true;
            }
        }
        // A measurement run must start from a known state, never from whatever collection the
        // user last had open. `App::save` is also suppressed so a run cannot overwrite prefs.
        #[cfg(feature = "bench")]
        if app.bench.is_some() {
            app.prefs.last_collection = None;
        }
        app.apply_theme();
        app.install_fonts();
        // Sweeps response spool files orphaned by a past crash; never blocks the first frame.
        app.service
            .runtime()
            .spawn_blocking(|| _ = duckie_http::sweep_stale_spool());
        if let Some(path) = std::env::args_os()
            .nth(1)
            .map(PathBuf::from)
            .filter(|path| path.exists())
            .or_else(|| app.prefs.last_collection.clone())
        {
            app.open_path(path);
        }
        app
    }
    pub fn apply_theme(&self) {
        self.ctx.set_theme(match self.prefs.appearance {
            1 => egui::ThemePreference::Light,
            2 => egui::ThemePreference::Dark,
            _ => egui::ThemePreference::System,
        });
        self.ctx.style_mut_of(egui::Theme::Dark, |style| {
            style.visuals.panel_fill = egui::Color32::from_rgb(32, 35, 41);
            style.visuals.window_fill = egui::Color32::from_rgb(32, 35, 41);
            style.visuals.extreme_bg_color = egui::Color32::from_rgb(23, 25, 29);
            style.visuals.faint_bg_color = egui::Color32::from_rgb(41, 45, 53);
            style.visuals.override_text_color = Some(egui::Color32::from_rgb(242, 244, 248));
            style.visuals.selection.bg_fill = egui::Color32::from_rgb(49, 72, 104);
            style.visuals.selection.stroke =
                egui::Stroke::new(1.0, egui::Color32::from_rgb(130, 183, 255));
        });
        self.ctx.all_styles_mut(|style| {
            style.spacing.item_spacing = egui::vec2(8.0, 8.0);
            style.spacing.button_padding = egui::vec2(10.0, 6.0);
            style.spacing.interact_size.y = 30.0;
            style.animation_time = 0.0;
            style
                .text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
            style
                .text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
            style
                .text_styles
                .insert(egui::TextStyle::Monospace, egui::FontId::monospace(13.0));
        });
    }
    fn install_fonts(&self) {
        let mut fonts = egui::FontDefinitions::default();
        for (name, path, family) in [
            (
                "Segoe UI",
                "C:/Windows/Fonts/segoeui.ttf",
                egui::FontFamily::Proportional,
            ),
            (
                "Consolas",
                "C:/Windows/Fonts/consola.ttf",
                egui::FontFamily::Monospace,
            ),
        ] {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert(name.into(), egui::FontData::from_owned(bytes).into());
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .insert(0, name.into());
            }
        }
        self.ctx.set_fonts(fonts);
    }
    pub fn dirty(&self) -> bool {
        self.env_dirty || self.drafts.iter().any(|d| d.dirty)
    }
    pub fn background(&self, job: impl FnOnce() -> IoEvent + Send + 'static) {
        let tx = self.io_tx.clone();
        let ctx = self.ctx.clone();
        self.service.runtime().spawn_blocking(move || {
            let event = job();
            let _ = tx.blocking_send(event);
            ctx.request_repaint();
        });
    }
    pub fn snapshot(&self) -> EnvironmentSnapshot {
        let e = &self.envs[self.env_index];
        EnvironmentSnapshot {
            name: e.name.clone(),
            values: e.values.clone(),
            secrets: self.secrets.get(&e.name).cloned().unwrap_or_default(),
        }
    }
    pub fn new_request(&mut self) {
        self.drafts.push(Draft::default());
        self.selected = self.drafts.len() - 1;
        self.focus_url = true;
        self.request_tab = RequestTab::Params;
    }
    pub fn touch(&mut self) {
        let d = &mut self.drafts[self.selected];
        d.dirty = true;
        d.revision += 1;
        d.error.clear();
    }
    pub fn send(&mut self) {
        if self.active.is_some() || self.service.busy() {
            return;
        }
        let env = self.snapshot();
        let d = &self.drafts[self.selected];
        let result = prepare(&d.request, &env, &RunBindings::default(), d.revision).and_then(|p| {
            self.service.send(
                p,
                d.source.clone(),
                d.request.tests.enabled,
                env.values.clone(),
            )
        });
        match result {
            Ok(token) => {
                self.active = Some((d.request.id.clone(), token, Instant::now()));
                self.active_environment = env.values;
                self.active_testing = false;
                self.drafts[self.selected].error.clear();
                self.status = "Sending request…".into();
            }
            Err(e) => self.drafts[self.selected].error = e.to_string(),
        }
    }
    pub fn rerun(&mut self) {
        if self.active.is_some() || self.service.busy() {
            return;
        }
        let d = &self.drafts[self.selected];
        let Some(view) = self.responses.get(&d.request.id) else {
            return;
        };
        match self.service.rerun_tests(
            view.result.clone(),
            d.source.clone(),
            d.revision,
            view.environment.clone(),
        ) {
            Ok(token) => {
                self.active = Some((d.request.id.clone(), token, Instant::now()));
                self.active_testing = true;
                self.response_tab = ResponseTab::Tests;
            }
            Err(e) => self.status = e.to_string(),
        }
    }
    pub fn preview(&self, id: String, run_id: String, body: BodyHandle, offset: u64) {
        self.background(move || IoEvent::Preview {
            id,
            run_id,
            offset,
            result: (|| {
                let bytes = body.read(offset, MIB)?;
                let binary = bytes.contains(&0) || std::str::from_utf8(&bytes).is_err();
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let pretty = if offset == 0 && body.len() <= MIB {
                    serde_json::from_slice::<serde_json::Value>(&bytes)
                        .ok()
                        .and_then(|v| serde_json::to_string_pretty(&v).ok())
                } else {
                    None
                };
                Ok((text, pretty, binary))
            })(),
        });
    }
    pub fn poll(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                RunEvent::Response(result) => {
                    let id = result.request_id.clone();
                    self.preview(id.clone(), result.run_id.clone(), result.body.clone(), 0);
                    self.clock += 1;
                    if !self.responses.contains_key(&id) && self.responses.len() >= 3 {
                        let selected = &self.drafts[self.selected].request.id;
                        if let Some(old) = self
                            .responses
                            .iter()
                            .filter(|(k, _)| *k != selected)
                            .min_by_key(|(_, v)| v.viewed)
                            .map(|(k, _)| k.clone())
                        {
                            self.responses.remove(&old);
                        }
                    }
                    self.status = result.outcome.to_string();
                    self.active_testing = result.outcome == Outcome::Complete;
                    self.responses.insert(
                        id,
                        ResponseView {
                            test_revision: result.summary.revision,
                            result,
                            preview: "Preparing preview…".into(),
                            pretty: None,
                            binary: false,
                            offset: 0,
                            report: None,
                            environment: self.active_environment.clone(),
                            viewed: self.clock,
                        },
                    );
                }
                RunEvent::Tests {
                    request_id,
                    run_id,
                    revision,
                    report,
                } => {
                    if let Some(view) = self.responses.get_mut(&request_id)
                        && view.result.run_id == run_id
                    {
                        view.report = Some(report);
                        view.test_revision = revision;
                    }
                }
                RunEvent::Error {
                    request_id,
                    message,
                } => {
                    if let Some(d) = self.drafts.iter_mut().find(|d| d.request.id == request_id) {
                        d.error = message.clone();
                    }
                    self.status = message;
                }
                RunEvent::Finished => {
                    self.active = None;
                    self.active_testing = false;
                }
            }
        }
        while let Ok(event) = self.io_rx.try_recv() {
            match event {
                IoEvent::Opened(result) => {
                    self.io_busy = false;
                    match result {
                        Ok(c) => self.use_collection(c),
                        Err(e) => self.status = format!("Could not open collection: {e}"),
                    }
                }
                IoEvent::Saved(result) => {
                    self.io_busy = false;
                    match result {
                        Ok(c) => {
                            for (d, saved) in self.drafts.iter_mut().zip(&c.requests) {
                                d.request = saved.definition.clone();
                                d.dirty = false;
                            }
                            self.env_dirty = false;
                            self.prefs.last_collection = Some(c.root.clone());
                            self.status = format!("Saved to {}", c.root.display());
                            self.collection = Some(c);
                            if let Some(action) = self.after_save.take() {
                                self.perform(action);
                            }
                        }
                        Err(e) => {
                            self.status = format!("Save failed: {e}");
                            self.after_save = None;
                        }
                    }
                }
                IoEvent::Imported(result) => {
                    self.io_busy = false;
                    if let Some(import) = &mut self.import {
                        import.cancel = None;
                        match result {
                            Ok(draft) => {
                                import.server = draft.servers.first().cloned().unwrap_or_default();
                                import.draft = Some(draft);
                                import.error.clear();
                            }
                            Err(e) => import.error = e.to_string(),
                        }
                    }
                }
                IoEvent::Preview {
                    id,
                    run_id,
                    offset,
                    result,
                } => {
                    if let Some(view) = self.responses.get_mut(&id)
                        && view.result.run_id == run_id
                    {
                        match result {
                            Ok((text, pretty, binary)) => {
                                view.preview = text;
                                view.pretty = pretty;
                                view.binary = binary;
                                view.offset = offset;
                            }
                            Err(e) => view.preview = format!("Preview unavailable: {e}"),
                        }
                    }
                }
                IoEvent::Message(result) => {
                    self.status = result.unwrap_or_else(|e| e.to_string());
                }
                IoEvent::Secrets(result) => match result {
                    Ok(file) => {
                        for (env, values) in file.environments {
                            self.secrets.entry(env).or_default().extend(values);
                        }
                        self.status="Loaded secrets into this session. Remember individual values to save them.".into();
                    }
                    Err(e) => self.status = e.to_string(),
                },
            }
        }
    }
    pub fn use_collection(&mut self, c: Collection) {
        self.drafts = c
            .requests
            .iter()
            .map(|r| Draft {
                request: r.definition.clone(),
                source: r.source.clone(),
                revision: 0,
                dirty: false,
                error: String::new(),
            })
            .collect();
        if self.drafts.is_empty() {
            self.drafts.push(Draft::default());
        }
        self.selected = 0;
        self.envs = c.environments.clone();
        self.env_index = 0;
        self.secrets = c.secrets.environments.clone();
        self.remember.clear();
        for (env, values) in &self.secrets {
            for key in values.keys() {
                self.remember.insert((env.clone(), key.clone()));
            }
        }
        self.env_dirty = false;
        self.responses.clear();
        self.prefs.last_collection = Some(c.root.clone());
        self.status = format!("Opened {}", c.manifest.name);
        self.collection = Some(c);
        self.focus_url = true;
    }
    pub fn open_path(&mut self, path: PathBuf) {
        self.io_busy = true;
        self.background(move || {
            IoEvent::Opened(Collection::open(if path.is_file() {
                path.parent().unwrap_or(&path)
            } else {
                &path
            }))
        });
    }
    pub fn save_collection(&mut self, as_new: bool) {
        if self.io_busy {
            return;
        }
        let mut collection = match self.collection.as_ref().filter(|_| !as_new) {
            None => {
                let Some(path) = rfd::FileDialog::new()
                    .set_title("Choose an empty folder for your Duckie collection")
                    .pick_folder()
                else {
                    self.after_save = None;
                    return;
                };
                match Collection::new(
                    path.clone(),
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                ) {
                    Ok(c) => c,
                    Err(e) => {
                        self.status = e.to_string();
                        self.after_save = None;
                        return;
                    }
                }
            }
            Some(collection) => collection.clone(),
        };
        collection.requests = self
            .drafts
            .iter()
            .map(|d| StoredRequest {
                definition: d.request.clone(),
                source: d.source.clone(),
            })
            .collect();
        collection.environments = self.envs.clone();
        for (env, key) in &self.remember {
            if let Some(value) = self.secrets.get(env).and_then(|v| v.get(key)) {
                collection
                    .secrets
                    .environments
                    .entry(env.clone())
                    .or_default()
                    .insert(key.clone(), value.clone());
            }
        }
        self.io_busy = true;
        self.status = "Saving collection…".into();
        self.background(move || IoEvent::Saved(collection.save().map(|_| collection)));
    }
    pub fn request_action(&mut self, action: Pending) {
        if self.io_busy {
            return;
        }
        if self.active.is_some() {
            self.status = "Finish or stop the active run before changing collections.".into();
            return;
        }
        if self.dirty() {
            self.pending = Some(action);
        } else {
            self.perform(action);
        }
    }
    pub fn perform(&mut self, action: Pending) {
        match action {
            Pending::Open => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Open Duckie collection folder")
                    .pick_folder()
                {
                    self.open_path(path);
                }
            }
            Pending::Reload => {
                if let Some(c) = &self.collection {
                    self.open_path(c.root.clone());
                }
            }
            Pending::NewCollection => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Create collection in an empty folder")
                    .pick_folder()
                {
                    match Collection::new(
                        path.clone(),
                        path.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                    ) {
                        Ok(c) => {
                            self.use_collection(c);
                            self.drafts[0].dirty = true;
                        }
                        Err(e) => self.status = e.to_string(),
                    }
                }
            }
            Pending::Import => self.import = Some(ImportUi::default()),
            Pending::Close => {
                self.allow_close = true;
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}
