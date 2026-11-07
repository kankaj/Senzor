//! The detail panel for several instances selected together: run them, and pin a field on all.

use std::collections::BTreeMap;

use eframe::egui::{self, Button, TextEdit};
use senzor_core::engine::{Command, EngineState, InstanceStatus, Target};
use senzor_core::value::Value;

use super::{Action, UiState, kill_button, stop_button};
use crate::document::Document;
use crate::tree::is_restartable;

/// How many names the heading lists before "and N more".
const NAMES_SHOWN: usize = 8;

pub fn show(
    ui: &mut egui::Ui,
    ids: &[String],
    doc: &Document,
    state: &EngineState,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    ui.heading(format!("{} instances", ids.len()));
    ui.label(names(ids));
    ui.weak(summary(ids, state));
    ui.weak("Ctrl+click an instance in the tree to add or remove it, Shift+click to add a range.");
    ui.separator();
    controls(ui, ids, state, actions);
    ui.separator();
    pin_row(ui, ids, doc, state, ui_state, actions);
}

/// The instances' IDs, the first few of them.
fn names(ids: &[String]) -> String {
    let shown: Vec<&str> = ids.iter().take(NAMES_SHOWN).map(String::as_str).collect();
    match ids.len().saturating_sub(NAMES_SHOWN) {
        0 => shown.join(", "),
        more => format!("{} and {more} more", shown.join(", ")),
    }
}

