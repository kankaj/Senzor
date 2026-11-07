//! Writing topics and payloads: colours for Jinja blocks and JSON, the variables a box can use, and
//! completing `{{ name }}`.

use std::ops::Range;

use eframe::egui;

/// What a piece of a topic or payload is, for its colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    Plain,
    /// `{{ … }}` or `{% … %}`, braces included.
    Jinja,
    /// A JSON string, quotes included.
    String,
    Number,
    /// `{ } [ ] : ,` outside Jinja blocks.
    Punctuation,
}

/// `text` cut into spans, in order, covering every byte.
pub fn spans(text: &str) -> Vec<(Range<usize>, Span)> {
    let mut spans: Vec<(Range<usize>, Span)> = Vec::new();
    let mut push = |range: Range<usize>, span: Span| match spans.last_mut() {
        // Plain text runs together into one span.
        Some((last, Span::Plain)) if span == Span::Plain && last.end == range.start => {
            last.end = range.end;
        }
        _ => spans.push((range, span)),
    };
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let end = if rest.starts_with("{{") || rest.starts_with("{%") {
            let close = if rest.starts_with("{{") { "}}" } else { "%}" };
            let end = rest[2..]
                .find(close)
                .map_or(text.len(), |at| i + 2 + at + 2);
            push(i..end, Span::Jinja);
            end
        } else if bytes[i] == b'"' {
            let mut j = i + 1;
            while j < text.len() && bytes[j] != b'"' {
                j += if bytes[j] == b'\\' { 2 } else { 1 };
            }
            let end = (j + 1).min(text.len());
            push(i..end, Span::String);
            end
        } else if starts_number(text, i) {
            let len = rest
                .char_indices()
                .skip(1)
                .find(|(_, c)| !(c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-')))
                .map_or(rest.len(), |(at, _)| at);
            push(i..i + len, Span::Number);
            i + len
        } else if matches!(bytes[i], b'{' | b'}' | b'[' | b']' | b':' | b',') {
            push(i..i + 1, Span::Punctuation);
            i + 1
        } else {
            let len = rest.chars().next().map_or(1, char::len_utf8);
            push(i..i + len, Span::Plain);
            i + len
        };
        i = end;
    }
    spans
}

/// Whether a number starts at byte `i`: a digit, or `-` before one, not inside a word.
fn starts_number(text: &str, i: usize) -> bool {
    let rest = &text[i..];
    let mut chars = rest.chars();
    let first = chars.next();
    let digit_first = match first {
        Some('-') => chars.next().is_some_and(|c| c.is_ascii_digit()),
        Some(c) => c.is_ascii_digit(),
        None => false,
    };
    let after_word = text[..i]
        .chars()
        .next_back()
        .is_some_and(|c| c.is_alphanumeric() || c == '_');
    digit_first && !after_word
}

/// Which box a text is for: the availability topic sees fewer variables than a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextKind {
    AvailabilityTopic,
    Message,
}

/// A variable a box can use, with what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    pub about: String,
}

/// The variables `kind` can use: the built-in ones, then the template's `fields` (messages only)
/// and the `params` its devices set.
pub fn variables(kind: TextKind, fields: &[String], params: &[String]) -> Vec<Variable> {
    let mut variables: Vec<Variable> = [
        ("device_id", "The instance ID, such as office-3"),
        ("index", "The instance's position in its device: 0, 1, 2, …"),
        ("template_id", "This template's ID"),
    ]
    .into_iter()
    .map(|(name, about)| Variable {
        name: name.into(),
        about: about.into(),
    })
    .collect();
    if kind == TextKind::Message {
        for (name, about) in [
            ("seq", "The number of the tick in this run, from 0"),
            ("now_ms", "The time of the tick, in milliseconds since 1970"),
            (
                "now_iso",
                "The time of the tick, such as 2026-11-07T12:00:00.000Z",
            ),
        ] {
            variables.push(Variable {
                name: name.into(),
                about: about.into(),
            });
        }
        variables.extend(fields.iter().map(|field| Variable {
            name: field.clone(),
            about: "A field's value on this tick".into(),
        }));
    }
    variables.extend(params.iter().map(|param| Variable {
        name: param.clone(),
        about: "A param that devices using this template set".into(),
    }));
    variables
}

/// The unfinished variable name just before `cursor` (a byte offset) inside an open `{{`: where
/// it starts and what is typed so far. `None` outside a `{{ … }}` block.
pub fn completion_at(text: &str, cursor: usize) -> Option<(usize, String)> {
    let before = text.get(..cursor)?;
    let open = before.rfind("{{")?;
    let inside = &before[open + 2..];
    if inside.contains("}}") {
        return None;
    }
    let typed: String = inside
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let start = cursor - typed.len();
    // Only spaces may come between `{{` and the name; anything else is a filter or an expression.
    text[open + 2..start]
        .trim()
        .is_empty()
        .then_some((start, typed))
}

