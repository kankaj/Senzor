use eframe::egui::{self, Button};
use senzor_core::engine::{Command, EngineState, Target};
use senzor_core::project::builtin::builtin_templates;

use super::{Action, FIX_PROBLEMS_FIRST, LeftTab, UiState, WARNING, status_dot};
use crate::document::Document;
use crate::selection::Selection;
use crate::tree::{
    DeviceNode, InstanceNode, OrphanCause, device_tree, instance_ids, is_restartable, orphan_cause,
};

/// Devices with more instances than this start collapsed: laying out thousands of rows every
/// frame makes the GUI sluggish. Expanding one by hand still works.
const OPEN_BY_DEFAULT_MAX_INSTANCES: usize = 50;

pub fn show(
    ui: &mut egui::Ui,
    doc: &Document,
    state: &EngineState,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        ui.selectable_value(&mut ui_state.left_tab, LeftTab::Devices, "Devices");
        ui.selectable_value(&mut ui_state.left_tab, LeftTab::Templates, "Templates");
    });
    ui.separator();
    let selection = ui_state.selection.as_ref();
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| match ui_state.left_tab {
            LeftTab::Devices => devices(ui, doc, state, selection, actions),
            LeftTab::Templates => templates(ui, doc, selection, actions),
        });
}

fn devices(
    ui: &mut egui::Ui,
    doc: &Document,
    state: &EngineState,
    selection: Option<&Selection>,
    actions: &mut Vec<Action>,
) {
    if ui.button("Add device").clicked() {
        actions.push(Action::AddDevice);
    }
    for node in device_tree(doc.file(), doc.validated(), state) {
        let id = ui.make_persistent_id(("device", node.index, &node.id));
        let default_open = node.instances.len() <= OPEN_BY_DEFAULT_MAX_INSTANCES;
        egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            id,
            default_open,
        )
        .show_header(ui, |ui| {
            let label = match node.index {
                Some(_) if node.blocked => format!("{} ×{} (has errors)", node.id, node.count),
                Some(_) => format!("{} ×{}", node.id, node.count),
                None => format!("{} (removed)", node.id),
            };
            let selected = node
                .index
                .is_some_and(|i| selection == Some(&Selection::Device(i)));
            let response = ui.selectable_label(selected, label);
            if response.clicked()
                && let Some(index) = node.index
            {
                actions.push(Action::Select(Some(Selection::Device(index))));
            }
            response.context_menu(|ui| device_menu(ui, &node, actions));
        })
        .body(|ui| {
            let orphan_cause = orphan_cause(doc.validated(), &node.id);
            for instance in &node.instances {
                instance_row(ui, instance, orphan_cause, selection, actions);
            }
        });
    }
}

/// One instance under its device; `orphan_cause` is what its badge says if it is an orphan.
fn instance_row(
    ui: &mut egui::Ui,
    instance: &InstanceNode,
    orphan_cause: OrphanCause,
    selection: Option<&Selection>,
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        status_dot(ui, &instance.status);
        let selected = selection == Some(&Selection::Instance(instance.id.clone()));
        let response = ui.selectable_label(selected, &instance.id);
        if response.clicked() {
            actions.push(Action::Select(Some(Selection::Instance(
                instance.id.clone(),
            ))));
        }
        response.context_menu(|ui| instance_menu(ui, instance, actions));
        if instance.needs_restart {
            ui.small(egui::RichText::new("restart to apply").color(WARNING));
        }
        if instance.orphaned {
            let badge = match orphan_cause {
                OrphanCause::DeviceHasErrors => "device has errors",
                OrphanCause::DeviceRemoved => "removed",
            };
            ui.small(egui::RichText::new(badge).color(WARNING));
        }
    });
}

