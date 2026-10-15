//! Text of inputs whose content may be invalid or not yet committed (durations, names, overrides).

use std::collections::HashMap;

/// One input's text and the model text it was created from.
#[derive(Debug, Clone, PartialEq)]
pub struct Buffer {
    pub text: String,
    origin: String,
}

impl Buffer {
    fn new(model: &str) -> Self {
        Self {
            text: model.to_string(),
            origin: model.to_string(),
        }
    }

    /// The widget wrote `model` from this text; keep the text as the user spelled it.
    pub fn committed(&mut self, model: &str) {
        self.origin = model.to_string();
    }
}

/// Buffers keyed per widget. A buffer starts over from the model whenever the model changed
/// somewhere else, so a box never shows text that does not match what will be saved.
#[derive(Debug, Default)]
pub struct Buffers(HashMap<String, Buffer>);

impl Buffers {
    pub fn get(&mut self, key: &str, model: &str) -> &mut Buffer {
        let buffer = self
            .0
            .entry(key.to_string())
            .or_insert_with(|| Buffer::new(model));
        if buffer.origin != model {
            *buffer = Buffer::new(model);
        }
        buffer
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_buffer_starts_from_the_model() {
        let mut buffers = Buffers::default();
        assert_eq!(buffers.get("k", "10s").text, "10s");
    }

    #[test]
    fn typed_text_is_kept_while_the_model_is_unchanged() {
        let mut buffers = Buffers::default();
        buffers.get("k", "10s").text.push('x');
        assert_eq!(buffers.get("k", "10s").text, "10sx");
    }

    #[test]
    fn a_model_change_from_elsewhere_starts_the_buffer_over() {
        let mut buffers = Buffers::default();
        buffers.get("k", "10s").text.push_str("5s");
        assert_eq!(buffers.get("k", "1m").text, "1m");
    }

    #[test]
    fn a_committed_buffer_keeps_its_spelling() {
        let mut buffers = Buffers::default();
        let buffer = buffers.get("k", "10s");
        buffer.text = "10s 5s".into();
        buffer.committed("15s");
        assert_eq!(buffers.get("k", "15s").text, "10s 5s");
    }

    #[test]
    fn clear_forgets_everything() {
        let mut buffers = Buffers::default();
        buffers.get("k", "a").text.push('b');
        buffers.clear();
        assert_eq!(buffers.get("k", "a").text, "a");
    }
}