/// `text` with the name being typed at `cursor` replaced by `name`, closed with ` }}` unless the
/// block is already closed; returns the text and the cursor after the insertion.
pub fn complete(text: &str, cursor: usize, name: &str) -> (String, usize) {
    let start = completion_at(text, cursor).map_or(cursor, |(start, _)| start);
    let rest = &text[cursor..];
    let closing = if rest.trim_start().starts_with("}}") {
        ""
    } else {
        " }}"
    };
    let completed = format!("{}{name}{closing}{rest}", &text[..start]);
    (completed, start + name.len() + closing.len())
}

/// `text` with `{{ name }}` inserted at `cursor`; returns the text and the cursor after it.
pub fn insert_variable(text: &str, cursor: usize, name: &str) -> (String, usize) {
    let block = format!("{{{{ {name} }}}}");
    let cursor = cursor.min(text.len());
    let inserted = format!("{}{block}{}", &text[..cursor], &text[cursor..]);
    (inserted, cursor + block.len())
}

/// A topic or payload box: Jinja blocks and JSON in colour, suggestions while a `{{ name` is
/// typed (Tab takes the first), and the box's text kept under `id` so [`insert_menu`] can reach
/// its cursor.
pub fn edit(
    ui: &mut egui::Ui,
    text: &mut String,
    id: egui::Id,
    multiline: bool,
    variables: &[Variable],
) -> egui::Response {
    let cursor = cursor_byte(ui.ctx(), id, text);
    let suggestions: Vec<&Variable> = cursor
        .and_then(|cursor| completion_at(text, cursor).map(|found| (cursor, found)))
        .map(|(_, (_, typed))| {
            variables
                .iter()
                .filter(|v| v.name.starts_with(&typed) && v.name != typed)
                .collect()
        })
        .unwrap_or_default();
    let focused = ui.memory(|m| m.has_focus(id));
    if focused
        && let (Some(cursor), Some(first)) = (cursor, suggestions.first())
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab))
    {
        let (completed, after) = complete(text, cursor, &first.name);
        *text = completed;
        set_cursor(ui.ctx(), id, text, after);
    }

    let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
        let mut job = layout_job(ui, buffer.as_str(), multiline);
        job.wrap.max_width = wrap_width;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    };
    let edit = if multiline {
        egui::TextEdit::multiline(text)
            .desired_rows(3)
            .code_editor()
    } else {
        egui::TextEdit::singleline(text)
    };
    let response = ui.add(
        edit.id(id)
            .desired_width(f32::INFINITY)
            .layouter(&mut layouter),
    );

    if response.has_focus() && !suggestions.is_empty() {
        ui.horizontal_wrapped(|ui| {
            ui.weak("Tab:");
            for variable in suggestions.iter().take(8) {
                let chip = ui
                    .small_button(&variable.name)
                    .on_hover_text(&variable.about);
                if chip.clicked()
                    && let Some(cursor) = cursor
                {
                    let (completed, after) = complete(text, cursor, &variable.name);
                    *text = completed;
                    set_cursor(ui.ctx(), id, text, after);
                    ui.memory_mut(|m| m.request_focus(id));
                }
            }
        });
    }
    response
}

/// The Insert variable menu for the box with `id`: each variable `text` can use, inserted as
/// `{{ name }}` at the box's cursor (or at the end), with the box taking the keyboard.
/// Screen readers and tests know it as "Insert variable: " followed by `name`, the box's label.
pub fn insert_menu(
    ui: &mut egui::Ui,
    id: egui::Id,
    name: &str,
    text: &mut String,
    variables: &[Variable],
) {
    let menu = ui.menu_button(
        egui::RichText::new(egui_phosphor::regular::BRACKETS_CURLY).weak(),
        |ui| {
            for variable in variables {
                if ui
                    .button(&variable.name)
                    .on_hover_text(&variable.about)
                    .clicked()
                {
                    let cursor = cursor_byte(ui.ctx(), id, text).unwrap_or(text.len());
                    let (inserted, after) = insert_variable(text, cursor, &variable.name);
                    *text = inserted;
                    set_cursor(ui.ctx(), id, text, after);
                    ui.memory_mut(|m| m.request_focus(id));
                    ui.close();
                }
            }
        },
    );
    let response = menu.response.on_hover_text("Insert a variable");
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            true,
            format!("Insert variable: {name}"),
        )
    });
}

/// The box's cursor as a byte offset into `text`, if it has one.
fn cursor_byte(ctx: &egui::Context, id: egui::Id, text: &str) -> Option<usize> {
    let state = egui::text_edit::TextEditState::load(ctx, id)?;
    let chars = state.cursor.char_range()?.primary.index.0;
    Some(
        text.char_indices()
            .nth(chars)
            .map_or(text.len(), |(byte, _)| byte),
    )
}