fn device_menu(ui: &mut egui::Ui, node: &DeviceNode, actions: &mut Vec<Action>) {
    if node.index.is_some()
        && ui
            .add_enabled(!node.blocked, Button::new("Start"))
            .on_disabled_hover_text(FIX_PROBLEMS_FIRST)
            .clicked()
    {
        actions.push(Action::Engine(Command::Start(Target::Device(
            node.id.clone(),
        ))));
        ui.close();
    }
    if ui.button("Stop").clicked() {
        for id in instance_ids(node) {
            actions.push(Action::Engine(Command::Stop(Target::Instance(id))));
        }
        ui.close();
    }
    // A device with errors runs nothing but orphans.
    if node.index.is_some() && !node.blocked {
        let running: Vec<&InstanceNode> = node
            .instances
            .iter()
            .filter(|i| is_restartable(&i.status, i.orphaned))
            .collect();
        if ui
            .add_enabled(!running.is_empty(), Button::new("Restart"))
            .on_disabled_hover_text("No running instances to restart")
            .clicked()
        {
            for instance in running {
                actions.push(Action::Engine(Command::Restart(Target::Instance(
                    instance.id.clone(),
                ))));
            }
            ui.close();
        }
    }
    if ui.button("Kill").clicked() {
        for id in instance_ids(node) {
            actions.push(Action::Engine(Command::Kill(Target::Instance(id))));
        }
        ui.close();
    }
}

fn instance_menu(ui: &mut egui::Ui, instance: &InstanceNode, actions: &mut Vec<Action>) {
    let target = || Target::Instance(instance.id.clone());
    if ui.button("Start").clicked() {
        actions.push(Action::Engine(Command::Start(target())));
        ui.close();
    }
    if ui.button("Stop").clicked() {
        actions.push(Action::Engine(Command::Stop(target())));
        ui.close();
    }
    // Not for an orphan: the engine only stops it on Restart, as it has no settings to start
    // with. Unlike the device menu, this also offers Restart on a stopped instance, which starts it.
    if !instance.orphaned && ui.button("Restart").clicked() {
        actions.push(Action::Engine(Command::Restart(target())));
        ui.close();
    }
    if ui.button("Kill").clicked() {
        actions.push(Action::Engine(Command::Kill(target())));
        ui.close();
    }
}

