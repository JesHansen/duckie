use crate::{state::*, ui::variables};
use anyhow::Context;
use duckie_model::*;
use duckie_openapi::{
    MAX_EXTERNAL_BYTES, MAX_EXTERNAL_DOCUMENTS, external_examples, external_references,
};
use duckie_storage::SecretsFile;
use eframe::egui;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::Path,
    time::Duration,
};

const IMPORT_DEADLINE: Duration = Duration::from_secs(10 * 60);

/// Fetched `$ref` documents (parsed) and `externalValue` example content (raw bytes), keyed by
/// resolved URI.
type Acquired = (
    BTreeMap<String, serde_json::Value>,
    BTreeMap<String, Vec<u8>>,
);

/// Follows `$ref`s and `externalValue` examples to sibling files, and onward from those, until
/// nothing new is referenced. Bounded by document count and total bytes so a spec cannot pull in
/// an unbounded tree. `$ref` targets must parse as the OpenAPI document structure; example
/// content is arbitrary and kept as raw bytes.
fn read_bounded(path: &Path, limit: usize) -> std::io::Result<Option<Vec<u8>>> {
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    Ok((bytes.len() <= limit).then_some(bytes))
}

fn local_target(root: &Path, uri: &str) -> anyhow::Result<std::path::PathBuf> {
    let target = std::fs::canonicalize(uri).with_context(|| format!("Cannot read {uri}"))?;
    if !target.starts_with(root) {
        anyhow::bail!("Reference is outside the selected specification folder: {uri}");
    }
    Ok(target)
}

fn acquire_local(
    base: &str,
    root_path: &Path,
    root: &serde_json::Value,
) -> (Acquired, Vec<String>) {
    let mut fetched = BTreeMap::new();
    let mut examples = BTreeMap::new();
    let mut contents = BTreeMap::new();
    let mut notes = vec![];
    let mut budget = MAX_EXTERNAL_BYTES;
    let mut visited = BTreeSet::new();
    let mut pending = external_references(root, base);
    let mut pending_examples = external_examples(root, base);
    while let Some(uri) = pending.pop() {
        if visited.contains(&uri) {
            continue;
        }
        if visited.len() >= MAX_EXTERNAL_DOCUMENTS {
            notes.push(format!(
                "Stopped after {MAX_EXTERNAL_DOCUMENTS} external file attempts"
            ));
            break;
        }
        visited.insert(uri.clone());
        let target = match local_target(root_path, &uri) {
            Ok(target) => target,
            Err(e) => {
                notes.push(e.to_string());
                continue;
            }
        };
        match read_bounded(&target, budget) {
            Ok(Some(bytes)) => {
                budget -= bytes.len();
                match duckie_openapi::parse_document(&bytes) {
                    Ok(document) => {
                        // A fetched document's own references are relative to it, not to the root.
                        pending.extend(external_references(&document, &uri));
                        pending_examples.extend(external_examples(&document, &uri));
                        fetched.insert(uri.clone(), document);
                    }
                    Err(e) => notes.push(format!("{uri}: {e}")),
                }
                contents.insert(uri, bytes);
            }
            Ok(None) => notes.push(format!("{uri}: external files exceed the size limit")),
            Err(e) => notes.push(format!("{uri}: {e}")),
        }
    }
    for uri in pending_examples {
        if let Some(bytes) = contents.get(&uri) {
            examples.insert(uri, bytes.clone());
            continue;
        }
        if visited.contains(&uri) {
            continue;
        }
        if visited.len() >= MAX_EXTERNAL_DOCUMENTS {
            notes.push(format!(
                "Stopped after {MAX_EXTERNAL_DOCUMENTS} external file attempts"
            ));
            break;
        }
        visited.insert(uri.clone());
        let target = match local_target(root_path, &uri) {
            Ok(target) => target,
            Err(e) => {
                notes.push(e.to_string());
                continue;
            }
        };
        match read_bounded(&target, budget) {
            Ok(Some(bytes)) => {
                budget -= bytes.len();
                contents.insert(uri.clone(), bytes.clone());
                examples.insert(uri, bytes);
            }
            Ok(None) => notes.push(format!("{uri}: external files exceed the size limit")),
            Err(e) => notes.push(format!("{uri}: {e}")),
        }
    }
    ((fetched, examples), notes)
}

#[derive(Default)]
struct RemoteFetch {
    bytes: Option<Vec<u8>>,
    received: u64,
}

#[derive(Clone, Copy)]
struct AcquisitionLimits {
    deadline: tokio::time::Instant,
    documents: usize,
    bytes: u64,
}

struct RemoteAcquirer<'a> {
    http: &'a duckie_http::HttpEngine,
    origin: Option<url::Origin>,
    env: &'a EnvironmentSnapshot,
    template: &'a RequestDefinition,
    cancel: &'a tokio_util::sync::CancellationToken,
    deadline: tokio::time::Instant,
}

/// The same, over HTTP. Credentials given for the spec are reused, so acquisition is restricted to
/// the spec's own origin: sending the token to another host because a document or example asked
/// would be a credential leak the user never agreed to.
impl RemoteAcquirer<'_> {
    fn check_active(&self) -> anyhow::Result<()> {
        if self.cancel.is_cancelled() {
            anyhow::bail!("Import cancelled");
        }
        if tokio::time::Instant::now() >= self.deadline {
            anyhow::bail!("Import exceeded the overall acquisition deadline");
        }
        Ok(())
    }

    async fn fetch(&self, uri: &str, budget: u64) -> anyhow::Result<RemoteFetch> {
        self.check_active()?;
        let Ok(target) = url::Url::parse(uri) else {
            return Ok(RemoteFetch::default());
        };
        if Some(&target.origin()) != self.origin.as_ref() {
            return Ok(RemoteFetch::default());
        }
        let remaining = self
            .deadline
            .saturating_duration_since(tokio::time::Instant::now());
        let request = RequestDefinition {
            url: uri.to_owned(),
            auth: self.template.auth.clone(),
            encoded_limit: budget.max(1),
            decoded_limit: budget.max(1),
            timeout_ms: self
                .template
                .timeout_ms
                .min(remaining.as_millis().try_into().unwrap_or(u64::MAX))
                .max(1),
            ..Default::default()
        };
        let Ok(prepared) = prepare(&request, self.env, &RunBindings::default(), 0) else {
            return Ok(RemoteFetch::default());
        };
        let Some(response) = self.http.execute(prepared, self.cancel.clone()).await.ok() else {
            return Ok(RemoteFetch::default());
        };
        if response.outcome == Outcome::Cancelled && self.cancel.is_cancelled() {
            anyhow::bail!("Import cancelled");
        }
        if tokio::time::Instant::now() >= self.deadline {
            anyhow::bail!("Import exceeded the overall acquisition deadline");
        }
        let received = response.encoded_bytes.max(response.body.len());
        if response.outcome != Outcome::Complete
            || !response.status.is_some_and(|s| (200..300).contains(&s))
        {
            return Ok(RemoteFetch {
                bytes: None,
                received,
            });
        }
        Ok(RemoteFetch {
            bytes: response.body.read(0, budget).ok(),
            received,
        })
    }
}

