use crate::{state::*, ui::variables};
use anyhow::Context;
use duckie_model::*;
use duckie_openapi::{
    MAX_EXTERNAL_BYTES, MAX_EXTERNAL_DOCUMENTS, external_examples, external_references,
};
use duckie_storage::{Collection, SecretsFile, StoredRequest};
use eframe::egui;
use std::collections::BTreeMap;

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
fn acquire_local(base: &str, root: &serde_json::Value) -> (Acquired, Vec<String>) {
    let mut fetched = BTreeMap::new();
    let mut examples = BTreeMap::new();
    let mut notes = vec![];
    let mut budget = MAX_EXTERNAL_BYTES;
    let mut pending = external_references(root, base);
    let mut pending_examples = external_examples(root, base);
    while let Some(uri) = pending.pop() {
        if fetched.contains_key(&uri) {
            continue;
        }
        if fetched.len() >= MAX_EXTERNAL_DOCUMENTS {
            notes.push(format!(
                "Stopped after {MAX_EXTERNAL_DOCUMENTS} referenced documents"
            ));
            break;
        }
        match std::fs::read(&uri) {
            Ok(bytes) if bytes.len() <= budget => match serde_json::from_slice(&bytes) {
                Ok(document) => {
                    budget -= bytes.len();
                    // A fetched document's own references are relative to it, not to the root.
                    pending.extend(external_references(&document, &uri));
                    pending_examples.extend(external_examples(&document, &uri));
                    fetched.insert(uri, document);
                }
                Err(e) => notes.push(format!("{uri}: {e}")),
            },
            Ok(_) => notes.push(format!("{uri}: referenced documents exceed the size limit")),
            Err(e) => notes.push(format!("{uri}: {e}")),
        }
    }
    for uri in pending_examples {
        if examples.contains_key(&uri) || fetched.len() + examples.len() >= MAX_EXTERNAL_DOCUMENTS {
            continue;
        }
        match std::fs::read(&uri) {
            Ok(bytes) if bytes.len() <= budget => {
                budget -= bytes.len();
                examples.insert(uri, bytes);
            }
            Ok(_) => notes.push(format!("{uri}: referenced example exceeds the size limit")),
            Err(e) => notes.push(format!("{uri}: {e}")),
        }
    }
    ((fetched, examples), notes)
}
/// The same, over HTTP. Credentials given for the spec are reused, so acquisition is restricted to
/// the spec's own origin: sending the token to another host because a document or example asked
/// would be a credential leak the user never agreed to.
async fn fetch_remote(
    http: &duckie_http::HttpEngine,
    origin: Option<&url::Origin>,
    uri: &str,
    budget: u64,
    env: &EnvironmentSnapshot,
    template: &RequestDefinition,
) -> Option<Vec<u8>> {
    let target = url::Url::parse(uri).ok()?;
    if Some(&target.origin()) != origin {
        return None;
    }
    let request = RequestDefinition {
        url: uri.to_owned(),
        auth: template.auth.clone(),
        encoded_limit: budget.max(1),
        decoded_limit: budget.max(1),
        ..Default::default()
    };
    let prepared = prepare(&request, env, &RunBindings::default(), 0).ok()?;
    let token = tokio_util::sync::CancellationToken::new();
    let response = http.execute(prepared, token).await.ok()?;
    if response.outcome != Outcome::Complete
        || !response.status.is_some_and(|s| (200..300).contains(&s))
    {
        return None;
    }
    response.body.read(0, budget).ok()
}
async fn acquire_remote(
    http: &duckie_http::HttpEngine,
    base: &str,
    root: &serde_json::Value,
    env: &EnvironmentSnapshot,
    template: &RequestDefinition,
) -> Acquired {
    let origin = url::Url::parse(base).ok().map(|u| u.origin());
    let mut fetched = BTreeMap::new();
    let mut examples = BTreeMap::new();
    let mut budget = MAX_EXTERNAL_BYTES as u64;
    let mut pending = external_references(root, base);
    let mut pending_examples = external_examples(root, base);
    while let Some(uri) = pending.pop() {
        if fetched.contains_key(&uri) || fetched.len() >= MAX_EXTERNAL_DOCUMENTS {
            continue;
        }
        let Some(bytes) = fetch_remote(http, origin.as_ref(), &uri, budget, env, template).await
        else {
            continue;
        };
        if let Ok(document) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            budget = budget.saturating_sub(bytes.len() as u64);
            pending.extend(external_references(&document, &uri));
            pending_examples.extend(external_examples(&document, &uri));
            fetched.insert(uri, document);
        }
    }
    for uri in pending_examples {
        if examples.contains_key(&uri) || fetched.len() + examples.len() >= MAX_EXTERNAL_DOCUMENTS {
            continue;
        }
        if let Some(bytes) = fetch_remote(http, origin.as_ref(), &uri, budget, env, template).await
        {
            budget = budget.saturating_sub(bytes.len() as u64);
            examples.insert(uri, bytes);
        }
    }
    (fetched, examples)
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
            ui.heading("Duckie 0.1.0");ui.label("A small, local HTTP workbench for Windows.");ui.separator();
            ui.label("Ctrl+N   New request\nCtrl+O   Open collection\nCtrl+S   Save collection\nCtrl+Enter   Send request\nCtrl+Shift+Enter   Rerun tests without HTTP\nCtrl+L   Focus URL\nCtrl+T   Focus bearer token\nCtrl+K   Find requests\nCtrl+B   Toggle sidebar\nEscape   Close dialog or stop work");
            ui.separator();ui.label("First development preview. See README.md and IMPLEMENTATION_STATUS.md for coverage and release gates.");
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
                ui.label("OpenAPI 3.0 / 3.1 / 3.2 JSON");
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
                        let response = http.execute(prepared, cancel).await?;
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
                        let (fetched, fetched_examples) =
                            acquire_remote(&http, &base, &root, &env, &request).await;
                        let doc_bases = duckie_openapi::document_keys(&fetched);
                        let mut notes = duckie_openapi::inline_external(&mut root, &base, &fetched);
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
                        if std::fs::metadata(&path)?.len() > 20 * MIB {
                            anyhow::bail!("OpenAPI file exceeds 20 MiB");
                        }
                        let base = path.to_string_lossy().replace('\\', "/");
                        let mut root: serde_json::Value =
                            serde_json::from_slice(&std::fs::read(&path)?)
                                .context("OpenAPI source must be valid JSON")?;
                        let ((fetched, fetched_examples), mut notes) = acquire_local(&base, &root);
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
                    let mut fresh = draft.operations[m.operation_index].finish();
                    let existing = &self.drafts[m.existing_index].request;
                    fresh.id = existing.id.clone();
                    fresh.tests = existing.tests.clone();
                    self.drafts[m.existing_index].request = fresh;
                    self.drafts[m.existing_index].dirty = true;
                }
            }
            for (row, &j) in plan.additions.iter().enumerate() {
                if import.apply_additions[row] {
                    self.drafts.push(Draft {
                        request: draft.operations[j].finish(),
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