fn templates(
    ui: &mut egui::Ui,
    doc: &Document,
    selection: Option<&Selection>,
    actions: &mut Vec<Action>,
) {
    if ui.button("Add template").clicked() {
        actions.push(Action::AddTemplate);
    }
    ui.add_space(4.0);
    ui.weak("Built-in");
    for template in builtin_templates() {
        let selected = selection == Some(&Selection::Builtin(template.id.clone()));
        if ui.selectable_label(selected, &template.id).clicked() {
            actions.push(Action::Select(Some(Selection::Builtin(
                template.id.clone(),
            ))));
        }
    }
    ui.add_space(4.0);
    ui.weak("Project");
    for (index, template) in doc.file().templates.iter().enumerate() {
        let selected = selection == Some(&Selection::Template(index));
        if ui.selectable_label(selected, &template.id).clicked() {
            actions.push(Action::Select(Some(Selection::Template(index))));
        }
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use senzor_core::engine::InstanceStatus;

    use super::*;
    use crate::ui::test_support::{Fixture, assert_disabled_with_hint, instance};

    const SRC: &str = r#"
[[devices]]
id = "big"
template = "co2"
count = 60

[[devices]]
id = "small"
template = "co2"
count = 3
"#;

    const BLOCKED: &str = "[[devices]]\nid = \"bad\"\ntemplate = \"gone\"\n";

    fn harness() -> Harness<'static, Fixture> {
        harness_with(SRC, EngineState::default())
    }

    /// Unlike [`Fixture::harness`], leaves what the panel pushes in `actions`, where the tests
    /// read it.
    fn harness_with(src: &str, state: EngineState) -> Harness<'static, Fixture> {
        let mut harness = Harness::new_ui_state(
            move |ui, fx: &mut Fixture| show(ui, &fx.doc, &state, &mut fx.ui, &mut fx.actions),
            Fixture::new(src),
        );
        harness.run();
        harness
    }

    /// An engine running `id` (of `device`) although it is not in the project.
    fn running_orphan(id: &str, device: &str) -> EngineState {
        let mut orphan = instance(device, InstanceStatus::Online);
        orphan.orphaned = true;
        let mut state = EngineState::default();
        state.instances.insert(id.to_string(), orphan);
        state
    }

    #[test]
    fn an_orphan_whose_device_has_errors_is_badged_device_has_errors() {
        let h = harness_with(BLOCKED, running_orphan("bad", "bad"));
        assert!(h.query_by_label("device has errors").is_some());
        assert!(h.query_by_label("removed").is_none());
    }

    #[test]
    fn an_orphan_whose_device_was_removed_is_badged_removed() {
        let h = harness_with(SRC, running_orphan("old-0", "old"));
        assert!(h.query_by_label("removed").is_some());
        assert!(h.query_by_label("device has errors").is_none());
    }

    /// Clicks a device's open/close arrow. It has no label of its own, so it is found as the
    /// space just left of the header label.
    fn click_toggle(harness: &mut Harness<'static, Fixture>, header: &str) {
        let label = harness.get_by_label(header).rect();
        let pos = egui::pos2(label.left() - 4.0, label.center().y);
        harness.hover_at(pos);
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
        harness.run();
    }

    #[test]
    fn a_device_with_many_instances_starts_collapsed() {
        let h = harness();
        assert!(h.query_by_label("big ×60").is_some());
        assert!(h.query_by_label("big-0").is_none());
    }

    #[test]
    fn a_device_with_few_instances_starts_expanded() {
        let h = harness();
        assert!(h.query_by_label("small-0").is_some());
    }

    fn choose_from_context_menu(h: &mut Harness<'static, Fixture>, label: &str, item: &str) {
        h.get_by_label(label).click_secondary();
        h.run();
        h.get_by_label(item).click();
        h.run();
    }

    fn restarted(h: &Harness<'static, Fixture>) -> Vec<String> {
        h.state()
            .actions
            .iter()
            .filter_map(|a| match a {
                Action::Engine(Command::Restart(Target::Instance(id))) => Some(id.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn restart_in_an_instance_menu_restarts_that_instance() {
        let mut h = harness();
        choose_from_context_menu(&mut h, "small-1", "Restart");
        assert_eq!(restarted(&h), ["small-1"]);
    }

    /// Restart would start a stopped instance and stop an orphan for good.
    #[test]
    fn restart_in_a_device_menu_restarts_only_its_running_instances() {
        let mut state = running_orphan("small-7", "small");
        for (id, status) in [
            ("small-0", InstanceStatus::Online),
            ("small-2", InstanceStatus::Connecting),
        ] {
            state
                .instances
                .insert(id.to_string(), instance("small", status));
        }
        let mut h = harness_with(SRC, state);
        choose_from_context_menu(&mut h, "small ×3", "Restart");
        assert_eq!(restarted(&h), ["small-0", "small-2"]);
    }

    /// Opens the context menu of `label` and checks that `item` is disabled with `hint` and
    /// pushes nothing when clicked.
    fn menu_item_is_disabled_with_hint(
        h: &mut Harness<'static, Fixture>,
        label: &str,
        item: &str,
        hint: &str,
    ) {
        h.get_by_label(label).click_secondary();
        h.run();
        assert_disabled_with_hint(h, item, hint);
    }

    #[test]
    fn restart_in_a_device_menu_is_disabled_when_none_of_its_instances_runs() {
        let mut h = harness();
        menu_item_is_disabled_with_hint(
            &mut h,
            "small ×3",
            "Restart",
            "No running instances to restart",
        );
    }

    #[test]
    fn start_in_the_menu_of_a_device_with_errors_is_disabled() {
        let mut h = harness_with(BLOCKED, EngineState::default());
        menu_item_is_disabled_with_hint(
            &mut h,
            "bad ×1 (has errors)",
            "Start",
            "Fix the problems in the Problems tab first",
        );
    }

    /// The engine only stops an orphan on Restart: it has no settings to start with.
    fn menu_offers_no_restart(label: &str) {
        let mut h = harness_with(BLOCKED, running_orphan("bad", "bad"));
        h.get_by_label(label).click_secondary();
        h.run();
        assert!(h.query_by_label("Restart").is_none());
        assert!(h.query_by_label("Stop").is_some());
        assert!(h.query_by_label("Kill").is_some());
    }

    #[test]
    fn the_menu_of_an_orphan_offers_no_restart() {
        menu_offers_no_restart("bad");
    }

    #[test]
    fn the_menu_of_a_device_with_errors_offers_no_restart() {
        menu_offers_no_restart("bad ×1 (has errors)");
    }

    #[test]
    fn a_collapsed_device_opens_when_its_arrow_is_clicked() {
        let mut h = harness();
        click_toggle(&mut h, "big ×60");
        assert!(h.query_by_label("big-0").is_some());
        assert!(h.state().actions.is_empty());
    }
}