async fn acquire_remote(
    http: &duckie_http::HttpEngine,
    base: &str,
    root: &serde_json::Value,
    env: &EnvironmentSnapshot,
    template: &RequestDefinition,
    cancel: &tokio_util::sync::CancellationToken,
    limits: AcquisitionLimits,
) -> anyhow::Result<(Acquired, Vec<String>)> {
    let acquirer = RemoteAcquirer {
        http,
        origin: url::Url::parse(base).ok().map(|u| u.origin()),
        env,
        template,
        cancel,
        deadline: limits.deadline,
    };
    let mut fetched = BTreeMap::new();
    let mut examples = BTreeMap::new();
    let mut contents = BTreeMap::new();
    let mut notes = vec![];
    let mut budget = limits.bytes;
    let mut visited = BTreeSet::new();
    let mut pending = external_references(root, base);
    let mut pending_examples = external_examples(root, base);
    while let Some(uri) = pending.pop() {
        acquirer.check_active()?;
        if visited.contains(&uri) {
            continue;
        }
        if visited.len() >= limits.documents {
            notes.push(format!(
                "Stopped after {} external request attempts",
                limits.documents
            ));
            break;
        }
        visited.insert(uri.clone());
        if budget == 0 {
            notes.push(format!(
                "Stopped after receiving {} bytes from external requests",
                limits.bytes
            ));
            break;
        }
        let result = acquirer.fetch(&uri, budget).await?;
        budget = budget.saturating_sub(result.received);
        if let Some(bytes) = result.bytes {
            match duckie_openapi::parse_document(&bytes) {
                Ok(document) => {
                    pending.extend(external_references(&document, &uri));
                    pending_examples.extend(external_examples(&document, &uri));
                    fetched.insert(uri.clone(), document);
                }
                Err(error) => notes.push(format!("{uri}: {error}")),
            }
            contents.insert(uri, bytes);
        }
    }
    for uri in pending_examples {
        acquirer.check_active()?;
        if let Some(bytes) = contents.get(&uri) {
            examples.insert(uri, bytes.clone());
            continue;
        }
        if visited.contains(&uri) {
            continue;
        }
        if visited.len() >= limits.documents {
            notes.push(format!(
                "Stopped after {} external request attempts",
                limits.documents
            ));
            break;
        }
        visited.insert(uri.clone());
        if budget == 0 {
            notes.push(format!(
                "Stopped after receiving {} bytes from external requests",
                limits.bytes
            ));
            break;
        }
        let result = acquirer.fetch(&uri, budget).await?;
        budget = budget.saturating_sub(result.received);
        if let Some(bytes) = result.bytes {
            contents.insert(uri.clone(), bytes.clone());
            examples.insert(uri, bytes);
        }
    }
    Ok(((fetched, examples), notes))
}

