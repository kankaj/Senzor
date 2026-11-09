use eframe::egui::{self, Button, DragValue, TextEdit};
use senzor_core::engine::{Command, Target};
use senzor_core::generator::GeneratorSpec;
use senzor_core::project::builtin::builtin_templates;
use senzor_core::project::file::{DEFAULT_INTERVAL, default_instance_id_pattern};
use senzor_core::project::{DeviceSpec, FieldSpec, Location};
use senzor_core::value::Value;

use super::{
    Action, BoxOrRow, ERROR, FIX_PROBLEMS_FIRST, UiState, checked, duration_edit, errors_at, help,
    name_edit, noise_edit, number_edit, replace_if_unchanged, script_edit, sequence_edit,
    shortcuts, value_edit,
};
use crate::buffers::Buffers;
use crate::document::Document;
use crate::edits::{
    FieldOverride, GeneratorKind, Removal, Rename, clear_setting, find_template, override_setting,
    override_settings, override_summary, set_count, set_override_kind, store_override_edits,
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
    let at = &Location::Device {
        id: original.id.clone(),
    };
    let has_errors = errors_at(ui, doc, at, shown_inline);
    ui.horizontal(|ui| {
        // A blocked device cannot start; its own errors are on this page, a broken template's or
        // broker's are only in the Problems tab.
        let hint = if has_errors {
            FIX_PROBLEMS_HERE
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
        if ui.button("Duplicate device").clicked() {
            actions.push(Action::DuplicateDevice(original.id.clone()));
        }
        if ui
            .button("Delete device")
            .on_hover_text(shortcuts::tip(
                ui.ctx(),
                "Delete the device",
                &shortcuts::DELETE,
            ))
            .clicked()
        {
            actions.push(Action::Remove(Removal::Device(index)));
            actions.push(Action::Select(None));
        }
    });
    ui.separator();

    egui::Grid::new("device-form")
        .num_columns(2)
        .show(ui, |ui| {
            let label = help::label(ui, "ID", help::DEVICE_ID);
            checked(ui, doc, at, "id", |ui| {
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
                actions.extend(rename.map(Action::Rename));
                response.labelled_by(label.id)
            });
            ui.end_row();

            help::label(ui, "Template", help::TEMPLATE);
            checked(ui, doc, at, "template", |ui| {
                egui::ComboBox::from_id_salt("device-template")
                    .selected_text(device.template.clone())
                    .show_ui(ui, |ui| {
                        for id in template_ids(doc) {
                            ui.selectable_value(&mut device.template, id.clone(), id);
                        }
                    })
                    .response
            });
            ui.end_row();

            let label = help::label(ui, "Count", help::COUNT);
            let mut count = device.count;
            checked(ui, doc, at, "count", |ui| {
                ui.add(DragValue::new(&mut count).range(1..=10_000))
                    .labelled_by(label.id)
            });
            set_count(&mut device, count);
            ui.end_row();

            let label = help::label(ui, "Instance IDs", help::INSTANCE_IDS);
            checked(ui, doc, at, "instance_id", |ui| {
                ui.horizontal(|ui| {
                    let mut custom = device.instance_id.is_some();
                    // Starting from the pattern in use keeps the instance IDs until the user types.
                    if ui.checkbox(&mut custom, "custom").changed() {
                        device.instance_id =
                            custom.then(|| default_instance_id_pattern(device.count).to_string());
                    }
                    match &mut device.instance_id {
                        Some(pattern) => Some(
                            ui.add(
                                TextEdit::singleline(pattern)
                                    .id_salt(format!("device/{index}/instance_id")),
                            )
                            .labelled_by(label.id),
                        ),
                        None if device.count == 1 => {
                            ui.weak(&device.id);
                            None
                        }
                        None => {
                            ui.weak(format!(
                                "{}-0 … {}-{}",
                                device.id,
                                device.id,
                                device.count - 1
                            ));
                            None
                        }
                    }
                })
                .box_or_row()
            });
            ui.end_row();

            help::label(ui, "Autostart", help::AUTOSTART);
            ui.checkbox(&mut device.autostart, "start when the project opens");
            ui.end_row();

            let label = help::label(ui, "Interval", help::DEVICE_INTERVAL);
            checked(ui, doc, at, "interval", |ui| {
                ui.horizontal(|ui| {
                    let mut custom = device.interval.is_some();
                    if ui.checkbox(&mut custom, "override").changed() {
                        device.interval = custom.then(|| {
                            template_interval(doc.file(), &device.template)
                                .unwrap_or(DEFAULT_INTERVAL)
                        });
                    }
                    device.interval.as_mut().map(|interval| {
                        duration_edit(
                            ui,
                            &mut ui_state.buffers,
                            &format!("device/{index}/interval"),
                            interval,
                        )
                        .labelled_by(label.id)
                    })
                })
                .box_or_row()
            });
            ui.end_row();
        });

    ui.separator();
    params(ui, index, &mut device, doc, ui_state, actions);
    ui.separator();
    overrides(ui, index, &mut device, doc, ui_state);

    actions.extend(replace_if_unchanged(original, device, move |f| {
        f.devices.get_mut(index)
    }));
}

/// Hover hint of a disabled Start device whose own problems are on its page.
const FIX_PROBLEMS_HERE: &str = "Fix the problems marked in red first";

/// The device fields the form shows its problems next to.
fn shown_inline(field: &str) -> bool {
    matches!(
        field,
        "id" | "template" | "count" | "instance_id" | "interval"
    ) || field.starts_with("params.")
        || field.starts_with("fields.")
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
    doc: &Document,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    help::heading(ui, "Params", help::PARAMS);
    let file = doc.file();
    let at = &Location::Device {
        id: file.devices[index].id.clone(),
    };
    let keys: Vec<String> = device.params.keys().cloned().collect();
    for key in keys {
        checked(ui, doc, at, &format!("params.{key}"), |ui| {
            ui.horizontal(|ui| {
                let (name_box, rename) = name_edit(
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
                Some(name_box)
            })
            .box_or_row()
        });
    }
    if ui.button("Add param").clicked() {
        let name = unique_name("param", device.params.keys());
        device.params.insert(name, Value::String(String::new()));
    }
}

/// One collapsible block per template field, with a row per setting of the field as the device
/// uses it. Overrides of fields the template does not have (after a template switch) get a
/// Remove button.
fn overrides(
    ui: &mut egui::Ui,
    index: usize,
    device: &mut DeviceSpec,
    doc: &Document,
    ui_state: &mut UiState,
) {
    let at = &Location::Device {
        id: doc.file().devices[index].id.clone(),
    };
    help::heading(ui, "Field overrides", help::FIELD_OVERRIDES);
    ui.weak(OVERRIDES_NOTE);
    let fields: Vec<(String, FieldSpec)> = find_template(doc.file(), &device.template)
        .map(|t| t.fields.clone().into_iter().collect())
        .unwrap_or_default();
    let stranded: Vec<String> = device
        .fields
        .keys()
        .filter(|f| !fields.iter().any(|(name, _)| name == *f))
        .cloned()
        .collect();
    for (name, field) in &fields {
        let mut table = device.fields.remove(name).unwrap_or_default();
        let block = OverrideBlock {
            key: format!("device/{index}/override/{name}"),
            name,
            field,
            doc,
            at,
        };
        block.show(ui, &mut table, &mut ui_state.buffers);
        if !table.is_empty() {
            device.fields.insert(name.clone(), table);
        }
    }
    for field in stranded {
        ui.horizontal(|ui| {
            checked(ui, doc, at, &format!("fields.{field}"), |ui| {
                ui.colored_label(ERROR, format!("{field} (not in the template)"))
            });
            if ui.button("Remove override").clicked() {
                device.fields.remove(&field);
            }
        });
    }
}

/// The note under the Field overrides heading.
const OVERRIDES_NOTE: &str =
    "Tick a setting to change it for this device; the others show the template's value.";

/// The block of one template field in Field overrides.
struct OverrideBlock<'a> {
    /// Prefix of the block's widget IDs and text buffers.
    key: String,
    /// The field's key in the template.
    name: &'a str,
    /// The template's field.
    field: &'a FieldSpec,
    doc: &'a Document,
    at: &'a Location,
}

impl OverrideBlock<'_> {
    /// Draws the block for the device's override `table` of the field, and edits the table.
    fn show(&self, ui: &mut egui::Ui, table: &mut toml::Table, buffers: &mut Buffers) {
        let view = FieldOverride::new(self.field, table);
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            ui.make_persistent_id(&self.key),
            !table.is_empty(),
        );
        // Problems with the override go under the header, so they show while it is closed.
        let summary = override_summary(table, view.kind);
        let header = checked(
            ui,
            self.doc,
            self.at,
            &format!("fields.{}", self.name),
            |ui| {
                ui.horizontal(|ui| {
                    state.show_toggle_button(ui, egui::collapsing_header::paint_default_icon);
                    let title = ui.add(
                        egui::Label::new(egui::RichText::new(self.title()).strong())
                            .sense(egui::Sense::click()),
                    );
                    if title.clicked() {
                        state.toggle(ui);
                    }
                    ui.weak(summary);
                    title
                })
                .inner
            },
        );
        state.show_body_indented(&header, ui, |ui| self.rows(ui, &view, table, buffers));
    }

    /// `co2 — CO2, ppm`: the field's key, then the name and unit the template gives it.
    fn title(&self) -> String {
        let labels: Vec<&str> = [&self.field.name, &self.field.unit]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect();
        if labels.is_empty() {
            self.name.to_string()
        } else {
            format!("{} — {}", self.name, labels.join(", "))
        }
    }

    /// A row per setting: the override checkbox, the setting's name and its control, enabled
    /// while the setting is overridden. Screen readers know the name as `<field> <setting>`
    /// (`co2 start`) and the checkbox as `override co2 start`, so fields can be told apart.
    fn rows(
        &self,
        ui: &mut egui::Ui,
        view: &FieldOverride,
        table: &mut toml::Table,
        buffers: &mut Buffers,
    ) {
        let mut edited = view.effective.clone();
        let mut kind = view.kind;
        let mut toggled = None;
        let mut remove = None;
        egui::Grid::new(format!("{}/settings", self.key))
            .num_columns(3)
            .show(ui, |ui| {
                for setting in override_settings(view.kind) {
                    let on = table.contains_key(setting);
                    let mut tick = on;
                    let locked = on && view.is_required(setting);
                    let checkbox = ui
                        .add_enabled(!locked, egui::Checkbox::without_text(&mut tick))
                        .on_hover_text("Override for this device")
                        .on_disabled_hover_text("Needed while the generator is overridden");
                    let accessible = format!("override {} {setting}", self.name);
                    checkbox.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::Checkbox,
                            !locked,
                            tick,
                            &accessible,
                        )
                    });
                    if tick != on {
                        toggled = Some((setting, tick));
                    }
                    let label = ui.label(setting);
                    let accessible = format!("{} {setting}", self.name);
                    label.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &accessible)
                    });
                    // In a scope of its own, so the control keeps its ID (and the keyboard)
                    // while the rows above change.
                    ui.push_id(setting, |ui| {
                        ui.horizontal(|ui| {
                            ui.add_enabled_ui(on, |ui| {
                                let control =
                                    self.setting_edit(ui, buffers, setting, &mut edited, &mut kind);
                                if let Some(control) = control {
                                    control.labelled_by(label.id);
                                }
                            });
                            if view.unreadable.contains(setting)
                                && let Some(value) = table.get(setting)
                            {
                                ui.colored_label(ERROR, format!("cannot read {value}"));
                            }
                        });
                    });
                    ui.end_row();
                }
                for key in &view.unknown {
                    ui.label("");
                    ui.colored_label(ERROR, key);
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            ERROR,
                            format!("{key} is not a setting of {}", view.kind.label()),
                        );
                        let button = ui.button("Remove");
                        let accessible = format!("Remove {} {key}", self.name);
                        button.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &accessible)
                        });
                        if button.clicked() {
                            remove = Some(key.clone());
                        }
                    });
                    ui.end_row();
                }
            });
        store_override_edits(table, &view.effective, &edited);
        if kind != view.kind {
            set_override_kind(table, kind);
        }
        match toggled {
            Some((setting, true)) => override_setting(table, view, self.name, setting),
            Some((setting, false)) => clear_setting(table, self.field, setting),
            None => {}
        }
        if let Some(key) = remove {
            table.remove(&key);
        }
    }

    /// The control of `setting` of `field`, editing it in place; a generator kind chosen goes
    /// to `kind`. Returns the control to name after the setting, if there is one.
    fn setting_edit(
        &self,
        ui: &mut egui::Ui,
        buffers: &mut Buffers,
        setting: &str,
        field: &mut FieldSpec,
        kind: &mut GeneratorKind,
    ) -> Option<egui::Response> {
        // Per kind, so a box of one generator does not keep the text of another's.
        let key = &format!("{}/{}/{setting}", self.key, kind.label());
        let response = match (setting, &mut field.generator) {
            ("generator", _) => {
                egui::ComboBox::from_id_salt(key)
                    .selected_text(kind.label())
                    .show_ui(ui, |ui| {
                        for option in GeneratorKind::ALL {
                            ui.selectable_value(kind, option, option.label());
                        }
                    })
                    .response
            }
            ("unit", _) => label_edit(ui, key, &mut field.unit, "none"),
            // Without a name, people see the field's key.
            ("name", _) => label_edit(ui, key, &mut field.name, self.name),
            ("noise", spec) => match noise_mut(spec) {
                Some(noise) => noise_edit(ui, noise),
                None => {
                    ui.weak("none");
                    return None;
                }
            },
            ("value", GeneratorSpec::Constant { value, .. })
            | ("start", GeneratorSpec::Script { start: value, .. }) => {
                value_edit(ui, buffers, key, value)
            }
            ("script", GeneratorSpec::Script { script, .. }) => script_edit(ui, key, script, None),
            ("values", GeneratorSpec::Sequence { values, .. }) => {
                sequence_edit(ui, buffers, key, values)
            }
            ("loop", GeneratorSpec::Sequence { looping, .. }) => {
                ui.add(egui::Checkbox::without_text(looping))
            }
            ("period", GeneratorSpec::Sine { period: time, .. })
            | ("phase", GeneratorSpec::Sine { phase: time, .. }) => {
                duration_edit(ui, buffers, key, time)
            }
            (setting, spec) => number_edit(ui, number_mut(spec, setting)?),
        };
        Some(response)
    }
}

