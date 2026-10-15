use std::collections::BTreeMap;
use std::time::SystemTime;

use eframe::egui::{self, DragValue, RichText, TextEdit};
use senzor_core::generator::GeneratorSpec;
use senzor_core::project::builtin::builtin_template;
use senzor_core::project::{Location, MessageSpec, ProjectFile, TemplateSpec};

use super::{
    Action, ERROR, ParsedEdit, UiState, duration_edit, errors_at, name_edit, replace_if_unchanged,
    value_edit,
};
use crate::buffers::Buffers;
use crate::document::Document;
use crate::edits::{
    DEFAULT_NOISE, GeneratorKind, Removal, Rename, default_generator, new_availability,
    new_message, parse_sequence, sequence_text, set_generator_kind, unique_name,
};
use crate::preview::preview;

/// The editor for project template `index`.
pub fn show_user(
    ui: &mut egui::Ui,
    index: usize,
    doc: &Document,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    let original = &doc.file().templates[index];
    let mut template = original.clone();
    ui.heading(format!("Template {}", original.id));
    errors_at(
        ui,
        doc,
        &Location::Template {
            id: original.id.clone(),
        },
    );
    if ui.button("Delete template").clicked() {
        actions.push(Action::Remove(Removal::Template(index)));
        actions.push(Action::Select(None));
    }
    ui.separator();
    editor(
        ui,
        &mut template,
        &format!("template/{index}"),
        Some(Renamable {
            index,
            file: doc.file(),
        }),
        ui_state,
        actions,
    );
    ui.separator();
    preview_section(ui, &template, doc);
    actions.extend(replace_if_unchanged(original, template, move |f| {
        f.templates.get_mut(index)
    }));
}

/// A read-only view of a built-in template, with "Copy into project".
pub fn show_builtin(
    ui: &mut egui::Ui,
    id: &str,
    doc: &Document,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    let Some(mut template) = builtin_template(id).cloned() else {
        ui.label(format!("There is no built-in template `{id}`."));
        return;
    };
    ui.heading(format!("Built-in template {id}"));
    ui.label("Built-in templates are read-only. Copy one into the project to change it.");
    if ui.button("Copy into project…").clicked() {
        actions.push(Action::AskCopyBuiltin(id.to_string()));
    }
    ui.separator();
    let mut ignored = Vec::new();
    ui.add_enabled_ui(false, |ui| {
        editor(
            ui,
            &mut template,
            &format!("builtin/{id}"),
            None,
            ui_state,
            &mut ignored,
        );
    });
    ui.separator();
    preview_section(ui, &template, doc);
}

/// The project template an editor shows, so its names can be renamed. Built-ins have none.
#[derive(Clone, Copy)]
struct Renamable<'a> {
    index: usize,
    file: &'a ProjectFile,
}

/// Edits `template` in place. With `renamable`, the ID and field names are name boxes that push
/// [`Rename`]s; without, they are plain labels.
fn editor(
    ui: &mut egui::Ui,
    template: &mut TemplateSpec,
    key: &str,
    renamable: Option<Renamable>,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    settings_grid(ui, template, key, renamable, ui_state, actions);
    ui.add_space(6.0);
    messages_editor(ui, &mut template.messages, key);
    ui.add_space(6.0);
    fields_editor(ui, &mut template.fields, key, renamable, ui_state, actions);
}

/// With `renamable`, a name box that pushes the [`Rename`] `rename` builds from the template's
/// index and the new name; otherwise a plain label. Returns the name box, if any.
fn name_or_label(
    ui: &mut egui::Ui,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
    renamable: Option<Renamable>,
    key: &str,
    name: &str,
    rename: impl Fn(usize, String) -> Rename,
) -> Option<egui::Response> {
    let Some(Renamable { index, file }) = renamable else {
        ui.label(name);
        return None;
    };
    let (response, committed) = name_edit(ui, ui_state, key, name, file, |to| rename(index, to));
    actions.extend(committed.map(Action::Rename));
    Some(response)
}

