use anyhow::Result;
use duckie_app::{ExecutionService, RunEvent};
use duckie_model::*;
use duckie_storage::{Collection, SecretsFile, StoredRequest};
use eframe::egui;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    time::{Instant, SystemTime},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Whole-body matches retained. Far above what anyone navigates by hand, and enough that the
/// count stays honest for a realistic search.
pub const MAX_BODY_HITS: usize = 50_000;
pub const MAX_HISTORY_ENTRIES: usize = 100;
pub const HISTORY_BODY_BUDGET: u64 = 20 * MIB;
fn trim_history(history: &mut VecDeque<HistoryEntry>) {
    while history.len() > MAX_HISTORY_ENTRIES {
        history.pop_front();
    }
    let mut retained: u64 = history
        .iter()
        .filter(|entry| !entry.body_evicted)
        .map(|entry| entry.result.body.len())
        .sum();
    while retained > HISTORY_BODY_BUDGET {
        let Some(entry) = history
            .iter_mut()
            .find(|entry| !entry.body_evicted && !entry.result.body.is_empty())
        else {
            break;
        };
        retained = retained.saturating_sub(entry.result.body.len());
        entry.result.body = BodyHandle::default();
        entry.body_evicted = true;
    }
}
/// Bytes of a response body to show at once.
///
/// Laying text out costs roughly 200 bytes of glyph and mesh data per character regardless of
/// how it is wrapped, so a page of minified JSON — one line of a million characters — cost over
/// 200 MiB against a 32 MiB budget. Line structure does not change that; only the number of
/// characters handed to the widget does. A body whose lines are short enough to be read normally
/// keeps the full page; anything with very long lines pages in smaller steps instead.
pub fn page_size(bytes: &[u8]) -> u64 {
    const LONG_LINE: usize = 4 * 1024;
    let longest = bytes
        .split(|b| *b == b'\n')
        .map(<[u8]>::len)
        .max()
        .unwrap_or(0);
    if longest > LONG_LINE { 128 * 1024 } else { MIB }
}
fn declared_charset(headers: &[(String, String)]) -> Option<String> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .and_then(|(_, value)| {
            value.split(';').skip(1).find_map(|parameter| {
                let (name, value) = parameter.split_once('=')?;
                name.trim().eq_ignore_ascii_case("charset").then(|| {
                    value
                        .trim()
                        .trim_matches(|c| c == '\'' || c == '"')
                        .to_owned()
                })
            })
        })
        .filter(|label| !label.is_empty())
}
/// How far a byte slice's UTF-8 validity reaches, distinguishing a genuinely invalid byte from a
/// multi-byte character simply cut off where the slice ends — the latter is expected any time a
/// preview reads or pages through a document in fixed-size chunks, not a sign of binary content.
fn utf8_validity(bytes: &[u8]) -> (usize, bool) {
    match std::str::from_utf8(bytes) {
        Ok(_) => (bytes.len(), false),
        Err(e) => (e.valid_up_to(), e.error_len().is_some()),
    }
}

fn uses_utf8_boundaries(selected: Option<&str>) -> bool {
    selected.is_none_or(|label| {
        encoding_rs::Encoding::for_label(label.as_bytes())
            .is_some_and(|encoding| encoding == encoding_rs::UTF_8)
    })
}

fn trim_orphaned_utf8_tail(bytes: &mut Vec<u8>) {
    let skip = bytes
        .iter()
        .take(3)
        .take_while(|&&byte| byte & 0xC0 == 0x80)
        .count();
    bytes.drain(..skip);
}

