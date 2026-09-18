use crate::{editor, state::*};
use duckie_model::*;
use eframe::egui::{self, Color32, RichText};
use std::time::Duration;

fn accent(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(130, 183, 255)
    } else {
        Color32::from_rgb(30, 83, 161)
    }
}
pub fn warning(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(241, 202, 119)
    } else {
        Color32::from_rgb(137, 82, 0)
    }
}
fn success(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::from_rgb(139, 220, 171)
    } else {
        Color32::from_rgb(24, 114, 66)
    }
}
fn pointer_part(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}
fn json_value_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}
struct JsonRow<'a> {
    label: String,
    pointer: String,
    value: &'a serde_json::Value,
    depth: usize,
}
#[cfg(test)]
fn json_rows<'a>(
    value: &'a serde_json::Value,
    expanded: &std::collections::BTreeSet<String>,
) -> Vec<JsonRow<'a>> {
    json_rows_filtered(value, expanded, false, "")
}
fn json_rows_filtered<'a>(
    value: &'a serde_json::Value,
    expanded: &std::collections::BTreeSet<String>,
    all: bool,
    filter: &str,
) -> Vec<JsonRow<'a>> {
    // The parsed tree has the existing 2 MiB/128-level bounds. Search all of it so
    // a match after the first 10,000 nodes can still be reached, but cap layout rows.
    fn matches(
        value: &serde_json::Value,
        label: &str,
        pointer: &str,
        query: &str,
        keep: &mut std::collections::BTreeSet<String>,
    ) -> bool {
        let mut found = label.to_lowercase().contains(query)
            || (!value.is_object()
                && !value.is_array()
                && json_value_text(value).to_lowercase().contains(query));
        match value {
            serde_json::Value::Object(values) => {
                for (key, value) in values {
                    found |= matches(
                        value,
                        key,
                        &format!("{pointer}/{}", pointer_part(key)),
                        query,
                        keep,
                    );
                }
            }
            serde_json::Value::Array(values) => {
                for (i, value) in values.iter().enumerate() {
                    found |= matches(
                        value,
                        &format!("[{i}]"),
                        &format!("{pointer}/{i}"),
                        query,
                        keep,
                    );
                }
            }
            _ => {}
        }
        if found {
            keep.insert(pointer.to_string());
        }
        found
    }
    let mut keep = std::collections::BTreeSet::new();
    if !filter.trim().is_empty() {
        matches(value, "$", "", &filter.trim().to_lowercase(), &mut keep);
    }
    let filtering = !filter.trim().is_empty();
    struct Visibility<'a> {
        expanded: &'a std::collections::BTreeSet<String>,
        keep: &'a std::collections::BTreeSet<String>,
        filtering: bool,
        all: bool,
    }
    fn visit<'a>(
        rows: &mut Vec<JsonRow<'a>>,
        label: String,
        pointer: String,
        value: &'a serde_json::Value,
        depth: usize,
        visibility: &Visibility<'_>,
    ) {
        if rows.len() >= 10_000 || (visibility.filtering && !visibility.keep.contains(&pointer)) {
            return;
        }
        rows.push(JsonRow {
            label,
            pointer: pointer.clone(),
            value,
            depth,
        });
        if !visibility.filtering && !visibility.all && !visibility.expanded.contains(&pointer) {
            return;
        }
        match value {
            serde_json::Value::Object(values) => {
                for (key, value) in values {
                    visit(
                        rows,
                        key.clone(),
                        format!("{pointer}/{}", pointer_part(key)),
                        value,
                        depth + 1,
                        visibility,
                    )
                }
            }
            serde_json::Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    if rows.len() >= 10_000 {
                        break;
                    }
                    visit(
                        rows,
                        format!("[{index}]"),
                        format!("{pointer}/{index}"),
                        value,
                        depth + 1,
                        visibility,
                    )
                }
            }
            _ => {}
        }
    }
    let mut rows = vec![];
    visit(
        &mut rows,
        "$".into(),
        String::new(),
        value,
        0,
        &Visibility {
            expanded,
            keep: &keep,
            filtering,
            all,
        },
    );
    rows
}

fn json_tree_key(tree: &mut JsonTreeState, rows: &[JsonRow<'_>], key: egui::Key) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let current = rows
        .iter()
        .position(|r| r.pointer == tree.selected)
        .unwrap_or(0);
    let row = &rows[current];
    let next = match key {
        egui::Key::ArrowDown => (current + 1).min(rows.len() - 1),
        egui::Key::ArrowUp => current.saturating_sub(1),
        egui::Key::Home => 0,
        egui::Key::End => rows.len() - 1,
        egui::Key::ArrowRight => {
            if row.value.is_object() || row.value.is_array() {
                if !tree.expand_all
                    && tree.filter.trim().is_empty()
                    && tree.expanded.insert(row.pointer.clone())
                {
                    current
                } else if rows.get(current + 1).is_some_and(|r| r.depth > row.depth) {
                    current + 1
                } else {
                    current
                }
            } else {
                current
            }
        }
        egui::Key::ArrowLeft => {
            if tree.expand_all {
                tree.expanded = rows
                    .iter()
                    .filter(|r| r.value.is_object() || r.value.is_array())
                    .map(|r| r.pointer.clone())
                    .collect();
                tree.expand_all = false;
            }
            if tree.filter.trim().is_empty() && tree.expanded.remove(&row.pointer) {
                current
            } else {
                rows[..current]
                    .iter()
                    .rposition(|r| r.depth < row.depth)
                    .unwrap_or(current)
            }
        }
        _ => current,
    };
    tree.selected = rows[next].pointer.clone();
    Some(next)
}
pub fn rows(ui: &mut egui::Ui, id: &str, rows: &mut Vec<Row>) -> bool {
    let mut changed = false;
    let mut remove = None;
    let mut append = false;
    let last = rows.len().saturating_sub(1);
    let available = ui.available_width();
    let (name_width, value_width) = row_widths(available);
    egui::ScrollArea::horizontal()
        .id_salt((id, "scroll"))
        .show(ui, |ui| {
            egui::Grid::new(id)
                .num_columns(4)
                .spacing([8.0, 6.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.weak("Use");
                    ui.weak("Name");
                    ui.weak("Value");
                    ui.end_row();
                    for (i, row) in rows.iter_mut().enumerate() {
                        changed |= ui
                            .checkbox(&mut row.enabled, "")
                            .on_hover_text("Include this row")
                            .changed();
                        let name_id = egui::Id::new((id, i, "name"));
                        let value_id = egui::Id::new((id, i, "value"));
                        let focused = ui.memory(|m| m.focused());
                        let enter = ui.input_mut(|input| {
                            (focused == Some(name_id) || focused == Some(value_id))
                                && input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                        });
                        let name_focused = ui.memory(|m| m.has_focus(name_id));
                        let name = ui.add_sized(
                            [name_width, ui.spacing().interact_size.y],
                            egui::TextEdit::singleline(&mut row.name)
                                .id(name_id)
                                .hint_text("Name"),
                        );
                        let value = ui.add_sized(
                            [value_width, ui.spacing().interact_size.y],
                            egui::TextEdit::singleline(&mut row.value)
                                .id(value_id)
                                .hint_text("Value"),
                        );
                        if enter {
                            if name_focused {
                                ui.memory_mut(|m| m.request_focus(value_id));
                            } else if i == last
                                && (!row.name.trim().is_empty() || !row.value.trim().is_empty())
                            {
                                append = true;
                            }
                        }
                        let edit = name.changed() | value.changed();
                        if edit {
                            row.raw = None;
                            row.raw_is_literal = false;
                            changed = true;
                        }
                        if ui.small_button("×").on_hover_text("Remove row").clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
        });
    if let Some(index) = remove {
        rows.remove(index);
        changed = true;
    }
    if ui.button("+ Add row").clicked() || (append && remove.is_none()) {
        let new_name = egui::Id::new((id, rows.len(), "name"));
        rows.push(Row::new("", ""));
        ui.memory_mut(|memory| memory.request_focus(new_name));
        changed = true;
    }
    changed
}

fn row_widths(available: f32) -> (f32, f32) {
    let name = ((available - 90.0) * 0.35).clamp(180.0, 260.0);
    (name, (available - name - 90.0).max(240.0))
}
pub fn variables(ui: &mut egui::Ui, id: &str, values: &mut Values) -> bool {
    let mut entries: Vec<Row> = values.iter().map(|(k, v)| Row::new(k, v)).collect();
    if rows(ui, id, &mut entries) {
        *values = entries.into_iter().map(|r| (r.name, r.value)).collect();
        true
    } else {
        false
    }
}

fn request_variables(ui: &mut egui::Ui, id: &str, draft: &mut Draft) -> bool {
    if draft.variable_rows.is_empty() && !draft.request.variables.is_empty() {
        draft.variable_rows = draft
            .request
            .variables
            .iter()
            .map(|(name, value)| Row::new(name, value))
            .collect();
    }
    if rows(ui, id, &mut draft.variable_rows) {
        draft.request.variables = variable_values(&draft.variable_rows);
        true
    } else {
        false
    }
}

fn variable_values(rows: &[Row]) -> Values {
    rows.iter()
        .filter(|row| !row.name.is_empty())
        .map(|row| (row.name.clone(), row.value.clone()))
        .collect()
}
/// Draws one of the request editors with the shared find bar above it, records whether the
/// caret is inside it, and reveals either the current match or a pending Go-to-line.
/// The shared state one of the request editors needs beyond its text.
struct Editing<'a> {
    find: &'a mut editor::Find,
    /// Set when the caret is inside this editor, for next frame's Ctrl+F routing.
    focused: &'a mut bool,
    /// A one-based line to reveal, from a failed assertion.
    goto: Option<u32>,
    syntax: editor::Syntax,
}
fn request_code(
    ui: &mut egui::Ui,
    id: &str,
    text: &mut String,
    rows: usize,
    editing: Editing<'_>,
) -> bool {
    let Editing {
        find,
        focused,
        goto,
        syntax,
    } = editing;
    let found = editor::hits(text, &find.query);
    let mut reveal = None;
    if find.open {
        ui.horizontal(|ui| {
            if editor::find_bar(ui, "editor-find", find, found.len()) {
                reveal = found.get(find.index).map(|hit| hit.chars.clone());
            }
            if ui.button("Close").clicked() {
                find.open = false;
            }
        });
    }
    if let Some(line) = goto {
        reveal = editor::line_range(text, line);
    }
    let result = editor::code(
        ui,
        id,
        text,
        rows,
        editor::Decoration {
            found: &found,
            current: find.index,
            reveal,
            syntax,
        },
    );
    *focused |= result.focused;
    result.changed
}
/// One sidebar line. Headers and requests share a uniform height so the list stays virtualized
/// by `show_rows`, which is what keeps a ten-thousand-request collection scrollable.
enum SidebarRow {
    Folder {
        name: String,
        count: usize,
        collapsed: bool,
    },
    Request(usize),
}
/// Character range of the page hit that starts at `byte`, for selecting a whole-body match once
/// its page has loaded. Falls back to a caret at that position when the hit is not on this page,
/// which happens when a page boundary splits the match.
fn char_range_at(text: &str, byte: usize, found: &[editor::Hit]) -> Option<std::ops::Range<usize>> {
    if let Some(hit) = found.iter().find(|hit| hit.bytes.start == byte) {
        return Some(hit.chars.clone());
    }
    let chars = text.char_indices().take_while(|(at, _)| *at < byte).count();
    Some(chars..chars)
}
/// The folder a request belongs to, with a name for those that declare none.
fn folder_of(request: &RequestDefinition) -> &str {
    let folder = request.folder.trim();
    if folder.is_empty() {
        "Ungrouped"
    } else {
        folder
    }
}
fn folder_picker(ui: &mut egui::Ui, id: egui::Id, folder: &mut String, folders: &[String]) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt(id)
            .selected_text(if folder.trim().is_empty() {
                "Ungrouped"
            } else {
                folder.as_str()
            })
            .show_ui(ui, |ui| {
                changed |= ui
                    .selectable_value(folder, String::new(), "Ungrouped")
                    .changed();
                for name in folders {
                    changed |= ui.selectable_value(folder, name.clone(), name).changed();
                }
            });
        let response = ui.text_edit_singleline(folder);
        changed |= response.changed();
        if response.lost_focus() {
            let trimmed = folder.trim().to_string();
            if *folder != trimmed {
                *folder = trimmed;
                changed = true;
            }
        }
    });
    changed
}

