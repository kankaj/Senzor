use eframe::egui::{self, Button};
use senzor_core::engine::{Command, EngineState, Target};
use senzor_core::project::builtin::builtin_templates;

use super::{
    Action, FIX_PROBLEMS_FIRST, LeftTab, UiState, WARNING, kill_button, status_dot, stop_button,
};
use crate::document::Document;
use crate::selection::Selection;
use crate::tree::{
    DeviceNode, InstanceNode, OrphanCause, StatusFilter, TreeFilter, device_tree, filter_tree,
    instance_ids, is_restartable, orphan_cause,
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
    if ui_state.left_tab == LeftTab::Devices {
        filter_row(ui, &mut ui_state.tree_filter);
    }
    let selection = ui_state.selection.as_ref();
    egui::ScrollArea::vertical()
        .auto_shrink([false; 2])
        .show(ui, |ui| match ui_state.left_tab {
            LeftTab::Devices => devices(ui, doc, state, selection, &ui_state.tree_filter, actions),
            LeftTab::Templates => templates(ui, doc, selection, actions),
        });
}

/// The search box and the status filter above the device tree.
fn filter_row(ui: &mut egui::Ui, filter: &mut TreeFilter) {
    ui.horizontal(|ui| {
        let icon = ui.label(egui_phosphor::regular::MAGNIFYING_GLASS);
        icon.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, "Find devices")
        });
        let status = egui::ComboBox::from_id_salt("tree-status")
            .selected_text(filter.status.label())
            .width(80.0);
        // The box takes what the status filter leaves of the row.
        let box_width = (ui.available_width() - 100.0).max(60.0);
        ui.add(
            egui::TextEdit::singleline(&mut filter.text)
                .hint_text("device or instance")
                .desired_width(box_width),
        )
        .labelled_by(icon.id);
        let response = status
            .show_ui(ui, |ui| {
                for option in StatusFilter::ALL {
                    ui.selectable_value(&mut filter.status, option, option.label());
                }
            })
            .response;
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Status"));
        response.on_hover_text("Show only instances in this state");
    });
}

fn devices(
    ui: &mut egui::Ui,
    doc: &Document,
    state: &EngineState,
    selection: Option<&Selection>,
    filter: &TreeFilter,
    actions: &mut Vec<Action>,
) {
    if ui.button("Add device").clicked() {
        actions.push(Action::AddDevice);
    }
    let tree = device_tree(doc.file(), doc.validated(), state);
    // Counted before the filter hides instances.
    let online: Vec<Option<String>> = tree.iter().map(online_badge).collect();
    let filtering = filter.is_active();
    let shown: Vec<(DeviceNode, Option<String>)> = if filtering {
        let kept = filter_tree(tree.clone(), filter);
        kept.into_iter()
            .map(|node| {
                let at = tree
                    .iter()
                    .position(|n| n.index == node.index && n.id == node.id);
                let badge = at.and_then(|at| online[at].clone());
                (node, badge)
            })
            .collect()
    } else {
        tree.into_iter().zip(online).collect()
    };
    if filtering && shown.is_empty() {
        ui.weak("No device or instance matches.");
    }
    for (node, badge) in shown {
        // A filtered tree opens what it found, without changing what stays open afterwards.
        let id = ui.make_persistent_id(("device", node.index, &node.id, filtering));
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
            if let Some(badge) = badge {
                ui.weak(badge);
            }
        })
        .body(|ui| {
            let orphan_cause = orphan_cause(doc.validated(), &node.id);
            for instance in &node.instances {
                instance_row(ui, instance, orphan_cause, selection, actions);
            }
        });
    }
}

