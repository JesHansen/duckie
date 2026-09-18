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
    if field.has_focus() {
        if crate::shortcuts::pressed(ui.ctx(), "find_previous") {
            find.step(total, false);
            reveal = true;
        } else if crate::shortcuts::pressed(ui.ctx(), "find_next") {
            find.step(total, true);
            reveal = true;
        }
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

/// What an editor is holding, which decides how it is coloured.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Syntax {
    None,
    Json,
    JavaScript,
}
struct Palette {
    string: egui::Color32,
    number: egui::Color32,
    keyword: egui::Color32,
    comment: egui::Color32,
    key: egui::Color32,
    punctuation: egui::Color32,
}
fn palette(dark: bool) -> Palette {
    use egui::Color32 as C;
    if dark {
        Palette {
            string: C::from_rgb(152, 195, 121),
            number: C::from_rgb(209, 154, 102),
            keyword: C::from_rgb(198, 120, 221),
            comment: C::from_rgb(127, 136, 150),
            key: C::from_rgb(97, 175, 239),
            punctuation: C::from_rgb(160, 168, 180),
        }
    } else {
        Palette {
            string: C::from_rgb(28, 111, 45),
            number: C::from_rgb(150, 78, 0),
            keyword: C::from_rgb(125, 41, 158),
            comment: C::from_rgb(110, 118, 129),
            key: C::from_rgb(17, 82, 158),
            punctuation: C::from_rgb(90, 98, 110),
        }
    }
}
/// Nudges an index forward onto a character boundary. The scanner works in bytes and can step
/// over an escape into the middle of a multi-byte character, which would panic when sliced.
fn boundary(text: &str, mut at: usize) -> usize {
    while at < text.len() && !text.is_char_boundary(at) {
        at += 1;
    }
    at.min(text.len())
}
fn word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}
/// Largest text that is syntax-coloured.
///
/// Colouring turns one run of text into thousands of separately formatted sections, and laying
/// those out is what costs — measured at 3.7 ms per frame for 25 KiB, 12.9 ms for 100 KiB and
/// 73 ms for a megabyte, against a 16 ms frame budget. Past this size the text draws plain, which
/// keeps a large response readable and scrollable instead of correct-looking and unusable.
pub const MAX_COLOURED: usize = 64 * 1024;

