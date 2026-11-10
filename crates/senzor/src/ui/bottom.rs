use eframe::egui::{self, RichText, TextEdit, TextStyle};
use senzor_core::engine::{EngineState, LogEntry};
use senzor_core::render::Payload;

use super::{Action, BottomTab, ERROR, UiState, request_field_focus, shortcuts};
use crate::document::Document;
use crate::format;
use crate::i18n::t;
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
        ui.selectable_value(
            &mut ui_state.bottom_tab,
            BottomTab::Log,
            t!("bottom-message-log"),
        );
        let count = doc.validated().errors.len();
        let label = if count == 0 {
            RichText::new(t!("bottom-problems"))
        } else {
            RichText::new(t!("bottom-problems-count", count = count)).color(ERROR)
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
    ui.horizontal(|ui| log_toolbar(ui, state, ui_state, actions));

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

    let columns = Columns::fitting(&entries);
    ui.label(
        RichText::new(format!(
            "{:<13} {:<instance$} {:<topic$} QoS  {}",
            t!("bottom-time"),
            t!("bottom-instance"),
            t!("bottom-topic"),
            t!("bottom-column-payload"),
            instance = columns.instance,
            topic = columns.topic,
        ))
        .monospace()
        .weak(),
    );
    let row_height = ui.text_style_height(&TextStyle::Monospace);
    let mut clicks = RowClick::default();
    // Both tabs draw into the same `ui`; their scroll areas need IDs of their own, or showing
    // the problems resets the log's position and its stick to the bottom.
    egui::ScrollArea::vertical()
        .id_salt("log")
        .auto_shrink([false; 2])
        .stick_to_bottom(true)
        .show_rows(ui, row_height, entries.len(), |ui, rows| {
            for entry in &entries[rows] {
                let selected = ui_state.log_selected == Some(entry.seq);
                let click = ui
                    .horizontal(|ui| log_row(ui, entry, columns, selected))
                    .inner;
                clicks.instance = click.instance.or(clicks.instance.take());
                clicks.row = click.row.or(clicks.row);
            }
        });
    if let Some(instance) = clicks.instance {
        ui_state.log_filter.instance = Some(instance);
    }
    if let Some(seq) = clicks.row {
        ui_state.log_selected = (ui_state.log_selected != Some(seq)).then_some(seq);
    }
}

/// Filter, the instance chip, Pause, Clear and Export… above the log.
fn log_toolbar(
    ui: &mut egui::Ui,
    state: &EngineState,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    let label = ui.label(t!("bottom-filter"));
    let filter = ui
        .add(
            TextEdit::singleline(&mut ui_state.log_filter.topic)
                .hint_text(t!("bottom-filter-placeholder")),
        )
        .labelled_by(label.id)
        .on_hover_text(shortcuts::tip(
            ui.ctx(),
            &t!("bottom-filter-hint"),
            &shortcuts::FIND,
        ));
    if std::mem::take(&mut ui_state.focus_log_filter) {
        filter.request_focus();
    }
    if let Some(instance) = ui_state.log_filter.instance.clone() {
        let every = t!("bottom-show-every-instance");
        let chip = ui
            .button(format!("{instance} {}", icons::X))
            .on_hover_text(&every);
        chip.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &every));
        if chip.clicked() {
            ui_state.log_filter.instance = None;
        }
    }
    let paused = ui_state.log_frozen.is_some();
    let pause = if paused {
        t!("bottom-resume")
    } else {
        t!("bottom-pause")
    };
    if ui.button(pause).clicked() {
        ui_state.log_frozen = if paused {
            None
        } else {
            Some(state.log.clone())
        };
    }
    let clear = ui
        .button(format!("{} {}", icons::TRASH, t!("bottom-clear")))
        .on_hover_text(t!("bottom-clear-hint"));
    let name = t!("bottom-clear-name");
    clear.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
    if clear.clicked() {
        ui_state.clear_log(&state.log);
    }
    let name = t!("bottom-export");
    let export = ui
        .button(format!("{} {name}", icons::EXPORT))
        .on_hover_text(t!("bottom-export-hint"));
    export.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
    if export.clicked() {
        actions.push(Action::ExportLog);
    }
}

/// Widths of the log's monospace columns, in characters.
#[derive(Clone, Copy)]
struct Columns {
    instance: usize,
    topic: usize,
}

impl Columns {
    /// As wide as the longest instance ID and topic shown; topics up to [`TOPIC_COLUMN_MAX`].
    fn fitting(entries: &[&LogEntry]) -> Self {
        let widest = |text: fn(&LogEntry) -> &str| {
            entries
                .iter()
                .map(|e| text(e).chars().count())
                .max()
                .unwrap_or(0)
        };
        Self {
            instance: widest(|e| &e.instance).max(t!("bottom-instance").chars().count()),
            topic: widest(|e| &e.topic).clamp(t!("bottom-topic").chars().count(), TOPIC_COLUMN_MAX),
        }
    }
}