/// ID, interval, QoS, retain and availability.
fn settings_grid(
    ui: &mut egui::Ui,
    template: &mut TemplateSpec,
    key: &str,
    renamable: Option<Renamable>,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    egui::Grid::new("template-form")
        .num_columns(2)
        .show(ui, |ui| {
            let label = ui.label("ID");
            let id_box = name_or_label(
                ui,
                ui_state,
                actions,
                renamable,
                &format!("{key}/id"),
                &template.id,
                |index, to| Rename::Template {
                    index,
                    from: template.id.clone(),
                    to,
                },
            );
            if let Some(id_box) = id_box {
                id_box.labelled_by(label.id);
            }
            ui.end_row();

            let label = ui.label("Interval");
            duration_edit(
                ui,
                &mut ui_state.buffers,
                &format!("{key}/interval"),
                &mut template.interval,
            )
            .labelled_by(label.id);
            ui.end_row();

            ui.label("QoS");
            egui::ComboBox::from_id_salt(format!("{key}/qos"))
                .selected_text(template.qos.to_string())
                .show_ui(ui, |ui| {
                    for qos in 0..=2u8 {
                        ui.selectable_value(&mut template.qos, qos, qos.to_string());
                    }
                });
            ui.end_row();

            ui.label("Retain");
            ui.checkbox(&mut template.retain, "retain data messages");
            ui.end_row();

            ui.label("Availability");
            let mut enabled = template.availability.is_some();
            if ui
                .checkbox(&mut enabled, "online/offline topic with Last Will")
                .changed()
            {
                template.availability = enabled.then(new_availability);
            }
            ui.end_row();
            if let Some(availability) = &mut template.availability {
                for (label, text, salt) in [
                    ("  Topic", &mut availability.topic, "topic"),
                    ("  Online payload", &mut availability.online, "online"),
                    ("  Offline payload", &mut availability.offline, "offline"),
                ] {
                    let label = ui.label(label);
                    ui.add(
                        TextEdit::singleline(text).id_salt(format!("{key}/availability/{salt}")),
                    )
                    .labelled_by(label.id);
                    ui.end_row();
                }
            }
        });
}

fn messages_editor(ui: &mut egui::Ui, messages: &mut Vec<MessageSpec>, key: &str) {
    ui.strong("Messages");
    let mut remove = None;
    for (i, message) in messages.iter_mut().enumerate() {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(format!("Message {}", i + 1));
                if ui.button("Remove").clicked() {
                    remove = Some(i);
                }
            });
            let label = ui.label("Topic");
            ui.add(
                TextEdit::singleline(&mut message.topic)
                    .id_salt(format!("{key}/message/{i}/topic"))
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(label.id);
            let label = ui.label("Payload");
            ui.add(
                TextEdit::multiline(&mut message.payload)
                    .id_salt(format!("{key}/message/{i}/payload"))
                    .code_editor()
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(label.id);
        });
    }
    if let Some(i) = remove {
        messages.remove(i);
    }
    if ui.button("Add message").clicked() {
        messages.push(new_message());
    }
}

fn fields_editor(
    ui: &mut egui::Ui,
    fields: &mut BTreeMap<String, GeneratorSpec>,
    key: &str,
    renamable: Option<Renamable>,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    ui.strong("Fields");
    let names: Vec<String> = fields.keys().cloned().collect();
    for name in names {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                name_or_label(
                    ui,
                    ui_state,
                    actions,
                    renamable,
                    &format!("{key}/field/{name}"),
                    &name,
                    |template, to| Rename::Field {
                        template,
                        from: name.clone(),
                        to,
                    },
                );
                let Some(spec) = fields.get_mut(&name) else {
                    return;
                };
                let mut kind = GeneratorKind::of(spec);
                egui::ComboBox::from_id_salt(format!("{key}/field/{name}/kind"))
                    .selected_text(kind.label())
                    .show_ui(ui, |ui| {
                        for option in GeneratorKind::ALL {
                            ui.selectable_value(&mut kind, option, option.label());
                        }
                    });
                set_generator_kind(spec, kind);
                if ui.button("Remove").clicked() {
                    fields.remove(&name);
                }
            });
            if let Some(spec) = fields.get_mut(&name) {
                generator_editor(
                    ui,
                    spec,
                    &format!("{key}/field/{name}"),
                    &mut ui_state.buffers,
                );
            }
        });
    }
    if ui.button("Add field").clicked() {
        let name = unique_name("field", fields.keys());
        fields.insert(name, default_generator(GeneratorKind::RandomWalk));
    }
}

fn number(ui: &mut egui::Ui, label: &str, value: &mut f64) {
    ui.label(label);
    ui.add(DragValue::new(value).speed(0.1));
}