/// How many of them are in each state, e.g. "2 online, 1 stopped".
fn summary(ids: &[String], state: &EngineState) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for id in ids {
        let word = match state.instances.get(id).map(|i| &i.status) {
            Some(InstanceStatus::Online) => "online",
            Some(InstanceStatus::Connecting) => "connecting",
            Some(InstanceStatus::Reconnecting { .. }) => "reconnecting",
            Some(InstanceStatus::Failed { .. }) => "failed",
            Some(InstanceStatus::Stopped) | None => "stopped",
        };
        *counts.entry(word).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(word, count)| format!("{count} {word}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Start, Stop, Restart and Kill, each for the instances it applies to.
fn controls(ui: &mut egui::Ui, ids: &[String], state: &EngineState, actions: &mut Vec<Action>) {
    let running = |id: &String| {
        state
            .instances
            .get(id)
            .is_some_and(|i| i.status.is_running())
    };
    // An orphan has no settings to start or restart with.
    let orphan = |id: &String| state.instances.get(id).is_some_and(|i| i.orphaned);
    let startable: Vec<&String> = ids
        .iter()
        .filter(|id| !running(id) && !orphan(id))
        .collect();
    let stoppable: Vec<&String> = ids.iter().filter(|id| running(id)).collect();
    let restartable: Vec<&String> = ids
        .iter()
        .filter(|id| {
            state
                .instances
                .get(*id)
                .is_some_and(|i| is_restartable(&i.status, i.orphaned))
        })
        .collect();
    let send = |actions: &mut Vec<Action>, ids: &[&String], command: fn(Target) -> Command| {
        for id in ids {
            actions.push(Action::Engine(command(Target::Instance((*id).clone()))));
        }
    };
    ui.horizontal(|ui| {
        if ui
            .add_enabled(!startable.is_empty(), Button::new("Start"))
            .on_hover_text("Start those that are stopped or failed")
            .on_disabled_hover_text("All of them run already")
            .clicked()
        {
            send(actions, &startable, Command::Start);
        }
        ui.add_enabled_ui(!stoppable.is_empty(), |ui| {
            if stop_button(ui).clicked() {
                send(actions, &stoppable, Command::Stop);
            }
        });
        if ui
            .add_enabled(!restartable.is_empty(), Button::new("Restart"))
            .on_hover_text(
                "Stop those that run, then start them with the project's current settings",
            )
            .on_disabled_hover_text("None of them runs")
            .clicked()
        {
            send(actions, &restartable, Command::Restart);
        }
        ui.add_enabled_ui(!stoppable.is_empty(), |ui| {
            if kill_button(ui).clicked() {
                send(actions, &stoppable, Command::Kill);
            }
        });
    });
}

/// Pin a field to one value on every running instance that has it, or unpin it.
fn pin_row(
    ui: &mut egui::Ui,
    ids: &[String],
    doc: &Document,
    state: &EngineState,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    let fields = common_fields(ids, doc);
    if fields.is_empty() {
        ui.weak("These instances have no field in common to pin.");
        return;
    }
    let field = match &ui_state.bulk_pin_field {
        Some(field) if fields.contains(field) => field.clone(),
        _ => fields[0].clone(),
    };
    let running: Vec<&String> = ids
        .iter()
        .filter(|id| {
            state
                .instances
                .get(*id)
                .is_some_and(|i| i.status.is_running())
        })
        .collect();
    let pinned: Vec<&String> = ids
        .iter()
        .filter(|id| {
            state
                .instances
                .get(*id)
                .is_some_and(|i| i.pinned.contains(&field))
        })
        .collect();
    ui.horizontal(|ui| {
        let label = ui.label("Pin");
        let mut chosen = field.clone();
        egui::ComboBox::from_id_salt("bulk-pin-field")
            .selected_text(&chosen)
            .show_ui(ui, |ui| {
                for name in &fields {
                    ui.selectable_value(&mut chosen, name.clone(), name);
                }
            })
            .response
            .labelled_by(label.id);
        ui_state.bulk_pin_field = Some(chosen);
        let to = ui.label("to");
        ui.add(TextEdit::singleline(&mut ui_state.bulk_pin_value).desired_width(120.0))
            .labelled_by(to.id);
        let value = ui_state.bulk_pin_value.trim();
        if ui
            .add_enabled(
                !running.is_empty() && !value.is_empty(),
                Button::new("Pin all"),
            )
            .on_hover_text("Pin the field on every selected instance that runs")
            .on_disabled_hover_text("Type a value, with at least one instance running")
            .clicked()
        {
            for id in &running {
                actions.push(Action::Engine(Command::Pin {
                    instance: (*id).clone(),
                    field: field.clone(),
                    value: Value::parse_input(value),
                }));
            }
        }
        if ui
            .add_enabled(!pinned.is_empty(), Button::new("Unpin all"))
            .on_disabled_hover_text("None of them has this field pinned")
            .clicked()
        {
            for id in &pinned {
                actions.push(Action::Engine(Command::Unpin {
                    instance: (*id).clone(),
                    field: field.clone(),
                }));
            }
        }
    });
}

/// The fields every one of `ids` has in the project, in name order.
fn common_fields(ids: &[String], doc: &Document) -> Vec<String> {
    let project = &doc.validated().project;
    let mut configs = ids.iter().filter_map(|id| project.instance(id));
    let Some(first) = configs.next() else {
        return Vec::new();
    };
    let mut fields: Vec<String> = first.fields.keys().cloned().collect();
    for config in configs {
        fields.retain(|f| config.fields.contains_key(f));
    }
    fields
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    use super::*;
    use crate::ui::test_support::{Fixture, instance, type_into_label};

    const SRC: &str = "[[devices]]\nid = \"d\"\ntemplate = \"co2\"\ncount = 3\n";

    fn ids() -> Vec<String> {
        ["d-0", "d-1", "d-2"].map(String::from).to_vec()
    }

    /// d-0 and d-1 online, d-2 stopped.
    fn harness() -> Harness<'static, Fixture> {
        let mut state = EngineState::default();
        for (id, status) in [
            ("d-0", InstanceStatus::Online),
            ("d-1", InstanceStatus::Online),
            ("d-2", InstanceStatus::Stopped),
        ] {
            state.instances.insert(id.into(), instance("d", status));
        }
        let mut harness = Harness::new_ui_state(
            move |ui, fx: &mut Fixture| {
                show(ui, &ids(), &fx.doc, &state, &mut fx.ui, &mut fx.actions);
            },
            Fixture::new(SRC),
        );
        harness.run();
        harness
    }

    fn commands(h: &Harness<'static, Fixture>) -> Vec<String> {
        h.state()
            .actions
            .iter()
            .filter_map(|a| match a {
                Action::Engine(command) => Some(format!("{command:?}")),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_heading_names_and_counts_them() {
        let h = harness();
        assert!(h.query_by_label("3 instances").is_some());
        assert!(h.query_by_label("d-0, d-1, d-2").is_some());
        assert!(h.query_by_label("2 online, 1 stopped").is_some());
    }

    #[test]
    fn stop_and_start_act_on_the_instances_they_apply_to() {
        let mut h = harness();
        h.get_by_label("Stop").click();
        h.run();
        h.get_by_label("Start").click();
        h.run();
        assert_eq!(
            commands(&h),
            [
                r#"Stop(Instance("d-0"))"#,
                r#"Stop(Instance("d-1"))"#,
                r#"Start(Instance("d-2"))"#,
            ]
        );
    }

    #[test]
    fn pin_all_pins_the_field_on_every_running_instance() {
        let mut h = harness();
        assert!(h.get_by_label("Pin all").accesskit_node().is_disabled());
        type_into_label(&mut h, "to", "2500");
        h.get_by_label("Pin all").click();
        h.run();
        let pins = commands(&h);
        assert_eq!(pins.len(), 2, "{pins:?}");
        assert!(
            pins[0].contains(r#"instance: "d-0", field: "co2""#),
            "{pins:?}"
        );
        assert!(pins[1].contains(r#"instance: "d-1""#), "{pins:?}");
    }
}
