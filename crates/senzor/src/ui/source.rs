//! The project file as text, to edit by hand: what Save writes, applied as one edit.

use eframe::egui::{self, Key, RichText, TextEdit};
use senzor_core::project::ProjectFile;

use super::{Action, ERROR, UiState, WARNING};
use crate::document::Document;
use crate::i18n::t;

/// The text in the editor, and what the document's text was when the editor took it.
#[derive(Debug, Clone, Default)]
pub struct SourceBuffer {
    pub text: String,
    /// The document's text the editor last showed or applied; the editor follows the document
    /// while `text` still equals it.
    origin: String,
    /// The document's revision `origin` was made at, so it is written again only after a change.
    revision: Option<u64>,
    /// Why the last Apply was refused: the text does not parse as a project.
    pub errors: Vec<String>,
}

impl SourceBuffer {
    /// Whether text was typed that is not applied yet.
    pub fn is_edited(&self) -> bool {
        self.text != self.origin
    }

    /// Takes the document's current text unless something typed would be lost.
    fn follow(&mut self, doc: &Document) {
        if self.revision == Some(doc.revision()) {
            return;
        }
        let current = doc.text_to_save();
        self.revision = Some(doc.revision());
        if !self.is_edited() {
            self.text.clone_from(&current);
            self.errors.clear();
        }
        self.origin = current;
    }
}

pub fn show(ui: &mut egui::Ui, doc: &Document, ui_state: &mut UiState, actions: &mut Vec<Action>) {
    let buffer = ui_state.source.get_or_insert_with(SourceBuffer::default);
    buffer.follow(doc);
    ui.heading(t!("source-title"));
    ui.label(RichText::new(t!("source-intro")).weak());
    let edited = buffer.is_edited();
    let mut apply = false;
    ui.horizontal(|ui| {
        apply = ui
            .add_enabled(edited, egui::Button::new(t!("source-apply")))
            .on_hover_text("Ctrl+Enter")
            .clicked();
        if ui
            .add_enabled(edited, egui::Button::new(t!("source-revert")))
            .clicked()
        {
            buffer.text.clone_from(&buffer.origin);
            buffer.errors.clear();
        }
        if edited {
            ui.label(RichText::new(t!("source-not-applied")).color(WARNING));
        }
    });
    for error in &buffer.errors {
        ui.label(RichText::new(error).color(ERROR));
    }
    let label = ui.label(t!("source-text"));
    let editor = ui
        .add(
            TextEdit::multiline(&mut buffer.text)
                .code_editor()
                .desired_width(f32::INFINITY)
                .desired_rows(30),
        )
        .labelled_by(label.id);
    if editor.has_focus() && ui.input(|i| i.modifiers.command && i.key_pressed(Key::Enter)) {
        apply = edited;
    }
    if apply {
        match ProjectFile::parse(&buffer.text) {
            Ok(_) => {
                buffer.errors.clear();
                buffer.origin.clone_from(&buffer.text);
                actions.push(Action::ApplyText(buffer.text.clone()));
            }
            Err(errors) => buffer.errors = errors.iter().map(ToString::to_string).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::ui::test_support::Fixture;

    const SRC: &str = "[[devices]]\nid = \"office\"\ntemplate = \"co2\"\n";

    fn harness() -> Harness<'static, Fixture> {
        Fixture::new(SRC).harness(|ui, fx| show(ui, &fx.doc, &mut fx.ui, &mut fx.actions))
    }

    fn type_text(h: &mut Harness<'static, Fixture>, text: &str) {
        h.state_mut().ui.source.as_mut().unwrap().text = text.to_string();
        h.run();
    }

    #[test]
    fn the_editor_shows_what_save_writes() {
        let h = harness();
        assert_eq!(
            h.get_by_label("Text").value().as_deref(),
            Some(h.state().doc.text_to_save().as_str())
        );
        assert!(h.query_by_label("Not applied yet").is_none());
    }

    #[test]
    fn apply_takes_the_text_as_the_project() {
        let mut h = harness();
        let text = "# Mine.\n[[devices]]\nid = \"lab\"\ntemplate = \"co2\"\n";
        type_text(&mut h, text);
        assert!(h.query_by_label("Not applied yet").is_some());
        h.get_by_label("Apply").click();
        h.run();
        assert_eq!(h.state().doc.file().devices[0].id, "lab");
        assert_eq!(h.state().doc.text_to_save(), text, "comments included");
        h.run();
        assert!(h.query_by_label("Not applied yet").is_none());
    }

    #[test]
    fn text_that_does_not_parse_shows_why_and_is_not_applied() {
        let mut h = harness();
        type_text(&mut h, "[[devices]\n");
        h.get_by_label("Apply").click();
        h.run();
        assert!(h.query_by_label_contains("line 1").is_some());
        assert_eq!(h.state().doc.file().devices[0].id, "office");
    }

    #[test]
    fn revert_drops_what_was_typed() {
        let mut h = harness();
        type_text(&mut h, "nonsense");
        h.get_by_label("Revert").click();
        h.run();
        assert_eq!(
            h.state().ui.source.as_ref().unwrap().text,
            h.state().doc.text_to_save()
        );
    }

    #[test]
    fn undoing_a_new_comment_takes_it_out_of_the_editor() {
        let mut h = harness();
        let text = format!("# A note.\n{}", h.state().doc.text_to_save());
        type_text(&mut h, &text);
        h.get_by_label("Apply").click();
        h.run();
        h.state_mut().doc.undo();
        h.run();
        assert!(
            !h.get_by_label("Text")
                .value()
                .unwrap()
                .contains("# A note.")
        );
    }

    #[test]
    fn the_project_file_speaks_czech() {
        crate::i18n::set_language(crate::i18n::Language::Czech);
        let mut h = harness();
        assert!(h.query_by_label("Soubor projektu").is_some());
        type_text(&mut h, "nonsense");
        assert!(h.query_by_label("Zatím nepoužito").is_some());
        h.get_by_label("Vrátit změny").click();
        h.run();
        assert!(h.query_by_label("Zatím nepoužito").is_none());
        assert!(h.query_by_label("Použít").is_some());
    }

    #[test]
    fn the_editor_follows_edits_made_elsewhere_until_something_is_typed() {
        let mut h = harness();
        h.state_mut().doc.edit(|f| f.devices[0].count = 3);
        h.run();
        assert!(
            h.get_by_label("Text")
                .value()
                .unwrap()
                .contains("count = 3")
        );

        type_text(&mut h, "typed");
        h.state_mut().doc.edit(|f| f.devices[0].count = 4);
        h.run();
        assert_eq!(h.get_by_label("Text").value().as_deref(), Some("typed"));
    }
}