fn generator_editor(ui: &mut egui::Ui, spec: &mut GeneratorSpec, key: &str, buffers: &mut Buffers) {
    let kind = GeneratorKind::of(spec).label();
    let key = format!("{key}/{kind}");
    ui.horizontal_wrapped(|ui| match spec {
        GeneratorSpec::Constant { value, noise } => {
            ui.label("value");
            value_edit(ui, buffers, &format!("{key}/value"), value);
            noise_editor(ui, noise);
        }
        GeneratorSpec::RandomWalk {
            start,
            min,
            max,
            step,
            noise,
        } => {
            number(ui, "start", start);
            number(ui, "min", min);
            number(ui, "max", max);
            number(ui, "step", step);
            noise_editor(ui, noise);
        }
        GeneratorSpec::Sine {
            center,
            amplitude,
            period,
            phase,
            noise,
        } => {
            number(ui, "center", center);
            number(ui, "amplitude", amplitude);
            ui.label("period");
            duration_edit(ui, buffers, &format!("{key}/period"), period);
            ui.label("phase");
            duration_edit(ui, buffers, &format!("{key}/phase"), phase);
            noise_editor(ui, noise);
        }
        GeneratorSpec::Uniform { min, max, noise } => {
            number(ui, "min", min);
            number(ui, "max", max);
            noise_editor(ui, noise);
        }
        GeneratorSpec::Sequence {
            values,
            looping,
            noise,
        } => {
            ui.label("values");
            ParsedEdit::new(&format!("{key}/values"), 200.0)
                .hint("open, closed")
                .show(
                    ui,
                    buffers,
                    values,
                    |values| sequence_text(values),
                    |text| Ok(parse_sequence(text)),
                );
            ui.checkbox(looping, "loop");
            noise_editor(ui, noise);
        }
    });
}

fn noise_editor(ui: &mut egui::Ui, noise: &mut Option<f64>) {
    let mut enabled = noise.is_some();
    if ui.checkbox(&mut enabled, "noise").changed() {
        *noise = enabled.then_some(DEFAULT_NOISE);
    }
    if let Some(noise) = noise {
        ui.add(DragValue::new(noise).speed(0.01).range(0.0..=f64::MAX));
    }
}