/// Coloured byte ranges, in order and non-overlapping. Everything not returned draws plain.
fn spans(text: &str, syntax: Syntax, palette: &Palette) -> Vec<(Range<usize>, egui::Color32)> {
    if syntax == Syntax::None || text.len() > MAX_COLOURED {
        return vec![];
    }
    let js = syntax == Syntax::JavaScript;
    let bytes = text.as_bytes();
    let mut found = vec![];
    let mut at = 0usize;
    while at < bytes.len() {
        let start = at;
        match bytes[at] {
            b'"' | b'\'' | b'`' if js || bytes[at] == b'"' => {
                let quote = bytes[at];
                at += 1;
                while at < bytes.len() {
                    match bytes[at] {
                        b'\\' => at += 2,
                        b if b == quote => {
                            at += 1;
                            break;
                        }
                        // An unterminated string should colour its line, not the rest of the file.
                        b'\n' if quote != b'`' => break,
                        _ => at += 1,
                    }
                }
                at = boundary(text, at.min(bytes.len()));
                // A JSON string before a colon is a property name, worth telling apart.
                let key = !js
                    && bytes[at..]
                        .iter()
                        .find(|b| !b.is_ascii_whitespace())
                        .is_some_and(|b| *b == b':');
                found.push((start..at, if key { palette.key } else { palette.string }));
            }
            b'/' if js && bytes.get(at + 1) == Some(&b'/') => {
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
                found.push((start..at, palette.comment));
            }
            b'/' if js && bytes.get(at + 1) == Some(&b'*') => {
                at += 2;
                while at + 1 < bytes.len() && !(bytes[at] == b'*' && bytes[at + 1] == b'/') {
                    at += 1;
                }
                at = boundary(text, (at + 2).min(bytes.len()));
                found.push((start..at, palette.comment));
            }
            b'0'..=b'9' => {
                while at < bytes.len()
                    && (bytes[at].is_ascii_digit() || matches!(bytes[at], b'.' | b'e' | b'E'))
                {
                    at += 1;
                }
                found.push((start..at, palette.number));
            }
            b'-' if bytes.get(at + 1).is_some_and(u8::is_ascii_digit) => {
                at += 1;
                while at < bytes.len()
                    && (bytes[at].is_ascii_digit() || matches!(bytes[at], b'.' | b'e' | b'E'))
                {
                    at += 1;
                }
                found.push((start..at, palette.number));
            }
            b if word_byte(b) && !b.is_ascii_digit() => {
                while at < bytes.len() && word_byte(bytes[at]) {
                    at += 1;
                }
                const JSON: [&str; 3] = ["true", "false", "null"];
                const JS: [&str; 14] = [
                    "const", "let", "var", "function", "return", "if", "else", "for", "while",
                    "throw", "new", "typeof", "test", "expect",
                ];
                let word = &text[start..at];
                if JSON.contains(&word) || (js && JS.contains(&word)) {
                    found.push((start..at, palette.keyword));
                }
            }
            b'{' | b'}' | b'[' | b']' | b'(' | b')' | b',' | b':' | b';' => {
                at += 1;
                found.push((start..at, palette.punctuation));
            }
            _ => at += 1,
        }
    }
    found
}
/// The bracket under the caret and its partner, so a pair can be tinted.
///
/// Brackets inside strings and comments are skipped by consulting the coloured spans, which
/// already know where those are, rather than scanning the text a second time.
fn bracket_pair(
    text: &str,
    caret: usize,
    coloured: &[(Range<usize>, egui::Color32)],
    palette: &Palette,
) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let quoted = |at: usize| {
        coloured.iter().any(|(range, colour)| {
            range.contains(&at) && (*colour == palette.string || *colour == palette.comment)
        })
    };
    const OPEN: &[u8] = b"{[(";
    const CLOSE: &[u8] = b"}])";
    // Prefer the bracket the caret sits on, then the one it sits just after.
    let here = [caret, caret.wrapping_sub(1)].into_iter().find(|at| {
        bytes
            .get(*at)
            .is_some_and(|b| OPEN.contains(b) || CLOSE.contains(b))
    })?;
    if quoted(here) {
        return None;
    }
    let byte = bytes[here];
    let forward = OPEN.contains(&byte);
    let (open, close) = if forward {
        (byte, CLOSE[OPEN.iter().position(|b| *b == byte)?])
    } else {
        (OPEN[CLOSE.iter().position(|b| *b == byte)?], byte)
    };
    let mut depth = 0i32;
    let mut at = here;
    loop {
        if !quoted(at) {
            if bytes[at] == open {
                depth += if forward { 1 } else { -1 };
            } else if bytes[at] == close {
                depth += if forward { -1 } else { 1 };
            }
            if depth == 0 {
                return Some((here, at));
            }
        }
        at = if forward { at + 1 } else { at.checked_sub(1)? };
        if at >= bytes.len() {
            return None;
        }
    }
}
fn caret_id(id: &str) -> egui::Id {
    egui::Id::new((id, "caret"))
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
/// How an editor decorates its text for one frame.
pub struct Decoration<'a> {
    /// Every match of the current find query, in order.
    pub found: &'a [Hit],
    /// Which of those is the current one, indexed into `found`.
    pub current: usize,
    /// A character range to select and scroll into view this frame.
    pub reveal: Option<Range<usize>>,
    pub syntax: Syntax,
}
pub fn code(
    ui: &mut Ui,
    id: &str,
    text: &mut dyn TextBuffer,
    rows: usize,
    decoration: Decoration<'_>,
) -> CodeResult {
    let Decoration {
        found,
        current,
        reveal,
        syntax,
    } = decoration;
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
    let colours = palette(ui.visuals().dark_mode);
    let coloured = spans(text.as_str(), syntax, &colours);
    // The caret is only known after the editor runs, so pairing follows it by one frame.
    let caret: Option<usize> = ui.ctx().memory(|m| m.data.get_temp(caret_id(id)));
    let pair = caret.and_then(|at| bracket_pair(text.as_str(), at, &coloured, &colours));
    let bracket = ui.visuals().selection.bg_fill.gamma_multiply(0.6);

    let mut layouter = |ui: &Ui, buffer: &dyn TextBuffer, _wrap: f32| {
        let text = buffer.as_str();
        let mut job = LayoutJob {
            wrap: egui::text::TextWrapping::no_max_width(),
            keep_trailing_whitespace: true,
            ..Default::default()
        };
        // Every position where either the colour or the background changes becomes a cut, so the
        // two are composed rather than one overriding the other.
        let mut cuts = vec![0usize, text.len()];
        for (range, _) in &coloured {
            cuts.push(range.start);
            cuts.push(range.end);
        }
        for hit in found {
            cuts.push(hit.bytes.start);
            cuts.push(hit.bytes.end);
        }
        if let Some((a, b)) = pair {
            cuts.extend([a, a + 1, b, b + 1]);
        }
        cuts.retain(|at| *at <= text.len() && text.is_char_boundary(*at));
        cuts.sort_unstable();
        cuts.dedup();
        // Both lists are sorted, so a cursor over each keeps this linear in the number of cuts.
        let (mut colour_at, mut hit_at) = (0usize, 0usize);
        for window in cuts.windows(2) {
            let (from, to) = (window[0], window[1]);
            while colour_at < coloured.len() && coloured[colour_at].0.end <= from {
                colour_at += 1;
            }
            while hit_at < found.len() && found[hit_at].bytes.end <= from {
                hit_at += 1;
            }
            let colour = coloured
                .get(colour_at)
                .filter(|(range, _)| range.contains(&from))
                .map_or(plain.color, |(_, colour)| *colour);
            let background = match found.get(hit_at) {
                Some(hit) if hit.bytes.contains(&from) => {
                    if hit_at == current {
                        selected
                    } else {
                        other
                    }
                }
                _ if pair.is_some_and(|(a, b)| from == a || from == b) => bracket,
                _ => egui::Color32::TRANSPARENT,
            };
            job.append(
                &text[from..to],
                0.0,
                TextFormat {
                    color: colour,
                    background,
                    ..plain.clone()
                },
            );
        }
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
            // Remember where the caret is for the next frame's bracket pairing.
            let caret = output.cursor_range.map_or(usize::MAX, |range| {
                let text = output.galley.text();
                text.char_indices()
                    .nth(range.primary.index.0)
                    .map_or(text.len(), |(at, _)| at)
            });
            ui.ctx()
                .memory_mut(|m| m.data.insert_temp(caret_id(id), caret));
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
    /// Names each coloured run so a test can talk about intent rather than RGB values.
    fn kinds(text: &str, syntax: Syntax, palette: &Palette) -> Vec<(&'static str, String)> {
        spans(text, syntax, palette)
            .into_iter()
            .map(|(range, colour)| {
                let kind = match colour {
                    c if c == palette.string => "string",
                    c if c == palette.number => "number",
                    c if c == palette.keyword => "keyword",
                    c if c == palette.comment => "comment",
                    c if c == palette.key => "key",
                    _ => "punct",
                };
                (kind, text[range].to_string())
            })
            .collect()
    }
    fn dark() -> Palette {
        palette(true)
    }
    #[test]
    fn json_colouring_tells_keys_from_string_values() {
        let palette = dark();
        assert_eq!(
            kinds(
                r#"{"name": "Duckie", "n": -1.5e3, "ok": true}"#,
                Syntax::Json,
                &palette
            ),
            [
                ("punct", "{".into()),
                ("key", "\"name\"".into()),
                ("punct", ":".into()),
                ("string", "\"Duckie\"".into()),
                ("punct", ",".into()),
                ("key", "\"n\"".into()),
                ("punct", ":".into()),
                ("number", "-1.5e3".into()),
                ("punct", ",".into()),
                ("key", "\"ok\"".into()),
                ("punct", ":".into()),
                ("keyword", "true".into()),
                ("punct", "}".into()),
            ]
        );
    }
    #[test]
    fn javascript_colouring_covers_comments_and_both_quote_styles() {
        let palette = dark();
        let coloured = kinds(
            "// note\nconst a = 'x'; /* b */ `t`",
            Syntax::JavaScript,
            &palette,
        );
        assert!(
            coloured.contains(&("comment", "// note".into())),
            "{coloured:?}"
        );
        assert!(
            coloured.contains(&("comment", "/* b */".into())),
            "{coloured:?}"
        );
        assert!(
            coloured.contains(&("keyword", "const".into())),
            "{coloured:?}"
        );
        assert!(coloured.contains(&("string", "'x'".into())), "{coloured:?}");
        assert!(coloured.contains(&("string", "`t`".into())), "{coloured:?}");
        // Single quotes are not strings in JSON, so the same text colours differently.
        let json = kinds("'x'", Syntax::Json, &palette);
        assert!(!json.iter().any(|(kind, _)| *kind == "string"), "{json:?}");
    }
    #[test]
    fn colouring_survives_multibyte_text_and_unterminated_strings() {
        let palette = dark();
        // An escape before a multi-byte character must not leave an index mid-character.
        for text in [
            r#"{"k": "a\é", "n": 1}"#,
            "\"unterminated\nnext",
            "ü\"v\"",
            "",
        ] {
            let coloured = spans(text, Syntax::Json, &palette);
            for (range, _) in &coloured {
                assert!(
                    text.is_char_boundary(range.start) && text.is_char_boundary(range.end),
                    "{text:?} produced {range:?}"
                );
                let _ = &text[range.clone()];
            }
        }
    }
    #[test]
    fn bracket_pairing_matches_across_nesting_and_ignores_quoted_brackets() {
        let palette = dark();
        let text = r#"{"a": [1, {"b": "}"}]}"#;
        let coloured = spans(text, Syntax::Json, &palette);
        // The outer braces pair with each other, past a nested object and a brace in a string.
        assert_eq!(
            bracket_pair(text, 0, &coloured, &palette),
            Some((0, text.len() - 1))
        );
        // From the closing brace the search runs backwards to the same pair.
        assert_eq!(
            bracket_pair(text, text.len() - 1, &coloured, &palette),
            Some((text.len() - 1, 0))
        );
        let open = text.find('[').unwrap();
        assert_eq!(
            bracket_pair(text, open, &coloured, &palette),
            Some((open, text.rfind(']').unwrap()))
        );
        // A brace inside a string is not a bracket at all.
        let quoted = text.find("\"}\"").unwrap() + 1;
        assert_eq!(bracket_pair(text, quoted, &coloured, &palette), None);
        // Nothing to pair with leaves no highlight rather than guessing.
        assert_eq!(bracket_pair("{", 0, &[], &palette), None);
        assert_eq!(bracket_pair("abc", 1, &[], &palette), None);
    }
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
