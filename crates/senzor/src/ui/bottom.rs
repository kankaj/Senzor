use eframe::egui::{self, RichText, TextEdit, TextStyle};
use senzor_core::engine::{EngineState, LogEntry};

use super::{Action, BottomTab, ERROR, UiState, request_field_focus, shortcuts};
use crate::document::Document;
use crate::format;
use crate::log_view::pretty_payload;
use crate::selection::problem_target;
use egui_phosphor::regular as icons;

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
        BottomTab::Log => log(ui, state, ui_state, actions),
        BottomTab::Problems => problems(ui, doc, actions),
    }
}

/// Topics longer than this are cut short in the log's column; the details show them whole.
const TOPIC_COLUMN_MAX: usize = 40;

fn log(ui: &mut egui::Ui, state: &EngineState, ui_state: &mut UiState, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let label = ui.label("Filter");
        let filter = ui
            .add(TextEdit::singleline(&mut ui_state.log_filter.topic).hint_text("topic contains…"))
            .labelled_by(label.id)
            .on_hover_text(shortcuts::tip(
                ui.ctx(),
                "Show only messages whose topic contains this",
                &shortcuts::FIND,
            ));
        if std::mem::take(&mut ui_state.focus_log_filter) {
            filter.request_focus();
        }
        if let Some(instance) = ui_state.log_filter.instance.clone() {
            let chip = ui
                .button(format!("{instance} {}", icons::X))
                .on_hover_text("Show every instance");
            chip.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Show every instance")
            });
            if chip.clicked() {
                ui_state.log_filter.instance = None;
            }
        }
        let paused = ui_state.log_frozen.is_some();
        if ui.button(if paused { "Resume" } else { "Pause" }).clicked() {
            ui_state.log_frozen = if paused {
                None
            } else {
                Some(state.log.clone())
            };
        }
        let export = ui
            .button(format!("{} Export…", icons::EXPORT))
            .on_hover_text("Save the messages shown here as a CSV file");
        export.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Export…"));
        if export.clicked() {
            actions.push(Action::ExportLog);
        }
    });

    let log = ui_state.log_frozen.as_ref().unwrap_or(&state.log);
    let entries: Vec<&LogEntry> = log
        .iter()
        .filter(|e| ui_state.log_filter.matches(e))
        .collect();
    let selected = ui_state
        .log_selected
        .and_then(|seq| log.iter().find(|e| e.seq == seq))
        .cloned();
    if let Some(entry) = &selected {
        egui::Panel::right("log-detail")
            .resizable(true)
            .default_size(340.0)
            .show(ui, |ui| details(ui, entry, &mut ui_state.log_selected));
    }

    // Monospace columns, as wide as the longest instance ID and topic shown.
    let instance_width = entries
        .iter()
        .map(|e| e.instance.chars().count())
        .max()
        .unwrap_or(0)
        .max("Instance".len());
    let topic_width = entries
        .iter()
        .map(|e| e.topic.chars().count())
        .max()
        .unwrap_or(0)
        .clamp("Topic".len(), TOPIC_COLUMN_MAX);
    ui.label(
        RichText::new(format!(
            "{:<13} {:<instance_width$} {:<topic_width$} QoS  Payload",
            "Time", "Instance", "Topic"
        ))
        .monospace()
        .weak(),
    );
    let row_height = ui.text_style_height(&TextStyle::Monospace);
    let mut clicked_instance = None;
    let mut clicked_row = None;
    // Both tabs draw into the same `ui`; their scroll areas need IDs of their own, or showing
    // the problems resets the log's position and its stick to the bottom.
    egui::ScrollArea::vertical()
        .id_salt("log")
        .auto_shrink([false; 2])
        .stick_to_bottom(true)
        .show_rows(ui, row_height, entries.len(), |ui, rows| {
            for entry in &entries[rows] {
                let selected = ui_state.log_selected == Some(entry.seq);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    // The selected message's row is highlighted cell by cell.
                    let highlight = selected.then(|| ui.visuals().selection.bg_fill);
                    let cell = |text: String| {
                        let text = RichText::new(text).monospace();
                        match highlight {
                            Some(fill) => text.background_color(fill),
                            None => text,
                        }
                    };
                    let clickable = |ui: &mut egui::Ui, text: RichText| {
                        ui.add(egui::Label::new(text).sense(egui::Sense::click()))
                    };
                    let time = clickable(ui, cell(format!("{:<13} ", format::clock(entry.time))));
                    let instance =
                        clickable(ui, cell(format!("{:<instance_width$} ", entry.instance)))
                            .on_hover_text(format!("Show only {}'s messages", entry.instance));
                    // Named for what a click does: the tree already has a widget per instance ID.
                    instance.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Label,
                            true,
                            format!("Show only {}'s messages", entry.instance),
                        )
                    });
                    let topic = clickable(
                        ui,
                        cell(format!("{:<topic_width$} ", cut(&entry.topic, topic_width))),
                    );
                    ui.label(cell(entry.qos.to_string()));
                    ui.label(cell(if entry.retain { "R" } else { " " }.to_string()))
                        .on_hover_text("Retained");
                    ui.label(cell("   ".to_string()));
                    let payload = clickable(ui, cell(format::log_payload(&entry.payload)));
                    if instance.clicked() {
                        clicked_instance = Some(entry.instance.clone());
                    }
                    if time.clicked() || topic.clicked() || payload.clicked() {
                        clicked_row = Some(entry.seq);
                    }
                });
            }
        });
    if let Some(instance) = clicked_instance {
        ui_state.log_filter.instance = Some(instance);
    }
    if let Some(seq) = clicked_row {
        ui_state.log_selected = (ui_state.log_selected != Some(seq)).then_some(seq);
    }
}