/// A field's unit or name. Unlike in the template, an empty box is an empty value, which
/// validation reports: an override cannot take a unit or name away.
fn label_edit(
    ui: &mut egui::Ui,
    key: &str,
    value: &mut Option<String>,
    hint: &str,
) -> egui::Response {
    let mut text = value.clone().unwrap_or_default();
    let response = ui.add(
        TextEdit::singleline(&mut text)
            .id_salt(key)
            .hint_text(hint)
            .desired_width(140.0),
    );
    if response.changed() {
        *value = Some(text);
    }
    response
}

fn noise_mut(spec: &mut GeneratorSpec) -> Option<&mut f64> {
    match spec {
        GeneratorSpec::Constant { noise, .. }
        | GeneratorSpec::RandomWalk { noise, .. }
        | GeneratorSpec::Sine { noise, .. }
        | GeneratorSpec::Uniform { noise, .. }
        | GeneratorSpec::Sequence { noise, .. }
        | GeneratorSpec::Script { noise, .. } => noise.as_mut(),
    }
}

/// The number parameter `name` of `spec`, if it has one.
fn number_mut<'a>(spec: &'a mut GeneratorSpec, name: &str) -> Option<&'a mut f64> {
    match (spec, name) {
        (GeneratorSpec::RandomWalk { start, .. }, "start") => Some(start),
        (GeneratorSpec::RandomWalk { min, .. } | GeneratorSpec::Uniform { min, .. }, "min") => {
            Some(min)
        }
        (GeneratorSpec::RandomWalk { max, .. } | GeneratorSpec::Uniform { max, .. }, "max") => {
            Some(max)
        }
        (GeneratorSpec::RandomWalk { step, .. }, "step") => Some(step),
        (GeneratorSpec::Sine { center, .. }, "center") => Some(center),
        (GeneratorSpec::Sine { amplitude, .. }, "amplitude") => Some(amplitude),
        _ => None,
    }
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

    fn rect_of(h: &Harness<'static, Fixture>, label: &str) -> egui::Rect {
        h.get_by_label(label).rect()
    }

    #[test]
    fn a_problem_with_a_value_shows_once_under_its_box() {
        let h = harness_for(
            "[[devices]]\nid = \"d\"\ntemplate = \"co2\"\ncount = 2\ninstance_id = \"x\"\n",
        );
        let message = "instance id `x` repeats; include {{ index }} in instance_id";
        assert_eq!(
            h.query_all_by_label(message).count(),
            1,
            "not also at the top"
        );
        assert!(rect_of(&h, message).top() >= rect_of(&h, "Instance IDs").bottom());
        assert!(
            rect_of(&h, message).bottom() <= rect_of(&h, "Autostart").top(),
            "right under the box, before the next row"
        );
    }

    /// The device's params break its template's topic; the form has no box for the topic.
    #[test]
    fn a_problem_the_form_has_no_box_for_shows_at_the_top() {
        let h = harness_for(
            "[[templates]]\nid = \"t\"\nmessages = [{ topic = \"a/{{ room }}\", payload = \"1\" }]\n\
             fields.v = { generator = \"constant\", value = 1 }\n\n\
             [[devices]]\nid = \"d\"\ntemplate = \"t\"\n",
        );
        let errors = &h.state().doc.validated().errors;
        assert_eq!(errors.len(), 1, "{errors:?}");
        let message = h.get_by_label(&errors[0].message).rect();
        assert!(message.bottom() <= rect_of(&h, "ID").top());
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
        assert_disabled_with_hint(&mut h, "Start device", FIX_PROBLEMS_HERE);
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

    // Field overrides. Screen readers (and tests) know a setting's label as `<field> <setting>`,
    // e.g. `co2 start`, and its override checkbox as `override co2 start`, so the same setting
    // of two fields can be told apart; the label shows only `start`.

    /// The `co2` block's header: the field's key, then its name and unit from the template.
    const CO2: &str = "co2 — CO2, ppm";

    const PLAIN: &str = "[[devices]]\nid = \"office\"\ntemplate = \"co2\"\n";

    fn with_override(table: &str) -> Harness<'static, Fixture> {
        harness_for(&format!("{PLAIN}fields.co2 = {table}\n"))
    }

    fn co2_override(h: &Harness<'static, Fixture>) -> Option<toml::Table> {
        h.state().doc.file().devices[0].fields.get("co2").cloned()
    }

    fn table(src: &str) -> toml::Table {
        toml::from_str(src).unwrap()
    }

    fn click(h: &mut Harness<'static, Fixture>, label: &str) {
        h.get_by_label(label).click();
        h.run();
    }

    /// A device without overrides, with the `co2` block opened.
    fn opened() -> Harness<'static, Fixture> {
        let mut h = harness_for(PLAIN);
        click(&mut h, CO2);
        h
    }

    #[test]
    fn a_script_override_shows_its_script_and_start() {
        // A field with an override starts open.
        let h = with_override(r#"{ generator = "script", start = 5, script = "value + 2" }"#);
        assert_eq!(
            h.get_by_label("co2 script").value().as_deref(),
            Some("value + 2")
        );
        assert_eq!(h.get_by_label("co2 start").value().as_deref(), Some("5"));
    }

    #[test]
    fn a_field_without_overrides_starts_collapsed_and_says_so() {
        let mut h = harness_for(PLAIN);
        assert!(h.query_by_label("not overridden").is_some());
        assert!(h.query_by_label("co2 start").is_none());
        click(&mut h, CO2);
        assert!(h.query_by_label("co2 start").is_some());
    }

    #[test]
    fn an_overridden_field_starts_open_and_lists_what_it_overrides() {
        let h = with_override("{ start = 1200, max = 2500 }");
        assert!(h.query_by_label("start, max overridden").is_some());
        assert_eq!(
            h.get_by_label("co2 start").value().as_deref(),
            Some("1200.0")
        );
    }

    #[test]
    fn a_setting_that_is_not_overridden_shows_the_templates_value_disabled() {
        let h = opened();
        let start = h.get_by_label("co2 start");
        assert_eq!(start.value().as_deref(), Some("600.0"));
        assert!(start.accesskit_node().is_disabled());
        assert_eq!(h.get_by_label("co2 unit").value().as_deref(), Some("ppm"));
    }

    #[test]
    fn ticking_an_override_writes_the_templates_value() {
        let mut h = opened();
        click(&mut h, "override co2 start");
        assert_eq!(co2_override(&h), Some(table("start = 600.0")));
        assert!(!h.get_by_label("co2 start").accesskit_node().is_disabled());
        let validated = h.state().doc.validated();
        assert!(validated.errors.is_empty(), "{:?}", validated.errors);
    }

    #[test]
    fn typing_a_number_one_character_per_frame_keeps_focus_and_changes_only_it() {
        let mut h = opened();
        click(&mut h, "override co2 start");
        h.get_by_label("co2 start").focus();
        h.run();
        type_per_frame(&mut h, &["9", "0", "0"]);
        assert_eq!(co2_override(&h), Some(table("start = 900.0")));
    }

    #[test]
    fn unticking_removes_the_setting_and_the_last_one_removes_the_override() {
        let mut h = with_override("{ start = 1200, max = 2500 }");
        click(&mut h, "override co2 max");
        assert_eq!(co2_override(&h), Some(table("start = 1200")));
        click(&mut h, "override co2 start");
        assert!(h.state().doc.file().devices[0].fields.is_empty());
        assert_eq!(
            h.get_by_label("co2 start").value().as_deref(),
            Some("600.0")
        );
    }

    #[test]
    fn overriding_the_generator_with_sine_writes_a_complete_sine() {
        let mut h = opened();
        click(&mut h, "override co2 generator");
        click(&mut h, "co2 generator");
        click(&mut h, "sine");
        assert_eq!(
            co2_override(&h),
            Some(table(
                "generator = \"sine\"\ncenter = 0.0\namplitude = 1.0\nperiod = \"1m\""
            ))
        );
        let validated = h.state().doc.validated();
        assert!(validated.errors.is_empty(), "{:?}", validated.errors);
        assert!(h.query_by_label("co2 amplitude").is_some());
        assert!(h.query_by_label("co2 step").is_none());
        // The kind's required parameters stay overridden while the generator is.
        assert!(
            h.get_by_label("override co2 center")
                .accesskit_node()
                .is_disabled()
        );
    }

    #[test]
    fn unticking_the_generator_drops_what_the_templates_generator_does_not_have() {
        let mut h = with_override(
            "{ generator = \"sine\", center = 5, amplitude = 1, period = \"1m\", unit = \"ppb\" }",
        );
        assert_eq!(h.get_by_label("co2 center").value().as_deref(), Some("5.0"));
        click(&mut h, "override co2 generator");
        assert_eq!(co2_override(&h), Some(table("unit = \"ppb\"")));
    }

    #[test]
    fn ticking_and_unticking_the_generator_leaves_the_field_as_it_was() {
        let mut h = with_override("{ start = 1200 }");
        click(&mut h, "override co2 generator");
        assert_eq!(
            co2_override(&h),
            Some(table(
                "generator = \"random_walk\"\nstart = 1200\nmin = 400.0\nmax = 2000.0\nstep = 15.0"
            ))
        );
        let validated = h.state().doc.validated();
        assert!(validated.errors.is_empty(), "{:?}", validated.errors);
        click(&mut h, "override co2 generator");
        assert_eq!(co2_override(&h), Some(table("start = 1200")));
    }

    #[test]
    fn the_unit_can_be_overridden() {
        let mut h = opened();
        click(&mut h, "override co2 unit");
        assert_eq!(co2_override(&h), Some(table("unit = \"ppm\"")));
        type_into_label(&mut h, "co2 unit", "v");
        assert_eq!(co2_override(&h), Some(table("unit = \"ppmv\"")));
    }

    #[test]
    fn ticking_noise_on_a_field_without_noise_starts_at_the_default_noise() {
        let mut h = opened();
        assert!(h.query_by_label("co2 noise").is_some());
        click(&mut h, "override co2 noise");
        assert_eq!(co2_override(&h), Some(table("noise = 0.1")));
    }

    /// A value from the file that does not fit its setting stays in the table, with a hint,
    /// until the setting is edited.
    #[test]
    fn a_value_that_cannot_be_read_is_kept_with_a_hint() {
        let mut h = with_override("{ start = \"high\" }");
        assert!(h.query_by_label("cannot read \"high\"").is_some());
        assert_eq!(co2_override(&h), Some(table("start = \"high\"")));
        h.get_by_label("co2 start").focus();
        h.run();
        type_per_frame(&mut h, &["7"]);
        assert_eq!(co2_override(&h), Some(table("start = 7.0")));
    }

    #[test]
    fn a_key_that_is_not_a_setting_can_be_removed() {
        let mut h = with_override("{ start = 1, strat = 5 }");
        assert!(
            h.query_by_label("strat is not a setting of random_walk")
                .is_some()
        );
        click(&mut h, "Remove co2 strat");
        assert_eq!(co2_override(&h), Some(table("start = 1")));
    }

    #[test]
    fn a_duration_that_does_not_parse_keeps_the_override_and_shows_a_hint() {
        let mut h = harness_for("[[devices]]\nid = \"office\"\ntemplate = \"temperature\"\n");
        click(&mut h, "temperature — Temperature, °C");
        click(&mut h, "override temperature period");
        let ticked = h.state().doc.file().devices[0].fields["temperature"].clone();
        assert_eq!(ticked["period"].as_str(), Some("1day"));
        type_into_label(&mut h, "temperature period", "x");
        assert!(h.query_by_label("use e.g. 10s, 500ms, 1h").is_some());
        assert_eq!(
            h.state().doc.file().devices[0].fields["temperature"],
            ticked
        );
    }

    #[test]
    fn a_problem_with_an_override_shows_once_under_its_header() {
        let h = with_override("{ start = 5000 }");
        let message = "field `co2` override: start must be between min and max";
        assert_eq!(h.query_all_by_label(message).count(), 1);
        assert!(rect_of(&h, message).top() >= rect_of(&h, CO2).bottom());
        assert!(rect_of(&h, message).bottom() <= rect_of(&h, "co2 generator").top());
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
        assert!(h.query_by_label(CO2).is_some());
        assert!(h.query_by_label("x").is_none());
    }
}