/// What a click in the log's rows asks for.
#[derive(Default)]
struct RowClick {
    /// Show only this instance's messages.
    instance: Option<String>,
    /// Show this message in full, or close it when it is shown.
    row: Option<u64>,
}

/// One message as a row of cells; the selected message's row is highlighted cell by cell.
fn log_row(ui: &mut egui::Ui, entry: &LogEntry, columns: Columns, selected: bool) -> RowClick {
    ui.spacing_mut().item_spacing.x = 0.0;
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
    let Columns {
        instance: instance_width,
        topic: topic_width,
    } = columns;
    let time = clickable(ui, cell(format!("{:<13} ", format::clock(entry.time))));
    let filter_label = t!("bottom-show-only", instance = entry.instance.as_str());
    let instance = clickable(ui, cell(format!("{:<instance_width$} ", entry.instance)))
        .on_hover_text(&filter_label);
    // Named for what a click does: the tree already has a widget per instance ID.
    instance
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &filter_label));
    let topic = clickable(
        ui,
        cell(format!("{:<topic_width$} ", cut(&entry.topic, topic_width))),
    );
    ui.label(cell(entry.qos.to_string()));
    ui.label(cell(if entry.retain { "R" } else { " " }.to_string()))
        .on_hover_text(t!("bottom-retained"));
    ui.label(cell("   ".to_string()));
    // Cut at the edge, so a long payload does not run under the shown message.
    let payload = ui.add(
        egui::Label::new(cell(format::log_payload(&entry.payload)))
            .truncate()
            .sense(egui::Sense::click()),
    );
    RowClick {
        instance: instance.clicked().then(|| entry.instance.clone()),
        row: (time.clicked() || topic.clicked() || payload.clicked()).then_some(entry.seq),
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
        ui.strong(t!("bottom-message"));
        let name = t!("bottom-close");
        let close = ui.small_button(icons::X).on_hover_text(&name);
        close.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
        if close.clicked() {
            *selected = None;
        }
    });
    egui::Grid::new("log-detail-grid")
        .num_columns(2)
        .show(ui, |ui| {
            let mut row = |name: String, value: String| {
                ui.weak(name);
                ui.label(value);
                ui.end_row();
            };
            row(
                t!("bottom-time"),
                humantime::format_rfc3339_millis(entry.time).to_string(),
            );
            row(t!("bottom-instance"), entry.instance.clone());
            row(t!("bottom-topic"), entry.topic.clone());
            row("QoS".to_string(), entry.qos.to_string());
            let retained = if entry.retain {
                t!("bottom-yes")
            } else {
                t!("bottom-no")
            };
            row(t!("bottom-retained"), retained);
            let properties = &entry.properties;
            if let Some(content_type) = &properties.content_type {
                row(t!("bottom-content-type"), content_type.clone());
            }
            if let Some(expiry) = properties.message_expiry {
                row(t!("bottom-message-expiry"), format!("{expiry} s"));
            }
            for (name, value) in &properties.user_properties {
                row(t!("bottom-user-property"), format!("{name} = {value}"));
            }
        });
    let Some(text) = entry.payload.as_text() else {
        binary_payload(ui, &entry.payload);
        return;
    };
    ui.horizontal(|ui| {
        ui.strong(t!("bottom-payload"));
        if ui
            .button(t!("bottom-copy-payload"))
            .on_hover_text(t!("bottom-copy-payload-hint"))
            .clicked()
        {
            ui.ctx().copy_text(text.to_string());
        }
    });
    egui::ScrollArea::vertical()
        .id_salt("log-detail-payload")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            ui.label(RichText::new(pretty_payload(text)).monospace());
        });
}

/// A binary payload in the details: its bytes as a hex dump, then the text they were encoded
/// from.
fn binary_payload(ui: &mut egui::Ui, payload: &Payload) {
    let bytes = payload.bytes();
    ui.horizontal(|ui| {
        ui.strong(t!("bottom-payload"));
        ui.weak(format!(
            "{}, {}",
            payload.encoding().label(),
            format::byte_count(bytes.len())
        ));
        if ui
            .button(t!("bottom-copy-hex"))
            .on_hover_text(t!("bottom-copy-hex-hint"))
            .clicked()
        {
            ui.ctx().copy_text(payload.hex());
        }
    });
    egui::ScrollArea::both()
        .id_salt("log-detail-payload")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            super::hex_dump_label(ui, bytes);
            ui.add_space(6.0);
            ui.strong(t!("bottom-rendered-text"));
            ui.label(RichText::new(pretty_payload(payload.text())).monospace());
        });
}