/// `text` cut to `width` characters, with `…` at the end when it was longer.
fn cut(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        text.to_string()
    } else {
        let kept: String = text.chars().take(width.saturating_sub(1)).collect();
        format!("{kept}…")
    }
}

/// The selected message in full: where and how it was published, and its payload indented.
fn details(ui: &mut egui::Ui, entry: &LogEntry, selected: &mut Option<u64>) {
    ui.horizontal(|ui| {
        ui.strong("Message");
        let close = ui.small_button(icons::X).on_hover_text("Close");
        close.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Close"));
        if close.clicked() {
            *selected = None;
        }
    });
    egui::Grid::new("log-detail-grid")
        .num_columns(2)
        .show(ui, |ui| {
            let mut row = |name: &str, value: String| {
                ui.weak(name);
                ui.label(value);
                ui.end_row();
            };
            row(
                "Time",
                humantime::format_rfc3339_millis(entry.time).to_string(),
            );
            row("Instance", entry.instance.clone());
            row("Topic", entry.topic.clone());
            row("QoS", entry.qos.to_string());
            row(
                "Retained",
                if entry.retain { "yes" } else { "no" }.to_string(),
            );
            let properties = &entry.properties;
            if let Some(content_type) = &properties.content_type {
                row("Content type", content_type.clone());
            }
            if let Some(expiry) = properties.message_expiry {
                row("Message expiry", format!("{expiry} s"));
            }
            for (name, value) in &properties.user_properties {
                row("User property", format!("{name} = {value}"));
            }
        });
    ui.horizontal(|ui| {
        ui.strong("Payload");
        if ui
            .button("Copy payload")
            .on_hover_text("Copy the payload as it was sent")
            .clicked()
        {
            ui.ctx().copy_text(entry.payload.clone());
        }
    });
    egui::ScrollArea::vertical()
        .id_salt("log-detail-payload")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            ui.label(RichText::new(pretty_payload(&entry.payload)).monospace());
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
                    if let Some(field) = &error.field {
                        request_field_focus(ui.ctx(), error.location.clone(), field.clone());
                    }
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
                seq: i as u64 + 1,
                time: SystemTime::UNIX_EPOCH,
                instance: "office".to_string(),
                topic: format!("topic-{i:03}"),
                payload: "1".to_string(),
                qos: 0,
                retain: false,
                properties: Default::default(),
            })
            .collect();
        EngineState {
            log,
            ..EngineState::default()
        }
    }

    /// A log with `office-0` and `lab-0` taking turns, newest last.
    fn two_instances() -> EngineState {
        let log = (0..6)
            .map(|i| LogEntry {
                seq: i as u64 + 1,
                time: SystemTime::UNIX_EPOCH,
                instance: if i % 2 == 0 { "office-0" } else { "lab-0" }.to_string(),
                topic: format!("b/{}/state", if i % 2 == 0 { "office" } else { "lab" }),
                payload: format!(r#"{{"co2":{}}}"#, 600 + i),
                qos: 1,
                retain: i == 5,
                properties: Default::default(),
            })
            .collect();
        EngineState {
            log,
            ..EngineState::default()
        }
    }

    fn log_harness(state: EngineState) -> egui_kittest::Harness<'static, Fixture> {
        Fixture::new("").harness(move |ui, fx| {
            show(ui, &fx.doc, &state, &mut fx.ui, &mut fx.actions);
        })
    }

    #[test]
    fn clicking_an_instance_in_the_log_shows_only_its_messages() {
        let mut h = log_harness(two_instances());
        h.get_all_by_label("Show only lab-0's messages")
            .next()
            .unwrap()
            .click();
        h.run();
        assert!(h.query_by_label_contains("b/office/state").is_none());
        assert_eq!(h.query_all_by_label_contains("b/lab/state").count(), 3);

        h.get_by_label("Show every instance").click();
        h.run();
        assert_eq!(h.query_all_by_label_contains("b/office/state").count(), 3);
    }

    #[test]
    fn the_log_shows_qos_and_retained() {
        let h = log_harness(two_instances());
        assert!(h.query_all_by_label("1").next().is_some(), "QoS column");
        assert_eq!(h.query_all_by_label("R").count(), 1, "one retained message");
    }

    #[test]
    fn selecting_a_message_shows_its_payload_indented() {
        let mut h = log_harness(two_instances());
        h.get_by_label_contains(r#"{"co2":605}"#).click();
        h.run();
        assert!(h.query_by_label("Message").is_some());
        assert!(h.query_by_label_contains("\"co2\": 605").is_some());
        assert!(h.query_by_label("Copy payload").is_some());
    }

    #[test]
    fn export_asks_the_app_to_save_the_shown_messages() {
        let mut h = log_harness(two_instances());
        h.get_by_label("Export…").click();
        h.run();
        assert!(
            h.state()
                .other
                .iter()
                .any(|a| matches!(a, Action::ExportLog))
        );
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