fn complete_utf8_len(bytes: &[u8], maximum: usize) -> usize {
    let length = maximum.min(bytes.len());
    utf8_validity(&bytes[..length]).0
}
fn decode_preview(
    bytes: &[u8],
    declared: Option<&str>,
    force_text: bool,
) -> (String, bool, Option<String>) {
    let encoding = declared.and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()));
    if let Some(encoding) = encoding {
        let (text, _, errors) = encoding.decode(bytes);
        return (
            text.into_owned(),
            !force_text && errors,
            Some(encoding.name().to_owned()),
        );
    }
    let (_, invalid) = utf8_validity(bytes);
    (
        String::from_utf8_lossy(bytes).into_owned(),
        !force_text && (declared.is_some() || bytes.contains(&0) || invalid),
        declared.map(str::to_owned),
    )
}
pub const EXAMPLE: &str = "test(\"returns a successful JSON response\", () => {\n  expect(response.status).toBe(200);\n  expect(response.header(\"content-type\")).toContain(\"application/json\");\n  expect(response.json()).toBeType(\"object\");\n});\n";
fn json_with_depth(bytes: &[u8], limit: usize) -> Result<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    let mut stack = vec![(&value, 1usize)];
    while let Some((value, depth)) = stack.pop() {
        if depth > limit {
            anyhow::bail!("JSON exceeds tree depth limit");
        }
        match value {
            serde_json::Value::Array(values) => stack.extend(values.iter().map(|v| (v, depth + 1))),
            serde_json::Value::Object(values) => {
                stack.extend(values.values().map(|v| (v, depth + 1)))
            }
            _ => {}
        }
    }
    Ok(value)
}
/// Plain text on the system clipboard, if any. Windows-only, like the rest of this desktop
/// shell: reading `CF_UNICODETEXT` through `OpenClipboard`/`GetClipboardData` is the ordinary
/// Win32 way to do this and needs no clipboard crate this app has no other use for. `None` for
/// an empty or non-text clipboard, or if it could not be opened (another application can hold it
/// briefly), never a panic.
#[cfg(windows)]
pub fn clipboard_text() -> Option<String> {
    use windows_sys::Win32::System::{
        DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard},
        Memory::{GlobalLock, GlobalUnlock},
    };
    const CF_UNICODETEXT: u32 = 13;
    unsafe {
        if OpenClipboard(std::ptr::null_mut()) == 0 {
            return None;
        }
        let handle = GetClipboardData(CF_UNICODETEXT);
        let text = if handle.is_null() {
            None
        } else {
            let ptr = GlobalLock(handle) as *const u16;
            if ptr.is_null() {
                None
            } else {
                let mut len = 0usize;
                while *ptr.add(len) != 0 {
                    len += 1;
                }
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
                GlobalUnlock(handle);
                Some(text)
            }
        };
        CloseClipboard();
        text
    }
}
#[cfg(not(windows))]
pub fn clipboard_text() -> Option<String> {
    None
}
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
    Json,
    Compare,
    Headers,
    Tests,
    Details,
}
#[derive(Default)]
pub struct Draft {
    pub request: RequestDefinition,
    /// Editable request-variable rows. Unlike the serialized `BTreeMap`, this keeps a newly
    /// appended blank row in place while its name is being entered.
    pub variable_rows: Vec<Row>,
    pub source: String,
    pub revision: u64,
    pub dirty: bool,
    pub error: String,
    /// True only right after `use_collection`, for a request whose body/test content
    /// `Collection::open` deferred reading. Cleared by `Duckie::ensure_loaded`, which every
    /// frame calls for the selected draft and which `save_collection` calls for every draft
    /// before it writes anything, so a request is never saved with placeholder content.
    pub pending: bool,
}
/// Whole-body search results. Offsets are absolute byte positions in the response body, so they
/// stay meaningful across page changes, unlike the per-page hits the editor highlights.
#[derive(Default)]
pub struct BodySearch {
    pub query: String,
    pub offsets: Vec<u64>,
    pub running: bool,
    pub capped: bool,
}
pub struct ResponseView {
    pub result: ExecutionResult,
    pub preview: String,
    pub pretty: Option<String>,
    /// Parsed only for complete JSON bodies within the tree's fixed budget.
    pub json: Option<serde_json::Value>,
    pub binary: bool,
    /// Encoding used for the displayed text, when it came from a declared supported charset.
    pub charset: Option<String>,
    /// An explicit display encoding chosen by the user; `None` follows Content-Type/UTF-8 detection.
    pub charset_override: Option<String>,
    pub offset: u64,
    /// Bytes shown per page. Shrinks for very long lines; see `page_size`.
    pub page: u64,
    pub search: BodySearch,
    /// Absolute offset of a match to select once the page holding it has loaded.
    pub reveal_at: Option<u64>,
    pub report: Option<TestReport>,
    pub test_revision: u64,
    pub environment: Values,
    pub viewed: u64,
}
pub struct HistoryEntry {
    pub captured_at: SystemTime,
    pub request: RequestDefinition,
    pub source: String,
    pub result: ExecutionResult,
    pub report: Option<TestReport>,
    pub test_revision: u64,
    pub body_evicted: bool,
}
pub struct PreviewContent {
    pub text: String,
    pub pretty: Option<String>,
    pub binary: bool,
    pub page: u64,
    pub charset: Option<String>,
}
pub enum IoEvent {
    Opened(Result<Collection>),
    Saved(Result<Collection>),
    Imported(Result<duckie_openapi::ImportDraft>),
    Preview {
        id: String,
        run_id: String,
        offset: u64,
        result: Result<PreviewContent>,
    },
    Searched {
        id: String,
        run_id: String,
        query: String,
        result: Result<(Vec<u64>, bool)>,
    },
    Message(Result<String>),
    Secrets(Result<SecretsFile>),
    DiskChanged(Vec<(String, duckie_storage::Change)>),
    SuiteFinished(Vec<duckie_app::HeadlessReport>),
}
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct Preferences {
    pub last_collection: Option<PathBuf>,
    pub sidebar: bool,
    pub appearance: u8,
    /// Folder names the user has collapsed in the sidebar.
    #[serde(default)]
    pub collapsed: BTreeSet<String>,
    /// Request id and environment name to restore when the same collection reopens. Both are
    /// matched by name rather than index, so they degrade to the first entry for another
    /// collection instead of selecting something arbitrary.
    #[serde(default)]
    pub selected: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
    /// Split positions. egui's own memory is deliberately not persisted, because text editor
    /// undo stacks can hold credentials, so these are carried here instead.
    #[serde(default)]
    pub sidebar_width: Option<f32>,
    #[serde(default)]
    pub request_height: Option<f32>,
}
#[derive(Clone, Copy)]
pub enum Pending {
    Open,
    NewCollection,
    Import,
    UpdateFromSpec,
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
    /// Reviewing changes against the open collection instead of importing into a new one.
    pub update: bool,
    pub plan: Option<duckie_openapi::ReimportPlan>,
    /// Parallel to `plan.matches`/`additions`/`removals`: which of each the review applies.
    pub apply_updates: Vec<bool>,
    pub apply_additions: Vec<bool>,
    pub apply_removals: Vec<bool>,
}
#[derive(Default)]
pub struct SuiteUi {
    pub open: bool,
    pub scope: u8,
    pub stop_on_failure: bool,
    pub running: bool,
    pub cancel: Option<CancellationToken>,
    pub results: Vec<duckie_app::HeadlessReport>,
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
    pub history: VecDeque<HistoryEntry>,
    pub history_open: bool,
    pub history_selected: Option<usize>,
    /// Exact editable input captured at Send, waiting for the response event that owns it.
    pub pending_history_input: Option<(RequestDefinition, String)>,
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
    pub json_path: String,
    pub json_selected: String,
    pub json_expanded: BTreeSet<String>,
    pub editor_find: crate::editor::Find,
    /// Last frame's focus, so Ctrl+F can route to whichever editor the caret is in.
    pub editor_focused: bool,
    /// A one-based line in the Tests editor to select and scroll to on the next frame.
    pub goto_line: Option<u32>,
    pub pretty: bool,
    pub prefs: Preferences,
    pub status: String,
    pub io_busy: bool,
    /// The request in flight: its id, how to stop it, when it started, and the byte counters the
    /// transport updates as the body arrives.
    pub active: Option<(String, CancellationToken, Instant, duckie_http::Progress)>,
    pub active_environment: Values,
    pub active_testing: bool,
    pub env_dialog: bool,
    pub import: Option<ImportUi>,
    pub suite: SuiteUi,
    pub request_preview: Option<Result<RequestPreview, String>>,
    pub compare_target: String,
    pub compare_ignored: String,
    pub baseline: Option<duckie_app::Baseline>,
    pub pending: Option<Pending>,
    pub after_save: Option<Pending>,
    pub allow_close: bool,
    pub delete: Option<usize>,
    pub about: bool,
    pub focus_url: bool,
    /// Set by Ctrl+T when the clipboard held no usable text, alongside switching to the Auth
    /// tab; the Auth tab's bearer token field claims focus on the frame it sees this set, then
    /// clears it, mirroring `focus_url`.
    pub focus_token: bool,
    /// How Ctrl+T reads the clipboard. A function pointer rather than calling `clipboard_text`
    /// directly so a test can swap in a deterministic stub instead of depending on — and
    /// mutating — whatever is actually on the real system clipboard.
    pub clipboard_read: fn() -> Option<String>,
    /// Bumped for every new scan; a worker whose generation no longer matches stops early.
    pub search_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
    /// Files found changed on disk, and the set already shown, so the same change is reported
    /// once rather than on every focus.
    pub disk_changes: Vec<(String, duckie_storage::Change)>,
    pub disk_dismissed: Vec<String>,
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
            history: VecDeque::new(),
            history_open: false,
            history_selected: None,
            pending_history_input: None,
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
            json_path: String::new(),
            json_selected: String::new(),
            json_expanded: [String::new()].into_iter().collect(),
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
            suite: SuiteUi::default(),
            request_preview: None,
            compare_target: String::new(),
            compare_ignored: String::new(),
            baseline: None,
            pending: None,
            after_save: None,
            allow_close: false,
            delete: None,
            about: false,
            focus_url: true,
            focus_token: false,
            clipboard_read: clipboard_text,
            search_generation: Default::default(),
            disk_changes: vec![],
            disk_dismissed: vec![],
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
    pub fn paste_curl(&mut self) {
        let Some(text) = (self.clipboard_read)() else {
            self.status = "Clipboard has no text cURL command".into();
            return;
        };
        match duckie_model::curl::import(&text) {
            Ok(mut request) => {
                request.id = self.drafts[self.selected].request.id.clone();
                self.drafts[self.selected] = Draft {
                    request,
                    dirty: true,
                    revision: self.drafts[self.selected].revision + 1,
                    ..Default::default()
                };
                self.request_tab = RequestTab::Params;
                self.focus_url = true;
                self.status = "Imported cURL into the current draft for review".into();
            }
            Err(error) => {
                self.drafts[self.selected].error = format!("Could not import cURL: {error}")
            }
        }
    }
    pub fn copy_curl(&mut self, shell: duckie_model::curl::Shell, credentials: bool) {
        let mut request = self.drafts[self.selected].request.clone();
        let snapshot = self.snapshot();
        if let Some(binding) = request.auth.bearer.take() {
            let value = if credentials {
                snapshot
                    .secrets
                    .get(&binding.secret)
                    .cloned()
                    .unwrap_or_else(|| "<missing secret>".into())
            } else {
                "<redacted>".into()
            };
            request
                .headers
                .push(Row::new("Authorization", format!("Bearer {value}")));
        }
        if let Some(binding) = request.auth.api_key.take() {
            let value = if credentials {
                snapshot
                    .secrets
                    .get(&binding.secret)
                    .cloned()
                    .unwrap_or_else(|| "<missing secret>".into())
            } else {
                "<redacted>".into()
            };
            request.headers.push(Row::new(binding.header, value));
        }
        match duckie_model::curl::export(&request, shell, credentials) {
            Ok(command) => {
                self.ctx.copy_text(command);
                self.status = if credentials {
                    "Copied cURL with credentials"
                } else {
                    "Copied redacted cURL"
                }
                .into();
            }
            Err(error) => self.status = format!("Could not export cURL: {error}"),
        }
    }
    pub fn refresh_request_preview(&mut self) {
        let d = &self.drafts[self.selected];
        self.request_preview = Some(
            preview(
                &d.request,
                &self.snapshot(),
                &RunBindings::default(),
                d.revision,
            )
            .map_err(|e| e.to_string()),
        );
    }
    pub fn start_suite(&mut self) {
        if self.suite.running || self.active.is_some() {
            return;
        }
        let selected_id = self.drafts[self.selected].request.id.clone();
        let folder = self.drafts[self.selected].request.folder.clone();
        let ids: Vec<_> = self
            .drafts
            .iter()
            .filter(|d| match self.suite.scope {
                0 => d.request.id == selected_id,
                1 => d.request.folder == folder,
                _ => true,
            })
            .map(|d| d.request.id.clone())
            .collect();
        for id in &ids {
            self.ensure_loaded(id);
            if self
                .drafts
                .iter()
                .find(|d| &d.request.id == id)
                .is_some_and(|d| d.pending)
            {
                self.status = "Could not hydrate every suite request".into();
                return;
            }
        }
        let requests = self
            .drafts
            .iter()
            .filter(|d| ids.contains(&d.request.id))
            .map(|d| (d.request.clone(), d.source.clone(), d.revision))
            .collect();
        let environment = self.snapshot();
        let cancel = CancellationToken::new();
        let token = cancel.clone();
        let http = self.service.http();
        let tx = self.io_tx.clone();
        let ctx = self.ctx.clone();
        let stop = self.suite.stop_on_failure;
        self.suite.running = true;
        self.suite.results.clear();
        self.suite.cancel = Some(cancel);
        self.status = format!("Running {} request(s)…", ids.len());
        self.service.runtime().spawn(async move {
            let reports = duckie_app::execute_headless_suite(
                &http,
                requests,
                environment,
                token,
                stop,
                false,
            )
            .await;
            let _ = tx.send(IoEvent::SuiteFinished(reports)).await;
            ctx.request_repaint();
        });
    }
    pub fn touch(&mut self) {
        let d = &mut self.drafts[self.selected];
        d.dirty = true;
        d.revision += 1;
        d.error.clear();
    }
    pub fn send(&mut self) {
        if self.active.is_some() || self.service.busy() || self.suite.running {
            return;
        }
        // Belt and suspenders: the per-frame check in `ui` already loads the selected draft
        // before this can run, but Send must never fire on a deferred placeholder body.
        if self.drafts[self.selected].pending {
            let id = self.drafts[self.selected].request.id.clone();
            self.ensure_loaded(&id);
            if self.drafts[self.selected].pending {
                self.drafts[self.selected].error = format!(
                    "Could not load request content before sending: {}",
                    self.status
                );
                return;
            }
        }
        let env = self.snapshot();
        let d = &self.drafts[self.selected];
        let history_input = (d.request.clone(), d.source.clone());
        let result = prepare(&d.request, &env, &RunBindings::default(), d.revision).and_then(|p| {
            self.service.send(
                p,
                d.source.clone(),
                d.request.tests.enabled,
                env.values.clone(),
            )
        });
        match result {
            Ok((token, progress)) => {
                self.pending_history_input = Some(history_input);
                self.active = Some((d.request.id.clone(), token, Instant::now(), progress));
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
                self.active = Some((
                    d.request.id.clone(),
                    token,
                    Instant::now(),
                    // A rerun sends nothing, so these counters stay at zero and show no bytes.
                    duckie_http::Progress::default(),
                ));
                self.active_testing = true;
                self.response_tab = ResponseTab::Tests;
            }
            Err(e) => self.status = e.to_string(),
        }
    }
    pub fn preview(
        &self,
        id: String,
        run_id: String,
        body: BodyHandle,
        headers: Vec<(String, String)>,
        offset: u64,
        charset_override: Option<String>,
    ) {
        self.background(move || IoEvent::Preview {
            id,
            run_id,
            offset,
            result: (|| {
                let mut bytes = body.read(offset, MIB)?;
                let declared = declared_charset(&headers);
                let selected = charset_override.as_deref().or(declared.as_deref());
                let forced = charset_override.is_some();
                // `offset` is an arbitrary byte position from paging, not necessarily a character
                // boundary. Align every UTF-8 path, whether detected, declared, or overridden.
                // Other declared/overridden encodings continue through `encoding_rs` unchanged.
                if offset > 0 && uses_utf8_boundaries(selected) {
                    // The leading bytes here are an orphaned tail with no lead byte to decode
                    // against (that lead byte was on the previous page, which isn't re-read) —
                    // drop them rather than let them read as invalid UTF-8 and misclassify the
                    // whole response as binary.
                    trim_orphaned_utf8_tail(&mut bytes);
                }
                let (_, binary, charset) = decode_preview(&bytes, selected, forced);
                let pretty = if offset == 0 && body.len() <= MIB {
                    serde_json::from_slice::<serde_json::Value>(&bytes)
                        .ok()
                        .and_then(|v| serde_json::to_string_pretty(&v).ok())
                } else {
                    None
                };
                let page = page_size(&bytes);
                let mut shown_len = (page as usize).min(bytes.len());
                // Likewise, trim the page slice back to a full character rather than cut one in
                // half at the end — the dropped tail bytes reappear at the start of the next page.
                if shown_len < bytes.len() && uses_utf8_boundaries(selected) {
                    shown_len = complete_utf8_len(&bytes, shown_len);
                }
                // Decode only the selected page after using the larger sample for classification.
                let shown = decode_preview(&bytes[..shown_len], selected, forced).0;
                Ok(PreviewContent {
                    text: shown,
                    pretty,
                    binary,
                    page,
                    charset,
                })
            })(),
        });
    }
    /// Scans the whole body for `query` on a background thread. The generation counter cancels a
    /// scan whose query the user has already replaced, so typing cannot pile up 50 MiB scans.
    pub fn search_body(
        &mut self,
        id: String,
        body: BodyHandle,
        query: String,
        charset: Option<String>,
    ) {
        let run_id = match self.responses.get_mut(&id) {
            Some(view) => {
                view.search = BodySearch {
                    query: query.clone(),
                    running: true,
                    ..Default::default()
                };
                view.result.run_id.clone()
            }
            None => return,
        };
        let generation = self.search_generation.clone();
        let mine = generation.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;
        self.background(move || {
            let cancelled = || generation.load(std::sync::atomic::Ordering::Acquire) != mine;
            let encoded;
            let needle = if let Some(encoding) = charset
                .as_deref()
                .and_then(|label| encoding_rs::Encoding::for_label(label.as_bytes()))
            {
                encoded = encoding.encode(&query).0.into_owned();
                encoded.as_slice()
            } else {
                query.as_bytes()
            };
            let result = body
                .find_all(needle, MAX_BODY_HITS, &cancelled)
                .map(|offsets| {
                    let capped = offsets.len() == MAX_BODY_HITS;
                    (offsets, capped)
                })
                .map_err(Into::into);
            IoEvent::Searched {
                id,
                run_id,
                query,
                result,
            }
        });
    }
    /// Re-checks the collection's files. Cheap enough to run whenever the window is focused,
    /// because the watcher carries only hashes rather than the collection's contents.
    pub fn check_disk(&mut self) {
        let Some(watcher) = self.collection.as_ref().map(|c| c.watcher()) else {
            return;
        };
        if self.io_busy {
            return;
        }
        self.background(move || IoEvent::DiskChanged(watcher.compare().unwrap_or_default()));
    }
    pub fn poll(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                RunEvent::Response(result) => {
                    let id = result.request_id.clone();
                    if let Some((request, source)) = self.pending_history_input.take() {
                        self.history.push_back(HistoryEntry {
                            captured_at: SystemTime::now(),
                            request,
                            source,
                            test_revision: result.summary.revision,
                            result: result.clone(),
                            report: None,
                            body_evicted: false,
                        });
                        trim_history(&mut self.history);
                        self.history_selected = Some(self.history.len().saturating_sub(1));
                    }
                    self.preview(
                        id.clone(),
                        result.run_id.clone(),
                        result.body.clone(),
                        result.headers.clone(),
                        0,
                        None,
                    );
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
                    let json =
                        if result.outcome == Outcome::Complete && result.body.len() <= 2 * MIB {
                            result
                                .body
                                .read(0, 2 * MIB)
                                .ok()
                                .and_then(|bytes| json_with_depth(&bytes, 128).ok())
                        } else {
                            None
                        };
                    self.responses.insert(
                        id,
                        ResponseView {
                            test_revision: result.summary.revision,
                            result,
                            preview: "Preparing preview…".into(),
                            pretty: None,
                            json,
                            binary: false,
                            charset: None,
                            charset_override: None,
                            offset: 0,
                            page: MIB,
                            search: Default::default(),
                            reveal_at: None,
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
                    if let Some(entry) = self
                        .history
                        .iter_mut()
                        .rev()
                        .find(|entry| entry.result.run_id == run_id)
                    {
                        entry.report = Some(report.clone());
                        entry.test_revision = revision;
                    }
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
                    self.pending_history_input = None;
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
                            self.disk_changes.clear();
                            self.disk_dismissed.clear();
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
                                if import.update {
                                    let base = if import.url_mode {
                                        import.source.clone()
                                    } else {
                                        import.source.replace('\\', "/")
                                    };
                                    let existing: Vec<RequestDefinition> =
                                        self.drafts.iter().map(|d| d.request.clone()).collect();
                                    let plan =
                                        duckie_openapi::plan_reimport(&existing, &draft, &base);
                                    import.apply_updates = vec![true; plan.matches.len()];
                                    import.apply_additions = vec![true; plan.additions.len()];
                                    import.apply_removals = vec![false; plan.removals.len()];
                                    import.plan = Some(plan);
                                }
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
                            Ok(preview) => {
                                view.preview = preview.text;
                                view.pretty = preview.pretty;
                                view.binary = preview.binary;
                                view.charset = preview.charset;
                                view.offset = offset;
                                view.page = preview.page;
                            }
                            Err(e) => view.preview = format!("Preview unavailable: {e}"),
                        }
                    }
                }
                IoEvent::Searched {
                    id,
                    run_id,
                    query,
                    result,
                } => {
                    // A result is only interesting while it still describes the response on
                    // screen and the query the user is still typing.
                    if let Some(view) = self.responses.get_mut(&id)
                        && view.result.run_id == run_id
                        && view.search.query == query
                    {
                        view.search.running = false;
                        match result {
                            Ok((offsets, capped)) => {
                                view.search.offsets = offsets;
                                view.search.capped = capped;
                            }
                            Err(e) => self.status = format!("Search failed: {e}"),
                        }
                    }
                }
                IoEvent::DiskChanged(changed) => {
                    let names: Vec<String> = changed.iter().map(|(name, _)| name.clone()).collect();
                    if names != self.disk_dismissed {
                        self.disk_dismissed.clone_from(&names);
                        self.disk_changes = changed;
                    }
                }
                IoEvent::SuiteFinished(reports) => {
                    self.suite.running = false;
                    self.suite.cancel = None;
                    self.status = format!("Suite finished: {} result(s)", reports.len());
                    self.suite.results = reports;
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
                variable_rows: r
                    .definition
                    .variables
                    .iter()
                    .map(|(name, value)| Row::new(name, value))
                    .collect(),
                source: r.source.clone(),
                revision: 0,
                dirty: false,
                error: String::new(),
                pending: !r.loaded,
            })
            .collect();
        if self.drafts.is_empty() {
            self.drafts.push(Draft::default());
        }
        self.selected = self
            .prefs
            .selected
            .as_ref()
            .and_then(|id| self.drafts.iter().position(|d| &d.request.id == id))
            .unwrap_or(0);
        self.envs = c.environments.clone();
        self.env_index = self
            .prefs
            .environment
            .as_ref()
            .and_then(|name| self.envs.iter().position(|e| &e.name == name))
            .unwrap_or(0);
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
            #[cfg(feature = "bench")]
            crate::bench::mark(&crate::bench::OPEN_STARTED);
            let opened = Collection::open(if path.is_file() {
                path.parent().unwrap_or(&path)
            } else {
                &path
            });
            #[cfg(feature = "bench")]
            crate::bench::mark(&crate::bench::OPEN_FINISHED);
            IoEvent::Opened(opened)
        });
    }
    /// Fills in a request's real body and test content if `Collection::open` deferred reading
    /// it — a cheap no-op once already loaded. Called every frame for the selected draft, and
    /// for every draft before `save_collection` writes anything, so a request already saved
    /// with real content is never overwritten with the empty placeholder it opened with.
    pub fn ensure_loaded(&mut self, id: &str) -> bool {
        if !self.drafts.iter().any(|d| d.request.id == id && d.pending) {
            return true;
        }
        let Some(collection) = &mut self.collection else {
            return false;
        };
        if let Err(e) = collection.ensure_loaded(id) {
            self.status = format!("Could not load {id}: {e}");
            return false;
        }
        let Some(stored) = collection.requests.iter().find(|r| r.definition.id == id) else {
            return false;
        };
        let source = stored.source.clone();
        let body = stored.definition.body.clone();
        if let Some(draft) = self.drafts.iter_mut().find(|d| d.request.id == id) {
            draft.source = source;
            draft.request.body = body;
            draft.pending = false;
        }
        true
    }
    pub fn save_collection(&mut self, as_new: bool) {
        if self.io_busy {
            return;
        }
        for id in self
            .drafts
            .iter()
            .filter(|d| d.pending)
            .map(|d| d.request.id.clone())
            .collect::<Vec<_>>()
        {
            self.ensure_loaded(&id);
        }
        if let Some(d) = self.drafts.iter().find(|d| d.pending) {
            self.status = format!(
                "Could not load \"{}\" before saving; nothing was written.",
                d.request.name
            );
            self.after_save = None;
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
            .map(|d| StoredRequest::new(d.request.clone(), d.source.clone()))
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
            Pending::UpdateFromSpec => {
                self.import = Some(ImportUi {
                    update: true,
                    ..Default::default()
                })
            }
            Pending::Close => {
                self.allow_close = true;
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;

    #[test]
    fn extracts_quoted_charset_case_insensitively() {
        let headers = vec![(
            "Content-Type".into(),
            "text/plain; format=flowed; CHARSET=\"windows-1252\"".into(),
        )];
        assert_eq!(declared_charset(&headers).as_deref(), Some("windows-1252"));
    }

    #[test]
    fn decodes_declared_legacy_text_without_changing_source_bytes() {
        let bytes = b"caf\xe9";
        let (text, binary, charset) = decode_preview(bytes, Some("windows-1252"), false);
        assert_eq!(text, "caf\u{e9}");
        assert!(!binary);
        assert_eq!(charset.as_deref(), Some("windows-1252"));
        assert_eq!(bytes, b"caf\xe9");
    }

    #[test]
    fn unsupported_or_invalid_text_requires_explicit_override() {
        let bytes = b"bad\xfftext";
        assert!(decode_preview(bytes, Some("made-up"), false).1);
        assert!(decode_preview(bytes, None, false).1);
        assert!(!decode_preview(bytes, None, true).1);
    }

    #[test]
    fn a_multibyte_character_cut_off_at_a_slice_boundary_is_not_binary() {
        // "café" — the trailing é is 2 bytes (0xC3 0xA9); slicing right after the lead byte
        // leaves an incomplete-but-not-invalid sequence, which a page or chunk boundary can do.
        let truncated = "café".as_bytes();
        let cut = &truncated[..truncated.len() - 1];
        assert!(std::str::from_utf8(cut).is_err());
        assert!(!decode_preview(cut, None, false).1);
        // A genuinely invalid byte (not just a truncated tail) must still be flagged.
        assert!(decode_preview(b"caf\xff", None, false).1);
    }

    #[test]
    fn declared_and_overridden_utf8_use_character_safe_page_boundaries() {
        assert!(uses_utf8_boundaries(None));
        assert!(uses_utf8_boundaries(Some("utf-8")));
        assert!(uses_utf8_boundaries(Some("UTF8")));
        assert!(!uses_utf8_boundaries(Some("windows-1252")));
        assert!(!uses_utf8_boundaries(Some("made-up")));

        // A page beginning on the trailing byte of é must discard that orphan, and a page ending
        // on its lead byte must stop before it. These are the same operations preview() applies
        // for automatic, declared, and user-overridden UTF-8.
        let mut beginning = vec![0xA9, b'd', b'u', b'c', b'k'];
        trim_orphaned_utf8_tail(&mut beginning);
        assert_eq!(std::str::from_utf8(&beginning).unwrap(), "duck");

        let ending = b"duck\xC3";
        let shown_len = complete_utf8_len(ending, ending.len());
        assert_eq!(std::str::from_utf8(&ending[..shown_len]).unwrap(), "duck");
    }

    fn history_entry(name: &str, bytes: usize) -> HistoryEntry {
        let request = RequestDefinition {
            name: name.into(),
            ..Default::default()
        };
        HistoryEntry {
            captured_at: SystemTime::now(),
            source: String::new(),
            result: ExecutionResult {
                request_id: request.id.clone(),
                run_id: name.into(),
                summary: RequestSummary {
                    method: "GET".into(),
                    url: "http://localhost/".into(),
                    headers: vec![],
                    environment: "dev".into(),
                    revision: 0,
                    timeout_ms: 1_000,
                },
                outcome: Outcome::Complete,
                status: Some(200),
                status_text: "OK".into(),
                headers: vec![],
                body_error: None,
                body: BodyHandle::Memory(std::sync::Arc::new(vec![0; bytes])),
                encoded_bytes: bytes as u64,
                duration_ms: 1,
                diagnostics: ResponseDiagnostics::default(),
            },
            request,
            report: None,
            test_revision: 0,
            body_evicted: false,
        }
    }

    #[test]
    fn history_keeps_metadata_while_evicting_oldest_bodies_within_its_budget() {
        let mut history = VecDeque::from([
            history_entry("old", 11 * MIB as usize),
            history_entry("new", 11 * MIB as usize),
        ]);
        trim_history(&mut history);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].request.name, "old");
        assert!(history[0].body_evicted);
        assert!(history[0].result.body.is_empty());
        assert!(!history[1].body_evicted);
        assert_eq!(history[1].result.body.len(), 11 * MIB);

        for index in 0..MAX_HISTORY_ENTRIES {
            history.push_back(history_entry(&format!("run {index}"), 0));
        }
        trim_history(&mut history);
        assert_eq!(history.len(), MAX_HISTORY_ENTRIES);
        assert_eq!(history.front().unwrap().request.name, "run 0");
    }
}
