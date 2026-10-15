use eframe::egui::{self, Button, DragValue, TextEdit};
use senzor_core::engine::{Command, Target};
use senzor_core::project::builtin::builtin_templates;
use senzor_core::project::file::{DEFAULT_INTERVAL, default_instance_id_pattern};
use senzor_core::project::{DeviceSpec, Location, ProjectFile};
use senzor_core::value::Value;

use super::{
    Action, ERROR, FIX_PROBLEMS_FIRST, ParsedEdit, UiState, duration_edit, errors_at, name_edit,
    replace_if_unchanged, value_edit,
};
use crate::document::Document;
use crate::edits::{
    Removal, Rename, find_template, override_from_text, override_to_text, set_count,
    template_interval, unique_name,
};

pub fn show(
    ui: &mut egui::Ui,
    index: usize,
    doc: &Document,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    let original = &doc.file().devices[index];
    let mut device = original.clone();
    ui.heading(format!("Device {}", original.id));
    let has_errors = errors_at(
        ui,
        doc,
        &Location::Device {
            id: original.id.clone(),
        },
    );
    ui.horizontal(|ui| {
        // A blocked device cannot start; its own errors are above, a broken template's or
        // broker's are only in the Problems tab.
        let hint = if has_errors {
            "Fix the problems listed above first"
        } else {
            FIX_PROBLEMS_FIRST
        };
        if ui
            .add_enabled(
                !doc.validated().blocked.contains(&original.id),
                Button::new("Start device"),
            )
            .on_disabled_hover_text(hint)
            .clicked()
        {
            actions.push(Action::Engine(Command::Start(Target::Device(
                original.id.clone(),
            ))));
        }
        if ui.button("Delete device").clicked() {
            actions.push(Action::Remove(Removal::Device(index)));
            actions.push(Action::Select(None));
        }
    });
    ui.separator();

    egui::Grid::new("device-form")
        .num_columns(2)
        .show(ui, |ui| {
            let label = ui.label("ID");
            let (response, rename) = name_edit(
                ui,
                ui_state,
                &format!("device/{index}/id"),
                &original.id,
                doc.file(),
                |to| Rename::Device {
                    index,
                    from: original.id.clone(),
                    to,
                },
            );
            response.labelled_by(label.id);
            actions.extend(rename.map(Action::Rename));
            ui.end_row();

            ui.label("Template");
            egui::ComboBox::from_id_salt("device-template")
                .selected_text(device.template.clone())
                .show_ui(ui, |ui| {
                    for id in template_ids(doc) {
                        ui.selectable_value(&mut device.template, id.clone(), id);
                    }
                });
            ui.end_row();

            let label = ui.label("Count");
            let mut count = device.count;
            ui.add(DragValue::new(&mut count).range(1..=10_000))
                .labelled_by(label.id);
            set_count(&mut device, count);
            ui.end_row();

            let label = ui.label("Instance IDs");
            ui.horizontal(|ui| {
                let mut custom = device.instance_id.is_some();
                // Starting from the pattern in use keeps the instance IDs until the user types.
                if ui.checkbox(&mut custom, "custom").changed() {
                    device.instance_id =
                        custom.then(|| default_instance_id_pattern(device.count).to_string());
                }
                match &mut device.instance_id {
                    Some(pattern) => {
                        ui.add(
                            TextEdit::singleline(pattern)
                                .id_salt(format!("device/{index}/instance_id")),
                        )
                        .labelled_by(label.id);
                    }
                    None if device.count == 1 => {
                        ui.weak(&device.id);
                    }
                    None => {
                        ui.weak(format!(
                            "{}-0 … {}-{}",
                            device.id,
                            device.id,
                            device.count - 1
                        ));
                    }
                }
            });
            ui.end_row();

            ui.label("Autostart");
            ui.checkbox(&mut device.autostart, "start when the project opens");
            ui.end_row();

            let label = ui.label("Interval");
            ui.horizontal(|ui| {
                let mut custom = device.interval.is_some();
                if ui.checkbox(&mut custom, "override").changed() {
                    device.interval = custom.then(|| {
                        template_interval(doc.file(), &device.template).unwrap_or(DEFAULT_INTERVAL)
                    });
                }
                if let Some(interval) = &mut device.interval {
                    duration_edit(
                        ui,
                        &mut ui_state.buffers,
                        &format!("device/{index}/interval"),
                        interval,
                    )
                    .labelled_by(label.id);
                }
            });
            ui.end_row();
        });

    ui.separator();
    params(ui, index, &mut device, doc.file(), ui_state, actions);
    ui.separator();
    overrides(ui, index, &mut device, doc, ui_state);

    actions.extend(replace_if_unchanged(original, device, move |f| {
        f.devices.get_mut(index)
    }));
}