/// "3/5 online" for a device with an instance that is not stopped; nothing for a stopped one.
fn online_badge(node: &DeviceNode) -> Option<String> {
    let active = node
        .instances
        .iter()
        .any(|i| i.status != senzor_core::engine::InstanceStatus::Stopped);
    active.then(|| format!("{}/{} online", node.online(), node.instances.len()))
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
    if stop_button(ui).clicked() {
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
    if kill_button(ui).clicked() {
        for id in instance_ids(node) {
            actions.push(Action::Engine(Command::Kill(Target::Instance(id))));
        }
        ui.close();
    }
    // An orphan's device is gone; there is nothing to copy.
    if node.index.is_some() {
        ui.separator();
        if ui.button("Duplicate").clicked() {
            actions.push(Action::DuplicateDevice(node.id.clone()));
            ui.close();
        }
    }
}

fn instance_menu(ui: &mut egui::Ui, instance: &InstanceNode, actions: &mut Vec<Action>) {
    let target = || Target::Instance(instance.id.clone());
    // An orphan has no settings to start with; it is already running anyway.
    if ui
        .add_enabled(!instance.orphaned, Button::new("Start"))
        .on_disabled_hover_text("This instance is no longer in the project")
        .clicked()
    {
        actions.push(Action::Engine(Command::Start(target())));
        ui.close();
    }
    if stop_button(ui).clicked() {
        actions.push(Action::Engine(Command::Stop(target())));
        ui.close();
    }
    // Not for an orphan: the engine only stops it on Restart, as it has no settings to start
    // with. Unlike the device menu, this also offers Restart on a stopped instance, which starts it.
    if !instance.orphaned && ui.button("Restart").clicked() {
        actions.push(Action::Engine(Command::Restart(target())));
        ui.close();
    }
    if kill_button(ui).clicked() {
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
        let response = ui.selectable_label(selected, &template.id);
        if response.clicked() {
            actions.push(Action::Select(Some(Selection::Template(index))));
        }
        response.context_menu(|ui| {
            if ui.button("Duplicate").clicked() {
                actions.push(Action::DuplicateTemplate(template.id.clone()));
                ui.close();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};
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
    fn duplicate_in_a_device_menu_asks_to_copy_that_device() {
        let mut h = harness();
        choose_from_context_menu(&mut h, "small ×3", "Duplicate");
        assert!(
            h.state()
                .actions
                .iter()
                .any(|a| matches!(a, Action::DuplicateDevice(id) if id == "small"))
        );
    }

    #[test]
    fn duplicate_in_a_template_menu_asks_to_copy_that_template() {
        let mut h = harness_with(
            "[[templates]]\nid = \"mine\"\nmessages = [{ topic = \"t\", payload = \"1\" }]\nfields.v = { generator = \"constant\", value = 1 }\n",
            EngineState::default(),
        );
        h.state_mut().ui.left_tab = LeftTab::Templates;
        h.run();
        choose_from_context_menu(&mut h, "mine", "Duplicate");
        assert!(
            h.state()
                .actions
                .iter()
                .any(|a| matches!(a, Action::DuplicateTemplate(id) if id == "mine"))
        );
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
    fn the_menu_of_an_orphan_cannot_start_it() {
        let mut h = harness_with(BLOCKED, running_orphan("bad", "bad"));
        h.get_by_label("bad").click_secondary();
        h.run();
        assert!(h.get_by_label("Start").accesskit_node().is_disabled());
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
    fn typing_in_find_devices_hides_the_devices_that_do_not_match() {
        let mut h = harness();
        let find = h.get_by_label("Find devices");
        find.focus();
        find.type_text("SMA");
        h.run();
        assert!(h.query_by_label("big ×60").is_none());
        assert!(h.query_by_label("small ×3").is_some());
        assert!(h.query_by_label("small-2").is_some());
    }

    #[test]
    fn a_filter_that_matches_nothing_says_so() {
        let mut h = harness();
        let find = h.get_by_label("Find devices");
        find.focus();
        find.type_text("nothing-like-this");
        h.run();
        assert!(h.query_by_label("No device or instance matches.").is_some());
    }

    #[test]
    fn the_status_filter_shows_only_instances_in_that_status() {
        let mut state = EngineState::default();
        state
            .instances
            .insert("small-1".into(), instance("small", InstanceStatus::Online));
        let mut h = harness_with(SRC, state);
        h.get_by_label("Status").click();
        h.run();
        h.get_by_label("Online").click();
        h.run();
        assert!(h.query_by_label("small-1").is_some());
        assert!(h.query_by_label("small-0").is_none());
        assert!(h.query_by_label("big ×60").is_none());
    }

    #[test]
    fn a_running_device_shows_how_many_of_its_instances_are_online() {
        let mut state = EngineState::default();
        state
            .instances
            .insert("small-1".into(), instance("small", InstanceStatus::Online));
        let h = harness_with(SRC, state);
        assert!(h.query_by_label("1/3 online").is_some());
        assert!(
            h.query_by_label("0/60 online").is_none(),
            "a stopped device shows no count"
        );
    }

    #[test]
    fn a_collapsed_device_opens_when_its_arrow_is_clicked() {
        let mut h = harness();
        click_toggle(&mut h, "big ×60");
        assert!(h.query_by_label("big-0").is_some());
        assert!(h.state().actions.is_empty());
    }
}
