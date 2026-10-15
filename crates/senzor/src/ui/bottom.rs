use eframe::egui::{self, RichText, TextEdit, TextStyle};
use senzor_core::engine::{EngineState, LogEntry};

use super::{Action, BottomTab, ERROR, UiState};
use crate::document::Document;
use crate::format;
use crate::selection::problem_target;

pub fn show(
    ui: &mut egui::Ui,
    doc: &Document,
    state: &EngineState,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        ui.selectable_value(&mut ui_state.bottom_tab, BottomTab::Log, "Message log");
        let count = doc.validated().errors.len();
        let label = if count == 0 {
            RichText::new("Problems")
        } else {
            RichText::new(format!("Problems ({count})")).color(ERROR)
        };
        ui.selectable_value(&mut ui_state.bottom_tab, BottomTab::Problems, label);
    });
    ui.separator();
    match ui_state.bottom_tab {
        BottomTab::Log => log(ui, state, ui_state),
        BottomTab::Problems => problems(ui, doc, actions),
    }
}

fn log(ui: &mut egui::Ui, state: &EngineState, ui_state: &mut UiState) {
    ui.horizontal(|ui| {
        let label = ui.label("Filter");
        ui.add(TextEdit::singleline(&mut ui_state.log_filter).hint_text("topic contains…"))
            .labelled_by(label.id);
        let paused = ui_state.log_frozen.is_some();
        if ui.button(if paused { "Resume" } else { "Pause" }).clicked() {
            ui_state.log_frozen = if paused {
                None
            } else {
                Some(state.log.clone())
            };
        }
    });
    let filter = ui_state.log_filter.as_str();
    let log = ui_state.log_frozen.as_ref().unwrap_or(&state.log);
    let entries: Vec<&LogEntry> = log.iter().filter(|e| e.topic.contains(filter)).collect();
    let row_height = ui.text_style_height(&TextStyle::Monospace);
    // Both tabs draw into the same `ui`; their scroll areas need IDs of their own, or showing
    // the problems resets the log's position and its stick to the bottom.
    egui::ScrollArea::vertical()
        .id_salt("log")
        .auto_shrink([false; 2])
        .stick_to_bottom(true)
        .show_rows(ui, row_height, entries.len(), |ui, rows| {
            for entry in &entries[rows] {
                let payload = format::log_payload(&entry.payload);
                ui.label(
                    RichText::new(format!(
                        "{}  {}  {}  {}",
                        format::clock(entry.time),
                        entry.instance,
                        entry.topic,
                        payload
                    ))
                    .monospace(),
                );
            }
        });
}

fn problems(ui: &mut egui::Ui, doc: &Document, actions: &mut Vec<Action>) {
    let errors = &doc.validated().errors;
    if errors.is_empty() {
        ui.label("No problems.");
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("problems")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            for error in errors {
                if ui.link(error.to_string()).clicked()
                    && let Some(target) = problem_target(&error.location, doc.file())
                {
                    actions.push(Action::GoTo(target));
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::ui::test_support::Fixture;

    /// One problem, so the Problems tab lists it in a scroll area of its own.
    const ONE_PROBLEM: &str = "[[devices]]\nid = \"bad\"\ntemplate = \"nope\"\n";

    /// An engine whose log holds `count` messages, on topics `topic-000`, `topic-001`, ….
    fn engine_with_log(count: usize) -> EngineState {
        let log = (0..count)
            .map(|i| LogEntry {
                time: SystemTime::UNIX_EPOCH,
                instance: "office".to_string(),
                topic: format!("topic-{i:03}"),
                payload: "1".to_string(),
                qos: 0,
                retain: false,
            })
            .collect();
        EngineState {
            log,
            ..EngineState::default()
        }
    }

    #[test]
    fn the_log_still_shows_its_newest_message_after_the_problems_tab() {
        let state = engine_with_log(100);
        let mut h = Fixture::new(ONE_PROBLEM).harness(move |ui, fx| {
            show(ui, &fx.doc, &state, &mut fx.ui, &mut fx.actions);
        });
        assert!(
            h.query_by_label_contains("topic-099").is_some(),
            "the log starts at its newest message"
        );

        h.get_by_label("Problems (1)").click();
        h.run();
        h.get_by_label("Message log").click();
        h.run();
        assert!(
            h.query_by_label_contains("topic-099").is_some(),
            "the log is still at its newest message"
        );
    }
}
