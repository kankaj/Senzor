use std::time::SystemTime;

use eframe::egui::{self, Button, TextEdit};
use senzor_core::engine::{Command, EngineState, Target};
use senzor_core::value::Value;

use super::{Action, ERROR, UiState, WARNING, status_dot};
use crate::document::Document;
use crate::format;
use crate::tree::{OrphanCause, is_restartable, orphan_cause};

pub fn show(
    ui: &mut egui::Ui,
    id: &str,
    doc: &Document,
    state: &EngineState,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    ui.heading(format!("Instance {id}"));
    let Some(instance) = state.instances.get(id) else {
        ui.label("This instance is no longer part of the project.");
        return;
    };
    let (word, detail) = format::status(&instance.status);
    ui.horizontal(|ui| {
        status_dot(ui, &instance.status);
        ui.strong(word);
        if let Some(detail) = detail {
            ui.colored_label(ERROR, detail);
        }
    });
    ui.label(format!("Device: {}", instance.device));
    ui.horizontal(|ui| {
        ui.label(format!("Published: {}", instance.published));
        ui.separator();
        ui.label(format!("Errors: {}", instance.errors));
        ui.separator();
        let last = instance.last_publish.map_or_else(
            || "never".to_string(),
            |t| format::ago(t, SystemTime::now()),
        );
        ui.label(format!("Last publish: {last}"));
    });
    if instance.needs_restart {
        ui.colored_label(
            WARNING,
            "The project changed; restart this instance to apply it.",
        );
    }
    if instance.orphaned {
        let text = match orphan_cause(doc.validated(), &instance.device) {
            OrphanCause::DeviceHasErrors => {
                "Its device has problems (see the Problems tab); it keeps running with its last valid settings."
            }
            OrphanCause::DeviceRemoved => {
                "This instance is no longer in the project; it disappears when stopped."
            }
        };
        ui.colored_label(WARNING, text);
    }

    let target = || Target::Instance(id.to_string());
    let running = instance.status.is_running();
    ui.horizontal(|ui| {
        if running {
            if ui.button("Stop").clicked() {
                actions.push(Action::Engine(Command::Stop(target())));
            }
            if is_restartable(&instance.status, instance.orphaned)
                && ui
                    .button("Restart")
                    .on_hover_text("Stop, then start again with the project's current settings")
                    .clicked()
            {
                actions.push(Action::Engine(Command::Restart(target())));
            }
            if ui
                .button("Kill")
                .on_hover_text("Drop the connection; the broker publishes the Last Will")
                .clicked()
            {
                actions.push(Action::Engine(Command::Kill(target())));
            }
        } else if ui.button("Start").clicked() {
            actions.push(Action::Engine(Command::Start(target())));
        }
    });
    ui.separator();

    let mut fields: Vec<String> = doc
        .validated()
        .project
        .instance(id)
        .map(|config| config.fields.keys().cloned().collect())
        .unwrap_or_default();
    for name in instance.values.keys() {
        if !fields.contains(name) {
            fields.push(name.clone());
        }
    }
    egui::Grid::new("instance-fields")
        .num_columns(4)
        .striped(true)
        .show(ui, |ui| {
            ui.strong("Field");
            ui.strong("Value");
            ui.strong("Pin to");
            ui.label("");
            ui.end_row();
            for field in fields {
                let name = ui.label(&field);
                let pinned = instance.pinned.contains(&field);
                let value = instance
                    .values
                    .get(&field)
                    .map_or_else(|| "–".to_string(), format::value);
                ui.label(if pinned {
                    format!("{value} (pinned)")
                } else {
                    value
                });
                let input = ui_state
                    .pin_inputs
                    .entry((id.to_string(), field.clone()))
                    .or_default();
                ui.add_sized([140.0, 20.0], TextEdit::singleline(input))
                    .labelled_by(name.id);
                if pinned {
                    if ui.add_enabled(running, Button::new("Unpin")).clicked() {
                        actions.push(Action::Engine(Command::Unpin {
                            instance: id.to_string(),
                            field: field.clone(),
                        }));
                    }
                } else if ui
                    // Pinning "" is a slip: templates that format the field fail on it every tick.
                    .add_enabled(running && !input.trim().is_empty(), Button::new("Pin"))
                    .clicked()
                {
                    actions.push(Action::Engine(Command::Pin {
                        instance: id.to_string(),
                        field: field.clone(),
                        value: Value::parse_input(input),
                    }));
                }
                ui.end_row();
            }
        });
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};
    use senzor_core::engine::{InstanceState, InstanceStatus};

    use super::*;
    use crate::ui::test_support::{Fixture, instance, type_into_label};

    const SRC: &str = r#"
[[templates]]
id = "air"
messages = [{ topic = "t/{{ device_id }}", payload = "{{ co2 }}" }]
fields.co2 = { generator = "constant", value = 600 }

[[devices]]
id = "office"
template = "air"
"#;

    fn running(device: &str) -> InstanceState {
        instance(device, InstanceStatus::Online)
    }

    /// The instance view of `id`, with the engine running `instance` under that ID.
    fn harness(src: &str, id: &'static str, instance: InstanceState) -> Harness<'static, Fixture> {
        let mut state = EngineState::default();
        state.instances.insert(id.to_string(), instance);
        Fixture::new(src).harness(move |ui, fx| {
            show(ui, id, &fx.doc, &state, &mut fx.ui, &mut fx.actions);
        })
    }

    fn pin_disabled(h: &Harness<'static, Fixture>) -> bool {
        h.get_by_label("Pin").accesskit_node().is_disabled()
    }

    fn orphan(device: &str) -> InstanceState {
        let mut instance = running(device);
        instance.orphaned = true;
        instance
    }

    const DEVICE_HAS_ERRORS: &str = "Its device has problems (see the Problems tab); it keeps running with its last valid settings.";
    const REMOVED: &str = "This instance is no longer in the project; it disappears when stopped.";

    #[test]
    fn an_orphan_whose_device_has_errors_says_it_keeps_its_last_valid_settings() {
        let h = harness(
            "[[devices]]\nid = \"office\"\ntemplate = \"gone\"\n",
            "office",
            orphan("office"),
        );
        assert!(h.query_by_label(DEVICE_HAS_ERRORS).is_some());
        assert!(h.query_by_label(REMOVED).is_none());
    }

    #[test]
    fn an_orphan_whose_device_was_removed_says_it_disappears_when_stopped() {
        let h = harness(SRC, "old-0", orphan("old"));
        assert!(h.query_by_label(REMOVED).is_some());
        assert!(h.query_by_label(DEVICE_HAS_ERRORS).is_none());
    }

    #[test]
    fn restart_is_offered_to_running_instances_but_not_to_orphans() {
        let h = harness(SRC, "office", running("office"));
        assert!(h.query_by_label("Restart").is_some());
        let h = harness(SRC, "old-0", orphan("old"));
        assert!(h.query_by_label("Restart").is_none());
        assert!(h.query_by_label("Stop").is_some());
    }

    #[test]
    fn pin_is_disabled_until_a_value_is_typed() {
        let mut h = harness(SRC, "office", running("office"));
        assert!(pin_disabled(&h));
        type_into_label(&mut h, "co2", "  ");
        assert!(pin_disabled(&h), "whitespace is no value");

        type_into_label(&mut h, "co2", "2500");
        assert!(!pin_disabled(&h));
        h.get_by_label("Pin").click();
        h.run();
        assert!(matches!(
            &h.state().other[..],
            [Action::Engine(Command::Pin { field, .. })] if field == "co2"
        ));
    }
}