fn template_ids(doc: &Document) -> Vec<String> {
    builtin_templates()
        .iter()
        .map(|t| t.id.clone())
        .chain(doc.file().templates.iter().map(|t| t.id.clone()))
        .collect()
}

/// Param rows. Names commit on Enter or when the box loses focus, so typing does not reorder rows.
fn params(
    ui: &mut egui::Ui,
    index: usize,
    device: &mut DeviceSpec,
    file: &ProjectFile,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    ui.strong("Params");
    let keys: Vec<String> = device.params.keys().cloned().collect();
    for key in keys {
        ui.horizontal(|ui| {
            let (_, rename) = name_edit(
                ui,
                ui_state,
                &format!("device/{index}/param/{key}"),
                &key,
                file,
                |to| Rename::Param {
                    device: index,
                    from: key.clone(),
                    to,
                },
            );
            actions.extend(rename.map(Action::Rename));

            if let Some(value) = device.params.get_mut(&key) {
                value_edit(
                    ui,
                    &mut ui_state.buffers,
                    &format!("device/{index}/param/{key}/value"),
                    value,
                );
            }
            if ui.button("Remove").clicked() {
                device.params.remove(&key);
            }
        });
    }
    if ui.button("Add param").clicked() {
        let name = unique_name("param", device.params.keys());
        device.params.insert(name, Value::String(String::new()));
    }
}