fn problems(ui: &mut egui::Ui, doc: &Document, actions: &mut Vec<Action>) {
    let errors = &doc.validated().errors;
    if errors.is_empty() {
        ui.label(t!("bottom-no-problems"));
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("problems")
        .auto_shrink([false; 2])
        .show(ui, |ui| {
            for error in errors {
                if ui
                    .link(error.to_string_in(crate::i18n::language()))
                    .clicked()
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
                payload: "1".into(),
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
                payload: format!(r#"{{"co2":{}}}"#, 600 + i).into(),
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
    fn a_long_payload_stops_short_of_the_shown_message() {
        let mut state = two_instances();
        state.log[0].payload = "long payload ".repeat(30).into();
        let mut h = log_harness(state);
        h.get_by_label_contains(r#"{"co2":605}"#).click();
        h.run();
        let panel_left = h.get_by_label("Message").rect().min.x;
        let row = h
            .get_all_by_label_contains("long payload long payload")
            .min_by(|a, b| a.rect().min.x.total_cmp(&b.rect().min.x))
            .unwrap();
        assert!(
            row.rect().max.x <= panel_left,
            "the row ends at {}, under the panel at {panel_left}",
            row.rect().max.x
        );
    }

    #[test]
    fn a_czech_window_lists_the_problems_in_czech() {
        crate::i18n::set_language(crate::i18n::Language::Czech);
        let mut h = Fixture::new(ONE_PROBLEM).harness(|ui, fx| {
            fx.ui.bottom_tab = BottomTab::Problems;
            show(
                ui,
                &fx.doc,
                &EngineState::default(),
                &mut fx.ui,
                &mut fx.actions,
            );
        });
        h.run();
        assert!(
            h.query_by_label("zařízení `bad`: neznámá šablona `nope`")
                .is_some(),
            "the problem and its place in Czech"
        );
    }

    /// A log with one CBOR message of 20 bytes.
    fn binary_log() -> EngineState {
        let entry = LogEntry {
            seq: 1,
            time: SystemTime::UNIX_EPOCH,
            instance: "office".to_string(),
            topic: "b/office/cbor".to_string(),
            payload: senzor_core::render::Payload::Binary {
                encoding: senzor_core::encoding::Encoding::Cbor,
                bytes: (0..20).collect(),
                text: r#"{"co2":600}"#.into(),
            },
            qos: 0,
            retain: false,
            properties: Default::default(),
        };
        EngineState {
            log: [entry].into(),
            ..EngineState::default()
        }
    }

    #[test]
    fn a_binary_message_shows_as_hex_with_its_encoding_and_size() {
        let mut h = log_harness(binary_log());
        let row = "cbor, 20 bytes: 00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f …";
        h.get_by_label(row).click();
        h.run();
        // The panel's default width fits 8 bytes a line.
        assert!(
            h.query_by_label_contains("0000  00 01 02 03 04 05 06 07\n0008  08 09")
                .is_some(),
            "the hex dump"
        );
        assert!(h.query_by_label_contains("0010  10 11 12 13").is_some());
        assert!(h.query_by_label("Rendered text").is_some());
        assert!(
            h.query_by_label_contains("\"co2\": 600").is_some(),
            "the text it came from, indented"
        );
        assert!(h.query_by_label("Copy as hex").is_some());
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
    fn clear_empties_the_log_and_closes_the_shown_message() {
        let mut h = log_harness(two_instances());
        h.get_by_label_contains(r#"{"co2":605}"#).click();
        h.run();
        h.get_by_label("Clear the log").click();
        h.run();
        assert!(h.query_by_label_contains("b/office/state").is_none());
        assert!(h.query_by_label_contains("b/lab/state").is_none());
        assert!(
            h.query_by_label("Copy payload").is_none(),
            "no message is shown"
        );
        assert_eq!(h.state().ui.log_selected, None);
    }

    #[test]
    fn the_message_log_speaks_czech() {
        crate::i18n::set_language(crate::i18n::Language::Czech);
        let mut h = log_harness(two_instances());
        for name in [
            "Záznam zpráv",
            "Problémy",
            "Filtr",
            "Pozastavit",
            "Vymazat záznam",
            "Exportovat…",
        ] {
            assert!(h.query_by_label(name).is_some(), "{name}");
        }
        assert!(h.query_by_label_contains("Čas").is_some(), "the header");
        h.get_by_label_contains(r#"{"co2":605}"#).click();
        h.run();
        assert!(h.query_by_label("Zpráva").is_some());
        assert!(h.query_by_label("Kopírovat payload").is_some());
        h.get_by_label("Problémy").click();
        h.run();
        assert!(h.query_by_label("Žádné problémy.").is_some());
    }

    #[test]
    fn the_problems_tab_counts_in_czech() {
        crate::i18n::set_language(crate::i18n::Language::Czech);
        let h = Fixture::new(ONE_PROBLEM).harness(|ui, fx| {
            show(
                ui,
                &fx.doc,
                &EngineState::default(),
                &mut fx.ui,
                &mut fx.actions,
            );
        });
        assert!(h.query_by_label("Problémy (1)").is_some());
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
