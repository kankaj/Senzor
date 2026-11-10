use std::fmt;

use crate::i18n::{Arg, Language, Text, text};

/// One problem found while loading a project. All problems are collected, not just the first.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{location}: {message}")]
pub struct ProjectError {
    pub location: Location,
    /// The problem in English, as the CLI, MCP and the docs show it.
    pub message: String,
    /// The TOML key path, relative to the location's table, of the value the problem is about:
    /// `port`, `tls.ca_file`, `fields.co2`, `messages[1].payload`, `availability.will_delay`.
    /// Messages count from 1, as the problem texts do. `None` when no single value is to blame.
    pub field: Option<String>,
    /// The problem as a translatable text; `message` is its English rendering.
    pub text: Text,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Location {
    /// A position in the TOML text (1-based, column counted in characters).
    Toml {
        line: usize,
        column: usize,
    },
    /// The file as a whole, when no position is known.
    File,
    Broker,
    Template {
        id: String,
    },
    Device {
        id: String,
    },
}

impl Location {
    /// The location as a translatable text.
    pub fn text(&self) -> Text {
        match self {
            Location::Toml { line, column } => {
                text!("location-line", line = *line, column = *column)
            }
            Location::File => text!("location-file"),
            Location::Broker => text!("location-broker"),
            Location::Template { id } => text!("location-template", id = id),
            Location::Device { id } => text!("location-device", id = id),
        }
    }

    /// The location in `lang`: `line 3, column 4`, ``template `air` ``, …
    pub fn to_string_in(&self, lang: Language) -> String {
        self.text().render(lang)
    }
}

/// The location in English.
impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.text().fmt(f)
    }
}

impl ProjectError {
    pub fn new(location: Location, text: Text) -> Self {
        Self {
            location,
            message: text.to_string(),
            field: None,
            text,
        }
    }

    /// The same problem, naming `field` as the value it is about.
    pub fn at(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    pub fn broker(text: Text) -> Self {
        Self::new(Location::Broker, text)
    }

    pub fn template(id: &str, text: Text) -> Self {
        Self::new(Location::Template { id: id.to_string() }, text)
    }

    pub fn device(id: &str, text: Text) -> Self {
        Self::new(Location::Device { id: id.to_string() }, text)
    }

    /// The problem's catalog key, such as `device-unknown-template`.
    pub fn key(&self) -> &'static str {
        self.text.key()
    }

    /// The values of the problem's variables.
    pub fn args(&self) -> &[(&'static str, Arg)] {
        self.text.args()
    }

    /// The problem in `lang`, without its location. In English it is [`Self::message`].
    pub fn message_in(&self, lang: Language) -> String {
        self.text.render(lang)
    }

    /// `<location>: <message>` in `lang`. In English it is `to_string()`.
    pub fn to_string_in(&self, lang: Language) -> String {
        format!(
            "{}: {}",
            self.location.to_string_in(lang),
            self.message_in(lang)
        )
    }
}

/// Converts a byte offset into a 1-based line and character column.
pub(crate) fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let column = text[line_start..offset].chars().count() + 1;
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_column_counts_characters_not_bytes() {
        let text = "# čidlo CO2\nkey = 1";
        assert_eq!(line_column(text, 0), (1, 1));
        let offset = text.find("CO2").unwrap();
        assert_eq!(line_column(text, offset), (1, 9));
        let offset = text.find("key").unwrap();
        assert_eq!(line_column(text, offset), (2, 1));
    }

    #[test]
    fn a_problem_names_its_field_without_changing_its_text() {
        let err = ProjectError::broker(text!("broker-port-zero"));
        assert_eq!(err.field, None);
        let err = err.at("port");
        assert_eq!(err.field.as_deref(), Some("port"));
        assert_eq!(err.to_string(), "broker: port must not be 0");
    }

    fn unknown_template() -> ProjectError {
        ProjectError::device("office", text!("device-unknown-template", template = "x"))
    }

    #[test]
    fn display_includes_location_and_message() {
        let err = unknown_template();
        assert_eq!(err.to_string(), "device `office`: unknown template `x`");
    }

    #[test]
    fn a_problem_shows_in_czech_with_its_location() {
        let err = unknown_template();
        assert_eq!(err.message, "unknown template `x`");
        assert_eq!(err.message_in(Language::English), err.message);
        assert_eq!(err.to_string_in(Language::English), err.to_string());
        assert_eq!(err.key(), "device-unknown-template");
        assert_eq!(err.args(), [("template", Arg::Str("x".into()))]);
        assert_eq!(err.message_in(Language::Czech), "neznámá šablona `x`");
        assert_eq!(
            err.to_string_in(Language::Czech),
            "zařízení `office`: neznámá šablona `x`"
        );
    }

    #[test]
    fn locations_show_in_english_and_czech() {
        let cases = [
            (
                Location::Toml { line: 3, column: 4 },
                "line 3, column 4",
                "řádek 3, sloupec 4",
            ),
            (Location::File, "project file", "soubor projektu"),
            (Location::Broker, "broker", "broker"),
            (
                Location::Template { id: "air".into() },
                "template `air`",
                "šablona `air`",
            ),
            (
                Location::Device {
                    id: "office".into(),
                },
                "device `office`",
                "zařízení `office`",
            ),
        ];
        for (location, english, czech) in cases {
            assert_eq!(location.to_string(), english);
            assert_eq!(location.to_string_in(Language::English), english);
            assert_eq!(location.to_string_in(Language::Czech), czech);
        }
    }
}