/// One line per template field: an inline TOML table merged onto (or replacing) the field.
/// Overrides of fields the template does not have (after a template switch) get a Remove button.
fn overrides(
    ui: &mut egui::Ui,
    index: usize,
    device: &mut DeviceSpec,
    doc: &Document,
    ui_state: &mut UiState,
) {
    ui.strong("Field overrides");
    ui.weak("Keys are merged onto the template's field; with `generator` they replace it.");
    let fields: Vec<String> = find_template(doc.file(), &device.template)
        .map(|t| t.fields.keys().cloned().collect())
        .unwrap_or_default();
    let stranded: Vec<String> = device
        .fields
        .keys()
        .filter(|f| !fields.contains(f))
        .cloned()
        .collect();
    egui::Grid::new("device-overrides")
        .num_columns(2)
        .show(ui, |ui| {
            for field in fields {
                let label = ui.label(&field);
                let mut table = device.fields.remove(&field);
                // Vertical, so a parse error goes under the box.
                ui.vertical(|ui| {
                    ParsedEdit::new(&format!("device/{index}/override/{field}"), 280.0)
                        .hint("e.g. { start = 1200 }")
                        .show(
                            ui,
                            &mut ui_state.buffers,
                            &mut table,
                            |table| table.as_ref().map(override_to_text).unwrap_or_default(),
                            override_from_text,
                        )
                        .labelled_by(label.id);
                });
                if let Some(table) = table {
                    device.fields.insert(field, table);
                }
                ui.end_row();
            }
            for field in stranded {
                ui.colored_label(ERROR, format!("{field} (not in the template)"));
                if ui.button("Remove override").clicked() {
                    device.fields.remove(&field);
                }
                ui.end_row();
            }
        });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    use super::*;
    use crate::ui::test_support::{
        Fixture, assert_disabled_with_hint, type_into_label, type_into_value, type_per_frame,
    };

    const SRC: &str = r#"
[[devices]]
id = "office"
template = "co2"
count = 2
fields.co2 = { start = 1200 }
"#;

    fn harness() -> Harness<'static, Fixture> {
        harness_for(SRC)
    }

    /// The form of the first device of `src`.
    fn harness_for(src: &str) -> Harness<'static, Fixture> {
        Fixture::new(src).harness(|ui, fx| {
            if !fx.doc.file().devices.is_empty() {
                show(ui, 0, &fx.doc, &mut fx.ui, &mut fx.actions);
            }
        })
    }

    #[test]
    fn add_param_adds_an_empty_param() {
        let mut h = harness();
        h.get_by_label("Add param").click();
        h.run();
        let params = &h.state().doc.file().devices[0].params;
        assert_eq!(params.get("param"), Some(&Value::String(String::new())));
    }

    #[test]
    fn an_override_that_does_not_parse_is_shown_and_not_applied() {
        let mut h = harness();
        type_into_label(&mut h, "co2", " junk");
        let fields = &h.state().doc.file().devices[0].fields;
        assert_eq!(fields["co2"]["start"].as_integer(), Some(1200));
        assert_eq!(
            h.get_by_label("co2").value().as_deref(),
            Some("{ start = 1200 } junk")
        );
        assert!(h.query_by_label_contains("expected").is_some());
    }

    #[test]
    fn an_invalid_interval_keeps_the_old_value_and_shows_a_hint() {
        let mut h = harness();
        h.get_by_label("override").click();
        h.run();
        assert_eq!(
            h.state().doc.file().devices[0].interval,
            Some(Duration::from_secs(10))
        );
        type_into_label(&mut h, "Interval", "x");
        assert!(h.query_by_label("use e.g. 10s, 500ms, 1h").is_some());
        assert_eq!(
            h.state().doc.file().devices[0].interval,
            Some(Duration::from_secs(10))
        );
    }

    #[test]
    fn start_device_starts_a_device_without_problems() {
        let mut h = harness();
        h.get_by_label("Start device").click();
        h.run();
        assert!(matches!(
            &h.state().other[..],
            [Action::Engine(Command::Start(Target::Device(id)))] if id == "office"
        ));
    }

    #[test]
    fn start_device_is_disabled_while_the_device_has_problems() {
        let mut h = harness_for("[[devices]]\nid = \"office\"\ntemplate = \"gone\"\n");
        assert_disabled_with_hint(
            &mut h,
            "Start device",
            "Fix the problems listed above first",
        );
    }

    #[test]
    fn start_device_is_disabled_while_its_template_has_problems() {
        let mut h = harness_for(
            r#"
[[templates]]
id = "broken"
messages = [{ topic = "a", payload = "{{ v" }]
fields.v = { generator = "constant", value = 1 }

[[devices]]
id = "office"
template = "broken"
"#,
        );
        assert_disabled_with_hint(
            &mut h,
            "Start device",
            "Fix the problems in the Problems tab first",
        );
    }

    #[test]
    fn delete_device_removes_it_and_clears_the_selection() {
        let mut h = harness();
        h.get_by_label("Delete device").click();
        h.run();
        assert!(h.state().doc.file().devices.is_empty());
        assert!(matches!(h.state().other[..], [Action::Select(None)]));
    }

    const WITH_PARAM: &str = r#"
