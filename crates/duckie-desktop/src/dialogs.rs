use crate::{state::*, ui::variables};
use anyhow::Context;
use duckie_model::*;
use duckie_openapi::{
    MAX_EXTERNAL_BYTES, MAX_EXTERNAL_DOCUMENTS, external_examples, external_references,
};
use duckie_storage::{Collection, SecretsFile, StoredRequest};
use eframe::egui;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::Path,
    time::Duration,
};

const IMPORT_DEADLINE: Duration = Duration::from_secs(60);

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
                match serde_json::from_slice(&bytes) {
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
            if let Ok(document) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                pending.extend(external_references(&document, &uri));
                pending_examples.extend(external_examples(&document, &uri));
                fetched.insert(uri.clone(), document);
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
        if self.env_dialog {
            self.environment_dialog(ctx);
        }
        if self.import.is_some() {
            self.import_dialog(ctx);
        }
        if let Some(action) = self.pending {
            let mut choice = 0;
            egui::Window::new("Unsaved changes")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label("Save your collection and current drafts before continuing?");
                    ui.horizontal(|ui| {
                        if ui.button("Save").clicked() {
                            choice = 1;
                        }
                        if ui.button("Discard").clicked() {
                            choice = 2;
                        }
                        if ui.button("Cancel").clicked() {
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
                ui.horizontal(|ui|{ui.selectable_value(&mut import.url_mode,false,"Local file");ui.selectable_value(&mut import.url_mode,true,"URL");});ui.separator();
                ui.label("Swagger 2.0 / OpenAPI 3.0 / 3.1 / 3.2 JSON");
                ui.horizontal(|ui|{ui.add(egui::TextEdit::singleline(&mut import.source).desired_width(650.0).hint_text(if import.url_mode{"https://api.example.com/openapi.json"}else{"Choose an OpenAPI JSON file"}));if !import.url_mode && ui.button("Browse…").clicked()&& let Some(path)=rfd::FileDialog::new().add_filter("OpenAPI JSON",&["json"]).pick_file(){import.source=path.to_string_lossy().into_owned();}});
                if import.url_mode{ui.collapsing("Authentication for this import only",|ui|{
                    ui.label("Bearer token");ui.add(egui::TextEdit::singleline(&mut import.bearer).password(true).desired_width(500.0));
                    ui.horizontal(|ui|{ui.label("API key header");ui.text_edit_singleline(&mut import.api_header);ui.label("Value");ui.add(egui::TextEdit::singleline(&mut import.api_value).password(true));});ui.weak("Import credentials are never copied to generated requests.");
                });}
                ui.add_space(16.0);ui.weak("Read creates a review draft. Generated API requests are never sent during import.");
                read=ui.add_enabled(!self.io_busy && !import.source.trim().is_empty(),egui::Button::new("Read and review")).clicked();
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
                    if !op.bodies.is_empty(){egui::ComboBox::from_id_salt("import-body").selected_text(&op.bodies[op.body_index].0).width(350.0).show_ui(right,|ui|{for (i,(name,_)) in op.bodies.iter().enumerate(){ui.selectable_value(&mut op.body_index,i,name);}});}
                    egui::ComboBox::from_id_salt("import-auth").selected_text(&op.auth_options[op.auth_index].0).width(350.0).show_ui(right,|ui|{for (i,(name,_,_)) in op.auth_options.iter().enumerate(){ui.selectable_value(&mut op.auth_index,i,name);}});
                    egui::ScrollArea::vertical().id_salt("operation-preview").max_height(270.0).show(right,|ui|{
                        for d in &op.diagnostics{ui.label(d);}for d in op.request.blockers.iter().chain(&op.auth_options[op.auth_index].2){ui.colored_label(ui.visuals().warn_fg_color,format!("Unsupported: {d}"));}
                        if let Some((_,Body::Json{text}|Body::Text{text}))=op.bodies.get(op.body_index){ui.add(egui::Label::new(egui::RichText::new(text).monospace()).selectable(true));}
                    });
                });
                ui.separator();for diagnostic in &draft.diagnostics{ui.label(diagnostic);}
                let count=draft.operations.iter().filter(|op|op.selected).count();let needs=draft.operations.iter().filter(|op|op.selected && (!op.diagnostics.is_empty() || !op.request.blockers.is_empty())).count();
                ui.label(format!("{count} operations selected · {needs} have diagnostics"));
                ui.horizontal(|ui|{back=ui.button("Back").clicked();commit=ui.add_enabled(count>0 && !self.io_busy,egui::Button::new(format!("Import {count} requests into a new folder…"))).clicked();});
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
                        let mut root: serde_json::Value = serde_json::from_slice(&bytes)
                            .context("OpenAPI source must be valid JSON")?;
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
                        let mut root: serde_json::Value = serde_json::from_slice(&bytes)
                            .context("OpenAPI source must be valid JSON")?;
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
        if commit
            && let Some(path) = rfd::FileDialog::new()
                .set_title("Choose an empty folder for the imported collection")
                .pick_folder()
        {
            let draft = import.draft.as_ref().unwrap();
            match Collection::new(path, draft.name.clone()) {
                Ok(mut collection) => {
                    collection.requests = draft
                        .operations
                        .iter()
                        .filter(|op| op.selected)
                        .map(|op| StoredRequest::new(op.finish(), String::new()))
                        .collect();
                    collection.environments[0]
                        .values
                        .insert("baseUrl".into(), import.server.clone());
                    self.io_busy = true;
                    self.background(move || IoEvent::Opened(collection.save().map(|_| collection)));
                    open = false;
                }
                Err(e) => import.error = e.to_string(),
            }
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
                    let existing_id = self.drafts[m.existing_index].request.id.clone();
                    self.ensure_loaded(&existing_id);
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
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

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

    #[tokio::test]
    async fn failed_remote_fetches_count_toward_the_attempt_limit() {
        let (origin, requests, _, server) =
            remote_server(|_| (b"not json".to_vec(), Duration::ZERO)).await;
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
    async fn remote_uri_can_supply_a_reference_and_an_example_with_one_request() {
        let shared = br#"{"type":"string"}"#;
        let (origin, requests, _, server) =
            remote_server(move |_| (shared.to_vec(), Duration::ZERO)).await;
        let root = serde_json::json!({
            "components": {
                "schemas": {"shared": {"$ref": "/shared.json"}},
                "examples": {"shared": {"externalValue": "/shared.json"}}
            }
        });
        let base = format!("{origin}/openapi.json");
        let uri = format!("{origin}/shared.json");
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
