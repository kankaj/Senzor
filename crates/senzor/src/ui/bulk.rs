//! The detail panel for several instances selected together: run them, and pin a field on all.

use std::collections::BTreeMap;

use eframe::egui::{self, Button, TextEdit};
use senzor_core::engine::{Command, EngineState, InstanceStatus, Target};
use senzor_core::value::Value;

use super::{Action, UiState, kill_button, stop_button};
use crate::document::Document;
use crate::i18n::t;
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
    ui.heading(t!("bulk-heading", count = ids.len()));
    ui.label(names(ids));
    ui.weak(summary(ids, state));
    ui.weak(t!("bulk-selection-hint"));
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
        more => t!("bulk-names-more", names = shown.join(", "), more = more),
    }
}

/// How many of them are in each state, e.g. "2 online, 1 stopped".
fn summary(ids: &[String], state: &EngineState) -> String {
    let mut counts: BTreeMap<Status, usize> = BTreeMap::new();
    for id in ids {
        let status = match state.instances.get(id).map(|i| &i.status) {
            Some(InstanceStatus::Online) => Status::Online,
            Some(InstanceStatus::Connecting) => Status::Connecting,
            Some(InstanceStatus::Reconnecting { .. }) => Status::Reconnecting,
            Some(InstanceStatus::Failed { .. }) => Status::Failed,
            Some(InstanceStatus::Stopped) | None => Status::Stopped,
        };
        *counts.entry(status).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(status, count)| status.count(count))
        .collect::<Vec<_>>()
        .join(", ")
}

/// An instance's state in [`summary`], in the order it lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Status {
    Connecting,
    Failed,
    Online,
    Reconnecting,
    Stopped,
}

impl Status {
    /// `count` instances in this state, e.g. "2 online".
    fn count(self, count: usize) -> String {
        match self {
            Status::Connecting => t!("bulk-count-connecting", count = count),
            Status::Failed => t!("bulk-count-failed", count = count),
            Status::Online => t!("bulk-count-online", count = count),
            Status::Reconnecting => t!("bulk-count-reconnecting", count = count),
            Status::Stopped => t!("bulk-count-stopped", count = count),
        }
    }
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
            .add_enabled(!startable.is_empty(), Button::new(t!("bulk-start")))
            .on_hover_text(t!("bulk-start-hint"))
            .on_disabled_hover_text(t!("bulk-start-disabled"))
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
            .add_enabled(!restartable.is_empty(), Button::new(t!("bulk-restart")))
            .on_hover_text(t!("bulk-restart-hint"))
            .on_disabled_hover_text(t!("bulk-restart-disabled"))
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
        ui.weak(t!("bulk-no-common-field"));
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
        let label = ui.label(t!("bulk-pin"));
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
        let to = ui.label(t!("bulk-pin-to"));
        ui.add(TextEdit::singleline(&mut ui_state.bulk_pin_value).desired_width(120.0))
            .labelled_by(to.id);
        let value = ui_state.bulk_pin_value.trim();
        if ui
            .add_enabled(
                !running.is_empty() && !value.is_empty(),
                Button::new(t!("bulk-pin-all")),
            )
            .on_hover_text(t!("bulk-pin-all-hint"))
            .on_disabled_hover_text(t!("bulk-pin-all-disabled"))
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
            .add_enabled(!pinned.is_empty(), Button::new(t!("bulk-unpin-all")))
            .on_disabled_hover_text(t!("bulk-unpin-all-disabled"))
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
    fn the_heading_counts_them_in_czech() {
        crate::i18n::set_language(crate::i18n::Language::Czech);
        let h = harness();
        assert!(h.query_by_label("3 instance").is_some());
        assert!(h.query_by_label("2 online, 1 zastavená").is_some());
        assert!(h.query_by_label("Připnout všem").is_some());
    }

    #[test]
    fn czech_counts_take_the_plural_their_number_needs() {
        crate::i18n::set_language(crate::i18n::Language::Czech);
        let ids: Vec<String> = (0..12).map(|i| format!("d-{i}")).collect();
        assert_eq!(
            names(&ids),
            "d-0, d-1, d-2, d-3, d-4, d-5, d-6, d-7 a 4 další"
        );
        assert_eq!(Status::Stopped.count(5), "5 zastavených");
        assert_eq!(Status::Failed.count(2), "2 selhaly");
        assert_eq!(t!("bulk-heading", count = 5), "5 instancí");
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