fn preview_section(ui: &mut egui::Ui, template: &TemplateSpec, doc: &Document) {
    ui.strong("Preview");
    let preview = preview(template, &doc.validated().project, SystemTime::now());
    match &preview.instance {
        Some(instance) => ui.weak(format!(
            "Rendered as {instance} with its params and sample values."
        )),
        None => ui.weak("No valid device uses this template, so params are not available."),
    };
    if let Some(error) = &preview.compile_error {
        ui.colored_label(ERROR, error);
        return;
    }
    if let Some(topic) = &preview.availability_topic {
        match topic {
            Ok(topic) => ui.label(RichText::new(format!("availability: {topic}")).monospace()),
            Err(e) => ui.colored_label(ERROR, format!("availability topic: {e}")),
        };
    }
    for (i, message) in preview.messages.iter().enumerate() {
        match message {
            Ok(message) => {
                ui.label(RichText::new(&message.topic).monospace().strong());
                ui.label(RichText::new(&message.payload).monospace());
            }
            Err(e) => {
                ui.colored_label(ERROR, format!("message {}: {e}", i + 1));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use senzor_core::value::Value;

    use super::*;
    use crate::ui::test_support::{Fixture, type_into_label, type_into_value, type_per_frame};

    const SRC: &str = r#"
[[templates]]
id = "air"
interval = "10s"
messages = [{ topic = "b/{{ device_id }}", payload = "{{ co2 }}" }]
fields.co2 = { generator = "random_walk", start = 600, min = 400, max = 2000, step = 15 }

[[devices]]
id = "office"
template = "air"
"#;

    /// `SRC` with `from` replaced by `to`; fails instead of returning `SRC` unchanged.
    fn src_replacing(from: &str, to: &str) -> String {
        assert!(SRC.contains(from), "SRC no longer contains {from:?}");
        SRC.replace(from, to)
    }

    fn user_harness() -> Harness<'static, Fixture> {
        user_harness_with(SRC)
    }

    /// The editor of the first project template of `src`.
    fn user_harness_with(src: &str) -> Harness<'static, Fixture> {
        Fixture::new(src).harness(|ui, fx| {
            if !fx.doc.file().templates.is_empty() {
                show_user(ui, 0, &fx.doc, &mut fx.ui, &mut fx.actions);
            }
        })
    }

    #[test]
    fn a_valid_interval_is_applied() {
        let mut h = user_harness();
        type_into_label(&mut h, "Interval", " 500ms");
        assert_eq!(
            h.state().doc.file().templates[0].interval,
            Duration::from_millis(10_500)
        );
    }

    #[test]
    fn an_invalid_interval_keeps_the_old_value_and_shows_a_hint() {
        let mut h = user_harness();
        type_into_label(&mut h, "Interval", "x");
        assert!(h.query_by_label("use e.g. 10s, 500ms, 1h").is_some());
        assert_eq!(
            h.state().doc.file().templates[0].interval,
            Duration::from_secs(10)
        );
    }

    #[test]
    fn switching_the_generator_resets_its_parameters() {
        let mut h = user_harness();
        h.get_by_value("random_walk").click();
        h.run();
        h.get_by_label("constant").click();
        h.run();
        assert_eq!(
            h.state().doc.file().templates[0].fields["co2"],
            default_generator(GeneratorKind::Constant)
        );
    }

    #[test]
    fn editing_a_text_constant_that_looks_like_a_number_keeps_it_text() {
        let mut h = user_harness_with(&src_replacing(
            r#"generator = "random_walk", start = 600, min = 400, max = 2000, step = 15"#,
            r#"generator = "constant", value = "007""#,
        ));
        type_into_value(&mut h, "007", "1");
        assert_eq!(
            h.state().doc.file().templates[0].fields["co2"],
            GeneratorSpec::Constant {
                value: Value::String("0071".into()),
                noise: None
            }
        );
        assert!(h.query_by_value("text").is_some(), "a type selector");
    }

    #[test]
    fn add_field_adds_a_random_walk() {
        let mut h = user_harness();
        h.get_by_label("Add field").click();
        h.run();
        assert_eq!(
            h.state().doc.file().templates[0].fields.get("field"),
            Some(&default_generator(GeneratorKind::RandomWalk))
        );
    }

    #[test]
    fn the_preview_renders_as_the_first_device() {
        let h = user_harness();
        assert!(
            h.query_by_label("Rendered as office with its params and sample values.")
                .is_some()
        );
        assert!(h.query_by_label("b/office").is_some());
        assert!(h.query_by_label("600").is_some());
    }

    #[test]
    fn a_payload_syntax_error_is_shown_in_the_preview() {
        let mut h = user_harness();
        type_into_label(&mut h, "Payload", " {{");
        assert!(
            h.query_all_by_label_contains("syntax error")
                .next()
                .is_some()
        );
    }

    #[test]
    fn builtin_templates_offer_a_copy_instead_of_editing() {
        let mut h = Fixture::new("").harness(|ui, fx| {
            show_builtin(ui, "co2", &fx.doc, &mut fx.ui, &mut fx.actions);
        });
        h.get_by_label("Copy into project…").click();
        h.run();
        assert!(matches!(&h.state().other[..], [Action::AskCopyBuiltin(id)] if id == "co2"));
        assert!(h.state().doc.file().templates.is_empty());
    }

    #[test]
    fn typing_a_payload_one_character_at_a_time_keeps_focus() {
        let mut h = user_harness();
        h.get_by_label("Payload").focus();
        h.run();
        type_per_frame(&mut h, &["{", "{", "a", "b"]);
        assert_eq!(
            h.state().doc.file().templates[0].messages[0].payload,
            "{{ co2 }}{{ab"
        );
    }

    #[test]
    fn deleting_with_a_pending_field_rename_does_not_panic() {
        let mut h = user_harness();
        type_into_value(&mut h, "co2", "x");
        h.get_by_label("Delete template").click();
        h.run();
        assert!(h.state().doc.file().templates.is_empty());
    }

    #[test]
    fn the_template_id_changes_only_when_the_box_is_left_and_devices_follow() {
        let mut h = user_harness();
        type_into_label(&mut h, "ID", "2");
        assert_eq!(h.state().doc.file().templates[0].id, "air");
        assert!(h.state().doc.validated().errors.is_empty());
        h.key_press(egui::Key::Enter);
        h.run();
        assert_eq!(h.state().doc.file().templates[0].id, "air2");
        assert_eq!(h.state().doc.file().devices[0].template, "air2");
    }

    #[test]
    fn renaming_a_field_carries_device_overrides() {
        let mut h = user_harness_with(&src_replacing(
            "template = \"air\"\n",
            "template = \"air\"\nfields.co2 = { start = 700 }\n",
        ));
        type_into_value(&mut h, "co2", "x");
        h.key_press(egui::Key::Enter);
        h.run();
        let file = h.state().doc.file();
        assert!(file.templates[0].fields.contains_key("co2x"));
        assert!(file.devices[0].fields.contains_key("co2x"));
        let errors: Vec<String> = h
            .state()
            .doc
            .validated()
            .errors
            .iter()
            .map(ToString::to_string)
            .collect();
        assert!(
            !errors
                .iter()
                .any(|e| e.contains("override of unknown field")),
            "{errors:?}"
        );
    }

    #[test]
    fn typing_sequence_values_one_character_at_a_time_keeps_focus_and_spelling() {
        let mut h = user_harness_with(&src_replacing(
            r#"generator = "random_walk", start = 600, min = 400, max = 2000, step = 15"#,
            r#"generator = "sequence", values = ["open", 1]"#,
        ));
        type_into_value(&mut h, "open, 1", ",");
        type_per_frame(&mut h, &["x", ","]);
        let spec = &h.state().doc.file().templates[0].fields["co2"];
        let GeneratorSpec::Sequence { values, .. } = spec else {
            panic!("{spec:?}")
        };
        assert_eq!(
            values,
            &[
                Value::String("open".into()),
                Value::Number(1.0),
                Value::String("x".into())
            ]
        );
        assert!(h.query_all_by_value("open, 1,x,").next().is_some());
    }
}