impl Duckie {
    pub fn dialogs(&mut self, ctx: &egui::Context) {
        if self.suite.open {
            self.suite_dialog(ctx);
        }
        if self.history_open {
            self.history_dialog(ctx);
        }
        if self.request_preview.is_some() {
            let mut open = true;
            egui::Window::new("Prepared request preview")
                .open(&mut open)
                .default_width(680.0)
                .show(ctx, |ui| match self.request_preview.as_ref().unwrap() {
                    Ok(preview) => {
                        ui.strong(format!(
                            "{} {}",
                            preview.summary.method, preview.summary.url
                        ));
                        ui.weak(&preview.note);
                        ui.separator();
                        ui.label(format!("Environment: {}", preview.summary.environment));
                        ui.strong("Application-prepared headers");
                        for (name, value) in &preview.summary.headers {
                            ui.monospace(format!("{name}: {value}"));
                        }
                        ui.strong("Body representation");
                        ui.add(
                            egui::Label::new(egui::RichText::new(&preview.body).monospace())
                                .selectable(true),
                        );
                        ui.strong("Variable sources");
                        if preview.variables.is_empty() {
                            ui.weak("No template variables used");
                        }
                        for item in &preview.variables {
                            ui.label(format!("{{{{{}}}}} — {}", item.reference, item.source));
                        }
                    }
                    Err(error) => {
                        ui.colored_label(ui.visuals().error_fg_color, error);
                        ui.weak("Correct the referenced request field, then preview again.");
                    }
                });
            if !open {
                self.request_preview = None;
            }
        }
        if self.env_dialog {
            self.environment_dialog(ctx);
        }
        if self.import.is_some() {
            self.import_dialog(ctx);
        }
        if let Some(action) = self.pending {
            use egui::{Key, KeyboardShortcut, Modifiers};
            let pressed = |modifiers, key| {
                ctx.input_mut(|input| {
                    input.consume_shortcut(&KeyboardShortcut::new(modifiers, key))
                })
            };
            let mut choice =
                if pressed(Modifiers::NONE, Key::Escape) || pressed(Modifiers::ALT, Key::C) {
                    3
                } else if pressed(Modifiers::ALT, Key::D) {
                    2
                } else if pressed(Modifiers::NONE, Key::Enter)
                    || pressed(Modifiers::ALT, Key::S)
                    || pressed(Modifiers::CTRL, Key::S)
                {
                    1
                } else {
                    0
                };
            egui::Window::new("Unsaved changes")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Save your collection and current drafts before continuing?");
                    ui.horizontal(|ui| {
                        if ui.button("Save (Enter / Alt+S)").clicked() {
                            choice = 1;
                        }
                        if ui.button("Discard (Alt+D)").clicked() {
                            choice = 2;
                        }
                        if ui.button("Cancel (Esc / Alt+C)").clicked() {
                            choice = 3;
                        }
                    });
                });
            if choice > 0 {
                self.pending = None;
            }
            match choice {
                1 => {
                    self.after_save = Some(action);
                    self.save_collection(false);
                }
                2 => self.perform(action),
                _ => {}
            }
        }
        if let Some(index) = self.delete {
            let mut choice = 0;
            egui::Window::new("Delete request?")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!(
                        "Remove “{}” from this collection?",
                        self.drafts[index].request.name
                    ));
                    ui.weak("The manifest changes on Save. Existing files remain on disk.");
                    ui.horizontal(|ui| {
                        if ui
                            .add_enabled(self.active.is_none(), egui::Button::new("Delete request"))
                            .clicked()
                        {
                            choice = 1;
                        }
                        if ui.button("Cancel").clicked() {
                            choice = 2;
                        }
                    });
                });
            if choice == 1 {
                let d = self.drafts.remove(index);
                self.responses.remove(&d.request.id);
                if self.drafts.is_empty() {
                    self.drafts.push(Draft::default());
                }
                self.selected = self.selected.min(self.drafts.len() - 1);
                self.env_dirty = true;
            }
            if choice > 0 {
                self.delete = None;
            }
        }
        if !self.disk_changes.is_empty() {
            self.disk_dialog(ctx);
        }
        if self.about {
            egui::Window::new("About Duckie").open(&mut self.about).resizable(false).show(ctx,|ui|{
            ui.heading("Duckie 1.0.0");ui.label("A small, local HTTP workbench for Windows.");ui.separator();
            ui.label("Ctrl+N   New request\nCtrl+O   Open collection\nCtrl+S   Save collection\nCtrl+Enter   Send request\nCtrl+Shift+Enter   Rerun tests without HTTP\nCtrl+L   Focus URL\nCtrl+T   Paste bearer token\nCtrl+K   Find requests\nCtrl+B   Toggle sidebar\nEscape   Close dialog or stop work");
            ui.separator();ui.label("See README.md and IMPLEMENTATION_STATUS.md for coverage and release gates.");
        });
        }
    }
    fn suite_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.suite.open;
        let selected = self.drafts[self.selected].request.clone();
        egui::Window::new("Run requests").open(&mut open).default_width(720.0).show(ctx,|ui|{
            ui.horizontal(|ui|{ui.selectable_value(&mut self.suite.scope,0,"Selected request");ui.selectable_value(&mut self.suite.scope,1,"Current folder");ui.selectable_value(&mut self.suite.scope,2,"Collection");});
            ui.checkbox(&mut self.suite.stop_on_failure,"Stop on first failure");ui.weak(format!("Environment: {} · requests run sequentially with no retries",self.envs[self.env_index].name));
            ui.separator();ui.strong("Operations");egui::ScrollArea::vertical().max_height(150.0).show(ui,|ui|for d in &self.drafts{let included=match self.suite.scope{0=>d.request.id==selected.id,1=>d.request.folder==selected.folder,_=>true};if included{ui.monospace(format!("{}  {}",d.request.method,d.request.address()));}});
            ui.horizontal(|ui|{
                if ui.add_enabled(!self.suite.running,egui::Button::new("Run")).clicked(){self.start_suite();}
                if self.suite.running&&ui.button("Cancel").clicked()&&let Some(token)=&self.suite.cancel{token.cancel();}
            });
            if self.suite.running{ui.spinner();ui.label("Running…");}
            if !self.suite.results.is_empty(){ui.separator();ui.strong("Results");egui::Grid::new("suite-results").striped(true).show(ui,|ui|{ui.weak("Result");ui.weak("Request");ui.weak("Status");ui.weak("Duration");ui.end_row();for r in &self.suite.results{let state=if r.execution_failed(){"ERROR"}else if r.assertion_failed(){"FAIL"}else{"PASS"};ui.label(state);ui.label(&r.request_name);ui.label(r.status.map_or("-".into(),|s|s.to_string()));ui.label(format!("{} ms",r.duration_ms));ui.end_row();}});ui.horizontal(|ui|{for (label,ext) in [("Export JSON…","json"),("Export JUnit…","xml"),("Export HTML…","html")]{if ui.button(label).clicked()&&let Some(path)=rfd::FileDialog::new().add_filter(ext,&[ext]).set_file_name(format!("duckie-report.{ext}")).save_file(){let content=match ext{"xml"=>duckie_app::junit(&self.suite.results),"html"=>duckie_app::html(&self.suite.results),_=>serde_json::to_string_pretty(&self.suite.results).unwrap_or_default()};self.status=match std::fs::write(&path,content){Ok(())=>format!("Saved report to {}",path.display()),Err(e)=>format!("Could not save report: {e}")};}}});ui.weak("Reports contain summaries by default; arbitrary response snippets require the CLI's explicit inclusion option.");}
        });
        if !open
            && self.suite.running
            && let Some(token) = &self.suite.cancel
        {
            token.cancel();
        }
        self.suite.open = open;
    }
    fn history_dialog(&mut self, ctx: &egui::Context) {
        let mut open = self.history_open;
        let mut replay = None;
        let mut clear = false;
        egui::Window::new("Session history")
            .open(&mut open)
            .default_width(820.0)
            .default_height(520.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.weak(format!(
                        "{} runs · up to {} entries and {} MiB of response bodies",
                        self.history.len(),
                        MAX_HISTORY_ENTRIES,
                        HISTORY_BODY_BUDGET / MIB
                    ));
                    if ui
                        .add_enabled(!self.history.is_empty(), egui::Button::new("Clear history"))
                        .clicked()
                    {
                        clear = true;
                    }
                });
                ui.weak("History is session-only. Eviction removes response bodies first; request and result metadata remain available.");
                ui.separator();
                ui.columns(2, |columns| {
                    egui::ScrollArea::vertical()
                        .id_salt("history-list")
                        .show(&mut columns[0], |ui| {
                            for (index, entry) in self.history.iter().enumerate().rev() {
                                let age = entry
                                    .captured_at
                                    .elapsed()
                                    .map(|elapsed| format!("{}s ago", elapsed.as_secs()))
                                    .unwrap_or_else(|_| "just now".into());
                                let state = if entry.result.outcome != Outcome::Complete {
                                    "ERROR"
                                } else if entry.report.as_ref().is_some_and(|report| {
                                    report.suite_error.is_some()
                                        || report.tests.iter().any(|test| !test.passed)
                                }) {
                                    "FAIL"
                                } else {
                                    "PASS"
                                };
                                let label = format!(
                                    "{state} · {} · {}\n{} · {} ms",
                                    entry.request.name,
                                    entry.result.summary.environment,
                                    age,
                                    entry.result.duration_ms
                                );
                                if ui
                                    .selectable_label(self.history_selected == Some(index), label)
                                    .clicked()
                                {
                                    self.history_selected = Some(index);
                                }
                            }
                        });
                    columns[1].separator();
                    let Some(index) = self.history_selected else {
                        columns[1].weak("Send a request to create the first history entry.");
                        return;
                    };
                    let Some(entry) = self.history.get(index) else {
                        return;
                    };
                    let ui = &mut columns[1];
                    ui.heading(&entry.request.name);
                    ui.monospace(format!("{} {}", entry.request.method, entry.request.address()));
                    ui.label(format!(
                        "Environment: {} · Draft revision: {}",
                        entry.result.summary.environment, entry.result.summary.revision
                    ));
                    ui.label(format!(
                        "Result: {}{} · {} ms",
                        entry.result.outcome,
                        entry
                            .result
                            .status
                            .map(|status| format!(" · HTTP {status}"))
                            .unwrap_or_default(),
                        entry.result.duration_ms
                    ));
                    if let Some(report) = &entry.report {
                        let passed = report.tests.iter().filter(|test| test.passed).count();
                        ui.label(format!(
                            "Tests: {passed} passed, {} failed · test revision {}",
                            report.tests.len() - passed,
                            entry.test_revision
                        ));
                    } else {
                        ui.weak("No test result captured");
                    }
                    if ui.button("Open as new draft").clicked() {
                        replay = Some(index);
                    }
                    ui.separator();
                    ui.strong("Captured editable input");
                    let mut definition =
                        serde_json::to_string_pretty(&entry.request).unwrap_or_default();
                    if definition.len() > 4096 {
                        let mut boundary = 4096;
                        while !definition.is_char_boundary(boundary) {
                            boundary -= 1;
                        }
                        definition.truncate(boundary);
                        definition.push_str("\n… input preview limited to 4 KiB");
                    }
                    egui::ScrollArea::vertical()
                        .max_height(140.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(egui::RichText::new(definition).monospace())
                                    .selectable(true),
                            );
                        });
                    ui.separator();
                    ui.strong("Captured effective request");
                    ui.monospace(format!(
                        "{} {}",
                        entry.result.summary.method, entry.result.summary.url
                    ));
                    for (name, value) in &entry.result.summary.headers {
                        ui.monospace(format!("{name}: {value}"));
                    }
                    ui.separator();
                    ui.strong("Captured response");
                    if entry.body_evicted {
                        ui.colored_label(
                            ui.visuals().warn_fg_color,
                            "Response body evicted from bounded history",
                        );
                    } else {
                        ui.label(format!("{} decoded bytes", entry.result.body.len()));
                        let bytes = entry.result.body.read(0, 4096).unwrap_or_default();
                        let mut preview = String::from_utf8_lossy(&bytes).into_owned();
                        if entry.result.body.len() > bytes.len() as u64 {
                            preview.push_str("\n… preview limited to 4 KiB");
                        }
                        egui::ScrollArea::vertical()
                            .max_height(160.0)
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(egui::RichText::new(preview).monospace())
                                        .selectable(true),
                                );
                            });
                    }
                });
            });
        if clear {
            self.history.clear();
            self.history_selected = None;
        }
        if let Some(index) = replay
            && let Some(entry) = self.history.get(index)
        {
            let mut request = entry.request.clone();
            request.id = new_id();
            request.name.push_str(" replay");
            request.tests.file.clear();
            let variable_rows = request
                .variables
                .iter()
                .map(|(name, value)| Row::new(name, value))
                .collect();
            self.drafts.push(Draft {
                request,
                variable_rows,
                source: entry.source.clone(),
                dirty: true,
                ..Default::default()
            });
            self.selected = self.drafts.len() - 1;
            self.request_tab = RequestTab::Params;
            self.focus_url = true;
            self.status = "Opened history snapshot as a new unsaved draft".into();
            open = false;
        }
        self.history_open = open;
    }
    /// Reports files another program changed under the collection, and offers the only two
    /// honest choices: take what is on disk, or keep the in-memory version and deal with it at
    /// save time, where the same hashes block an overwrite.
    fn disk_dialog(&mut self, ctx: &egui::Context) {
        let dirty = self.drafts.iter().any(|d| d.dirty) || self.env_dirty;
        let mut choice = 0;
        let mut open = true;
        egui::Window::new("Changed on disk")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} file(s) under this collection changed outside Duckie.",
                    self.disk_changes.len()
                ));
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(ui, |ui| {
                        egui::Grid::new("disk-changes")
                            .striped(true)
                            .show(ui, |ui| {
                                for (path, change) in &self.disk_changes {
                                    ui.label(match change {
                                        duckie_storage::Change::Added => "Added",
                                        duckie_storage::Change::Removed => "Removed",
                                        duckie_storage::Change::Modified => "Modified",
                                    });
                                    ui.label(egui::RichText::new(path).monospace());
                                    // File names are ids, so name the request a path belongs to.
                                    ui.weak(self.request_named(path).unwrap_or_default());
                                    ui.end_row();
                                }
                            });
                    });
                ui.add_space(8.0);
                if dirty {
                    ui.colored_label(
                        crate::ui::warning(ui),
                        "You have unsaved changes. Reloading discards them.",
                    );
                }
                ui.horizontal(|ui| {
                    if ui
                        .button("Reload from disk")
                        .on_hover_text("Reopen the collection, discarding anything unsaved")
                        .clicked()
                    {
                        choice = 1;
                    }
                    if ui
                        .button("Keep my version")
                        .on_hover_text(
                            "Saving still refuses to overwrite until you reload or save elsewhere",
                        )
                        .clicked()
                    {
                        choice = 2;
                    }
                });
            });
        if choice == 1 {
            self.disk_changes.clear();
            self.perform(Pending::Reload);
        } else if choice == 2 || !open {
            self.disk_changes.clear();
        }
    }
    /// The request a tracked path belongs to, for paths that name one.
    pub fn request_named(&self, path: &str) -> Option<String> {
        let collection = self.collection.as_ref()?;
        let index = collection
            .manifest
            .requests
            .iter()
            .position(|request| request == path)?;
        Some(collection.requests.get(index)?.definition.name.clone())
    }

    fn hydrate_update_matches(
        &mut self,
        plan: &duckie_openapi::ReimportPlan,
        apply_updates: &[bool],
    ) -> bool {
        for existing_id in plan
            .matches
            .iter()
            .enumerate()
            .filter(|(row, _)| apply_updates[*row])
            .map(|(_, matched)| self.drafts[matched.existing_index].request.id.clone())
            .collect::<Vec<_>>()
        {
            if !self.ensure_loaded(&existing_id) {
                return false;
            }
        }
        true
    }

    fn environment_dialog(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let mut dirty = false;
        let mut load = false;
        let mut export = false;
        egui::Window::new("Environments").open(&mut open).default_width(680.0).show(ctx,|ui| {
            ui.horizontal(|ui|{egui::ComboBox::from_id_salt("edit-env").selected_text(&self.envs[self.env_index].name).show_ui(ui,|ui|{for (i,env) in self.envs.iter().enumerate(){ui.selectable_value(&mut self.env_index,i,&env.name);}});ui.add(egui::TextEdit::singleline(&mut self.new_env).hint_text("New environment name").desired_width(170.0));if ui.button("Add environment").clicked(){let name=self.new_env.trim();if !name.is_empty() && name.chars().all(|c|c.is_alphanumeric()||c=='-'||c=='_') && !self.envs.iter().any(|e|e.name==name){self.envs.push(Environment{name:name.into(),..Default::default()});self.env_index=self.envs.len()-1;self.new_env.clear();dirty=true;}}});
            ui.separator();ui.strong("Variables");ui.weak("Use {{env.name}} in a URL, header, or body.");
            dirty|=variables(ui,"environment-vars",&mut self.envs[self.env_index].values);
            ui.add_space(12.0);ui.strong("Secrets");ui.weak("Remember credentials in the request's Auth tab. Loaded values are session-only until remembered.");
            let env=&self.envs[self.env_index].name;
            if let Some(values)=self.secrets.get_mut(env){for (key,value) in values{ui.horizontal(|ui|{ui.label(key);dirty|=ui.add(egui::TextEdit::singleline(value).password(true).desired_width(330.0)).changed();ui.weak(if self.remember.contains(&(env.clone(),key.clone())){"Remembered"}else{"This session"});});}}
            ui.separator();ui.weak(self.collection.as_ref().map(|c|c.root.display().to_string()).unwrap_or("Scratch workspace — choose a folder on Save".into()));
            ui.horizontal(|ui|{load=ui.button("Load secrets file…").clicked();export=ui.button("Export secrets…").clicked();if ui.button("Done").clicked(){self.env_dialog=false;}});
            ui.weak("Export writes plaintext credentials to a separate file you select.");
        });
        if dirty {
            self.env_dirty = true;
        }
        if !open {
            self.env_dialog = false;
        }
        if load
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Secrets JSON", &["json"])
                .pick_file()
        {
            self.background(move || IoEvent::Secrets(duckie_storage::load_secrets(&path)));
        }
        if export
            && let Some(path) = rfd::FileDialog::new()
                .set_title("Export plaintext credentials to a separate file")
                .set_file_name("duckie-secrets.json")
                .save_file()
        {
            let secrets = SecretsFile {
                schema_version: 1,
                environments: self.secrets.clone(),
            };
            self.background(move || {
                IoEvent::Message(
                    duckie_storage::export_secrets(&path, &secrets)
                        .map(|_| format!("Exported plaintext secrets to {}", path.display())),
                )
            });
        }
    }
    fn import_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut import) = self.import.take() else {
            return;
        };
        let mut open = true;
        let mut read = false;
        let mut commit = false;
        let mut apply = false;
        let mut back = false;
        let title = if import.update {
            "Update from spec"
        } else {
            "Import OpenAPI"
        };
        egui::Window::new(title).open(&mut open).default_size([880.0,570.0]).show(ctx,|ui|{
            match import.draft.as_mut() { None => {
                let mut focus_read = false;
                ui.horizontal(|ui|{ui.selectable_value(&mut import.url_mode,true,"URL");ui.selectable_value(&mut import.url_mode,false,"Local file");});ui.separator();
                ui.label("Swagger 2.0 / OpenAPI 3.0 / 3.1 / 3.2 · JSON or YAML");
                ui.horizontal(|ui| {
                    let field = ui.add(egui::TextEdit::singleline(&mut import.source).desired_width(650.0).hint_text(if import.url_mode { "https://api.example.com/openapi.yaml" } else { "Choose an OpenAPI JSON or YAML file" }));
                    focus_read = import.url_mode && field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));
                    if import.focus_source {
                        if import.url_mode { field.request_focus(); }
                        import.focus_source = false;
                    }
                    if !import.url_mode && ui.button("Browse…").clicked() && let Some(path) = rfd::FileDialog::new().add_filter("OpenAPI", &["json", "yaml", "yml"]).pick_file() {
                        import.source = path.to_string_lossy().into_owned();
                    }
                });
                if import.url_mode{ui.collapsing("Authentication for this import only",|ui|{
                    ui.label("Bearer token");ui.add(egui::TextEdit::singleline(&mut import.bearer).password(true).desired_width(500.0));
                    ui.horizontal(|ui|{ui.label("API key header");ui.text_edit_singleline(&mut import.api_header);ui.label("Value");ui.add(egui::TextEdit::singleline(&mut import.api_value).password(true));});ui.weak("Import credentials are never copied to generated requests.");
                });}
                ui.add_space(16.0);ui.weak("Read creates a review draft. Generated API requests are never sent during import.");
                let can_read = !self.io_busy && !import.source.trim().is_empty();
                let read_button = ui.add_enabled(can_read, egui::Button::new("Read and review"));
                read = read_button.clicked();
                if focus_read && can_read { read_button.request_focus(); }
            }, Some(draft) if import.update => {
                ui.label(format!("Comparing against the open collection · OpenAPI {}", draft.version));
                if let Some(plan) = &import.plan {
                    let unchanged = plan.matches.iter().filter(|m| !m.changed).count();
                    ui.separator();
                    egui::ScrollArea::vertical().id_salt("plan-review").max_height(400.0).show(ui, |ui| {
                        if plan.matches.iter().any(|m| m.changed) {
                            ui.strong("Changed");
                            for (row, m) in plan.matches.iter().enumerate() {
                                if !m.changed { continue; }
                                let existing = &self.drafts[m.existing_index].request;
                                let fresh = &draft.operations[m.operation_index].request;
                                ui.checkbox(&mut import.apply_updates[row], format!("{}  {}", fresh.method, existing.name));
                            }
                        }
                        if !plan.additions.is_empty() {
                            ui.add_space(8.0);
                            ui.strong("New");
                            for (row, &j) in plan.additions.iter().enumerate() {
                                let op = &draft.operations[j];
                                ui.checkbox(&mut import.apply_additions[row], format!("{}  {}", op.request.method, op.request.name));
                            }
                        }
                        if !plan.removals.is_empty() {
                            ui.add_space(8.0);
                            ui.strong("No longer in the spec");
                            ui.weak("Unchecked requests stay in the collection.");
                            for (row, &i) in plan.removals.iter().enumerate() {
                                let existing = &self.drafts[i].request;
                                ui.checkbox(&mut import.apply_removals[row], format!("{}  {}", existing.method, existing.name));
                            }
                        }
                    });
                    ui.separator();
                    ui.weak(format!("{unchanged} operation(s) unchanged, not shown"));
                    for diagnostic in &draft.diagnostics { ui.label(diagnostic); }
                    let selected = import.apply_updates.iter().filter(|b| **b).count()
                        + import.apply_additions.iter().filter(|b| **b).count()
                        + import.apply_removals.iter().filter(|b| **b).count();
                    ui.horizontal(|ui| {
                        back = ui.button("Back").clicked();
                        apply = ui.add_enabled(selected > 0 && !self.io_busy, egui::Button::new(format!("Apply {selected} change(s) to this collection"))).clicked();
                    });
                }
            }, Some(draft) => {
                ui.horizontal(|ui|{ui.label("Collection");ui.text_edit_singleline(&mut draft.name);ui.weak(format!("OpenAPI {}",draft.version));});
                ui.horizontal(|ui|{ui.label("Server / baseUrl");ui.add(egui::TextEdit::singleline(&mut import.server).desired_width(540.0));egui::ComboBox::from_id_salt("import-server").selected_text("Servers").show_ui(ui,|ui|{for server in &draft.servers{ui.selectable_value(&mut import.server,server.clone(),server);}});});
                ui.separator();
                ui.columns(2,|columns|{
                    let left=&mut columns[0];left.add(egui::TextEdit::singleline(&mut import.filter).hint_text("Find operations…").desired_width(f32::INFINITY));
                    left.horizontal(|ui|{if ui.button("Select all").clicked(){for op in &mut draft.operations{op.selected=true;}}
                    if ui.button("None").clicked(){for op in &mut draft.operations{op.selected=false;}}});
                    egui::ScrollArea::vertical().id_salt("import-operations").max_height(310.0).show(left,|ui|{
                        for (i,op) in draft.operations.iter_mut().enumerate(){if !format!("{} {}",op.request.name,op.request.url).to_lowercase().contains(&import.filter.to_lowercase()){continue;}ui.horizontal(|ui|{ui.checkbox(&mut op.selected,"");if ui.selectable_label(import.selected==i,format!("{}  {}",op.request.method,op.request.name)).clicked(){import.selected=i;}});}
                    });
                    let right=&mut columns[1];let op=&mut draft.operations[import.selected];right.strong(format!("{} {}",op.request.method,op.request.url));
                    if !op.bodies.is_empty() {
                        egui::ComboBox::from_id_salt("import-body")
                            .selected_text(&op.bodies[op.body_index].0)
                            .width(350.0)
                            .show_ui(right, |ui| {
                                for (i, (name, _, _)) in op.bodies.iter().enumerate() {
                                    ui.selectable_value(&mut op.body_index, i, name);
                                }
                            });
                    }
                    egui::ComboBox::from_id_salt("import-auth")
                        .selected_text(&op.auth_options[op.auth_index].0)
                        .width(350.0)
                        .show_ui(right, |ui| {
                            for (i, (name, _, _)) in op.auth_options.iter().enumerate() {
                                ui.selectable_value(&mut op.auth_index, i, name);
                            }
                        });
                    egui::ScrollArea::vertical()
                        .id_salt("operation-preview")
                        .max_height(270.0)
                        .show(right, |ui| {
                            for diagnostic in &op.diagnostics {
                                ui.label(diagnostic);
                            }
                            for blocker in op
                                .request
                                .blockers
                                .iter()
                                .chain(&op.auth_options[op.auth_index].2)
                                .chain(
                                    op.bodies
                                        .get(op.body_index)
                                        .map(|(_, _, blockers)| blockers)
                                        .into_iter()
                                        .flatten(),
                                )
                            {
                                ui.colored_label(
                                    ui.visuals().warn_fg_color,
                                    format!("Unsupported: {blocker}"),
                                );
                            }
                            if let Some((_, Body::Json { text } | Body::Text { text }, _)) =
                                op.bodies.get(op.body_index)
                            {
                                ui.add(
                                    egui::Label::new(egui::RichText::new(text).monospace())
                                        .selectable(true),
                                );
                            }
                        });
                });
                ui.separator();
                for diagnostic in &draft.diagnostics {
                    ui.label(diagnostic);
                }
                let count = draft.operations.iter().filter(|op| op.selected).count();
                let needs = draft
                    .operations
                    .iter()
                    .filter(|op| op.selected && op.has_diagnostics())
                    .count();
                ui.label(format!("{count} operations selected · {needs} have diagnostics"));
                ui.horizontal(|ui| {
                    back = ui.button("Back").clicked();
                    let can_commit = count > 0 && !self.io_busy;
                    let import_button = ui.add_enabled(can_commit, egui::Button::new(format!("Import {count} requests")));
                    commit = import_button.clicked();
                    if import.focus_commit {
                        if can_commit { import_button.request_focus(); }
                        import.focus_commit = false;
                    }
                });
            }}
            if self.io_busy{ui.label("Reading and parsing…");if let Some(token)=&import.cancel&& ui.button("Cancel read").clicked(){token.cancel();}}
            if !import.error.is_empty(){ui.colored_label(ui.visuals().error_fg_color,&import.error);}
        });
        if back {
            import.draft = None;
            import.plan = None;
            import.error.clear();
        }
        if read {
            import.error.clear();
            self.io_busy = true;
            if import.url_mode {
                let source = import.source.clone();
                let bearer = import.bearer.clone();
                let api_header = import.api_header.clone();
                let api_value = import.api_value.clone();
                let http = self.service.http();
                let tx = self.io_tx.clone();
                let wake = self.ctx.clone();
                let cancel = tokio_util::sync::CancellationToken::new();
                import.cancel = Some(cancel.clone());
                self.service.runtime().spawn(async move {
                    let result = async {
                        let deadline = tokio::time::Instant::now() + IMPORT_DEADLINE;
                        let mut request = RequestDefinition {
                            url: source,
                            encoded_limit: 20 * MIB,
                            decoded_limit: 20 * MIB,
                            ..Default::default()
                        };
                        let mut env = EnvironmentSnapshot::default();
                        if !bearer.is_empty() {
                            request.auth.bearer = Some(SecretBinding {
                                secret: "importBearer".into(),
                            });
                            env.secrets.insert("importBearer".into(), bearer);
                        }
                        if !api_value.is_empty() {
                            request.auth.api_key = Some(ApiKey {
                                header: api_header,
                                secret: "importKey".into(),
                            });
                            env.secrets.insert("importKey".into(), api_value);
                        }
                        let base = request.url.clone();
                        let prepared = prepare(&request, &env, &RunBindings::default(), 0)?;
                        let response = http.execute(prepared, cancel.clone()).await?;
                        if response.outcome != Outcome::Complete {
                            anyhow::bail!("Import failed: {}", response.outcome);
                        }
                        if !response.status.is_some_and(|s| (200..300).contains(&s)) {
                            anyhow::bail!(
                                "Import returned HTTP {} {}",
                                response.status.unwrap_or_default(),
                                response.status_text
                            );
                        }
                        let bytes = response.body.read(0, 20 * MIB)?;
                        let mut root = duckie_openapi::parse_document(&bytes)?;
                        let ((fetched, fetched_examples), mut notes) = acquire_remote(
                            &http,
                            &base,
                            &root,
                            &env,
                            &request,
                            &cancel,
                            AcquisitionLimits {
                                deadline,
                                documents: MAX_EXTERNAL_DOCUMENTS,
                                bytes: MAX_EXTERNAL_BYTES as u64,
                            },
                        )
                        .await?;
                        let doc_bases = duckie_openapi::document_keys(&fetched);
                        notes.extend(duckie_openapi::inline_external(&mut root, &base, &fetched));
                        notes.extend(duckie_openapi::inline_examples(
                            &mut root,
                            &base,
                            &doc_bases,
                            &fetched_examples,
                        ));
                        tokio::task::spawn_blocking(move || {
                            duckie_openapi::import_value_based(root, &base, &doc_bases).map(
                                |mut draft| {
                                    draft.diagnostics.extend(notes);
                                    draft
                                },
                            )
                        })
                        .await?
                    }
                    .await;
                    let _ = tx.send(IoEvent::Imported(result)).await;
                    wake.request_repaint();
                });
            } else {
                let path = std::path::PathBuf::from(&import.source);
                self.background(move || {
                    IoEvent::Imported((|| {
                        let base = path.to_string_lossy().replace('\\', "/");
                        let canonical = std::fs::canonicalize(&path)?;
                        let root_path = canonical
                            .parent()
                            .context("OpenAPI source has no containing folder")?;
                        let bytes = read_bounded(&canonical, 20 * MIB as usize)?
                            .context("OpenAPI file exceeds 20 MiB")?;
                        let mut root = duckie_openapi::parse_document(&bytes)?;
                        let ((fetched, fetched_examples), mut notes) =
                            acquire_local(&base, root_path, &root);
                        let doc_bases = duckie_openapi::document_keys(&fetched);
                        notes.extend(duckie_openapi::inline_external(&mut root, &base, &fetched));
                        notes.extend(duckie_openapi::inline_examples(
                            &mut root,
                            &base,
                            &doc_bases,
                            &fetched_examples,
                        ));
                        duckie_openapi::import_value_based(root, &base, &doc_bases).map(
                            |mut draft| {
                                draft.diagnostics.extend(notes);
                                draft
                            },
                        )
                    })())
                });
            }
        }
        // Importing does not touch disk: the app already runs perfectly well with no collection
        // at all (the blank scratch screen at startup), so a fresh import just becomes that same
        // in-memory working set. The user only picks a folder if and when they choose to Save —
        // the common case of "import, call a few endpoints, throw it away" never needs one.
        if commit {
            let draft = import.draft.as_ref().unwrap();
            self.drafts = draft
                .operations
                .iter()
                .filter(|op| op.selected)
                .map(|op| {
                    let request = op.finish();
                    Draft {
                        variable_rows: request
                            .variables
                            .iter()
                            .map(|(name, value)| Row::new(name, value))
                            .collect(),
                        request,
                        source: String::new(),
                        revision: 0,
                        dirty: true,
                        error: String::new(),
                        pending: false,
                    }
                })
                .collect();
            if self.drafts.is_empty() {
                self.drafts.push(Draft::default());
            }
            self.selected = 0;
            self.envs = vec![Environment::default()];
            self.envs[0]
                .values
                .insert("baseUrl".into(), import.server.clone());
            self.env_index = 0;
            self.secrets.clear();
            self.remember.clear();
            self.responses.clear();
            self.env_dirty = true;
            self.collection = None;
            self.prefs.last_collection = None;
            self.status = format!("Imported {} — not saved yet.", draft.name);
            self.focus_url = true;
            open = false;
        }
        if apply
            && let Some(plan) = import.plan.as_ref()
            && !self.hydrate_update_matches(plan, &import.apply_updates)
        {
            import.error = format!(
                "Could not load existing request content before applying updates: {}",
                self.status
            );
            apply = false;
        }
        if apply && let (Some(draft), Some(plan)) = (import.draft.take(), import.plan.take()) {
            // Whole-operation replace: every field but id and tests comes from the fresh spec,
            // exactly as a first import would produce it. Removal is by id, decided up front,
            // because applying updates first would shift the indices the plan was computed against.
            let remove_ids: Vec<String> = plan
                .removals
                .iter()
                .zip(&import.apply_removals)
                .filter(|(_, apply)| **apply)
                .map(|(&i, _)| self.drafts[i].request.id.clone())
                .collect();
            for (row, m) in plan.matches.iter().enumerate() {
                if import.apply_updates[row] {
                    // Hydrate first: if this draft hasn't been selected since the collection
                    // opened, `existing.tests` is still an empty placeholder, and copying it
                    // below would silently wipe out real, saved tests.
                    let mut fresh = draft.operations[m.operation_index].finish();
                    let existing = &self.drafts[m.existing_index].request;
                    fresh.id = existing.id.clone();
                    fresh.tests = existing.tests.clone();
                    self.drafts[m.existing_index].variable_rows = fresh
                        .variables
                        .iter()
                        .map(|(name, value)| Row::new(name, value))
                        .collect();
                    self.drafts[m.existing_index].request = fresh;
                    self.drafts[m.existing_index].dirty = true;
                    // `fresh.body` came straight from the spec, not a deferred placeholder, so
                    // this draft must not be re-hydrated later — that would overwrite it with
                    // the old stored body.
                    self.drafts[m.existing_index].pending = false;
                }
            }
            for (row, &j) in plan.additions.iter().enumerate() {
                if import.apply_additions[row] {
                    let request = draft.operations[j].finish();
                    let variable_rows = request
                        .variables
                        .iter()
                        .map(|(name, value)| Row::new(name, value))
                        .collect();
                    self.drafts.push(Draft {
                        request,
                        variable_rows,
                        source: String::new(),
                        revision: 0,
                        dirty: true,
                        error: String::new(),
                        pending: false,
                    });
                }
            }
            let removed = remove_ids.len();
            for id in remove_ids {
                if let Some(pos) = self.drafts.iter().position(|d| d.request.id == id) {
                    self.drafts.remove(pos);
                    self.responses.remove(&id);
                }
            }
            if self.drafts.is_empty() {
                self.drafts.push(Draft::default());
            }
            self.selected = self.selected.min(self.drafts.len() - 1);
            // Manifest membership changed even where no single request's content did.
            self.env_dirty = true;
            self.status = format!(
                "Applied {} update(s), {} addition(s) and {removed} removal(s). Save to write them to disk.",
                plan.matches
                    .iter()
                    .enumerate()
                    .filter(|(row, _)| import.apply_updates[*row])
                    .count(),
                plan.additions
                    .iter()
                    .enumerate()
                    .filter(|(row, _)| import.apply_additions[*row])
                    .count(),
            );
            open = false;
        }
        if open {
            self.import = Some(import);
        } else if let Some(token) = import.cancel {
            token.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use duckie_storage::StoredRequest;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn spec_update_stops_when_existing_tests_cannot_be_hydrated() {
        let dir = tempfile::tempdir().unwrap();
        let mut collection =
            duckie_storage::Collection::new(dir.path().to_path_buf(), "collection".into()).unwrap();
        collection.requests.push(StoredRequest::new(
            RequestDefinition {
                id: "one".into(),
                ..Default::default()
            },
            "test('preserved', () => {});".into(),
        ));
        collection.save().unwrap();

        let opened = duckie_storage::Collection::open(dir.path()).unwrap();
        std::fs::remove_file(dir.path().join("tests/one.test.js")).unwrap();
        let ctx = egui::Context::default();
        let mut app = Duckie::new(&eframe::CreationContext::_new_kittest(ctx));
        app.use_collection(opened);
        assert!(app.drafts[0].pending);

        let plan = duckie_openapi::ReimportPlan {
            matches: vec![duckie_openapi::Match {
                existing_index: 0,
                operation_index: 0,
                changed: true,
            }],
            additions: vec![],
            removals: vec![],
        };
        assert!(!app.hydrate_update_matches(&plan, &[true]));
        assert!(app.drafts[0].pending);
        assert!(app.drafts[0].source.is_empty());
        assert!(app.status.contains("Could not load one"));
    }

    async fn remote_server<F>(
        response: F,
    ) -> (
        String,
        Arc<Mutex<Vec<String>>>,
        Arc<tokio::sync::Notify>,
        tokio::task::JoinHandle<()>,
    )
    where
        F: Fn(&str) -> (Vec<u8>, Duration) + Send + Sync + 'static,
    {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::new(tokio::sync::Notify::new());
        let recorded = requests.clone();
        let notify = seen.clone();
        let response = Arc::new(response);
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let response = response.clone();
                let recorded = recorded.clone();
                let notify = notify.clone();
                tokio::spawn(async move {
                    let mut input = vec![0; 4096];
                    let Ok(read) = socket.read(&mut input).await else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&input[..read]);
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/")
                        .to_owned();
                    recorded.lock().unwrap().push(path.clone());
                    notify.notify_one();
                    let (body, delay) = response(&path);
                    tokio::time::sleep(delay).await;
                    let headers = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(headers.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                });
            }
        });
        (format!("http://{address}"), requests, seen, task)
    }

    fn remote_template() -> RequestDefinition {
        RequestDefinition {
            proxy: ProxyMode::Direct,
            ..Default::default()
        }
    }

    #[test]
    fn local_acquisition_stays_inside_the_selected_spec_folder() {
        let temp = tempfile::tempdir().unwrap();
        let specs = temp.path().join("specs");
        std::fs::create_dir(&specs).unwrap();
        std::fs::write(specs.join("inside.txt"), b"inside").unwrap();
        std::fs::write(temp.path().join("outside.txt"), b"outside").unwrap();
        let base = specs
            .join("openapi.json")
            .to_string_lossy()
            .replace('\\', "/");
        let root = serde_json::json!({
            "examples": {
                "inside": {"externalValue": "inside.txt"},
                "outside": {"externalValue": "../outside.txt"}
            }
        });

        let ((_, examples), notes) =
            acquire_local(&base, &std::fs::canonicalize(&specs).unwrap(), &root);

        let inside = duckie_openapi::join_ref(&base, "inside.txt");
        let outside = duckie_openapi::join_ref(&base, "../outside.txt");
        assert_eq!(examples.get(&inside).unwrap(), b"inside");
        assert!(!examples.contains_key(&outside));
        assert!(
            notes
                .iter()
                .any(|note| note.contains("outside the selected"))
        );
    }

    #[test]
    fn bounded_file_read_does_not_return_an_oversized_file() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), b"123456789").unwrap();
        assert!(read_bounded(temp.path(), 8).unwrap().is_none());
    }

    #[test]
    fn local_uri_can_supply_a_reference_and_an_example_with_one_read() {
        let temp = tempfile::tempdir().unwrap();
        let shared = br#"{"type":"string"}"#;
        std::fs::write(temp.path().join("shared.json"), shared).unwrap();
        let base = temp
            .path()
            .join("openapi.json")
            .to_string_lossy()
            .replace('\\', "/");
        let root = serde_json::json!({
            "components": {
                "schemas": {"shared": {"$ref": "shared.json"}},
                "examples": {"shared": {"externalValue": "shared.json"}}
            }
        });

        let ((documents, examples), _) =
            acquire_local(&base, &std::fs::canonicalize(temp.path()).unwrap(), &root);

        let uri = duckie_openapi::join_ref(&base, "shared.json");
        assert!(documents.contains_key(&uri));
        assert_eq!(examples.get(&uri).unwrap(), shared);
    }

    #[test]
    fn local_yaml_references_are_parsed_and_followed_transitively() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("paths.yaml"),
            b"forecast:\n  get:\n    operationId: forecast\n    responses:\n      '200':\n        description: ok\ncomponents:\n  schemas:\n    weather:\n      $ref: schema.yml\n",
        )
        .unwrap();
        std::fs::write(temp.path().join("schema.yml"), b"type: object\n").unwrap();
        let base = temp
            .path()
            .join("openapi.yaml")
            .to_string_lossy()
            .replace('\\', "/");
        let root = serde_json::json!({"pathItem": {"$ref": "paths.yaml"}});

        let ((documents, _), notes) =
            acquire_local(&base, &std::fs::canonicalize(temp.path()).unwrap(), &root);

        let paths = duckie_openapi::join_ref(&base, "paths.yaml");
        let schema = duckie_openapi::join_ref(&paths, "schema.yml");
        assert!(documents.contains_key(&paths), "{notes:?}");
        assert!(documents.contains_key(&schema), "{notes:?}");
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[tokio::test]
    async fn failed_remote_fetches_count_toward_the_attempt_limit() {
        let (origin, requests, _, server) =
            remote_server(|_| (b"\xff".to_vec(), Duration::ZERO)).await;
        let references: Vec<_> = (0..6)
            .map(|i| serde_json::json!({"$ref": format!("/bad-{i}.json")}))
            .collect();
        let root = serde_json::json!({"references": references});
        let base = format!("{origin}/openapi.json");
        let cancel = tokio_util::sync::CancellationToken::new();

        let ((fetched, _), notes) = acquire_remote(
            &duckie_http::HttpEngine::default(),
            &base,
            &root,
            &EnvironmentSnapshot::default(),
            &remote_template(),
            &cancel,
            AcquisitionLimits {
                deadline: tokio::time::Instant::now() + Duration::from_secs(5),
                documents: 3,
                bytes: 100,
            },
        )
        .await
        .unwrap();

        server.abort();
        assert!(fetched.is_empty());
        assert_eq!(requests.lock().unwrap().len(), 3);
        assert!(
            notes
                .iter()
                .any(|note| note.contains("3 external request attempts"))
        );
    }

    #[tokio::test]
    async fn invalid_remote_bodies_consume_the_byte_budget() {
        let (origin, requests, _, server) =
            remote_server(|_| (b"xxxxx".to_vec(), Duration::ZERO)).await;
        let root = serde_json::json!({
            "references": [
                {"$ref": "/one.json"},
                {"$ref": "/two.json"}
            ]
        });
        let base = format!("{origin}/openapi.json");
        let cancel = tokio_util::sync::CancellationToken::new();

        let (_, notes) = acquire_remote(
            &duckie_http::HttpEngine::default(),
            &base,
            &root,
            &EnvironmentSnapshot::default(),
            &remote_template(),
            &cancel,
            AcquisitionLimits {
                deadline: tokio::time::Instant::now() + Duration::from_secs(5),
                documents: 10,
                bytes: 5,
            },
        )
        .await
        .unwrap();

        server.abort();
        assert_eq!(requests.lock().unwrap().len(), 1);
        assert!(notes.iter().any(|note| note.contains("receiving 5 bytes")));
    }

    #[tokio::test]
    async fn failed_remote_uri_is_visited_only_once() {
        let (origin, requests, _, server) = remote_server(|path| {
            if matches!(path, "/a.json" | "/b.json") {
                (br#"{"$ref":"/fail.json"}"#.to_vec(), Duration::ZERO)
            } else {
                (b"x".to_vec(), Duration::ZERO)
            }
        })
        .await;
        let root = serde_json::json!({
            "references": [{"$ref": "/a.json"}, {"$ref": "/b.json"}]
        });
        let base = format!("{origin}/openapi.json");
        let cancel = tokio_util::sync::CancellationToken::new();

        acquire_remote(
            &duckie_http::HttpEngine::default(),
            &base,
            &root,
            &EnvironmentSnapshot::default(),
            &remote_template(),
            &cancel,
            AcquisitionLimits {
                deadline: tokio::time::Instant::now() + Duration::from_secs(5),
                documents: 10,
                bytes: 100,
            },
        )
        .await
        .unwrap();

        server.abort();
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests.iter().filter(|path| *path == "/fail.json").count(),
            1
        );
    }

    #[tokio::test]
    async fn remote_yaml_uri_can_supply_a_reference_and_an_example_with_one_request() {
        let shared = b"type: string\n";
        let (origin, requests, _, server) =
            remote_server(move |_| (shared.to_vec(), Duration::ZERO)).await;
        let root = serde_json::json!({
            "components": {
                "schemas": {"shared": {"$ref": "/shared.yaml"}},
                "examples": {"shared": {"externalValue": "/shared.yaml"}}
            }
        });
        let base = format!("{origin}/openapi.json");
        let uri = format!("{origin}/shared.yaml");
        let cancel = tokio_util::sync::CancellationToken::new();

        let ((documents, examples), _) = acquire_remote(
            &duckie_http::HttpEngine::default(),
            &base,
            &root,
            &EnvironmentSnapshot::default(),
            &remote_template(),
            &cancel,
            AcquisitionLimits {
                deadline: tokio::time::Instant::now() + Duration::from_secs(5),
                documents: 10,
                bytes: 100,
            },
        )
        .await
        .unwrap();

        server.abort();
        assert_eq!(requests.lock().unwrap().len(), 1);
        assert!(documents.contains_key(&uri));
        assert_eq!(examples.get(&uri).unwrap(), shared);
    }

    #[tokio::test]
    async fn overall_deadline_stops_external_acquisition_before_a_request() {
        let (origin, requests, _, server) =
            remote_server(|_| (b"{}".to_vec(), Duration::ZERO)).await;
        let root = serde_json::json!({"$ref": "/late.json"});
        let base = format!("{origin}/openapi.json");
        let cancel = tokio_util::sync::CancellationToken::new();

        let error = acquire_remote(
            &duckie_http::HttpEngine::default(),
            &base,
            &root,
            &EnvironmentSnapshot::default(),
            &remote_template(),
            &cancel,
            AcquisitionLimits {
                deadline: tokio::time::Instant::now(),
                documents: 10,
                bytes: 100,
            },
        )
        .await
        .unwrap_err();

        server.abort();
        assert!(requests.lock().unwrap().is_empty());
        assert!(error.to_string().contains("deadline"));
    }

    #[tokio::test]
    async fn cancellation_reaches_an_external_fetch() {
        let (origin, _, request_started, server) =
            remote_server(|_| (b"{}".to_vec(), Duration::from_secs(5))).await;
        let root = serde_json::json!({"$ref": "/slow.json"});
        let base = format!("{origin}/openapi.json");
        let cancel = tokio_util::sync::CancellationToken::new();
        let http = duckie_http::HttpEngine::default();
        let env = EnvironmentSnapshot::default();
        let template = remote_template();
        let acquisition = acquire_remote(
            &http,
            &base,
            &root,
            &env,
            &template,
            &cancel,
            AcquisitionLimits {
                deadline: tokio::time::Instant::now() + Duration::from_secs(10),
                documents: 10,
                bytes: 100,
            },
        );
        tokio::pin!(acquisition);
        tokio::select! {
            result = &mut acquisition => panic!("acquisition completed before cancellation: {result:?}"),
            _ = request_started.notified() => {}
        }
        cancel.cancel();

        let error = tokio::time::timeout(Duration::from_secs(1), &mut acquisition)
            .await
            .expect("cancellation should promptly stop the fetch")
            .unwrap_err();

        server.abort();
        assert!(error.to_string().contains("cancelled"));
    }
}