/// Puts the box's cursor at byte offset `at` of `text`.
fn set_cursor(ctx: &egui::Context, id: egui::Id, text: &str, at: usize) {
    let mut state = egui::text_edit::TextEditState::load(ctx, id).unwrap_or_default();
    let chars = text[..at.min(text.len())].chars().count();
    state
        .cursor
        .set_char_range(Some(egui::text::CCursorRange::one(
            egui::text::CCursor::new(chars),
        )));
    state.store(ctx, id);
}

/// `text` laid out with a colour per [`Span`].
fn layout_job(ui: &egui::Ui, text: &str, multiline: bool) -> egui::text::LayoutJob {
    let visuals = ui.visuals();
    let font = if multiline {
        egui::TextStyle::Monospace.resolve(ui.style())
    } else {
        egui::TextStyle::Body.resolve(ui.style())
    };
    let (string, number) = if visuals.dark_mode {
        (
            egui::Color32::from_rgb(152, 195, 121),
            egui::Color32::from_rgb(229, 192, 123),
        )
    } else {
        (
            egui::Color32::from_rgb(56, 118, 29),
            egui::Color32::from_rgb(170, 95, 0),
        )
    };
    let mut job = egui::text::LayoutJob::default();
    for (range, span) in spans(text) {
        let color = match span {
            Span::Plain => visuals.text_color(),
            Span::Jinja => visuals.selection.stroke.color,
            Span::String => string,
            Span::Number => number,
            Span::Punctuation => visuals.weak_text_color(),
        };
        job.append(
            &text[range],
            0.0,
            egui::TextFormat::simple(font.clone(), color),
        );
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(&str, Span)> {
        spans(text)
            .into_iter()
            .map(|(range, span)| (&text[range], span))
            .collect()
    }

    #[test]
    fn jinja_blocks_json_strings_and_numbers_get_their_own_spans() {
        assert_eq!(
            kinds(r#"{"co2": {{ co2 | int }}, "on": true, "n": -1.5}"#),
            [
                ("{", Span::Punctuation),
                (r#""co2""#, Span::String),
                (":", Span::Punctuation),
                (" ", Span::Plain),
                ("{{ co2 | int }}", Span::Jinja),
                (",", Span::Punctuation),
                (" ", Span::Plain),
                (r#""on""#, Span::String),
                (":", Span::Punctuation),
                (" true", Span::Plain),
                (",", Span::Punctuation),
                (" ", Span::Plain),
                (r#""n""#, Span::String),
                (":", Span::Punctuation),
                (" ", Span::Plain),
                ("-1.5", Span::Number),
                ("}", Span::Punctuation),
            ]
        );
    }

    #[test]
    fn a_topic_and_an_unclosed_block_are_spanned_too() {
        assert_eq!(
            kinds("b/{{ room }}/{% if x %}"),
            [
                ("b/", Span::Plain),
                ("{{ room }}", Span::Jinja),
                ("/", Span::Plain),
                ("{% if x %}", Span::Jinja),
            ]
        );
        assert_eq!(
            kinds("a/{{ ro"),
            [("a/", Span::Plain), ("{{ ro", Span::Jinja)]
        );
        assert_eq!(kinds(""), []);
    }

    #[test]
    fn a_message_sees_more_variables_than_the_availability_topic() {
        let names = |kind| -> Vec<String> {
            variables(kind, &["co2".into()], &["room".into()])
                .into_iter()
                .map(|v| v.name)
                .collect()
        };
        assert_eq!(
            names(TextKind::Message),
            [
                "device_id",
                "index",
                "template_id",
                "seq",
                "now_ms",
                "now_iso",
                "co2",
                "room"
            ]
        );
        assert_eq!(
            names(TextKind::AvailabilityTopic),
            ["device_id", "index", "template_id", "room"]
        );
    }

    #[test]
    fn the_name_being_typed_inside_a_block_is_found() {
        assert_eq!(completion_at("{{ co", 5), Some((3, "co".to_string())));
        assert_eq!(completion_at("x {{", 4), Some((4, String::new())));
        assert_eq!(completion_at("{{co2", 5), Some((2, "co2".to_string())));
        assert_eq!(
            completion_at("{{ co2 }} x", 11),
            None,
            "after a closed block"
        );
        assert_eq!(
            completion_at("{{ co2 | ro", 11),
            None,
            "a filter, not a variable"
        );
        assert_eq!(completion_at("plain co", 8), None);
    }

    #[test]
    fn completing_replaces_the_typed_part_and_closes_the_block() {
        assert_eq!(complete("{{ co", 5, "co2"), ("{{ co2 }}".to_string(), 9));
        assert_eq!(
            complete("a {{ c }} b", 6, "co2"),
            ("a {{ co2 }} b".to_string(), 8),
            "an already closed block stays as it is"
        );
    }

    #[test]
    fn inserting_a_variable_puts_a_block_at_the_cursor() {
        assert_eq!(
            insert_variable("ab", 1, "seq"),
            ("a{{ seq }}b".to_string(), 10)
        );
        assert_eq!(insert_variable("", 0, "seq"), ("{{ seq }}".to_string(), 9));
    }
}
