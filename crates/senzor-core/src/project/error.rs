use std::fmt;

/// One problem found while loading a project. All problems are collected, not just the first.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{location}: {message}")]
pub struct ProjectError {
    pub location: Location,
    pub message: String,
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

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Location::Toml { line, column } => write!(f, "line {line}, column {column}"),
            Location::File => write!(f, "project file"),
            Location::Broker => write!(f, "broker"),
            Location::Template { id } => write!(f, "template `{id}`"),
            Location::Device { id } => write!(f, "device `{id}`"),
        }
    }
}

impl ProjectError {
    pub fn new(location: Location, message: impl Into<String>) -> Self {
        Self {
            location,
            message: message.into(),
        }
    }

    pub fn broker(message: impl Into<String>) -> Self {
        Self::new(Location::Broker, message)
    }

    pub fn template(id: &str, message: impl Into<String>) -> Self {
        Self::new(Location::Template { id: id.to_string() }, message)
    }

    pub fn device(id: &str, message: impl Into<String>) -> Self {
        Self::new(Location::Device { id: id.to_string() }, message)
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
    fn display_includes_location_and_message() {
        let err = ProjectError::device("office", "unknown template `x`");
        assert_eq!(err.to_string(), "device `office`: unknown template `x`");
    }
}
