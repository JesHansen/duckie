//! Shared code-editor chrome: a pinned line-number gutter, and find with real match navigation.
use eframe::egui::{
    self, Align, Rect, TextBuffer, Ui,
    text::{CCursor, CCursorRange, LayoutJob, TextFormat},
};
use std::ops::Range;

/// A single occurrence, in both the byte range a `LayoutJob` section needs and the
/// character range a `Galley` cursor needs.
pub struct Hit {
    pub bytes: Range<usize>,
    pub chars: Range<usize>,
}
/// Bounds the per-frame cost of a one-character query over a megabyte preview page.
pub const MAX_HITS: usize = 5_000;

/// Case-sensitive, non-overlapping occurrences of `query`, in one pass over the text.
pub fn hits(text: &str, query: &str) -> Vec<Hit> {
    if query.is_empty() {
        return vec![];
    }
    let mut found = vec![];
    let (mut byte, mut chars) = (0usize, 0usize);
    for (start, hit) in text.match_indices(query) {
        chars += text[byte..start].chars().count();
        byte = start;
        found.push(Hit {
            bytes: start..start + hit.len(),
            chars: chars..chars + hit.chars().count(),
        });
        if found.len() == MAX_HITS {
            break;
        }
    }
    found
}
/// Character range of a one-based line, used to reveal a reported assertion failure.
pub fn line_range(text: &str, line: u32) -> Option<Range<usize>> {
    let mut start = 0usize;
    for (number, content) in text.lines().enumerate() {
        if number as u32 + 1 == line {
            return Some(start..start + content.chars().count());
        }
        start += content.chars().count() + 1;
    }
    None
}

/// One find session. The index is clamped rather than reset so navigation survives edits.
#[derive(Default)]
pub struct Find {
    pub query: String,
    pub index: usize,
    pub open: bool,
}
impl Find {
    fn step(&mut self, total: usize, forward: bool) {
        if total == 0 {
            self.index = 0;
        } else if forward {
            self.index = (self.index + 1) % total;
        } else {
            self.index = (self.index + total - 1) % total;
        }
    }
}

/// Find field, match counter and navigation. Returns true when the caller should reveal
/// the current match: the query changed, or the user stepped to another one.
pub fn find_bar(ui: &mut Ui, id: &str, find: &mut Find, total: usize) -> bool {
    let mut reveal = false;
    let previous = find.query.clone();
    let field = ui.add(
        egui::TextEdit::singleline(&mut find.query)
            .id(egui::Id::new(id))
            .hint_text("Find…   Ctrl+F")
            .desired_width(160.0),
    );
    if find.query != previous {
        find.index = 0;
        reveal = true;
    }
    find.index = find.index.min(total.saturating_sub(1));
    // Enter steps forward and Shift+Enter back, matching the buttons beside the field.
    if field.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        find.step(total, !ui.input(|i| i.modifiers.shift));
        reveal = true;
    }
    let enabled = total > 0;
    if ui
        .add_enabled(enabled, egui::Button::new("◀"))
        .on_hover_text("Previous match · Shift+Enter")
        .clicked()
    {
        find.step(total, false);
        reveal = true;
    }
    if ui
        .add_enabled(enabled, egui::Button::new("▶"))
        .on_hover_text("Next match · Enter")
        .clicked()
    {
        find.step(total, true);
        reveal = true;
    }
    if !find.query.is_empty() {
        ui.weak(if total == 0 {
            "No matches".to_string()
        } else if total == MAX_HITS {
            format!("{} of first {MAX_HITS}", find.index + 1)
        } else {
            format!("{} of {total}", find.index + 1)
        });
    }
    reveal && !find.query.is_empty()
}