[[devices]]
id = "office"
template = "co2"
params = { room = "kitchen" }
"#;

    #[test]
    fn deleting_with_a_pending_param_rename_deletes_the_right_device() {
        let src = format!("{WITH_PARAM}\n[[devices]]\nid = \"lab\"\ntemplate = \"co2\"\n");
        let mut h = harness_for(&src);
        type_into_value(&mut h, "room", "2");
        h.get_by_label("Delete device").click();
        h.run();
        let ids: Vec<&str> = h
            .state()
            .doc
            .file()
            .devices
            .iter()
            .map(|d| d.id.as_str())
            .collect();
        assert_eq!(ids, ["lab"]);
    }

    #[test]
    fn deleting_the_only_device_with_a_pending_param_rename_does_not_panic() {
        let mut h = harness_for(WITH_PARAM);
        type_into_value(&mut h, "room", "2");
        h.get_by_label("Delete device").click();
        h.run();
        assert!(h.state().doc.file().devices.is_empty());
    }

    fn with_room(value: &str) -> Harness<'static, Fixture> {
        harness_for(&format!(
            "[[devices]]\nid = \"office\"\ntemplate = \"co2\"\nparams = {{ room = {value} }}\n"
        ))
    }

    fn room(h: &Harness<'static, Fixture>) -> Value {
        h.state().doc.file().devices[0].params["room"].clone()
    }

    fn switch_type(h: &mut Harness<'static, Fixture>, from: &str, to: &str) {
        h.get_by_value(from).click();
        h.run();
        h.get_by_label(to).click();
        h.run();
    }

    #[test]
    fn editing_a_text_param_that_looks_like_a_number_keeps_it_text() {
        let mut h = with_room(r#""007""#);
        type_into_value(&mut h, "007", "1");
        assert_eq!(room(&h), Value::String("0071".into()));
    }

    #[test]
    fn switching_a_param_to_number_converts_its_text() {
        let mut h = with_room(r#""42""#);
        switch_type(&mut h, "text", "number");
        assert_eq!(room(&h), Value::Number(42.0));
    }

    #[test]
    fn an_invalid_number_param_keeps_the_old_value_and_shows_a_hint() {
        let mut h = with_room("42");
        type_into_value(&mut h, "42", "x");
        assert_eq!(room(&h), Value::Number(42.0));
        assert!(h.query_by_label("use a number, e.g. 42 or 22.5").is_some());
    }

    #[test]
    fn switching_the_type_keeps_the_unapplied_text() {
        let mut h = with_room("42");
        type_into_value(&mut h, "42", "x");
        switch_type(&mut h, "number", "text");
        assert_eq!(room(&h), Value::String("42x".into()));
        assert!(h.query_all_by_value("42x").next().is_some());
    }

    #[test]
    fn a_type_the_text_does_not_fit_cannot_be_chosen() {
        let mut h = with_room(r#""abc""#);
        h.get_by_value("text").click();
        h.run();
        assert!(h.get_by_label("bool").accesskit_node().is_disabled());
        let number = h.get_by_label("number");
        assert!(number.accesskit_node().is_disabled());
        number.hover();
        h.run();
        assert!(h.query_by_label("use a number, e.g. 42 or 22.5").is_some());

        h.get_by_label("number").click();
        h.run();
        assert_eq!(room(&h), Value::String("abc".into()));
        assert!(h.query_all_by_value("abc").next().is_some());
    }

    #[test]
    fn the_device_id_changes_only_when_the_box_is_left() {
        let mut h = harness();
        type_into_label(&mut h, "ID", "-b");
        assert_eq!(h.state().doc.file().devices[0].id, "office");
        h.key_press(egui::Key::Enter);
        h.run();
        assert_eq!(h.state().doc.file().devices[0].id, "office-b");
    }

    #[test]
    fn typing_an_instance_id_pattern_one_character_at_a_time_keeps_focus() {
        let mut h = harness();
        h.get_by_label("custom").click();
        h.run();
        h.get_all_by_value("{{ id }}-{{ index }}")
            .next()
            .unwrap()
            .focus();
        h.run();
        type_per_frame(&mut h, &["{", "{", "x"]);
        assert_eq!(
            h.state().doc.file().devices[0].instance_id.as_deref(),
            Some("{{ id }}-{{ index }}{{x")
        );
    }

    #[test]
    fn ticking_custom_instance_ids_keeps_the_instance_ids_of_a_single_device() {
        let mut h = harness_for("[[devices]]\nid = \"office\"\ntemplate = \"co2\"\n");
        h.get_by_label("custom").click();
        h.run();
        let ids: Vec<&str> = h
            .state()
            .doc
            .validated()
            .project
            .instances()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(ids, ["office"]);
    }

    #[test]
    fn an_unedited_custom_instance_id_follows_the_count() {
        let mut h = harness_for("[[devices]]\nid = \"office\"\ntemplate = \"co2\"\n");
        h.get_by_label("custom").click();
        h.run();
        h.get_by_label("Count").focus();
        h.run();
        type_per_frame(&mut h, &["3"]);
        assert_eq!(h.state().doc.file().devices[0].count, 3);
        let validated = h.state().doc.validated();
        assert!(validated.errors.is_empty(), "{:?}", validated.errors);
        let ids: Vec<&str> = validated
            .project
            .instances()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(ids, ["office-0", "office-1", "office-2"]);
    }

    #[test]
    fn an_override_of_a_field_the_template_lacks_can_be_removed() {
        let mut h = harness_for(
            "[[devices]]\nid = \"d\"\ntemplate = \"temperature\"\nfields.co2 = { start = 1 }\n",
        );
        assert!(
            h.query_by_label_contains("co2 (not in the template)")
                .is_some()
        );
        h.get_by_label("Remove override").click();
        h.run();
        assert!(h.state().doc.file().devices[0].fields.is_empty());
    }

    /// Validation runs the device on the built-in (a project template cannot replace one), so
    /// its overrides must be for the built-in's fields.
    #[test]
    fn overrides_offer_the_builtin_fields_when_a_project_template_reuses_its_id() {
        let h = harness_for(
            r#"
[[templates]]
id = "co2"
messages = [{ topic = "a", payload = "b" }]
fields.x = { generator = "constant", value = 1 }

[[devices]]
id = "office"
template = "co2"
"#,
        );
        assert!(h.query_by_label("co2").is_some());
        assert!(h.query_by_label("x").is_none());
    }

    #[test]
    fn the_interval_override_starts_at_the_template_interval() {
        let mut h = harness_for(
            r#"
[[templates]]
id = "slow"
interval = "3s"
messages = [{ topic = "a", payload = "b" }]
fields.v = { generator = "constant", value = 1 }

[[devices]]
id = "office"
template = "slow"
"#,
        );
        h.get_by_label("override").click();
        h.run();
        assert_eq!(
            h.state().doc.file().devices[0].interval,
            Some(Duration::from_secs(3))
        );
    }

    #[test]
    fn the_interval_override_of_a_device_with_an_unknown_template_starts_at_10s() {
        let mut h = harness_for("[[devices]]\nid = \"office\"\ntemplate = \"gone\"\n");
        h.get_by_label("override").click();
        h.run();
        assert_eq!(
            h.state().doc.file().devices[0].interval,
            Some(Duration::from_secs(10))
        );
    }

    #[test]
    fn turning_the_interval_override_off_and_on_shows_the_model_value() {
        let mut h = harness();
        h.get_by_label("override").click();
        h.run();
        type_into_label(&mut h, "Interval", "5s");
        assert_eq!(
            h.state().doc.file().devices[0].interval,
            Some(Duration::from_secs(15))
        );
        h.get_by_label("override").click();
        h.run();
        h.get_by_label("override").click();
        h.run();
        assert_eq!(h.get_by_label("Interval").value().as_deref(), Some("10s"));
    }

    #[test]
    fn a_valid_override_keeps_its_spelling_and_clearing_it_removes_the_override() {
        let mut h = harness_for("[[devices]]\nid = \"office\"\ntemplate = \"co2\"\n");
        type_into_label(&mut h, "co2", "{start=");
        type_per_frame(&mut h, &["5}"]);
        assert_eq!(
            h.state().doc.file().devices[0].fields["co2"]["start"].as_integer(),
            Some(5)
        );
        assert_eq!(h.get_by_label("co2").value().as_deref(), Some("{start=5}"));
        assert!(h.query_by_label_contains("expected").is_none());
        for _ in 0.."{start=5}".len() {
            h.key_press(egui::Key::Backspace);
            h.run();
        }
        assert!(h.state().doc.file().devices[0].fields.is_empty());
        assert_eq!(h.get_by_label("co2").value().as_deref(), Some(""));
    }
}