/// Minimum room for the selected request editor. Row-based tabs grow with their contents so the
/// request/response split cannot hide newly added parameters, headers, or authentication fields.
fn request_pane_height(tab: RequestTab, request: &RequestDefinition) -> f32 {
    const CHROME: f32 = 145.0;
    const GRID_ROW: f32 = 32.0;
    match tab {
        RequestTab::Params => {
            CHROME + 105.0 + GRID_ROW * (request.variables.len() + request.query.len()) as f32
        }
        RequestTab::Headers => CHROME + 80.0 + GRID_ROW * request.headers.len() as f32,
        RequestTab::Auth => {
            CHROME
                + if request.auth.bearer.is_some() || request.auth.api_key.is_some() {
                    155.0
                } else {
                    75.0
                }
        }
        _ => 330.0,
    }
}
impl Duckie {
    /// Groups the visible requests under their folders, in first-appearance order, which is the
    /// order the manifest stores and therefore the order the user controls.
    fn sidebar_rows(&self) -> Vec<SidebarRow> {
        let search = self.search.to_lowercase();
        let mut groups: Vec<(&str, Vec<usize>)> = vec![];
        let mut index: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for (i, d) in self.drafts.iter().enumerate() {
            let searchable = format!(
                "{} {} {} {}",
                d.request.name, d.request.method, d.request.url, d.request.folder
            )
            .to_lowercase();
            let matches = search
                .split_whitespace()
                .all(|word| searchable.contains(word));
            if !matches {
                continue;
            }
            let folder = folder_of(&d.request);
            match index.get(folder) {
                Some(at) => groups[*at].1.push(i),
                None => {
                    index.insert(folder, groups.len());
                    groups.push((folder, vec![i]));
                }
            }
        }
        let mut rows = vec![];
        for (name, items) in groups {
            // A search has to reach into folders the user left closed, or matches would vanish.
            let collapsed = search.is_empty() && self.prefs.collapsed.contains(name);
            rows.push(SidebarRow::Folder {
                name: name.to_owned(),
                count: items.len(),
                collapsed,
            });
            if !collapsed {
                rows.extend(items.into_iter().map(SidebarRow::Request));
            }
        }
        rows
    }
    /// Swaps a request with its neighbour inside the same folder, so reordering cannot silently
    /// move it somewhere else. Saving writes the new order to the manifest.
    fn reorder(&mut self, index: usize, up: bool) {
        let folder = folder_of(&self.drafts[index].request).to_owned();
        let neighbour = if up {
            (0..index)
                .rev()
                .find(|i| folder_of(&self.drafts[*i].request) == folder)
        } else {
            (index + 1..self.drafts.len()).find(|i| folder_of(&self.drafts[*i].request) == folder)
        };
        let Some(neighbour) = neighbour else { return };
        self.drafts.swap(index, neighbour);
        if self.selected == index {
            self.selected = neighbour;
        } else if self.selected == neighbour {
            self.selected = index;
        }
        // Order lives in the manifest, so the collection is now dirty even though no request is.
        self.env_dirty = true;
    }
    fn shortcuts(&mut self, ctx: &egui::Context) {
        // The unsaved-changes dialog handles its own shortcuts and continuation.
        if self.pending.is_some() {
            return;
        }
        use egui::{Key, KeyboardShortcut as Shortcut, Modifiers};
        let pressed =
            |modifiers, key| ctx.input_mut(|i| i.consume_shortcut(&Shortcut::new(modifiers, key)));
        if pressed(Modifiers::CTRL | Modifiers::SHIFT, Key::O) {
            self.request_action(Pending::Import);
        }
        if pressed(Modifiers::CTRL, Key::O) {
            self.request_action(Pending::Open);
        }
        if pressed(Modifiers::CTRL, Key::N) {
            self.new_request();
        }
        if pressed(Modifiers::CTRL, Key::S) {
            self.save_collection(false);
        }
        if pressed(Modifiers::CTRL | Modifiers::SHIFT, Key::Enter) {
            self.rerun();
        }
        if pressed(Modifiers::CTRL, Key::Enter) {
            self.send();
        }
        if pressed(Modifiers::CTRL, Key::L) {
            self.focus_url = true;
        }
        // Grab a token, hit Ctrl+T — the common case of getting a fresh bearer token into the
        // active request without switching to the Auth tab and pasting by hand.
        if pressed(Modifiers::CTRL, Key::T) {
            self.request_tab = RequestTab::Auth;
            // Request authentication is a single choice. Pasting the common bearer credential
            // must never leave an API key enabled as a second, hidden authentication scheme.
            if self.drafts[self.selected]
                .request
                .auth
                .api_key
                .take()
                .is_some()
            {
                self.touch();
            }
            if self.drafts[self.selected].request.auth.bearer.is_none() {
                self.drafts[self.selected].request.auth.bearer = Some(SecretBinding {
                    secret: "internalBearer".into(),
                });
                self.touch();
            }
            let secret_key = self.drafts[self.selected]
                .request
                .auth
                .bearer
                .as_ref()
                .unwrap()
                .secret
                .clone();
            match (self.clipboard_read)()
                .map(|t| t.trim().to_owned())
                .filter(|t| !t.is_empty())
            {
                Some(token) => {
                    let env = self.envs[self.env_index].name.clone();
                    self.secrets
                        .entry(env.clone())
                        .or_default()
                        .insert(secret_key.clone(), token);
                    self.status = format!("Pasted the clipboard into \"{secret_key}\" ({env}).");
                    self.touch();
                }
                // No usable text on the clipboard: fall back to focusing the field for a manual
                // paste instead of silently doing nothing.
                None => {
                    self.focus_token = true;
                    self.status = "Clipboard has no text to use as a bearer token.".into();
                }
            }
        }
        if pressed(Modifiers::CTRL, Key::B) {
            self.prefs.sidebar = !self.prefs.sidebar;
        }
        if pressed(Modifiers::CTRL, Key::K) {
            self.prefs.sidebar = true;
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("request-search")));
        }
        // Ctrl+F belongs to whichever editor holds the caret; the response preview is the default.
        if pressed(Modifiers::CTRL, Key::F) {
            // An already-open editor find keeps Ctrl+F, so pressing it twice does not jump away.
            if (self.editor_focused || self.editor_find.open)
                && matches!(self.request_tab, RequestTab::Body | RequestTab::Tests)
            {
                self.editor_find.open = true;
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("editor-find")));
            } else {
                self.response_tab = ResponseTab::Body;
                ctx.memory_mut(|m| m.request_focus(egui::Id::new("response-find")));
            }
        }
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            if self.pending.is_some() {
                self.pending = None;
            } else if self.env_dialog {
                self.env_dialog = false;
            } else if let Some(import) = self.import.take() {
                if let Some(token) = import.cancel {
                    token.cancel();
                }
            } else if let Some((_, token, ..)) = &self.active {
                token.cancel();
            } else if self.suite.running
                && let Some(token) = &self.suite.cancel
            {
                token.cancel();
            }
        }
    }
    pub fn top_menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New request                 Ctrl+N").clicked() {
                    self.new_request();
                    ui.close();
                }
                if ui.button("New collection…").clicked() {
                    self.request_action(Pending::NewCollection);
                    ui.close();
                }
                if ui.button("Open collection…          Ctrl+O").clicked() {
                    self.request_action(Pending::Open);
                    ui.close();
                }
                if ui.button("Import OpenAPI…    Ctrl+Shift+O").clicked() {
                    self.request_action(Pending::Import);
                    ui.close();
                }
                if ui.button("Paste cURL from clipboard").clicked() {
                    self.paste_curl();
                    ui.close();
                }
                if ui.button("Update from spec…").clicked() {
                    self.request_action(Pending::UpdateFromSpec);
                    ui.close();
                }
                ui.separator();
                if ui.button("Save collection              Ctrl+S").clicked() {
                    self.save_collection(false);
                    ui.close();
                }
                if ui.button("Save a copy to new folder…").clicked() {
                    self.save_collection(true);
                    ui.close();
                }
                if ui
                    .add_enabled(
                        self.collection.is_some(),
                        egui::Button::new("Reload from disk…"),
                    )
                    .clicked()
                {
                    self.request_action(Pending::Reload);
                    ui.close();
                }
            });
            ui.menu_button("Request", |ui| {
                if ui
                    .add_enabled(
                        self.active.is_none() && !self.suite.running,
                        egui::Button::new("Send                         Ctrl+Enter"),
                    )
                    .clicked()
                {
                    self.send();
                    ui.close();
                }
                if ui.button("Preview prepared request").clicked() {
                    self.refresh_request_preview();
                    ui.close();
                }
                if ui.button("Run requests…").clicked() {
                    self.suite.open = true;
                    ui.close();
                }
                if ui.button("Session history…").clicked() {
                    self.history_open = true;
                    if self.history_selected.is_none() && !self.history.is_empty() {
                        self.history_selected = Some(self.history.len() - 1);
                    }
                    ui.close();
                }
                if ui.button("Duplicate request").clicked() {
                    let id = self.drafts[self.selected].request.id.clone();
                    self.duplicate_request(&id);
                    ui.close();
                }
                ui.separator();
                ui.menu_button("Copy as cURL", |ui| {
                    if ui.button("POSIX (redacted)").clicked() {
                        self.copy_curl(duckie_model::curl::Shell::Posix, false);
                        ui.close();
                    }
                    if ui.button("PowerShell (redacted)").clicked() {
                        self.copy_curl(duckie_model::curl::Shell::PowerShell, false);
                        ui.close();
                    }
                    ui.separator();
                    ui.weak("The commands below may expose credentials.");
                    if ui.button("POSIX with credentials").clicked() {
                        self.copy_curl(duckie_model::curl::Shell::Posix, true);
                        ui.close();
                    }
                    if ui.button("PowerShell with credentials").clicked() {
                        self.copy_curl(duckie_model::curl::Shell::PowerShell, true);
                        ui.close();
                    }
                });
                if ui.button("Close response").clicked() {
                    self.responses
                        .remove(&self.drafts[self.selected].request.id);
                    ui.close();
                }
                if ui.button("Delete request…").clicked() {
                    self.delete = Some(self.drafts[self.selected].request.id.clone());
                    ui.close();
                }
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut self.prefs.sidebar, "Request sidebar     Ctrl+B");
                ui.separator();
                let old = self.prefs.appearance;
                ui.selectable_value(&mut self.prefs.appearance, 0, "System appearance");
                ui.selectable_value(&mut self.prefs.appearance, 1, "Light");
                ui.selectable_value(&mut self.prefs.appearance, 2, "Dark");
                if old != self.prefs.appearance {
                    self.apply_theme();
                }
            });
            if ui.button("Help").clicked() {
                self.about = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("Edit…")
                    .on_hover_text("Edit local environments and secrets")
                    .clicked()
                {
                    self.env_dialog = true;
                }
                egui::ComboBox::from_id_salt("environment")
                    .selected_text(&self.envs[self.env_index].name)
                    .width(150.0)
                    .show_ui(ui, |ui| {
                        for (i, env) in self.envs.iter().enumerate() {
                            ui.selectable_value(&mut self.env_index, i, &env.name);
                        }
                    });
                ui.weak("Environment");
            });
        });
    }
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let search_id = egui::Id::new("request-search");
        let focused = ui.memory(|m| m.has_focus(search_id));
        let mut direction = 0isize;
        let mut open = false;
        if focused {
            ui.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                    direction = 1;
                }
                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                    direction = -1;
                }
                open = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                    self.search.clear();
                    self.search_selected = None;
                }
            });
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("DUCKIE").strong().size(18.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("+")
                    .on_hover_text("New request · Ctrl+N")
                    .clicked()
                {
                    self.new_request();
                }
            });
        });
        ui.add_space(4.0);
        ui.weak(
            self.collection
                .as_ref()
                .map(|c| c.manifest.name.as_str())
                .unwrap_or("Scratch workspace"),
        );
        let search_field = ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .id(egui::Id::new("request-search"))
                .event_filter(egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                    ..Default::default()
                })
                .hint_text("Find requests…   Ctrl+K")
                .desired_width(f32::INFINITY),
        );
        ui.add_space(4.0);
        let rows = self.sidebar_rows();
        let matches: Vec<usize> = rows
            .iter()
            .filter_map(|r| match r {
                SidebarRow::Request(i) => Some(*i),
                _ => None,
            })
            .collect();
        if search_field.changed() || !matches.contains(&self.search_selected.unwrap_or(usize::MAX))
        {
            self.search_selected = matches.first().copied();
        }
        if direction != 0 && !matches.is_empty() {
            let at = matches
                .iter()
                .position(|i| Some(*i) == self.search_selected)
                .unwrap_or(0);
            self.search_selected = Some(
                matches[(at as isize + direction).clamp(0, matches.len() as isize - 1) as usize],
            );
        }
        if focused && direction != 0 {
            search_field.request_focus();
        }
        if open && let Some(index) = self.search_selected {
            self.selected = index;
            self.focus_url = true;
            self.clock += 1;
            if let Some(view) = self.responses.get_mut(&self.drafts[index].request.id) {
                view.viewed = self.clock;
            }
        }

        let mut toggle = None;
        let mut move_by = None;
        let mut duplicate = None;
        let mut copy = None;
        let mut move_folder = None;
        let folders: Vec<_> = self
            .drafts
            .iter()
            .map(|d| d.request.folder.trim().to_string())
            .filter(|f| !f.is_empty())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut scroll = egui::ScrollArea::vertical().id_salt("requests");
        if focused
            && (direction != 0 || search_field.changed())
            && let Some(at) = rows.iter().position(
                |r| matches!(r, SidebarRow::Request(i) if Some(*i) == self.search_selected),
            )
        {
            scroll = scroll.vertical_scroll_offset(at as f32 * 38.0);
        }
        scroll
            .max_height((ui.available_height() - 125.0).max(100.0))
            .show_rows(ui, 38.0, rows.len(), |ui, range| {
                for row in &rows[range] {
                    match row {
                        SidebarRow::Folder {
                            name,
                            count,
                            collapsed,
                        } => {
                            let label = format!(
                                "{}  {name}  ({count})",
                                if *collapsed { "▶" } else { "▼" }
                            );
                            if ui
                                .add_sized(
                                    [ui.available_width(), 32.0],
                                    egui::Button::new("")
                                        .left_text(RichText::new(label).size(13.0).strong())
                                        .frame(false),
                                )
                                .on_hover_text("Show or hide this folder")
                                .clicked()
                            {
                                toggle = Some(name.clone());
                            }
                        }
                        SidebarRow::Request(i) => {
                            let i = *i;
                            let d = &self.drafts[i];
                            let text = format!(
                                "    {:6} {}{}",
                                d.request.method,
                                d.request.name,
                                if d.dirty { " *" } else { "" }
                            );
                            let response = ui
                                .add_sized(
                                    [ui.available_width(), 32.0],
                                    egui::Button::selectable(
                                        if focused {
                                            self.search_selected == Some(i)
                                        } else {
                                            self.selected == i
                                        },
                                        "",
                                    )
                                    .left_text(RichText::new(text).size(13.0)),
                                )
                                .on_hover_text(format!(
                                    "{}\n{}",
                                    d.request.folder,
                                    d.request.address()
                                ));
                            response.context_menu(|ui| {
                                if ui.button("Duplicate request").clicked() {
                                    duplicate = Some(d.request.id.clone());
                                    ui.close();
                                }
                                if ui.button("Delete request…").clicked() {
                                    self.delete = Some(d.request.id.clone());
                                    ui.close();
                                }
                                ui.menu_button("Move to folder", |ui| {
                                    let edit_id = egui::Id::new(("move-folder", &d.request.id));
                                    let mut folder = ui
                                        .data(|data| data.get_temp::<String>(edit_id))
                                        .unwrap_or_else(|| d.request.folder.clone());
                                    folder_picker(ui, edit_id, &mut folder, &folders);
                                    ui.data_mut(|data| data.insert_temp(edit_id, folder.clone()));
                                    if ui.button("Move").clicked() {
                                        move_folder = Some((d.request.id.clone(), folder));
                                        ui.close();
                                    }
                                });
                                ui.menu_button("Copy as cURL (redacted)", |ui| {
                                    for (label, shell) in [
                                        ("POSIX", duckie_model::curl::Shell::Posix),
                                        ("PowerShell", duckie_model::curl::Shell::PowerShell),
                                    ] {
                                        if ui.button(label).clicked() {
                                            copy = Some((d.request.id.clone(), shell));
                                            ui.close();
                                        }
                                    }
                                });
                                ui.separator();
                                if ui.button("Move up").clicked() {
                                    move_by = Some((i, true));
                                    ui.close();
                                }
                                if ui.button("Move down").clicked() {
                                    move_by = Some((i, false));
                                    ui.close();
                                }
                                ui.weak("Order is saved with the collection.");
                            });
                            if response.clicked() {
                                self.selected = i;
                                self.clock += 1;
                                if let Some(v) = self.responses.get_mut(&self.drafts[i].request.id)
                                {
                                    v.viewed = self.clock;
                                }
                            }
                        }
                    }
                }
            });
        if let Some(name) = toggle
            && !self.prefs.collapsed.remove(&name)
        {
            self.prefs.collapsed.insert(name);
        }
        if let Some((index, up)) = move_by {
            self.reorder(index, up);
        }
        if let Some(id) = duplicate {
            self.duplicate_request(&id);
        }
        if let Some((id, shell)) = copy {
            self.copy_curl_for(&id, shell, false);
        }
        if let Some((id, folder)) = move_folder {
            self.move_request_to_folder(&id, &folder);
        }
        ui.separator();
        if ui.button("Open collection…").clicked() {
            self.request_action(Pending::Open);
        }
        if ui.button("New collection…").clicked() {
            self.request_action(Pending::NewCollection);
        }
        if ui.button("Import OpenAPI…    Ctrl+Shift+O").clicked() {
            self.request_action(Pending::Import);
        }
        if focused && direction != 0 {
            ui.memory_mut(|m| m.request_focus(search_id));
        }
    }
    fn request_header(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        ui.horizontal(|ui| {
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut self.drafts[self.selected].request.name)
                        .font(egui::FontId::proportional(18.0))
                        .frame(egui::Frame::NONE)
                        .desired_width((ui.available_width() - 105.0).max(100.0)),
                )
                .changed();
            if self.drafts[self.selected].dirty {
                ui.weak("*");
            }
            if ui
                .button("Save")
                .on_hover_text("Save collection · Ctrl+S")
                .clicked()
            {
                self.save_collection(false);
            }
        });
        ui.add_space(4.0);
        let mut send = false;
        let mut cancel = false;
        let mut completion = None;
        ui.horizontal(|ui| {
            let d = &mut self.drafts[self.selected];
            egui::ComboBox::from_id_salt("method")
                .selected_text(RichText::new(&d.request.method).color(accent(ui)).strong())
                .width(90.0)
                .show_ui(ui, |ui| {
                    for method in [
                        "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "TRACE",
                        "QUERY",
                    ] {
                        changed |= ui
                            .selectable_value(&mut d.request.method, method.into(), method)
                            .changed();
                    }
                    ui.separator();
                    ui.label("Custom method");
                    changed |= ui.text_edit_singleline(&mut d.request.method).changed();
                });
            let mut address = d.request.address();
            let url_id = ui.make_persistent_id("url");
            let cursor_id = url_id.with("completion-cursor");
            let dismiss_id = url_id.with("completion-dismissed");
            let selection_id = url_id.with("completion-selection");
            let caret = ui
                .data(|data| data.get_temp::<usize>(cursor_id))
                .unwrap_or(address.chars().count());
            let byte = address
                .char_indices()
                .nth(caret)
                .map_or(address.len(), |(i, _)| i);
            let start = address[..byte]
                .rfind("{{")
                .filter(|start| !address[start + 2..byte].contains(['{', '}', ' ', '\n']));
            let mut inserted_caret = None;
            let mut names = Vec::new();
            if let Some(start) = start {
                let prefix = &address[start + 2..byte];
                names.extend(
                    self.envs[self.env_index]
                        .values
                        .keys()
                        .map(|name| format!("env.{name}")),
                );
                names.extend(
                    d.request
                        .variables
                        .keys()
                        .map(|name| format!("request.{name}")),
                );
                if let Some(secrets) = self.secrets.get(&self.envs[self.env_index].name) {
                    names.extend(secrets.keys().map(|name| format!("secret.{name}")));
                }
                names.retain(|name| name.starts_with(prefix));
                names.sort();
                names.dedup();
            }
            let dismissed = ui
                .data(|data| data.get_temp::<String>(dismiss_id))
                .as_deref()
                == Some(&address);
            let focused = ui.memory(|m| m.has_focus(url_id));
            let mut selected = ui
                .data(|data| data.get_temp::<usize>(selection_id))
                .unwrap_or(0)
                .min(names.len().saturating_sub(1));
            if focused && !dismissed && !names.is_empty() {
                let mut insert = false;
                let mut dismiss = false;
                ui.input_mut(|input| {
                    if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                        selected = (selected + 1).min(names.len() - 1);
                    }
                    if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                        selected = selected.saturating_sub(1);
                    }
                    insert = input.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
                    dismiss = input.consume_key(egui::Modifiers::NONE, egui::Key::Escape);
                });
                if dismiss {
                    ui.data_mut(|data| data.insert_temp(dismiss_id, address.clone()));
                } else if insert {
                    let start = start.unwrap();
                    let end = if address[byte..].starts_with("}}") {
                        byte + 2
                    } else {
                        byte
                    };
                    address.replace_range(start..end, &format!("{{{{{}}}}}", names[selected]));
                    inserted_caret = Some(
                        address[..start].chars().count() + names[selected].chars().count() + 4,
                    );
                    d.request.set_address(&address);
                    changed = true;
                    ui.data_mut(|data| data.insert_temp(dismiss_id, address.clone()));
                    self.focus_url = true;
                } else {
                    completion = Some((start.unwrap(), byte, names.clone(), selected));
                }
            }
            ui.data_mut(|data| data.insert_temp(selection_id, selected));
            let mut output = egui::TextEdit::singleline(&mut address)
                .id(url_id)
                .event_filter(egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                    ..Default::default()
                })
                .font(egui::TextStyle::Monospace)
                .hint_text("Enter a URL, or use {{env.baseUrl}}/path")
                .desired_width((ui.available_width() - 110.0).max(100.0))
                .show(ui);
            if let Some(caret) = inserted_caret {
                let cursor = egui::text::CCursorRange::one(egui::text::CCursor::new(caret));
                output.state.cursor.set_char_range(Some(cursor));
                output.state.clone().store(ui.ctx(), url_id);
                output.cursor_range = Some(cursor);
            }
            if let Some(cursor) = output.cursor_range {
                ui.data_mut(|data| data.insert_temp(cursor_id, cursor.primary.index));
            }
            let field = output.response.response;
            if field.changed() {
                d.request.set_address(&address);
                changed = true;
            }
            if self.focus_url {
                field.request_focus();
                self.focus_url = false;
            }
            if let Some((id, ..)) = &self.active {
                if id == &d.request.id {
                    cancel = ui
                        .add_sized(
                            [95.0, 32.0],
                            egui::Button::new(if self.active_testing {
                                "Stop tests"
                            } else {
                                "Cancel"
                            }),
                        )
                        .clicked();
                } else {
                    ui.add_enabled(false, egui::Button::new("Send"));
                }
            } else {
                send = ui
                    .add_sized(
                        [95.0, 32.0],
                        egui::Button::new(RichText::new("Send").strong().color(
                            if ui.visuals().dark_mode {
                                Color32::from_rgb(17, 30, 49)
                            } else {
                                Color32::WHITE
                            },
                        ))
                        .fill(accent(ui)),
                    )
                    .on_hover_text("Send once · Ctrl+Enter")
                    .clicked();
            }
        });
        if let Some((start, byte, names, selected)) = completion {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.weak("Variable names · ↑/↓ select · Enter inserts · Escape closes");
                egui::ScrollArea::vertical()
                    .max_height(120.0)
                    .show(ui, |ui| {
                        for (i, name) in names.iter().enumerate() {
                            if ui.selectable_label(i == selected, name).clicked() {
                                let d = &mut self.drafts[self.selected];
                                let mut address = d.request.address();
                                if address.is_char_boundary(start)
                                    && address.is_char_boundary(byte)
                                    && byte <= address.len()
                                {
                                    let end = if address[byte..].starts_with("}}") {
                                        byte + 2
                                    } else {
                                        byte
                                    };
                                    address.replace_range(start..end, &format!("{{{{{name}}}}}"));
                                    d.request.set_address(&address);
                                    changed = true;
                                    self.focus_url = true;
                                }
                            }
                        }
                    });
            });
        }
        if changed {
            self.touch();
        }
        if send {
            self.send();
        }
        if cancel && let Some((_, token, ..)) = &self.active {
            token.cancel();
        }
        if !self.drafts[self.selected].error.is_empty() {
            ui.colored_label(
                ui.visuals().error_fg_color,
                &self.drafts[self.selected].error,
            );
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (tab, label) in [
                (RequestTab::Params, "Params"),
                (RequestTab::Headers, "Headers"),
                (RequestTab::Auth, "Auth"),
                (RequestTab::Body, "Body"),
                (RequestTab::Tests, "Tests"),
                (RequestTab::Settings, "Settings"),
            ] {
                ui.selectable_value(&mut self.request_tab, tab, label);
            }
        });
        ui.separator();
    }
    fn editor(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        let mut run_tests = false;
        // Recomputed below by whichever code editor this tab draws, for next frame's Ctrl+F.
        self.editor_focused = false;
        match self.request_tab {
            RequestTab::Params => {
                ui.label(RichText::new("Query parameters").strong());
                changed |= rows(ui, "query", &mut self.drafts[self.selected].request.query);
                ui.add_space(8.0);
                ui.separator();
                ui.label(RichText::new("Template variables").strong());
                ui.weak(
                    "Used only where referenced as {{request.name}} in the URL, headers, or body.",
                );
                changed |= request_variables(ui, "request-vars", &mut self.drafts[self.selected]);
            }
            RequestTab::Headers => {
                changed |= rows(
                    ui,
                    "headers",
                    &mut self.drafts[self.selected].request.headers,
                );
                ui.add_space(8.0);
                ui.weak("Authentication and body headers are added when you send. Duplicate manual auth headers block Send.");
            }
            RequestTab::Auth => {
                let focus_token = self.focus_token;
                self.focus_token = false;
                let d = &mut self.drafts[self.selected];
                // Bearer wins when opening a legacy request that enabled both schemes. This also
                // repairs the draft so saving cannot preserve an impossible UI state.
                if d.request.auth.bearer.is_some() && d.request.auth.api_key.take().is_some() {
                    changed = true;
                }
                let mut mode = if d.request.auth.bearer.is_some() {
                    1
                } else if d.request.auth.api_key.is_some() {
                    2
                } else {
                    0
                };
                let old_mode = mode;
                ui.horizontal(|ui| {
                    ui.label("Authentication");
                    ui.radio_value(&mut mode, 0, "None");
                    ui.radio_value(&mut mode, 1, "Bearer token");
                    ui.radio_value(&mut mode, 2, "API key header");
                });
                if mode != old_mode {
                    d.request.auth = match mode {
                        1 => Auth {
                            bearer: Some(SecretBinding {
                                secret: "internalBearer".into(),
                            }),
                            api_key: None,
                        },
                        2 => Auth {
                            bearer: None,
                            api_key: Some(ApiKey {
                                header: "X-API-Key".into(),
                                secret: "apiKey".into(),
                            }),
                        },
                        _ => Auth::default(),
                    };
                    changed = true;
                }
                let env = self.envs[self.env_index].name.clone();
                if let Some(b) = &mut d.request.auth.bearer {
                    ui.add_space(8.0);
                    ui.strong("Bearer token");
                    ui.horizontal(|ui| {
                        ui.label("Secret key");
                        changed |= ui.text_edit_singleline(&mut b.secret).changed();
                    });
                    changed |= secret_input(
                        ui,
                        &env,
                        &b.secret,
                        &mut self.secrets,
                        &mut self.remember,
                        &mut self.reveal,
                        focus_token,
                    );
                }
                if let Some(k) = &mut d.request.auth.api_key {
                    ui.add_space(8.0);
                    ui.strong("API key");
                    ui.horizontal(|ui| {
                        ui.label("Header name");
                        changed |= ui.text_edit_singleline(&mut k.header).changed();
                        ui.label("Secret key");
                        changed |= ui.text_edit_singleline(&mut k.secret).changed();
                    });
                    changed |= secret_input(
                        ui,
                        &env,
                        &k.secret,
                        &mut self.secrets,
                        &mut self.remember,
                        &mut self.reveal,
                        false,
                    );
                }
                ui.add_space(8.0);
                ui.weak("Paste a token with or without the Bearer prefix. Values stay in this session unless remembered.");
                if self.collection.is_none() {
                    ui.weak("Save chooses a collection folder before remembered credentials can be written.");
                }
            }
            RequestTab::Body => {
                let d = &mut self.drafts[self.selected];
                let mut mode = match d.request.body {
                    Body::None => 0,
                    Body::Json { .. } => 1,
                    Body::Text { .. } => 2,
                    Body::Form { .. } => 3,
                    Body::Multipart { .. } => 4,
                    Body::File { .. } => 5,
                };
                let old = mode;
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("body-mode")
                        .selected_text(
                            [
                                "None",
                                "JSON",
                                "Text",
                                "Form URL-encoded",
                                "Multipart",
                                "File",
                            ][mode],
                        )
                        .show_ui(ui, |ui| {
                            for (i, label) in [
                                "None",
                                "JSON",
                                "Text",
                                "Form URL-encoded",
                                "Multipart",
                                "File",
                            ]
                            .iter()
                            .enumerate()
                            {
                                ui.selectable_value(&mut mode, i, *label);
                            }
                        });
                    ui.weak("Body bytes change only when you edit or format.");
                });
                if mode != old {
                    let text = match &d.request.body {
                        Body::Text { text } | Body::Json { text } => text.clone(),
                        _ => String::new(),
                    };
                    d.request.body = match mode {
                        1 => Body::Json { text },
                        2 => Body::Text { text },
                        3 => Body::Form { rows: vec![] },
                        4 => Body::Multipart { parts: vec![] },
                        5 => Body::File {
                            path: String::new(),
                            content_type: "application/octet-stream".into(),
                        },
                        _ => Body::None,
                    };
                    changed = true;
                }
                let is_json = matches!(d.request.body, Body::Json { .. });
                match &mut d.request.body {
                    Body::None => {
                        ui.add_space(25.0);
                        ui.weak("This request has no body.");
                    }
                    Body::Json { text } | Body::Text { text } => {
                        if is_json && ui.button("Format JSON").clicked() {
                            match serde_json::from_str::<serde_json::Value>(text) {
                                Ok(v) => {
                                    *text = serde_json::to_string_pretty(&v).unwrap();
                                    changed = true;
                                }
                                Err(e) => d.error = format!("JSON: {e}"),
                            }
                        }
                        changed |= request_code(
                            ui,
                            "body-code",
                            text,
                            9,
                            Editing {
                                find: &mut self.editor_find,
                                focused: &mut self.editor_focused,
                                goto: None,
                                syntax: if is_json {
                                    editor::Syntax::Json
                                } else {
                                    editor::Syntax::None
                                },
                            },
                        );
                        ui.weak("{{variables}} use literal substitution; inserted values are not automatically JSON-escaped.");
                    }
                    Body::Form { rows: fields } => changed |= rows(ui, "form", fields),
                    Body::Multipart { parts } => {
                        let mut remove = None;
                        for (i, p) in parts.iter_mut().enumerate() {
                            ui.push_id(i, |ui| {
                                ui.horizontal(|ui| {
                                    changed |= ui.checkbox(&mut p.enabled, "").changed();
                                    changed |= ui
                                        .add(
                                            egui::TextEdit::singleline(&mut p.name)
                                                .desired_width(130.0)
                                                .hint_text("Name"),
                                        )
                                        .changed();
                                    changed |= ui.checkbox(&mut p.file, "File").changed();
                                    changed |= ui
                                        .add(
                                            egui::TextEdit::singleline(&mut p.value)
                                                .desired_width(280.0)
                                                .hint_text("Value or selected file"),
                                        )
                                        .changed();
                                    if p.file
                                        && ui.button("Browse…").clicked()
                                        && let Some(path) = rfd::FileDialog::new().pick_file()
                                    {
                                        p.value = path.to_string_lossy().into_owned();
                                        changed = true;
                                    }
                                    if ui.small_button("×").clicked() {
                                        remove = Some(i);
                                    }
                                });
                            });
                        }
                        if let Some(i) = remove {
                            parts.remove(i);
                            changed = true;
                        }
                        if ui.button("+ Add part").clicked() {
                            parts.push(Part {
                                enabled: true,
                                name: String::new(),
                                value: String::new(),
                                file: false,
                            });
                            changed = true;
                        }
                    }
                    Body::File { path, content_type } => {
                        ui.horizontal(|ui| {
                            ui.label("Content type");
                            changed |= ui.text_edit_singleline(content_type).changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label(if path.is_empty() {
                                "No file selected"
                            } else {
                                path.as_str()
                            });
                            if ui.button("Choose file…").clicked()
                                && let Some(p) = rfd::FileDialog::new().pick_file()
                            {
                                *path = p.to_string_lossy().into_owned();
                                changed = true;
                            }
                        });
                        ui.weak("Streams from disk. An absolute file path is not portable with the collection.");
                    }
                }
            }
            RequestTab::Tests => {
                let ready = self
                    .responses
                    .get(&self.drafts[self.selected].request.id)
                    .is_some_and(|v| v.result.outcome == Outcome::Complete)
                    && self.active.is_none();
                let goto = self.goto_line.take();
                let d = &mut self.drafts[self.selected];
                ui.horizontal(|ui| {
                    changed |= ui
                        .checkbox(&mut d.request.tests.enabled, "Run after Send")
                        .changed();
                    if ui.button("Insert example").clicked() {
                        d.source.push_str(EXAMPLE);
                        changed = true;
                    }
                    if let Some(view)=self.responses.get(&d.request.id) {
                        if let Some(status)=view.result.status && ui.button(format!("Expect observed status {status}")).clicked(){d.source.push_str(&format!("\ntest(\"returns status {status}\", () => {{\n  expect(response.status).toBe({status});\n}});\n"));changed=true;}
                        if ui.button("Expect duration under 1000 ms").clicked(){d.source.push_str("\ntest(\"responds within the chosen limit\", () => {\n  expect(response.durationMs).toBeLessThan(1000); // choose the contract limit\n});\n");changed=true;}
                    }
                    run_tests = ui
                        .add_enabled(ready, egui::Button::new("Run tests"))
                        .on_hover_text("Does not send an HTTP request · Ctrl+Shift+Enter")
                        .clicked();
                });
                if d.source.is_empty() {
                    ui.weak("Assert a response with test(), expect(), and response. JavaScript runs only when you ask.");
                }
                changed |= request_code(
                    ui,
                    "test-code",
                    &mut d.source,
                    10,
                    Editing {
                        find: &mut self.editor_find,
                        focused: &mut self.editor_focused,
                        goto,
                        syntax: editor::Syntax::JavaScript,
                    },
                );
                ui.weak("JavaScript · 2 second limit · No network, filesystem, or Node.js API");
            }
            RequestTab::Settings => {
                let d = &mut self.drafts[self.selected];
                egui::Grid::new("settings").num_columns(2).show(ui, |ui| {
                    ui.label("Folder / group");
                    changed |= ui.text_edit_singleline(&mut d.request.folder).changed();
                    ui.end_row();
                    ui.label("Timeout (ms)");
                    changed |= ui
                        .add(egui::DragValue::new(&mut d.request.timeout_ms).range(1..=3_600_000))
                        .changed();
                    ui.end_row();
                    ui.label("Encoded response cap (MiB)");
                    let mut limit = d.request.encoded_limit / MIB;
                    if ui
                        .add(egui::DragValue::new(&mut limit).range(1..=1024))
                        .changed()
                    {
                        d.request.encoded_limit = limit * MIB;
                        changed = true;
                    }
                    ui.end_row();
                    ui.label("Decoded response cap (MiB)");
                    let mut limit = d.request.decoded_limit / MIB;
                    if ui
                        .add(egui::DragValue::new(&mut limit).range(1..=1024))
                        .changed()
                    {
                        d.request.decoded_limit = limit * MIB;
                        changed = true;
                    }
                    ui.end_row();
                });
                let old = match d.request.proxy {
                    ProxyMode::System => 0,
                    ProxyMode::Direct => 1,
                    ProxyMode::Explicit { .. } => 2,
                };
                let mut mode = old;
                ui.horizontal(|ui| {
                    ui.label("Proxy");
                    for (i, name) in ["System", "Direct", "Explicit"].iter().enumerate() {
                        ui.selectable_value(&mut mode, i, *name);
                    }
                });
                if old != mode {
                    d.request.proxy = match mode {
                        1 => ProxyMode::Direct,
                        2 => ProxyMode::Explicit {
                            url: String::new(),
                            bypass: String::new(),
                        },
                        _ => ProxyMode::System,
                    };
                    changed = true;
                }
                if let ProxyMode::Explicit { url, bypass } = &mut d.request.proxy {
                    ui.horizontal(|ui| {
                        ui.label("Proxy URL");
                        changed |= ui.text_edit_singleline(url).changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label("Bypass hosts");
                        changed |= ui.text_edit_singleline(bypass).changed();
                    });
                }
                ui.weak("Windows certificate trust · TLS verification on · No automatic redirects or retries");
                if !d.request.blockers.is_empty() {
                    ui.separator();
                    ui.colored_label(warning(ui), "Import issues block Send");
                    for b in &d.request.blockers {
                        ui.label(b);
                    }
                    if ui
                        .button("I corrected these fields — clear import issues")
                        .clicked()
                    {
                        d.request.blockers.clear();
                        changed = true;
                    }
                }
            }
        }
        if changed {
            self.touch();
        }
        if run_tests {
            self.rerun();
        }
    }
    fn response(&mut self, ui: &mut egui::Ui) {
        let id = self.drafts[self.selected].request.id.clone();
        let compare_candidates: Vec<_> = self
            .responses
            .iter()
            .filter(|(other, _)| *other != &id)
            .map(|(other, v)| {
                (
                    other.clone(),
                    format!("{} · {}", v.result.summary.method, v.result.summary.url),
                    v.result.clone(),
                )
            })
            .collect();
        let Some(view) = self.responses.get_mut(&id) else {
            ui.label(RichText::new("Response").strong());
            ui.separator();
            ui.add_space((ui.available_height() * 0.25).max(16.0));
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Send a request to see its response").size(19.0));
                ui.add_space(8.0);
                ui.weak("Enter an endpoint above, then press Ctrl+Enter.");
            });
            return;
        };
        ui.horizontal_wrapped(|ui| {
            ui.strong("Response");
            if let Some(status) = view.result.status {
                let color = if (200..300).contains(&status) {
                    success(ui)
                } else {
                    warning(ui)
                };
                ui.label(
                    RichText::new(format!("{status} {}", view.result.status_text))
                        .color(color)
                        .strong(),
                );
            }
            ui.weak(format!("{} ms", view.result.duration_ms));
            ui.weak(format!("{} decoded", size(view.result.body.len())));
            if let Some(report) = &view.report {
                let passed = report.tests.iter().filter(|t| t.passed).count();
                ui.label(format!(
                    "Tests: {passed} passed, {} failed",
                    report.tests.len() - passed
                ));
                if report.suite_error.is_some() {
                    ui.colored_label(ui.visuals().error_fg_color, "Suite error");
                }
            }
        });
        if view.result.outcome != Outcome::Complete {
            ui.colored_label(warning(ui), view.result.outcome.to_string());
            if let Some(detail) = &view.result.body_error {
                ui.weak(detail);
            }
            ui.weak("Automatic tests skipped for an incomplete response.");
        }
        let current = &self.drafts[self.selected];
        if view.result.summary.revision != current.revision {
            ui.colored_label(warning(ui), "Response from previous draft");
        }
        if view.result.summary.environment != self.envs[self.env_index].name {
            ui.colored_label(
                warning(ui),
                format!("Response from {}", view.result.summary.environment),
            );
        }
        if self.active.as_ref().is_some_and(|(r, ..)| r == &id) {
            ui.weak(if self.active_testing {
                "Tests running…"
            } else {
                "Previous response — sending a new request…"
            });
        }
        if matches!(view.result.status, Some(401 | 403))
            && current.request.auth.bearer.is_some()
            && ui.button("Replace token").clicked()
        {
            self.request_tab = RequestTab::Auth;
        }
        ui.horizontal(|ui| {
            for (tab, label) in [
                (ResponseTab::Body, "Body"),
                (ResponseTab::Json, "JSON tree"),
                (ResponseTab::Compare, "Compare"),
                (ResponseTab::Headers, "Headers"),
                (ResponseTab::Tests, "Test results"),
                (ResponseTab::Details, "Request details"),
            ] {
                ui.selectable_value(&mut self.response_tab, tab, label);
            }
        });
        ui.separator();
        let mut page = None;
        let mut save = None;
        let mut location = None;
        let mut start_search = None;
        let mut go_to = None;
        let mut assertion_snippet = None;
        match self.response_tab {
            ResponseTab::Body => {
                let shown = if self.pretty {
                    view.pretty.as_ref().unwrap_or(&view.preview)
                } else {
                    &view.preview
                };
                let found = editor::hits(shown, &self.response_find.query);
                let mut reveal = None;
                // Raw text is the body byte-for-byte, so whole-body offsets map onto the page.
                // The pretty-printed view is a different string, so find stays page-local there.
                let whole_body = !self.pretty || view.pretty.is_none();
                ui.horizontal(|ui| {
                    ui.add_enabled_ui(view.pretty.is_some(), |ui| {
                        ui.selectable_value(&mut self.pretty, true, "Pretty");
                        ui.selectable_value(&mut self.pretty, false, "Raw");
                    });
                    let total = if whole_body {
                        view.search.offsets.len()
                    } else {
                        found.len()
                    };
                    if editor::find_bar(ui, "response-find", &mut self.response_find, total) {
                        if whole_body {
                            go_to = view.search.offsets.get(self.response_find.index).copied();
                        } else {
                            reveal = found
                                .get(self.response_find.index)
                                .map(|hit| hit.chars.clone());
                        }
                    }
                    if shown.len() > editor::MAX_COLOURED {
                        ui.weak("Plain above 64 KiB");
                    }
                    if !self.response_find.query.is_empty() {
                        if !whole_body && view.result.body.len() > view.page {
                            // The pretty view cannot be mapped back to body offsets, so its count
                            // covers the page alone; say so rather than implying otherwise.
                            ui.weak("on this page");
                        } else if whole_body && view.search.query != self.response_find.query {
                            start_search = Some(self.response_find.query.clone());
                            ui.weak("Searching…");
                        } else if view.search.running {
                            ui.weak("Searching…");
                        } else if view.search.capped {
                            ui.weak("first matches only");
                        }
                    }
                    let paged = view.offset > 0 || view.result.body.len() > view.page;
                    let mode = if self.pretty && view.pretty.is_some() {
                        "Pretty"
                    } else {
                        "Raw"
                    };
                    if ui
                        .button(if paged { "Copy page" } else { "Copy body" })
                        .on_hover_text(format!(
                            "Copy displayed {mode} text. Save body exports the retained body."
                        ))
                        .clicked()
                    {
                        ui.ctx().copy_text(
                            if self.pretty {
                                view.pretty.as_ref().unwrap_or(&view.preview)
                            } else {
                                &view.preview
                            }
                            .clone(),
                        );
                    }
                    if ui
                        .button(if view.result.outcome == Outcome::Complete {
                            "Save body…"
                        } else {
                            "Save partial body…"
                        })
                        .clicked()
                    {
                        save = Some(view.result.body.clone());
                    }
                });
                // Page size is chosen per response, so paging must follow it rather than
                // assume the maximum; see `page_size`.
                let step = view.page.max(1);
                if view.result.body.len() > step {
                    ui.horizontal_wrapped(|ui| {
                        ui.weak(format!(
                            "Showing bytes {}–{} of {}",
                            view.offset,
                            (view.offset + step).min(view.result.body.len()),
                            view.result.body.len()
                        ));
                        if ui
                            .add_enabled(view.offset > 0, egui::Button::new("Previous page"))
                            .clicked()
                        {
                            page = Some(view.offset.saturating_sub(step));
                        }
                        if ui
                            .add_enabled(
                                view.offset + step < view.result.body.len(),
                                egui::Button::new("Next page"),
                            )
                            .clicked()
                        {
                            page = Some(view.offset + step);
                        }
                    });
                }
                if view.binary {
                    ui.add_space(20.0);
                    ui.label("Binary response");
                    if let Some(charset) = &view.charset {
                        ui.weak(format!(
                            "The declared charset {charset:?} is not supported."
                        ));
                    } else {
                        ui.weak("The bytes are not valid UTF-8 or contain NUL characters.");
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Preview as UTF-8").clicked() {
                            view.charset_override = Some("utf-8".into());
                            page = Some(view.offset);
                        }
                        if ui.button("Preview as Windows-1252").clicked() {
                            view.charset_override = Some("windows-1252".into());
                            page = Some(view.offset);
                        }
                    });
                    ui.weak(
                        "Replacement characters may be shown; saving keeps the original bytes.",
                    );
                } else {
                    ui.horizontal_wrapped(|ui| {
                        if let Some(charset) = &view.charset {
                            ui.weak(format!(
                                "Decoded as {charset}{}",
                                if view.charset_override.is_some() {
                                    " (override)"
                                } else {
                                    ""
                                }
                            ));
                        }
                        if view.charset_override.is_some()
                            && ui.button("Use detected encoding").clicked()
                        {
                            view.charset_override = None;
                            page = Some(view.offset);
                        }
                    });
                    // A match waiting for its page: reveal it now that the page has arrived.
                    if let Some(at) = view.reveal_at
                        && (view.offset..view.offset + view.page).contains(&at)
                    {
                        reveal = char_range_at(shown, (at - view.offset) as usize, &found);
                        view.reveal_at = None;
                    }
                    // Highlighting is page-local, so the current whole-body match has to be
                    // renumbered against this page to be the one drawn as current.
                    let current = if whole_body {
                        let before = view
                            .search
                            .offsets
                            .iter()
                            .take_while(|at| **at < view.offset)
                            .count();
                        self.response_find.index.saturating_sub(before)
                    } else {
                        self.response_find.index
                    };
                    // Read-only: `&str` is an immutable `TextBuffer`, so the editor cannot edit it.
                    // The id carries the request so each one keeps its own scroll position.
                    editor::code(
                        ui,
                        &format!("response-text-{id}"),
                        &mut shown.as_str(),
                        4,
                        editor::Decoration {
                            found: &found,
                            current,
                            reveal,
                            // Colour only when the server said it is JSON; guessing from the bytes
                            // would mis-colour plain text that merely starts with a brace.
                            syntax: if view.result.headers.iter().any(|(name, value)| {
                                name.eq_ignore_ascii_case("content-type") && value.contains("json")
                            }) {
                                editor::Syntax::Json
                            } else {
                                editor::Syntax::None
                            },
                        },
                    );
                }
            }
            ResponseTab::Json => {
                if let Some(json) = &view.json {
                    ui.horizontal(|ui| {
                        ui.label("Pointer");
                        ui.text_edit_singleline(&mut view.tree.path);
                        if ui.button("Go").clicked() {
                            if json.pointer(&view.tree.path).is_some() {
                                view.tree.selected = view.tree.path.clone();
                                let mut pointer = view.tree.path.as_str();
                                while let Some((parent, _)) = pointer.rsplit_once('/') {
                                    view.tree.expanded.insert(parent.to_string());
                                    pointer = parent;
                                }
                            } else {
                                self.status = "JSON pointer was not found".into();
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Filter keys / values");
                        ui.text_edit_singleline(&mut view.tree.filter);
                        if ui.button("Clear filter").clicked() {
                            view.tree.filter.clear();
                        }
                        if ui.button("Expand all").clicked() {
                            view.tree.expand_all = true;
                        }
                        if ui.button("Collapse all").clicked() {
                            view.tree.expand_all = false;
                            view.tree.expanded.clear();
                            view.tree.selected.clear();
                        }
                    });
                    ui.weak("Click a value, then use ↑/↓, ←/→, Home/End. Filtering keeps matching ancestors open.");
                    let selected = json.pointer(&view.tree.selected).unwrap_or(json);
                    ui.horizontal_wrapped(|ui| {
                        ui.monospace(if view.tree.selected.is_empty() {
                            "/ (root)"
                        } else {
                            &view.tree.selected
                        });
                        if ui.button("Copy value").clicked() {
                            ui.ctx().copy_text(json_value_text(selected));
                        }
                        if ui.button("Copy JSON Pointer").clicked() {
                            ui.ctx().copy_text(view.tree.selected.clone());
                        }
                        let sensitive=view.tree.selected.to_ascii_lowercase().contains("token")||view.tree.selected.to_ascii_lowercase().contains("password")||view.tree.selected.to_ascii_lowercase().contains("secret");
                        if ui.button("Assert type").clicked(){let kind=match selected{serde_json::Value::Null=>"null",serde_json::Value::Bool(_)=>"boolean",serde_json::Value::Number(_)=>"number",serde_json::Value::String(_)=>"string",serde_json::Value::Array(_)=>"array",serde_json::Value::Object(_)=>"object"};let p=serde_json::to_string(&view.tree.selected).unwrap();assertion_snippet=Some(format!("\ntest(\"JSON value has expected type\", () => {{\n  expect(response.jsonPointer({p}), {p}).toBeType(\"{kind}\");\n}});\n"));}
                        if matches!(selected,serde_json::Value::Array(_))&&ui.button("Assert array length").clicked(){let len=selected.as_array().unwrap().len();let p=serde_json::to_string(&view.tree.selected).unwrap();assertion_snippet=Some(format!("\ntest(\"JSON array has expected length\", () => {{\n  expect(response.jsonPointer({p}).length, {p}).toBe({len});\n}});\n"));}
                        if !sensitive&&ui.button("Assert observed value").clicked(){let p=serde_json::to_string(&view.tree.selected).unwrap();let value=serde_json::to_string(selected).unwrap();assertion_snippet=Some(format!("\ntest(\"JSON value matches the observed contract\", () => {{\n  expect(response.jsonPointer({p}), {p}).toEqual({value});\n}});\n"));}
                    });
                    ui.separator();
                    let rows = json_rows_filtered(
                        json,
                        &view.tree.expanded,
                        view.tree.expand_all,
                        &view.tree.filter,
                    );
                    let count = rows.len();
                    let height = ui.spacing().interact_size.y;
                    let tree_id = egui::Id::new(("json-tree", &view.result.run_id));
                    let mut reveal = None;
                    if ui.memory(|m| m.has_focus(tree_id)) {
                        for key in [
                            egui::Key::ArrowDown,
                            egui::Key::ArrowUp,
                            egui::Key::ArrowLeft,
                            egui::Key::ArrowRight,
                            egui::Key::Home,
                            egui::Key::End,
                        ] {
                            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key)) {
                                reveal = json_tree_key(&mut view.tree, &rows, key);
                            }
                        }
                    }
                    let mut scroll = egui::ScrollArea::vertical().id_salt(tree_id);
                    if let Some(index) = reveal {
                        scroll = scroll.vertical_scroll_offset(index as f32 * height);
                    }
                    let output = scroll.show_rows(ui, height, count, |ui, range| {
                        for row in &rows[range] {
                            ui.horizontal(|ui| {
                                ui.add_space(row.depth as f32 * 16.0);
                                let container = matches!(
                                    row.value,
                                    serde_json::Value::Object(_) | serde_json::Value::Array(_)
                                );
                                if container
                                    && ui
                                        .small_button(
                                            if view.tree.expand_all
                                                || !view.tree.filter.trim().is_empty()
                                                || view.tree.expanded.contains(&row.pointer)
                                            {
                                                "▾"
                                            } else {
                                                "▸"
                                            },
                                        )
                                        .clicked()
                                {
                                    if view.tree.expand_all {
                                        view.tree.expanded = rows
                                            .iter()
                                            .filter(|r| r.value.is_object() || r.value.is_array())
                                            .map(|r| r.pointer.clone())
                                            .collect();
                                        view.tree.expand_all = false;
                                    }
                                    if !view.tree.expanded.remove(&row.pointer) {
                                        view.tree.expanded.insert(row.pointer.clone());
                                    }
                                } else if !container {
                                    ui.add_space(22.0);
                                }
                                let detail = match row.value {
                                    serde_json::Value::Object(v) => format!("{{{}}}", v.len()),
                                    serde_json::Value::Array(v) => format!("[{}]", v.len()),
                                    _ => json_value_text(row.value),
                                };
                                if ui
                                    .selectable_label(
                                        view.tree.selected == row.pointer,
                                        format!("{}: {detail}", row.label),
                                    )
                                    .clicked()
                                {
                                    view.tree.selected = row.pointer.clone();
                                    ui.memory_mut(|m| m.request_focus(tree_id));
                                }
                            });
                        }
                    });
                    ui.interact(
                        output.inner_rect,
                        tree_id,
                        egui::Sense::focusable_noninteractive(),
                    );
                    if count == 0 {
                        ui.weak("No matching keys or values");
                    }
                    if count >= 10_000 {
                        ui.weak("Showing the first 10,000 expanded values");
                    }
                } else {
                    ui.weak("JSON tree is available for valid complete JSON responses up to 2 MiB and 128 levels. Use Body for this response.");
                }
            }
            ResponseTab::Compare => {
                ui.weak("Compare without sending. JSON object order is ignored; array order is significant.");
                egui::ComboBox::from_id_salt("compare-target")
                    .selected_text(if self.compare_target == "baseline" {
                        "Saved baseline"
                    } else {
                        compare_candidates
                            .iter()
                            .find(|(id, _, _)| id == &self.compare_target)
                            .map(|(_, name, _)| name.as_str())
                            .unwrap_or("Choose retained response")
                    })
                    .show_ui(ui, |ui| {
                        if self.baseline.is_some() {
                            ui.selectable_value(
                                &mut self.compare_target,
                                "baseline".into(),
                                "Saved baseline",
                            );
                        }
                        for (id, name, _) in &compare_candidates {
                            ui.selectable_value(&mut self.compare_target, id.clone(), name);
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label("Ignored JSON pointers");
                    ui.text_edit_singleline(&mut self.compare_ignored);
                });
                ui.weak("Separate active ignore rules with commas.");
                ui.horizontal(|ui| {
                    if ui.button("Save current as baseline…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("Duckie baseline", &["json"])
                            .set_file_name("response.baseline.json")
                            .save_file()
                    {
                        match duckie_app::Baseline::from_result(&view.result).and_then(|b| {
                            b.save(&path)?;
                            Ok(b)
                        }) {
                            Ok(b) => {
                                self.baseline = Some(b);
                                self.compare_target = "baseline".into();
                            }
                            Err(e) => self.status = format!("Could not save baseline: {e}"),
                        }
                    }
                    if ui.button("Load baseline…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("Duckie baseline", &["json"])
                            .pick_file()
                    {
                        match duckie_app::Baseline::load(&path) {
                            Ok(b) => {
                                self.baseline = Some(b);
                                self.compare_target = "baseline".into();
                            }
                            Err(e) => self.status = format!("Could not load baseline: {e}"),
                        }
                    }
                });
                let ignored = self
                    .compare_ignored
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                let comparison = if self.compare_target == "baseline" {
                    self.baseline
                        .as_ref()
                        .map(|b| duckie_app::compare_baseline(&view.result, b, &ignored))
                } else {
                    compare_candidates
                        .iter()
                        .find(|(id, _, _)| id == &self.compare_target)
                        .map(|(_, _, r)| duckie_app::compare(&view.result, r, &ignored))
                };
                if let Some(c) = comparison {
                    if let Some((a, b)) = c.status {
                        ui.label(format!("Status: {:?} → {:?}", a, b));
                    }
                    let empty =
                        c.status.is_none() && c.headers.is_empty() && c.differences.is_empty();
                    for h in &c.headers {
                        ui.monospace(h);
                    }
                    for d in &c.differences {
                        ui.monospace(d);
                    }
                    if empty {
                        ui.label("No differences");
                    }
                    if c.truncated {
                        ui.weak("Body comparison limited to the first 2 MiB.");
                    }
                }
            }
            ResponseTab::Headers => {
                egui::ScrollArea::both().show(ui, |ui| {
                    egui::Grid::new("response-headers")
                        .striped(true)
                        .show(ui, |ui| {
                            for (name, value) in &view.result.headers {
                                ui.label(RichText::new(name).monospace());
                                ui.add(egui::Label::new(value).selectable(true));
                                if ui.small_button("Copy").clicked() {
                                    ui.ctx().copy_text(format!("{name}: {value}"));
                                }
                                if ui.small_button("Assert present").clicked() {
                                    let literal=serde_json::to_string(name).unwrap();
                                    assertion_snippet=Some(format!("\ntest(\"response includes header\", () => {{\n  expect(response.header({literal})).toBeType(\"string\");\n}});\n"));
                                }
                                ui.end_row();
                            }
                        });
                });
            }
            ResponseTab::Tests => {
                ui.weak(format!(
                    "Response revision {} · test revision {} · {}",
                    view.result.summary.revision,
                    view.test_revision,
                    view.result.summary.environment
                ));
                if let Some(report) = &view.report {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if let Some(error) = &report.suite_error {
                            ui.colored_label(ui.visuals().error_fg_color, error);
                        }
                        if report.tests.is_empty() && report.suite_error.is_none() {
                            ui.weak("No tests defined");
                        }
                        for case in &report.tests {
                            ui.horizontal(|ui| {
                                ui.colored_label(
                                    if case.passed {
                                        success(ui)
                                    } else {
                                        ui.visuals().error_fg_color
                                    },
                                    if case.passed { "PASS" } else { "FAIL" },
                                );
                                ui.label(&case.name);
                                if !case.passed
                                    && ui
                                        .small_button(match case.line {
                                            Some(line) => format!("Go to line {line}"),
                                            None => "Go to code".into(),
                                        })
                                        .clicked()
                                {
                                    self.request_tab = RequestTab::Tests;
                                    self.goto_line = case.line;
                                }
                            });
                            if let Some(error) = &case.error {
                                ui.add(
                                    egui::Label::new(RichText::new(error).monospace())
                                        .selectable(true),
                                );
                            }
                            ui.separator();
                        }
                        if !report.console.is_empty() {
                            ui.strong("Console");
                            for line in &report.console {
                                ui.monospace(line);
                            }
                        }
                    });
                } else {
                    ui.weak("No test results yet. Write assertions in the request's Tests tab.");
                }
            }
            ResponseTab::Details => {
                egui::ScrollArea::both().show(ui, |ui| {
                    ui.weak("Effective request summary · Credentials masked");
                    ui.monospace(format!(
                        "{} {}",
                        view.result.summary.method, view.result.summary.url
                    ));
                    ui.label(format!(
                        "Environment: {} · Draft revision: {} · Timeout: {} ms",
                        view.result.summary.environment,
                        view.result.summary.revision,
                        view.result.summary.timeout_ms
                    ));
                    ui.label(format!(
                        "{} received encoded bytes · {} decoded bytes",
                        view.result.encoded_bytes,
                        view.result.body.len()
                    ));
                    ui.separator();
                    ui.strong("Connection and timing diagnostics");
                    ui.label(format!(
                        "HTTP version: {}",
                        view.result
                            .diagnostics
                            .http_version
                            .as_deref()
                            .unwrap_or("Unavailable")
                    ));
                    ui.label(format!("Total: {} ms", view.result.duration_ms));
                    ui.label(match view.result.diagnostics.headers_ms {
                        Some(ms) => format!("Request start to response headers: {ms} ms"),
                        None => "Request start to response headers: unavailable".into(),
                    });
                    ui.label(match view.result.diagnostics.body_and_decode_ms {
                        Some(ms) => format!("Body transfer and decoding: {ms} ms"),
                        None => "Body transfer and decoding: unavailable".into(),
                    });
                    ui.weak("DNS, TCP, TLS, proxy, request upload, server wait, transfer, decoding, and spool writes overlap or are not exposed separately by the transport. Connection reuse is unavailable.");
                    for (n, v) in &view.result.summary.headers {
                        ui.monospace(format!("{n}: {v}"));
                    }
                    if view.result.status.is_some_and(|s| (300..400).contains(&s))
                        && let Some((_, value)) = view
                            .result
                            .headers
                            .iter()
                            .find(|(n, _)| n.eq_ignore_ascii_case("location"))
                    {
                        ui.label(format!("Location: {value}"));
                        if ui.button("Open Location as request").clicked() {
                            location = Some((view.result.summary.url.clone(), value.clone()));
                        }
                    }
                });
            }
        }
        // A jump lands the match a little way into the page so there is context before it.
        if let Some(at) = go_to {
            let start = at.saturating_sub(2048);
            view.reveal_at = Some(at);
            if !(view.offset..view.offset + view.page).contains(&at) {
                page = Some(start);
            }
        }
        if page.is_some() || start_search.is_some() {
            // Take everything needed from the view before calling back into `self`.
            let run_id = view.result.run_id.clone();
            let body = view.result.body.clone();
            let headers = view.result.headers.clone();
            let charset_override = view.charset_override.clone();
            let charset = view.charset.clone();
            if let Some(offset) = page {
                self.preview(
                    id.clone(),
                    run_id,
                    body.clone(),
                    headers,
                    offset,
                    charset_override,
                );
            }
            if let Some(query) = start_search {
                self.search_body(id, body, query, charset);
            }
        }
        if let Some(body) = save
            && let Some(path) = rfd::FileDialog::new()
                .set_file_name("response.bin")
                .save_file()
        {
            self.background(move || {
                IoEvent::Message(
                    body.save(&path)
                        .map(|_| format!("Saved body to {}", path.display()))
                        .map_err(Into::into),
                )
            });
        }
        if let Some(snippet) = assertion_snippet {
            self.drafts[self.selected].source.push_str(&snippet);
            self.drafts[self.selected].dirty = true;
            self.drafts[self.selected].revision += 1;
            self.request_tab = RequestTab::Tests;
        }
        if let Some((base, value)) = location {
            match url_join(&base, &value) {
                Ok(url) => {
                    self.new_request();
                    self.drafts[self.selected].request.set_literal_address(&url);
                    self.drafts[self.selected].request.name = "Redirect location".into();
                    self.touch();
                }
                Err(e) => self.status = e,
            }
        }
    }
}
fn url_join(base: &str, value: &str) -> Result<String, String> {
    duckie_model::resolve_location(base, value)
}
fn size(n: u64) -> String {
    if n >= MIB {
        format!("{:.1} MiB", n as f64 / MIB as f64)
    } else if n >= 1024 {
        format!("{:.1} KiB", n as f64 / 1024.0)
    } else {
        format!("{n} B")
    }
}
fn secret_input(
    ui: &mut egui::Ui,
    env: &str,
    key: &str,
    secrets: &mut std::collections::BTreeMap<String, Values>,
    remember: &mut std::collections::BTreeSet<(String, String)>,
    reveal: &mut bool,
    focus: bool,
) -> bool {
    let mut changed = false;
    ui.push_id((env, key), |ui| {
        let value = secrets
            .entry(env.into())
            .or_default()
            .entry(key.into())
            .or_default();
        ui.horizontal(|ui| {
            ui.label("Value");
            let field = ui.add(
                egui::TextEdit::singleline(value)
                    .password(!*reveal)
                    .desired_width(430.0)
                    .hint_text("Paste credential"),
            );
            changed |= field.changed();
            if focus {
                field.request_focus();
            }
            ui.checkbox(reveal, "Show");
        });
        if value.is_empty() {
            ui.colored_label(warning(ui), format!("No value for {key} in {env}"));
        }
        let pair = (env.into(), key.into());
        let mut save = remember.contains(&pair);
        if ui.checkbox(&mut save, "Remember in secrets file").changed() {
            if save {
                remember.insert(pair);
            } else {
                remember.remove(&pair);
            }
            changed = true;
        }
        ui.weak(if save {
            "Source: .duckie/secrets.json (written on Save)"
        } else {
            "Source: this session"
        });
    });
    changed
}
impl eframe::App for Duckie {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll();
        if let Some(draft) = self.drafts.get(self.selected)
            && draft.pending
        {
            let id = draft.request.id.clone();
            self.ensure_loaded(&id);
        }
        if !self.io_busy && self.pending.is_none() && self.import.is_none() && !self.env_dialog {
            self.shortcuts(&ctx);
        }
        if self.pending.is_some()
            || self.delete.is_some()
            || self.env_dialog
            || self.import.is_some()
            || self.about
        {
            ui.disable();
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && (self.dirty() || self.io_busy || self.active.is_some() || self.suite.running)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.active.is_some() {
                if let Some((_, token, ..)) = &self.active {
                    token.cancel();
                }
                self.status = "Stopping active run. Close again when it finishes.".into();
            } else if self.suite.running {
                if let Some(token) = &self.suite.cancel {
                    token.cancel();
                }
                self.status = "Stopping suite. Close again when it finishes.".into();
            } else if !self.io_busy {
                self.pending = Some(Pending::Close);
            }
        }
        egui::Panel::top("menu").show(ui, |ui| {
            ui.add_enabled_ui(!self.io_busy, |ui| self.top_menu(ui));
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(if self.dirty() {
                        "Unsaved changes"
                    } else {
                        "All changes saved"
                    })
                    .size(12.0),
                );
                ui.separator();
                ui.add(egui::Label::new(RichText::new(&self.status).size(12.0)).truncate());
            });
        });
        if let Some((id, _, started, progress)) = &self.active {
            let name = self
                .drafts
                .iter()
                .find(|d| &d.request.id == id)
                .map(|d| d.request.name.clone())
                .unwrap_or_default();
            let elapsed = started.elapsed().as_secs_f32();
            // Encoded bytes are what actually arrived; decoded is what the response will hold, and
            // the two differ enough on a compressed body to be worth showing separately.
            let (received, decoded, total) =
                (progress.encoded(), progress.decoded(), progress.total());
            let transferred = if received == 0 {
                String::new()
            } else if total > received {
                format!(" · {} of {}", size(received), size(total))
            } else if decoded > received {
                format!(" · {} received, {} decoded", size(received), size(decoded))
            } else {
                format!(" · {} received", size(received))
            };
            egui::Panel::top("active-run").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{} · {name} · {elapsed:.1}s{transferred}",
                        if self.active_testing {
                            "Testing"
                        } else {
                            "Sending"
                        }
                    ));
                    // A declared length gives a real bar; without one the byte count is all there is.
                    if total > 0 && received < total {
                        ui.add(
                            egui::ProgressBar::new(received as f32 / total as f32)
                                .desired_width(140.0),
                        );
                    }
                    if ui
                        .small_button(if self.active_testing {
                            "Stop tests"
                        } else {
                            "Cancel"
                        })
                        .clicked()
                        && let Some((_, token, ..)) = &self.active
                    {
                        token.cancel();
                    }
                });
            });
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        // The window's enforced minimum width is 900 (see main.rs); the sidebar's own minimum
        // size is 190, which always leaves room for the central panel above that floor, so the
        // sidebar is never hidden out from under Ctrl+B/Ctrl+K at a supported window size.
        if self.prefs.sidebar {
            let panel = egui::Panel::left("sidebar")
                .default_size(self.prefs.sidebar_width.unwrap_or(240.0))
                .size_range(190.0..=350.0)
                .resizable(true)
                .show(ui, |ui| {
                    ui.add_enabled_ui(!self.io_busy, |ui| self.sidebar(ui));
                });
            // Read back what the user dragged it to; `App::save` persists it.
            self.prefs.sidebar_width = Some(panel.response.rect.width());
        }
        egui::CentralPanel::default().show(ui, |ui| {
            let available = ui.available_height();
            let content_height =
                request_pane_height(self.request_tab, &self.drafts[self.selected].request);
            let minimum = content_height.min((available - 170.0).max(220.0));
            let panel = egui::Panel::top("request-pane")
                .resizable(true)
                .default_size(self.prefs.request_height.unwrap_or(330.0).max(minimum))
                .min_size(minimum)
                .max_size((available - 170.0).max(minimum))
                .show(ui, |ui| {
                    ui.add_enabled_ui(!self.io_busy, |ui| {
                        ui.push_id(self.drafts[self.selected].request.id.clone(), |ui| {
                            self.request_header(ui);
                            egui::ScrollArea::vertical()
                                .id_salt("request-editor")
                                .show(ui, |ui| {
                                    self.editor(ui);
                                });
                        });
                    });
                });
            self.prefs.request_height = Some(panel.response.rect.height());
            egui::CentralPanel::default().show(ui, |ui| self.response(ui));
        });
        // Someone may have edited the collection in another program while Duckie was away.
        if ctx.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::WindowFocused(true)))
        }) {
            self.check_disk();
        }
        self.dialogs(&ctx);
        #[cfg(feature = "bench")]
        self.bench_frame(&ctx);
        #[cfg(feature = "screenshot")]
        if let Ok(path) = std::env::var("DUCKIE_CAPTURE_PATH") {
            self.capture_frames += 1;
            if self.capture_frames == 4 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            ctx.input(|input| {
                for event in &input.events {
                    if let egui::Event::Screenshot { image, .. } = event {
                        let rgba: Vec<u8> = image
                            .pixels
                            .iter()
                            .flat_map(|pixel| pixel.to_array())
                            .collect();
                        image::save_buffer(
                            &path,
                            &rgba,
                            image.width() as u32,
                            image.height() as u32,
                            image::ColorType::Rgba8,
                        )
                        .expect("Cannot save development screenshot");
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            });
            self.allow_close = true;
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        #[cfg(feature = "bench")]
        if self.bench.is_some() {
            return;
        }
        self.prefs.selected = self.drafts.get(self.selected).map(|d| d.request.id.clone());
        self.prefs.environment = self.envs.get(self.env_index).map(|e| e.name.clone());
        eframe::set_value(storage, "duckie-preferences", &self.prefs);
    }
    fn persist_egui_memory(&self) -> bool {
        false
    } // Text editor undo stacks can contain credentials.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some((_, token, ..)) = &self.active {
            token.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn row_export_and_duplicate_hydrate_deferred_content() {
        let dir = tempfile::tempdir().unwrap();
        let mut collection =
            duckie_storage::Collection::new(dir.path().into(), "rows".into()).unwrap();
        let request = RequestDefinition {
            url: "http://localhost/".into(),
            body: Body::Text {
                text: "retained body".into(),
            },
            ..Default::default()
        };
        let id = request.id.clone();
        collection.requests.push(duckie_storage::StoredRequest::new(
            request,
            "// retained tests".into(),
        ));
        collection.save().unwrap();
        let (_, mut app) = app_with(&[""]);
        app.use_collection(duckie_storage::Collection::open(dir.path()).unwrap());
        assert!(app.drafts[0].pending);
        app.copy_curl_for(&id, duckie_model::curl::Shell::Posix, false);
        assert!(!app.drafts[0].pending);
        assert_eq!(app.status, "Copied redacted cURL");
        app.use_collection(duckie_storage::Collection::open(dir.path()).unwrap());
        app.duplicate_request(&id);
        assert_eq!(app.drafts[1].source, "// retained tests");
        assert!(
            matches!(&app.drafts[1].request.body, Body::Text { text } if text == "retained body")
        );
    }
    #[test]
    fn pending_delete_keeps_row_identity_after_reorder() {
        let (_, mut app) = app_with(&["A", "A"]);
        let id = app.drafts[1].request.id.clone();
        app.delete = Some(id.clone());
        app.reorder(1, true);
        assert_eq!(app.delete.as_deref(), Some(id.as_str()));
        assert_eq!(
            app.drafts
                .iter()
                .position(|d| Some(&d.request.id) == app.delete.as_ref()),
            Some(0)
        );
        app.delete_request(&id);
        assert_eq!(app.drafts.len(), 1);
        assert_ne!(app.drafts[0].request.id, id);
        assert!(app.env_dirty);
    }
    #[test]
    fn request_actions_target_the_given_identity() {
        let (_, mut app) = app_with(&["A", "B"]);
        app.selected = 0;
        let id = app.drafts[1].request.id.clone();
        app.drafts[1].source = "// second request".into();
        app.drafts[1].request.body = Body::Text {
            text: "second body".into(),
        };
        app.move_request_to_folder(&id, " C ");
        assert_eq!(app.drafts[0].request.folder, "A");
        assert_eq!(app.drafts[1].request.folder, "C");
        app.duplicate_request(&id);
        assert_eq!(app.selected, 2);
        assert_ne!(app.drafts[2].request.id, id);
        assert_eq!(app.drafts[2].source, "// second request");
        assert!(
            matches!(&app.drafts[2].request.body, Body::Text { text } if text == "second body")
        );
        assert!(app.drafts[2].request.tests.file.is_empty());
        assert!(app.drafts[2].dirty);
    }
    #[test]
    fn json_filter_reaches_descendants_and_expand_all_stays_bounded() {
        let json = serde_json::json!({"a/b": {"~key": "Needle"}, "other": 2});
        let expanded = std::collections::BTreeSet::new();
        let rows = json_rows_filtered(&json, &expanded, false, "needle");
        assert_eq!(
            rows.iter().map(|r| r.pointer.as_str()).collect::<Vec<_>>(),
            ["", "/a~1b", "/a~1b/~0key"]
        );
        assert!(json_rows_filtered(&json, &expanded, false, "absent").is_empty());
        let large =
            serde_json::Value::Array((0..20_000).map(|i| serde_json::json!({"n":i})).collect());
        assert_eq!(
            json_rows_filtered(&large, &expanded, true, "").len(),
            10_000
        );
        let late = json_rows_filtered(&large, &expanded, false, "19999");
        assert!(late.iter().any(|r| r.pointer == "/19999/n"));
    }
    #[test]
    fn json_keyboard_moves_and_expands_without_sharing_state() {
        let json = serde_json::json!({"a": {"b": 1}, "z": 2});
        let mut tree = JsonTreeState::default();
        let rows = json_rows_filtered(&json, &tree.expanded, false, "");
        json_tree_key(&mut tree, &rows, egui::Key::ArrowDown);
        assert_eq!(tree.selected, "/a");
        json_tree_key(&mut tree, &rows, egui::Key::ArrowRight);
        assert!(tree.expanded.contains("/a"));
        let rows = json_rows_filtered(&json, &tree.expanded, false, "");
        json_tree_key(&mut tree, &rows, egui::Key::ArrowRight);
        assert_eq!(tree.selected, "/a/b");
        json_tree_key(&mut tree, &rows, egui::Key::ArrowLeft);
        assert_eq!(tree.selected, "/a");
        json_tree_key(&mut tree, &rows, egui::Key::ArrowLeft);
        assert!(!tree.expanded.contains("/a"));
        let fresh = JsonTreeState::default();
        assert!(fresh.selected.is_empty());
        assert_eq!(fresh.expanded.len(), 1);
    }
    #[test]
    fn suite_navigation_uses_identity_and_checks_executed_source() {
        let (_, mut app) = app_with(&["", ""]);
        let id = app.drafts[1].request.id.clone();
        app.drafts[1].source = "assert.ok(false);".into();
        app.suite
            .sources
            .insert(id.clone(), app.drafts[1].source.clone());
        let report = duckie_app::HeadlessReport {
            request_id: id,
            request_name: "executed".into(),
            environment: "dev".into(),
            revision: 7,
            outcome: duckie_model::Outcome::Complete,
            status: Some(200),
            duration_ms: 0,
            response_bytes: 0,
            response_snippet: None,
            error: None,
            tests: Some(duckie_model::TestReport {
                tests: vec![duckie_model::TestCase {
                    name: "fails".into(),
                    passed: false,
                    error: None,
                    line: Some(1),
                }],
                ..Default::default()
            }),
        };
        app.drafts.swap(0, 1);
        app.navigate_suite_result(&report);
        assert_eq!(app.selected, 0);
        assert!(app.request_tab == RequestTab::Tests);
        assert_eq!(app.goto_line, Some(1));
        app.drafts[0].source.push_str("\n// changed");
        app.navigate_suite_result(&report);
        assert_eq!(app.goto_line, None);
        assert!(app.status.contains("tests have changed"));
        app.drafts.remove(0);
        app.navigate_suite_result(&report);
        assert!(app.status.contains("deleted"));
    }
    #[test]
    fn environment_lifecycle_keeps_credentials_scoped() {
        let (_, mut app) = app_with(&[""]);
        app.envs = vec![duckie_model::Environment {
            name: "dev".into(),
            ..Default::default()
        }];
        app.env_index = 0;
        app.secrets.clear();
        app.remember.clear();
        app.secrets.insert(
            "dev".into(),
            duckie_model::Values::from([("token".into(), "private".into())]),
        );
        app.remember.insert(("dev".into(), "token".into()));
        app.new_env = "copy".into();
        app.name_environment(true);
        assert!(!app.secrets.contains_key("copy"));
        app.delete_environment();
        app.secrets.insert(
            "renamed".into(),
            duckie_model::Values::from([("orphan".into(), "old import".into())]),
        );
        app.remember.insert(("renamed".into(), "orphan".into()));
        app.new_env = "renamed".into();
        app.name_environment(false);
        assert!(!app.secrets.contains_key("dev"));
        assert!(!app.secrets["renamed"].contains_key("orphan"));
        assert!(app.remember.contains(&("renamed".into(), "token".into())));
        app.remove_secret("renamed", "token");
        assert!(app.secrets["renamed"].is_empty());
        assert!(app.remember.is_empty());
        app.delete_environment();
        assert_eq!(app.envs.len(), 1);
        assert_eq!(app.env_index, 0);
    }
    use super::*;
    use eframe::App;
    #[test]
    fn native_ui_paths_render_and_edits_remain_scoped() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Duckie::new(&cc);
        let mut frame = eframe::Frame::_new_kittest();
        app.drafts[0].request.url = "http://localhost:8080".into();
        app.drafts[0].request.auth.bearer = Some(SecretBinding {
            secret: "token".into(),
        });
        app.secrets.insert(
            "dev".into(),
            Values::from([("token".into(), "session-token".into())]),
        );
        app.envs.push(Environment {
            name: "staging".into(),
            ..Default::default()
        });
        app.env_index = 1;
        assert!(app.snapshot().secrets.is_empty());
        app.env_index = 0;
        for width in [900.0, 1280.0] {
            for tab in [
                RequestTab::Params,
                RequestTab::Headers,
                RequestTab::Auth,
                RequestTab::Body,
                RequestTab::Tests,
                RequestTab::Settings,
            ] {
                app.request_tab = tab;
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 820.0),
                    )),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
                assert!(!output.shapes.is_empty());
                output.textures_delta.clear();
            }
        }
        app.touch();
        let id = app.drafts[0].request.id.clone();
        app.new_request();
        assert_ne!(id, app.drafts[1].request.id);
        assert!(app.drafts[0].dirty);
        assert!(app.responses.is_empty());
        assert!(!app.service.busy());
    }
    /// Sends the Ctrl+T shortcut for one frame and drains the resulting texture delta.
    fn press_ctrl_t(ctx: &egui::Context, app: &mut Duckie, frame: &mut eframe::Frame) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 820.0),
            )),
            events: vec![egui::Event::Key {
                key: egui::Key::T,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::CTRL,
            }],
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| app.ui(ui, frame));
        output.textures_delta.clear();
    }
    #[test]
    fn ctrl_t_pastes_clipboard_text_into_the_bearer_secret() {
        let ctx = egui::Context::default();
        let mut app = Duckie::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        let mut frame = eframe::Frame::_new_kittest();
        app.request_tab = RequestTab::Params;
        app.clipboard_read = || Some("  eyJhbGciOi...\n".into());
        assert!(app.drafts[0].request.auth.bearer.is_none());
        app.drafts[0].request.auth.api_key = Some(ApiKey {
            header: "X-API-Key".into(),
            secret: "old-api-key".into(),
        });

        press_ctrl_t(&ctx, &mut app, &mut frame);
        assert!(app.request_tab == RequestTab::Auth);
        assert!(
            app.drafts[0].request.auth.api_key.is_none(),
            "pasting a bearer token selects bearer authentication exclusively"
        );
        // A bearer binding is created so there is a secret for the clipboard text to land in.
        let secret = app.drafts[0]
            .request
            .auth
            .bearer
            .as_ref()
            .unwrap()
            .secret
            .clone();
        assert!(app.drafts[0].dirty);
        assert!(!app.focus_token, "the clipboard already supplied the value");
        let env = app.envs[app.env_index].name.clone();
        assert_eq!(
            app.secrets
                .get(&env)
                .and_then(|v| v.get(&secret))
                .map(String::as_str),
            Some("eyJhbGciOi..."),
            "surrounding whitespace is trimmed"
        );

        // Pressing it again with a bearer already set must not disturb the existing binding,
        // only the secret's value.
        app.drafts[0].request.auth.bearer = Some(SecretBinding {
            secret: "existing".into(),
        });
        app.drafts[0].dirty = false;
        app.clipboard_read = || Some("fresh-token".into());
        press_ctrl_t(&ctx, &mut app, &mut frame);
        assert_eq!(
            app.drafts[0].request.auth.bearer.as_ref().unwrap().secret,
            "existing"
        );
        assert_eq!(
            app.secrets
                .get(&env)
                .and_then(|v| v.get("existing"))
                .map(String::as_str),
            Some("fresh-token")
        );
        assert!(app.drafts[0].dirty, "the secret's value did change");
    }
    #[test]
    fn row_based_request_tabs_expand_the_request_pane() {
        let mut request = RequestDefinition::default();
        let empty_headers = request_pane_height(RequestTab::Headers, &request);
        request.headers.extend([
            Row::new("Accept", "application/json"),
            Row::new("X-Trace", "one"),
            Row::new("X-Debug", "two"),
        ]);
        assert_eq!(
            request_pane_height(RequestTab::Headers, &request),
            empty_headers + 3.0 * 32.0
        );

        let empty_params = request_pane_height(RequestTab::Params, &request);
        request
            .query
            .extend([Row::new("param1", "var1"), Row::new("param2", "var2")]);
        assert_eq!(
            request_pane_height(RequestTab::Params, &request),
            empty_params + 2.0 * 32.0
        );
    }
    #[test]
    fn row_editors_keep_useful_field_widths_in_the_minimum_window() {
        let (name, value) = row_widths(500.0);
        assert!(name >= 180.0);
        assert!(value >= 240.0);
    }
    #[test]
    fn blank_request_variable_stays_appended_while_its_name_is_entered() {
        let mut rows = vec![Row::new("existing", "one")];
        rows.push(Row::new("", ""));
        assert_eq!(rows[1].name, "");

        rows[1].name.push('x');
        let values = variable_values(&rows);

        assert_eq!(rows[0].name, "existing");
        assert_eq!(rows[1].name, "x");
        assert_eq!(values.get("x").map(String::as_str), Some(""));
    }
    #[test]
    fn ctrl_t_falls_back_to_focusing_the_field_when_the_clipboard_has_no_text() {
        let ctx = egui::Context::default();
        let mut app = Duckie::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        let mut frame = eframe::Frame::_new_kittest();
        app.clipboard_read = || None;

        press_ctrl_t(&ctx, &mut app, &mut frame);
        assert!(app.request_tab == RequestTab::Auth);
        assert!(app.drafts[0].request.auth.bearer.is_some());
        // The flag is consumed the same frame the Auth tab renders, not left pending.
        assert!(!app.focus_token);
    }
    #[test]
    fn lazy_loaded_requests_hydrate_on_selection_and_before_save() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = duckie_storage::Collection::new(dir.path().to_path_buf(), "c".into()).unwrap();
        for id in ["one", "two"] {
            c.requests.push(duckie_storage::StoredRequest::new(
                RequestDefinition {
                    id: id.into(),
                    body: Body::Json {
                        text: format!("{{\"id\":\"{id}\"}}"),
                    },
                    ..Default::default()
                },
                format!("test('{id}', () => {{}});"),
            ));
        }
        c.save().unwrap();

        let opened = duckie_storage::Collection::open(dir.path()).unwrap();
        assert!(opened.requests.iter().all(|r| !r.loaded));

        let ctx = egui::Context::default();
        let mut app = Duckie::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        app.use_collection(opened);
        assert!(app.drafts[0].pending && app.drafts[1].pending);
        assert!(app.drafts[0].source.is_empty());

        // A frame with drafts[0] selected loads only that one draft.
        let mut frame = eframe::Frame::_new_kittest();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 820.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
        output.textures_delta.clear();
        assert!(!app.drafts[0].pending, "the selected draft loads");
        assert!(app.drafts[1].pending, "an unselected draft stays deferred");
        assert_eq!(app.drafts[0].source, "test('one', () => {});");
        match &app.drafts[0].request.body {
            Body::Json { text } => assert_eq!(text, "{\"id\":\"one\"}"),
            other => panic!("expected Json, {}", serde_json::to_string(other).unwrap()),
        }

        // save_collection hydrates every remaining pending draft up front, before it ever builds
        // the write set — a request must never be saved with the empty placeholder it opened
        // with just because it was never selected.
        app.save_collection(false);
        assert!(
            !app.drafts[1].pending,
            "save must hydrate every draft, not only the selected one"
        );
        assert_eq!(app.drafts[1].source, "test('two', () => {});");
    }
    fn key_input(key: egui::Key) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 900.0),
            )),
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        }
    }
    #[test]
    fn enter_row_entry_moves_focus_and_only_appends_populated_rows() {
        let ctx = egui::Context::default();
        let mut entries = vec![Row::new("Accept", "json")];
        let name = egui::Id::new(("keyboard-rows", 0usize, "name"));
        let value = egui::Id::new(("keyboard-rows", 0usize, "value"));
        ctx.memory_mut(|m| m.request_focus(name));
        let mut output = ctx.run_ui(key_input(egui::Key::Enter), |ui| {
            rows(ui, "keyboard-rows", &mut entries);
        });
        output.textures_delta.clear();
        assert!(ctx.memory(|m| m.has_focus(value)));
        let mut output = ctx.run_ui(key_input(egui::Key::Enter), |ui| {
            rows(ui, "keyboard-rows", &mut entries);
        });
        output.textures_delta.clear();
        assert_eq!(entries.len(), 2);
        ctx.memory_mut(|m| m.request_focus(egui::Id::new(("keyboard-rows", 1usize, "value"))));
        let mut output = ctx.run_ui(key_input(egui::Key::Enter), |ui| {
            rows(ui, "keyboard-rows", &mut entries);
        });
        output.textures_delta.clear();
        assert_eq!(entries.len(), 2);
    }
    #[test]
    fn search_keyboard_opens_without_sending_and_words_match_independently() {
        let (ctx, mut app) = app_with(&["users", "orders"]);
        app.focus_url = false;
        app.drafts[0].request.method = "POST".into();
        app.drafts[0].request.url = "https://local.test/users".into();
        app.search = "USERS post".into();
        assert!(
            app.sidebar_rows()
                .iter()
                .any(|r| matches!(r, SidebarRow::Request(0)))
        );
        app.search.clear();
        app.selected = 0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| app.sidebar(ui),
        );
        output.textures_delta.clear();
        ctx.memory_mut(|m| m.request_focus(egui::Id::new("request-search")));
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.sidebar(ui));
        output.textures_delta.clear();
        let mut output = ctx.run_ui(key_input(egui::Key::ArrowDown), |ui| app.sidebar(ui));
        output.textures_delta.clear();
        assert_eq!(app.search_selected, Some(1));
        let mut output = ctx.run_ui(key_input(egui::Key::Enter), |ui| app.sidebar(ui));
        output.textures_delta.clear();
        assert_eq!(app.selected, 1);
        assert!(app.focus_url);
        assert!(app.active.is_none());
    }
    #[test]
    fn url_completion_inserts_a_name_without_a_secret_value() {
        let (ctx, mut app) = app_with(&["users"]);
        app.drafts[0]
            .request
            .set_address("https://local.test/{{secret.to");
        app.secrets
            .entry(app.envs[app.env_index].name.clone())
            .or_default()
            .insert("token".into(), "private-value".into());
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.request_header(ui));
        output.textures_delta.clear();
        let mut output = ctx.run_ui(key_input(egui::Key::Enter), |ui| app.request_header(ui));
        output.textures_delta.clear();
        assert_eq!(
            app.drafts[0].request.address(),
            "https://local.test/{{secret.token}}"
        );
        assert!(app.active.is_none());
    }
    fn app_with(folders: &[&str]) -> (egui::Context, Duckie) {
        let ctx = egui::Context::default();
        let mut app = Duckie::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        app.drafts = folders
            .iter()
            .enumerate()
            .map(|(i, folder)| Draft {
                request: RequestDefinition {
                    name: format!("r{i}"),
                    folder: (*folder).into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .collect();
        (ctx, app)
    }
    fn shape(app: &Duckie) -> Vec<String> {
        app.sidebar_rows()
            .iter()
            .map(|row| match row {
                SidebarRow::Folder {
                    name,
                    count,
                    collapsed,
                } => {
                    format!(
                        "[{name} {count}{}]",
                        if *collapsed { " closed" } else { "" }
                    )
                }
                SidebarRow::Request(i) => app.drafts[*i].request.name.clone(),
            })
            .collect()
    }
    #[test]
    fn sidebar_groups_by_folder_in_first_appearance_order() {
        let (_ctx, mut app) = app_with(&["B", "A", "B", ""]);
        assert_eq!(
            shape(&app),
            ["[B 2]", "r0", "r2", "[A 1]", "r1", "[Ungrouped 1]", "r3"]
        );
        // Collapsing hides the folder's requests but keeps its header and count.
        app.prefs.collapsed.insert("B".into());
        assert_eq!(
            shape(&app),
            ["[B 2 closed]", "[A 1]", "r1", "[Ungrouped 1]", "r3"]
        );
        // A search must reach into a closed folder, or its matches would be unreachable.
        app.search = "r2".into();
        assert_eq!(shape(&app), ["[B 1]", "r2"]);
    }
    #[test]
    fn reorder_swaps_within_a_folder_only_and_follows_the_selection() {
        let (_ctx, mut app) = app_with(&["A", "B", "A"]);
        app.selected = 0;
        // r0 and r2 share folder A even though r1 sits between them.
        app.reorder(0, false);
        assert_eq!(shape(&app), ["[A 2]", "r2", "r0", "[B 1]", "r1"]);
        assert_eq!(app.selected, 2, "selection follows the moved request");
        // r1 is alone in B, so it cannot move and nothing changes.
        let before = shape(&app);
        app.reorder(1, true);
        app.reorder(1, false);
        assert_eq!(shape(&app), before);
    }
    #[test]
    fn opening_a_collection_restores_the_remembered_request_and_environment() {
        let dir = tempfile::tempdir().unwrap();
        let mut collection =
            duckie_storage::Collection::new(dir.path().to_path_buf(), "c".into()).unwrap();
        collection.requests = ["a", "b", "c"]
            .iter()
            .map(|id| {
                duckie_storage::StoredRequest::new(
                    RequestDefinition {
                        id: (*id).into(),
                        ..Default::default()
                    },
                    String::new(),
                )
            })
            .collect();
        collection.environments = ["dev", "staging"]
            .iter()
            .map(|name| Environment {
                name: (*name).into(),
                ..Default::default()
            })
            .collect();

        let ctx = egui::Context::default();
        let mut app = Duckie::new(&eframe::CreationContext::_new_kittest(ctx.clone()));
        app.prefs.selected = Some("c".into());
        app.prefs.environment = Some("staging".into());
        app.use_collection(collection.clone());
        assert_eq!(app.drafts[app.selected].request.id, "c");
        assert_eq!(app.envs[app.env_index].name, "staging");

        // Names from another collection must not select something arbitrary.
        app.prefs.selected = Some("not-here".into());
        app.prefs.environment = Some("prod".into());
        app.use_collection(collection);
        assert_eq!(app.selected, 0);
        assert_eq!(app.env_index, 0);
    }
    #[test]
    fn char_range_at_selects_the_page_hit_or_falls_back_to_a_caret() {
        let text = "üduck.duck";
        let found = editor::hits(text, "duck");
        // "duck" starts at byte 2 because "ü" is two bytes, but at character 1.
        assert_eq!(char_range_at(text, 2, &found), Some(1..5));
        assert_eq!(char_range_at(text, 7, &found), Some(6..10));
        // An offset that is not the start of a hit leaves a caret there instead.
        assert_eq!(char_range_at(text, 6, &found), Some(5..5));
    }
    #[test]
    fn json_tree_uses_escaped_pointers_and_only_expands_selected_containers() {
        let json = serde_json::json!({"a/b": {"~key": [1, 2]}});
        let collapsed = json_rows(&json, &[String::new()].into_iter().collect());
        assert_eq!(
            collapsed
                .iter()
                .map(|r| r.pointer.as_str())
                .collect::<Vec<_>>(),
            vec!["", "/a~1b"]
        );
        let expanded = json_rows(
            &json,
            &[String::new(), "/a~1b".into()].into_iter().collect(),
        );
        assert!(expanded.iter().any(|r| r.pointer == "/a~1b/~0key"));
        assert_eq!(json.pointer("/a~1b/~0key/1"), Some(&serde_json::json!(2)));
    }
    /// Frame construction time under input, with a thousand requests listed and a megabyte of
    /// response on screen. This is CPU time to produce a frame, so it is a lower bound on
    /// input-to-paint: it excludes upload, present and the compositor.
    ///
    /// Ignored by default; take it with
    /// `cargo test -p duckie-desktop --release -- --ignored --nocapture`.
    #[test]
    #[ignore = "measurement"]
    fn frame_time_under_input() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Duckie::new(&cc);
        let mut frame = eframe::Frame::_new_kittest();
        app.drafts = (0..1000)
            .map(|i| Draft {
                request: RequestDefinition {
                    name: format!("request {i}"),
                    folder: format!("folder {}", i % 20),
                    ..Default::default()
                },
                ..Default::default()
            })
            .collect();
        let id = app.drafts[0].request.id.clone();
        let lines: usize = std::env::var("DUCKIE_MEASURE_LINES")
            .ok()
            .and_then(|v| v.parse().ok())
            // Just under the colouring cap by default, which is the worst case that still
            // colours; a larger page draws plain and is cheaper.
            .unwrap_or(5_000);
        let body: String = std::iter::repeat_n("{\"duck\": 1},\n", lines).collect();
        app.responses.insert(
            id.clone(),
            ResponseView {
                tree: JsonTreeState::default(),
                json: None,
                result: ExecutionResult {
                    request_id: id,
                    run_id: "r".into(),
                    summary: RequestSummary {
                        method: "GET".into(),
                        url: "http://localhost/".into(),
                        headers: vec![],
                        environment: "dev".into(),
                        revision: 0,
                        timeout_ms: 30_000,
                    },
                    outcome: Outcome::Complete,
                    status: Some(200),
                    status_text: "OK".into(),
                    // Declared JSON, so the preview is syntax-coloured and the measurement covers
                    // the scanner rather than only the plain path.
                    headers: vec![("content-type".into(), "application/json".into())],
                    body_error: None,
                    body: BodyHandle::Memory(std::sync::Arc::new(body.clone().into_bytes())),
                    encoded_bytes: body.len() as u64,
                    duration_ms: 1,
                    diagnostics: ResponseDiagnostics::default(),
                },
                preview: body,
                pretty: None,
                binary: false,
                charset: None,
                charset_override: None,
                offset: 0,
                page: MIB,
                search: Default::default(),
                reveal_at: None,
                report: None,
                test_revision: 0,
                environment: Values::new(),
                viewed: 0,
            },
        );
        app.response_find.query = "duck".into();

        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0));
        let mut texture_bytes = 0usize;
        let mut samples = vec![];
        for i in 0..40 {
            // Every other frame carries a keystroke, so the measurement includes the work an
            // edit triggers rather than only a repaint of unchanged state.
            let events = if i % 2 == 0 {
                vec![egui::Event::Text("x".into())]
            } else {
                vec![]
            };
            let input = egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            };
            let started = std::time::Instant::now();
            let mut output = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
            samples.push(started.elapsed().as_micros());
            for deltas in output.textures_delta.set.values() {
                for delta in deltas {
                    texture_bytes += match &delta.image {
                        egui::epaint::ImageData::Color(image) => image.pixels.len() * 4,
                    };
                }
            }
            output.textures_delta.clear();
        }
        samples.sort_unstable();
        println!(
            "frame construction: min {} median {} p95 {} max {} microseconds (target 16 ms input-to-paint)",
            samples[0],
            samples[samples.len() / 2],
            samples[samples.len() * 95 / 100],
            samples[samples.len() - 1],
        );
        println!("GPU texture bytes uploaded across 40 frames: {texture_bytes}");
    }
    #[test]
    fn the_disk_change_dialog_renders_and_names_the_request_a_path_belongs_to() {
        let dir = tempfile::tempdir().unwrap();
        let mut collection =
            duckie_storage::Collection::new(dir.path().to_path_buf(), "c".into()).unwrap();
        collection.requests.push(duckie_storage::StoredRequest::new(
            RequestDefinition {
                id: "one".into(),
                name: "Fetch a duck".into(),
                ..Default::default()
            },
            String::new(),
        ));
        collection.save().unwrap();

        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Duckie::new(&cc);
        let mut frame = eframe::Frame::_new_kittest();
        app.use_collection(collection);
        assert_eq!(
            app.request_named("requests/one.request.json").as_deref(),
            Some("Fetch a duck"),
            "a request file is labelled with the request it holds"
        );
        assert_eq!(app.request_named("duckie.json"), None);

        app.disk_changes = vec![
            ("duckie.json".into(), duckie_storage::Change::Modified),
            (
                "requests/one.request.json".into(),
                duckie_storage::Change::Removed,
            ),
        ];
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 820.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
        assert!(!output.shapes.is_empty());
        output.textures_delta.clear();
    }
    #[test]
    fn page_size_shrinks_only_for_very_long_lines() {
        let wrapped: Vec<u8> = "x".repeat(79).into_bytes();
        let mut multiline = vec![];
        for _ in 0..40_000 {
            multiline.extend_from_slice(&wrapped);
            multiline.push(b'\n');
        }
        assert_eq!(
            page_size(&multiline),
            MIB,
            "readable lines keep the full page"
        );
        assert_eq!(page_size(b"short\nlines\n"), MIB);
        assert_eq!(page_size(&[]), MIB);
        // One unbroken line is the minified-JSON case that blew the memory budget.
        let minified = vec![b'x'; 200 * 1024];
        assert!(page_size(&minified) < MIB);
        // A single long line anywhere is enough, even among short ones.
        let mut mixed = b"short\n".to_vec();
        mixed.extend(std::iter::repeat_n(b'x', 8 * 1024));
        assert!(page_size(&mixed) < MIB);
    }
    /// Renders every response tab with a live find query and a pending Go-to-line, so the
    /// highlighting layouter, the gutter painter and the reveal path all execute.
    #[test]
    fn find_and_go_to_line_render_over_a_response() {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let mut app = Duckie::new(&cc);
        let mut frame = eframe::Frame::_new_kittest();
        let id = app.drafts[0].request.id.clone();
        app.drafts[0].source = "test('a', () => {\n  expect(1).toBe(2);\n});\n".into();
        app.responses.insert(
            id.clone(),
            ResponseView {
                tree: JsonTreeState::default(),
                json: Some(serde_json::json!({"duck":1,"duck2":2})),
                result: ExecutionResult {
                    request_id: id.clone(),
                    run_id: "run".into(),
                    summary: RequestSummary {
                        method: "GET".into(),
                        url: "http://localhost/".into(),
                        headers: vec![],
                        environment: "dev".into(),
                        revision: 0,
                        timeout_ms: 30_000,
                    },
                    outcome: Outcome::Complete,
                    status: Some(200),
                    status_text: "OK".into(),
                    headers: vec![("content-type".into(), "application/json".into())],
                    body_error: None,
                    body: BodyHandle::Memory(std::sync::Arc::new(
                        br#"{"duck":1,"duck2":2}"#.to_vec(),
                    )),
                    encoded_bytes: 20,
                    duration_ms: 4,
                    diagnostics: ResponseDiagnostics::default(),
                },
                preview: "{\"duck\":1,\n\"duck2\":2}".into(),
                pretty: None,
                binary: false,
                charset: None,
                charset_override: None,
                offset: 0,
                // Smaller than the body, so the paging controls render too.
                page: 8,
                search: Default::default(),
                reveal_at: None,
                report: Some(TestReport {
                    tests: vec![TestCase {
                        name: "a".into(),
                        passed: false,
                        error: Some("Assertion failed (toBe)".into()),
                        line: Some(2),
                    }],
                    ..Default::default()
                }),
                test_revision: 0,
                environment: Values::new(),
                viewed: 0,
            },
        );
        app.response_find.query = "duck".into();
        app.response_find.index = 1;
        app.editor_find.query = "expect".into();
        app.editor_find.open = true;
        app.goto_line = Some(2);
        app.request_tab = RequestTab::Tests;
        for tab in [
            ResponseTab::Body,
            ResponseTab::Headers,
            ResponseTab::Tests,
            ResponseTab::Details,
        ] {
            app.response_tab = tab;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 820.0),
                )),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
            assert!(!output.shapes.is_empty());
            output.textures_delta.clear();
        }
        // The Tests editor consumes the pending line so a later frame does not fight the caret.
        assert_eq!(app.goto_line, None);
    }
}