pub struct CodeResult {
    pub changed: bool,
    pub focused: bool,
}
/// A monospace editor with a line-number gutter, highlighted find matches, and an optional
/// character range to select and scroll into view this frame.
///
/// Text deliberately never wraps: a wrapped line would break the gutter's one-number-per-line
/// mapping, and horizontal scrolling is the normal expectation in a code editor.
pub fn code(
    ui: &mut Ui,
    id: &str,
    text: &mut dyn TextBuffer,
    rows: usize,
    found: &[Hit],
    current: usize,
    reveal: Option<Range<usize>>,
) -> CodeResult {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let line_height = ui.fonts_mut(|f| f.row_height(&font)) + ui.spacing().extra_text_line_spacing;
    let digits = text.as_str().lines().count().max(1).ilog10() as usize + 1;
    let width = ui.fonts_mut(|f| f.glyph_width(&font, '0'));
    let gutter = width * digits.max(2) as f32 + 12.0;
    let plain = TextFormat {
        font_id: font.clone(),
        color: ui.visuals().text_color(),
        line_height: Some(line_height),
        ..Default::default()
    };
    let selected = ui.visuals().selection.bg_fill;
    let other = selected.gamma_multiply(0.4);
    let mut layouter = |ui: &Ui, buffer: &dyn TextBuffer, _wrap: f32| {
        let text = buffer.as_str();
        let mut job = LayoutJob {
            wrap: egui::text::TextWrapping::no_max_width(),
            keep_trailing_whitespace: true,
            ..Default::default()
        };
        let mut at = 0usize;
        for (index, hit) in found.iter().enumerate() {
            if at < hit.bytes.start {
                job.append(&text[at..hit.bytes.start], 0.0, plain.clone());
            }
            job.append(
                &text[hit.bytes.clone()],
                0.0,
                TextFormat {
                    background: if index == current { selected } else { other },
                    ..plain.clone()
                },
            );
            at = hit.bytes.end;
        }
        job.append(&text[at..], 0.0, plain.clone());
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let mut result = CodeResult {
        changed: false,
        focused: false,
    };
    egui::ScrollArea::both().id_salt(id).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.add_space(gutter);
            let output = egui::TextEdit::multiline(text)
                .id_salt(id)
                .font(egui::TextStyle::Monospace)
                .code_editor()
                .desired_width(f32::INFINITY)
                .desired_rows(rows)
                .layouter(&mut layouter)
                .show(ui);
            result.changed = output.response.response.changed();
            result.focused = output.response.response.has_focus();
            if let Some(range) = reveal {
                let (from, to) = (CCursor::new(range.start), CCursor::new(range.end));
                let mut state = output.state.clone();
                state.cursor.set_char_range(Some(CCursorRange {
                    primary: to,
                    secondary: from,
                    h_pos: None,
                }));
                state.store(ui.ctx(), output.response.response.id);
                let offset = output.galley_pos.to_vec2();
                let target = output
                    .galley
                    .pos_from_cursor(from)
                    .union(output.galley.pos_from_cursor(to))
                    .translate(offset);
                ui.scroll_to_rect(target, Some(Align::Center));
            }
            paint_gutter(ui, gutter, &output.galley, output.galley_pos.y);
        });
    });
    result
}
/// Numbers are painted last, over an opaque strip pinned to the viewport's left edge, so the
/// gutter stays put while the text scrolls horizontally beneath it.
fn paint_gutter(ui: &Ui, width: f32, galley: &egui::Galley, top: f32) {
    let clip = ui.clip_rect();
    let painter = ui.painter();
    let strip = Rect::from_min_max(
        egui::pos2(clip.left(), clip.top()),
        egui::pos2(clip.left() + width, clip.bottom()),
    );
    painter.rect_filled(strip, 0.0, ui.visuals().extreme_bg_color);
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let color = ui.visuals().weak_text_color();
    let mut number = 1u32;
    let mut new_line = true;
    for row in &galley.rows {
        let y = top + row.pos.y;
        if new_line && y + row.row.size.y >= clip.top() && y <= clip.bottom() {
            painter.text(
                egui::pos2(strip.right() - 6.0, y),
                egui::Align2::RIGHT_TOP,
                number,
                font.clone(),
                color,
            );
        }
        if new_line {
            number += 1;
        }
        new_line = row.ends_with_newline;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hits_report_byte_and_character_ranges_past_multibyte_text() {
        let text = "ü ab ü ab";
        let found = hits(text, "ab");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].bytes, 3..5);
        assert_eq!(found[0].chars, 2..4);
        // The second match sits after two 2-byte characters, so bytes and chars diverge.
        assert_eq!(found[1].bytes, 9..11);
        assert_eq!(found[1].chars, 7..9);
        assert!(hits(text, "").is_empty());
        assert!(hits(text, "zz").is_empty());
    }
    #[test]
    fn hits_are_capped() {
        let text = "a".repeat(MAX_HITS + 50);
        assert_eq!(hits(&text, "a").len(), MAX_HITS);
    }
    #[test]
    fn line_range_selects_whole_lines_by_one_based_number() {
        let text = "first\nsecond\nthird";
        assert_eq!(line_range(text, 1), Some(0..5));
        assert_eq!(line_range(text, 2), Some(6..12));
        assert_eq!(line_range(text, 3), Some(13..18));
        assert_eq!(line_range(text, 4), None);
        assert_eq!(line_range(text, 0), None);
    }
    #[test]
    fn stepping_wraps_in_both_directions_and_survives_an_empty_result() {
        let mut find = Find::default();
        find.step(3, true);
        assert_eq!(find.index, 1);
        find.step(3, false);
        assert_eq!(find.index, 0);
        find.step(3, false);
        assert_eq!(find.index, 2);
        find.step(3, true);
        assert_eq!(find.index, 0);
        find.step(0, true);
        assert_eq!(find.index, 0);
    }
}
