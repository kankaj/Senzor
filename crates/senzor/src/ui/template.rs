use std::collections::BTreeMap;
use std::time::SystemTime;

use eframe::egui::{self, RichText, TextEdit};
use senzor_core::engine::unannounced_fields;
use senzor_core::generator::GeneratorSpec;
use senzor_core::project::builtin::builtin_template;
use senzor_core::project::{FieldSpec, Location, MessageSpec, MqttVersion, TemplateSpec};

use super::template_text::{self, TextKind, Variable};
use super::{
    Action, BoxOrRow, ERROR, UiState, WARNING, checked, duration_edit, errors_at, help,
    mqtt5_control, name_edit, noise_edit, number_edit, optional_duration_edit, optional_text,
    replace_if_unchanged, sequence_edit, shortcuts, value_edit,
};
use crate::buffers::Buffers;
use crate::document::Document;
use crate::edits::{
    DEFAULT_NOISE, GeneratorKind, Removal, Rename, default_generator, new_availability,
    new_message, set_generator_kind, unique_name,
};
use crate::preview::{json_problem, preview};

/// The editor for project template `index`.
/// Without `preview_beside`, the preview is a section under the editor; with it, the window
/// shows it in a panel of its own ([`preview_beside`]).
pub fn show_user(
    ui: &mut egui::Ui,
    index: usize,
    doc: &Document,
    preview_beside: bool,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    let original = &doc.file().templates[index];
    let mut template = original.clone();
    ui.heading(format!("Template {}", original.id));
    let at = &Location::Template {
        id: original.id.clone(),
    };
    errors_at(ui, doc, at, shown_inline);
    ui.horizontal(|ui| {
        if ui.button("Duplicate template").clicked() {
            actions.push(Action::DuplicateTemplate(original.id.clone()));
        }
        if ui
            .button("Delete template")
            .on_hover_text(shortcuts::tip(
                ui.ctx(),
                "Delete the template",
                &shortcuts::DELETE,
            ))
            .clicked()
        {
            actions.push(Action::Remove(Removal::Template(index)));
            actions.push(Action::Select(None));
        }
    });
    ui.separator();
    editor(
        ui,
        &mut template,
        &format!("template/{index}"),
        Some(ProjectTemplate { index, doc, at }),
        uses_mqtt5(doc),
        ui_state,
        actions,
    );
    if !preview_beside {
        preview_below(ui, &format!("template/{index}"), &template, doc);
    }
    actions.extend(replace_if_unchanged(original, template, move |f| {
        f.templates.get_mut(index)
    }));
}

/// A read-only view of a built-in template, with "Copy into project".
pub fn show_builtin(
    ui: &mut egui::Ui,
    id: &str,
    doc: &Document,
    preview_beside: bool,
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
            uses_mqtt5(doc),
            ui_state,
            &mut ignored,
        );
    });
    if !preview_beside {
        preview_below(ui, &format!("builtin/{id}"), &template, doc);
    }
}

/// The preview as the editor's last section, for a narrow window.
fn preview_below(ui: &mut egui::Ui, key: &str, template: &TemplateSpec, doc: &Document) {
    section(
        ui,
        &format!("{key}/section/preview"),
        |ui| title(ui, "Preview"),
        |ui| preview_section(ui, template, doc),
    );
}

/// The project template an editor shows: its index, so its names can be renamed, and where its
/// problems are. Built-ins have none.
#[derive(Clone, Copy)]
struct ProjectTemplate<'a> {
    index: usize,
    doc: &'a Document,
    at: &'a Location,
}

/// Draws one value with `add`, marked with its problems when the editor shows a `project`
/// template (see [`checked`]).
fn cell(
    ui: &mut egui::Ui,
    project: Option<ProjectTemplate>,
    field: &str,
    add: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> egui::Response {
    match project {
        Some(ProjectTemplate { doc, at, .. }) => checked(ui, doc, at, field, add),
        None => add(ui),
    }
}

/// The template fields the editor shows its problems next to.
fn shown_inline(field: &str) -> bool {
    matches!(field, "id" | "interval" | "qos" | "messages" | "fields")
        || field.starts_with("fields.")
        || field.starts_with("messages[")
        || field.starts_with("availability.")
}

fn uses_mqtt5(doc: &Document) -> bool {
    doc.file().broker.mqtt_version == MqttVersion::V5
}

/// Edits `template` in place. For a `project` template, the ID and field names are name boxes that push
/// [`Rename`]s; without, they are plain labels. MQTT 5 settings are usable when `mqtt5`.
fn editor(
    ui: &mut egui::Ui,
    template: &mut TemplateSpec,
    key: &str,
    project: Option<ProjectTemplate>,
    mqtt5: bool,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    section(
        ui,
        &format!("{key}/section/settings"),
        |ui| title(ui, "Settings"),
        |ui| settings_grid(ui, template, key, project, mqtt5, ui_state, actions),
    );
    let variables = variables_for(TextKind::Message, template, project);
    section(
        ui,
        &format!("{key}/section/messages"),
        |ui| cell(ui, project, "messages", |ui| title(ui, "Messages")),
        |ui| {
            messages_editor(
                ui,
                &mut template.messages,
                key,
                project,
                mqtt5,
                &variables,
                &mut ui_state.buffers,
            );
        },
    );
    section(
        ui,
        &format!("{key}/section/fields"),
        |ui| {
            cell(ui, project, "fields", |ui| {
                ui.horizontal(|ui| {
                    let title = title(ui, "Fields");
                    help::mark(ui, "Fields", help::FIELDS);
                    title
                })
                .inner
            })
        },
        |ui| fields_editor(ui, &mut template.fields, key, project, ui_state, actions),
    );
}

/// A part of the editor that folds away: clicking its title (or the arrow before it) opens and
/// closes it. Every section starts open; egui remembers what you fold.
fn section(
    ui: &mut egui::Ui,
    id: &str,
    add_title: impl FnOnce(&mut egui::Ui) -> egui::Response,
    add_body: impl FnOnce(&mut egui::Ui),
) {
    let mut clicked = false;
    let mut header = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        egui::Id::new(id),
        true,
    )
    .show_header(ui, |ui| clicked = add_title(ui).clicked());
    if clicked {
        header.toggle();
    }
    header.body_unindented(|ui| add_body(ui));
    ui.add_space(4.0);
}

/// A section's title, which opens and closes it when clicked.
fn title(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Label::new(RichText::new(text).strong().size(15.0)).sense(egui::Sense::click()))
}

/// The preview of the selected template, for the panel beside the editor on a wide window.
pub fn preview_beside(ui: &mut egui::Ui, template: &TemplateSpec, doc: &Document) {
    ui.add(egui::Label::new(
        RichText::new("Preview").strong().size(15.0),
    ));
    preview_section(ui, template, doc);
}

/// The variables a `kind` of text in `template` can use: the built-in ones, its fields, and the
/// params of the project's devices that use it.
fn variables_for(
    kind: TextKind,
    template: &TemplateSpec,
    project: Option<ProjectTemplate>,
) -> Vec<Variable> {
    let fields: Vec<String> = template.fields.keys().cloned().collect();
    let mut params: Vec<String> = project
        .map(|p| {
            p.doc
                .file()
                .devices
                .iter()
                .filter(|d| d.template == template.id)
                .flat_map(|d| d.params.keys().cloned())
                .collect()
        })
        .unwrap_or_default();
    params.sort();
    params.dedup();
    template_text::variables(kind, &fields, &params)
}

/// A label for a template text box, with its `?` and the Insert variable menu; returns the label.
fn text_label(
    ui: &mut egui::Ui,
    name: &str,
    help: help::Help,
    id: egui::Id,
    text: &mut String,
    variables: &[Variable],
) -> egui::Response {
    ui.horizontal(|ui| {
        let label = help::label(ui, name, help);
        template_text::insert_menu(ui, id, name.trim(), text, variables);
        label
    })
    .inner
}

/// For a `project` template, a name box that pushes the [`Rename`] `rename` builds from the template's
/// index and the new name; otherwise a plain label. Returns the name box, if any.
fn name_or_label(
    ui: &mut egui::Ui,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
    project: Option<ProjectTemplate>,
    key: &str,
    name: &str,
    rename: impl Fn(usize, String) -> Rename,
) -> Option<egui::Response> {
    let Some(ProjectTemplate { index, doc, .. }) = project else {
        ui.label(name);
        return None;
    };
    let (response, committed) =
        name_edit(ui, ui_state, key, name, doc.file(), |to| rename(index, to));
    actions.extend(committed.map(Action::Rename));
    Some(response)
}

/// Which fields Home Assistant gets, as the first instance of the template would announce them.
fn discovery_note(template: &TemplateSpec, project: Option<ProjectTemplate>) -> Option<RichText> {
    let validated = project?.doc.validated();
    let Some(instance) = validated
        .project
        .instances()
        .find(|i| i.template_id == template.id)
    else {
        return Some(RichText::new("No device uses this template yet.").weak());
    };
    let left_out = unannounced_fields(instance);
    Some(if left_out.is_empty() {
        RichText::new("Every field is announced.").weak()
    } else {
        RichText::new(format!(
            "Not announced, not in one place of a payload: {}",
            left_out.join(", ")
        ))
        .color(WARNING)
    })
}

/// ID, interval, QoS, retain, Home Assistant and availability.
fn settings_grid(
    ui: &mut egui::Ui,
    template: &mut TemplateSpec,
    key: &str,
    project: Option<ProjectTemplate>,
    mqtt5: bool,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    let topic_variables = variables_for(TextKind::AvailabilityTopic, template, project);
    egui::Grid::new("template-form")
        .num_columns(2)
        .show(ui, |ui| {
            let label = help::label(ui, "ID", help::TEMPLATE_ID);
            cell(ui, project, "id", |ui| {
                let id_box = name_or_label(
                    ui,
                    ui_state,
                    actions,
                    project,
                    &format!("{key}/id"),
                    &template.id,
                    |index, to| Rename::Template {
                        index,
                        from: template.id.clone(),
                        to,
                    },
                );
                match id_box {
                    Some(id_box) => id_box.labelled_by(label.id),
                    None => label.clone(),
                }
            });
            ui.end_row();

            let label = help::label(ui, "Interval", help::TEMPLATE_INTERVAL);
            cell(ui, project, "interval", |ui| {
                duration_edit(
                    ui,
                    &mut ui_state.buffers,
                    &format!("{key}/interval"),
                    &mut template.interval,
                )
                .labelled_by(label.id)
            });
            ui.end_row();

            help::label(ui, "QoS", help::QOS);
            cell(ui, project, "qos", |ui| {
                egui::ComboBox::from_id_salt(format!("{key}/qos"))
                    .selected_text(template.qos.to_string())
                    .show_ui(ui, |ui| {
                        for qos in 0..=2u8 {
                            ui.selectable_value(&mut template.qos, qos, qos.to_string());
                        }
                    })
                    .response
            });
            ui.end_row();

            help::label(ui, "Retain", help::RETAIN);
            ui.checkbox(&mut template.retain, "retain data messages");
            ui.end_row();

            help::label(ui, "Home Assistant", help::HOME_ASSISTANT);
            ui.checkbox(&mut template.home_assistant, "announce over MQTT Discovery");
            ui.end_row();
            if template.home_assistant
                && let Some(note) = discovery_note(template, project)
            {
                ui.label("");
                ui.label(note);
                ui.end_row();
            }

            help::label(ui, "Availability", help::AVAILABILITY);
            let mut enabled = template.availability.is_some();
            if ui
                .checkbox(&mut enabled, "online/offline topic with Last Will")
                .changed()
            {
                template.availability = enabled.then(new_availability);
            }
            ui.end_row();
            if let Some(availability) = &mut template.availability {
                let id = egui::Id::new(format!("{key}/availability/topic"));
                let label = text_label(
                    ui,
                    "  Topic",
                    help::TOPIC,
                    id,
                    &mut availability.topic,
                    &topic_variables,
                );
                cell(ui, project, "availability.topic", |ui| {
                    template_text::edit(ui, &mut availability.topic, id, false, &topic_variables)
                        .labelled_by(label.id)
                });
                ui.end_row();
                // The online and offline payloads are plain text, not templates.
                for (label, text, salt) in [
                    ("  Online payload", &mut availability.online, "online"),
                    ("  Offline payload", &mut availability.offline, "offline"),
                ] {
                    let label = ui.label(label);
                    cell(ui, project, &format!("availability.{salt}"), |ui| {
                        ui.add(
                            TextEdit::singleline(text)
                                .id_salt(format!("{key}/availability/{salt}")),
                        )
                        .labelled_by(label.id)
                    });
                    ui.end_row();
                }
                let label = help::label(ui, "  Will delay", help::WILL_DELAY);
                let has_value = availability.will_delay.is_some();
                cell(ui, project, "availability.will_delay", |ui| {
                    mqtt5_control(ui, mqtt5, has_value, |ui| {
                        optional_duration_edit(
                            ui,
                            &mut ui_state.buffers,
                            &format!("{key}/availability/will_delay"),
                            &mut availability.will_delay,
                        )
                        .labelled_by(label.id)
                    })
                });
                ui.end_row();
            }
        });
}

fn messages_editor(
    ui: &mut egui::Ui,
    messages: &mut Vec<MessageSpec>,
    key: &str,
    project: Option<ProjectTemplate>,
    mqtt5: bool,
    variables: &[Variable],
    buffers: &mut Buffers,
) {
    let mut remove = None;
    for (i, message) in messages.iter_mut().enumerate() {
        ui.group(|ui| {
            ui.horizontal(|ui| {
                ui.label(format!("Message {}", i + 1));
                if ui.button("Remove").clicked() {
                    remove = Some(i);
                }
            });
            // Problems name messages from 1.
            let field = |name: &str| format!("messages[{}].{name}", i + 1);
            let id = egui::Id::new(format!("{key}/message/{i}/topic"));
            let label = text_label(ui, "Topic", help::TOPIC, id, &mut message.topic, variables);
            cell(ui, project, &field("topic"), |ui| {
                template_text::edit(ui, &mut message.topic, id, false, variables)
                    .labelled_by(label.id)
            });
            let id = egui::Id::new(format!("{key}/message/{i}/payload"));
            let label = text_label(
                ui,
                "Payload",
                help::PAYLOAD,
                id,
                &mut message.payload,
                variables,
            );
            cell(ui, project, &field("payload"), |ui| {
                template_text::edit(ui, &mut message.payload, id, true, variables)
                    .labelled_by(label.id)
            });
            let has_properties = message.has_properties();
            egui::CollapsingHeader::new("MQTT 5 properties")
                .id_salt(format!("{key}/message/{i}/properties"))
                .default_open(has_properties)
                .show(ui, |ui| {
                    mqtt5_control(ui, mqtt5, has_properties, |ui| {
                        ui.vertical(|ui| {
                            properties_editor(
                                ui,
                                message,
                                &format!("{key}/message/{i}"),
                                project,
                                &field,
                                buffers,
                            );
                        });
                    });
                });
        });
    }
    if let Some(i) = remove {
        messages.remove(i);
    }
    if ui.button("Add message").clicked() {
        messages.push(new_message());
    }
}

/// Message expiry, content type and user properties of one message.
/// `field` gives the key path of one of the message's properties.
fn properties_editor(
    ui: &mut egui::Ui,
    message: &mut MessageSpec,
    key: &str,
    project: Option<ProjectTemplate>,
    field: &dyn Fn(&str) -> String,
    buffers: &mut Buffers,
) {
    egui::Grid::new(format!("{key}/properties"))
        .num_columns(2)
        .show(ui, |ui| {
            let label = help::label(ui, "Message expiry", help::MESSAGE_EXPIRY);
            cell(ui, project, &field("message_expiry"), |ui| {
                optional_duration_edit(
                    ui,
                    buffers,
                    &format!("{key}/message_expiry"),
                    &mut message.message_expiry,
                )
                .labelled_by(label.id)
            });
            ui.end_row();

            let label = help::label(ui, "Content type", help::CONTENT_TYPE);
            cell(ui, project, &field("content_type"), |ui| {
                optional_text(ui, &mut message.content_type, "none").labelled_by(label.id)
            });
            ui.end_row();
        });
    cell(ui, project, &field("user_properties"), |ui| {
        help::label(ui, "User properties", help::USER_PROPERTIES)
    });
    let names: Vec<String> = message.user_properties.keys().cloned().collect();
    let mut rename = None;
    let mut remove = None;
    // Name boxes whose text was committed or dropped; their buffers start over.
    let mut settled = Vec::new();
    for (name, value) in &mut message.user_properties {
        ui.horizontal(|ui| {
            // Keyed by the committed name, so the box keeps focus while a new name is typed.
            let id = format!("{key}/user_property/{name}");
            let name_key = format!("{id}/name");
            let buffer = buffers.get(&name_key, name);
            let response = ui.add(
                TextEdit::singleline(&mut buffer.text)
                    .id_salt(&name_key)
                    .hint_text("name")
                    .desired_width(90.0),
            );
            let typed = buffer.text.trim();
            let problem = if typed.is_empty() {
                Some("a name is required".to_string())
            } else if typed != name && names.iter().any(|n| n == typed) {
                Some(format!("`{typed}` is used already"))
            } else {
                None
            };
            ui.label("=");
            ui.add(
                TextEdit::singleline(value)
                    .id_salt(format!("{id}/value"))
                    .hint_text("value, e.g. {{ room }}")
                    .desired_width(150.0),
            );
            if ui.button("Remove property").clicked() {
                remove = Some(name.clone());
                settled.push(name_key.clone());
            }
            if response.has_focus() {
                if let Some(problem) = &problem {
                    ui.colored_label(ERROR, problem);
                }
            } else if buffer.text != *name {
                // Left the box: apply a valid new name, otherwise go back to the old one.
                if problem.is_none() && typed != name {
                    rename = Some((name.clone(), typed.to_string()));
                }
                settled.push(name_key);
            }
        });
    }
    for name_key in settled {
        buffers.remove(&name_key);
    }
    if let Some((from, to)) = rename
        && let Some(value) = message.user_properties.remove(&from)
    {
        message.user_properties.insert(to, value);
    }
    if let Some(name) = remove {
        message.user_properties.remove(&name);
    }
    if ui.button("Add user property").clicked() {
        let name = unique_name("name", message.user_properties.keys());
        message.user_properties.insert(name, String::new());
    }
}

fn fields_editor(
    ui: &mut egui::Ui,
    fields: &mut BTreeMap<String, FieldSpec>,
    key: &str,
    project: Option<ProjectTemplate>,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    let names: Vec<String> = fields.keys().cloned().collect();
    for name in names {
        ui.group(|ui| {
            cell(ui, project, &format!("fields.{name}"), |ui| {
                ui.horizontal(|ui| {
                    let name_box = name_or_label(
                        ui,
                        ui_state,
                        actions,
                        project,
                        &format!("{key}/field/{name}"),
                        &name,
                        |template, to| Rename::Field {
                            template,
                            from: name.clone(),
                            to,
                        },
                    );
                    let Some(spec) = fields.get_mut(&name).map(|f| &mut f.generator) else {
                        return name_box;
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
                    name_box
                })
                .box_or_row()
            });
            if let Some(field) = fields.get_mut(&name) {
                ui.horizontal(|ui| {
                    let key = format!("{key}/field/{name}");
                    let label = help::label(ui, "Name", help::FIELD_NAME);
                    describe_edit(ui, &mut field.name, &format!("{key}/name"), &name)
                        .labelled_by(label.id);
                    let label = help::label(ui, "Unit", help::FIELD_UNIT);
                    describe_edit(ui, &mut field.unit, &format!("{key}/unit"), "e.g. ppm")
                        .labelled_by(label.id);
                });
            }
            if let Some(spec) = fields.get_mut(&name).map(|f| &mut f.generator) {
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
        fields.insert(name, default_generator(GeneratorKind::RandomWalk).into());
    }
}

fn number(ui: &mut egui::Ui, label: &str, value: &mut f64) {
    ui.label(label);
    number_edit(ui, value);
}

/// A field's name or unit: an empty box means none, so the file gets no empty value.
fn describe_edit(
    ui: &mut egui::Ui,
    value: &mut Option<String>,
    key: &str,
    hint: &str,
) -> egui::Response {
    let mut text = value.clone().unwrap_or_default();
    let response = ui.add(
        TextEdit::singleline(&mut text)
            .id_salt(key)
            .hint_text(hint)
            .desired_width(110.0),
    );
    if response.changed() {
        *value = (!text.is_empty()).then_some(text);
    }
    response
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
            sequence_edit(ui, buffers, &format!("{key}/values"), values);
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
        noise_edit(ui, noise);
    }
}

fn preview_section(ui: &mut egui::Ui, template: &TemplateSpec, doc: &Document) {
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
                let properties = &message.properties;
                if let Some(problem) =
                    json_problem(&message.payload, properties.content_type.as_deref())
                {
                    ui.colored_label(
                        WARNING,
                        format!("{} {problem}", egui_phosphor::regular::WARNING),
                    );
                }
                if let Some(content_type) = &properties.content_type {
                    ui.weak(format!("content type: {content_type}"));
                }
                if let Some(expiry) = properties.message_expiry {
                    ui.weak(format!("message expiry: {expiry} s"));
                }
                for (name, value) in &properties.user_properties {
                    ui.weak(format!("user property: {name} = {value}"));
                }
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
    use egui_kittest::kittest::{NodeT, Queryable};
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
                show_user(ui, 0, &fx.doc, false, &mut fx.ui, &mut fx.actions);
            }
        })
    }

    /// `SRC` with `params.room = "office"` on the device and `properties` on the message.
    fn src_with_properties(broker: &str, properties: &str) -> String {
        format!(
            "{broker}\n{}",
            src_replacing(
                r#"payload = "{{ co2 }}" }]"#,
                &format!(r#"payload = "{{{{ co2 }}}}"{properties} }}]"#),
            )
            .replace(
                "template = \"air\"\n",
                "template = \"air\"\nparams = { room = \"office\" }\n"
            )
        )
    }

    #[test]
    fn turning_on_home_assistant_says_which_fields_are_announced() {
        let mut h = user_harness();
        assert!(h.query_by_label("Every field is announced.").is_none());
        h.get_by_label("announce over MQTT Discovery").click();
        h.run();
        assert!(h.state().doc.file().templates[0].home_assistant);
        h.run();
        assert!(h.query_by_label("Every field is announced.").is_some());
    }

    #[test]
    fn a_field_home_assistant_cannot_read_is_named() {
        let src = src_replacing(r#"id = "air""#, "id = \"air\"\nhome_assistant = true")
            .replace(r#"payload = "{{ co2 }}""#, r#"payload = "co2={{ co2 }}""#);
        let h = user_harness_with(&src);
        assert!(
            h.query_by_label("Not announced, not in one place of a payload: co2")
                .is_some()
        );
    }

    #[test]
    fn the_preview_shows_rendered_mqtt5_properties() {
        let h = user_harness_with(&src_with_properties(
            "",
            r#", message_expiry = "60s", content_type = "application/json", user_properties = { room = "{{ room }}" }"#,
        ));
        assert!(h.query_by_label("user property: room = office").is_some());
        assert!(h.query_by_label("content type: application/json").is_some());
        assert!(h.query_by_label("message expiry: 60 s").is_some());
    }

    #[test]
    fn adding_and_removing_a_user_property_edits_the_message() {
        let mut h = user_harness();
        h.get_by_label("MQTT 5 properties").click();
        h.run();
        h.get_by_label("Add user property").click();
        h.run();
        let message = &h.state().doc.file().templates[0].messages[0];
        assert_eq!(
            message.user_properties.get("name").map(String::as_str),
            Some("")
        );
        assert!(h.state().doc.is_dirty());

        h.get_by_label("Remove property").click();
        h.run();
        assert!(
            h.state().doc.file().templates[0].messages[0]
                .user_properties
                .is_empty()
        );
    }

    #[test]
    fn a_user_property_is_renamed_when_its_name_box_is_left() {
        let mut h = user_harness_with(&src_with_properties(
            "",
            r#", user_properties = { zone = "{{ room }}" }"#,
        ));
        // The box and its text both carry the value; the box comes first.
        h.query_all_by_value("zone").next().unwrap().focus();
        h.run();
        h.key_press(egui::Key::End);
        type_per_frame(&mut h, &["d"]);
        let properties = &h.state().doc.file().templates[0].messages[0].user_properties;
        assert!(
            properties.contains_key("zone"),
            "not renamed while typing: {properties:?}"
        );

        h.key_press(egui::Key::Tab);
        h.run();
        h.run();
        let properties = &h.state().doc.file().templates[0].messages[0].user_properties;
        assert_eq!(
            properties.get("zoned").map(String::as_str),
            Some("{{ room }}")
        );
        assert!(!properties.contains_key("zone"));
    }

    /// Focuses the name box of user property `name`, replaces its text with `typed`, and leaves it.
    fn rename_user_property(h: &mut Harness<'static, Fixture>, name: &str, typed: &str) {
        // The box and its text both carry the value; the box comes first.
        h.query_all_by_value(name).next().unwrap().focus();
        h.run();
        for _ in 0..name.len() {
            h.key_press(egui::Key::Backspace);
            h.run();
        }
        let keys: Vec<String> = typed.chars().map(String::from).collect();
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        type_per_frame(h, &keys);
        h.key_press(egui::Key::Tab);
        h.run();
        h.run();
    }

    fn user_properties(h: &Harness<'static, Fixture>) -> Vec<String> {
        h.state().doc.file().templates[0].messages[0]
            .user_properties
            .keys()
            .cloned()
            .collect()
    }

    #[test]
    fn adding_after_a_rename_starts_with_a_fresh_name_box() {
        let mut h = user_harness();
        h.get_by_label("MQTT 5 properties").click();
        h.run();
        h.get_by_label("Add user property").click();
        h.run();
        rename_user_property(&mut h, "name", "room");
        assert_eq!(user_properties(&h), ["room"]);

        h.get_by_label("Add user property").click();
        h.run();
        h.run();
        assert_eq!(user_properties(&h), ["name", "room"]);
        // Before the fix, the new property's box showed the stale `room` and an error.
        assert!(
            h.query_all_by_value("name").next().is_some(),
            "the new name box shows `name`"
        );
        assert_eq!(
            h.query_all_by_value("room").count(),
            2,
            "only `room`'s own box"
        );
    }

    #[test]
    fn renaming_back_and_forth_settles() {
        let mut h = user_harness_with(&src_with_properties(
            "",
            r#", user_properties = { a = "1" }"#,
        ));
        rename_user_property(&mut h, "a", "b");
        rename_user_property(&mut h, "b", "a");
        for _ in 0..5 {
            assert_eq!(user_properties(&h), ["a"], "the editor keeps renaming");
            h.run();
        }
    }

    #[test]
    fn a_name_used_already_is_dropped_when_the_box_is_left() {
        let mut h = user_harness_with(&src_with_properties(
            "",
            r#", user_properties = { a = "1", b = "2" }"#,
        ));
        rename_user_property(&mut h, "a", "b");
        assert_eq!(user_properties(&h), ["a", "b"]);
        assert!(
            h.query_all_by_value("a").next().is_some(),
            "the box shows `a` again"
        );
    }

    #[test]
    fn user_property_names_are_trimmed() {
        let mut h = user_harness_with(&src_with_properties(
            "",
            r#", user_properties = { a = "1" }"#,
        ));
        rename_user_property(&mut h, "a", "room ");
        assert_eq!(user_properties(&h), ["room"]);
    }

    #[test]
    fn typing_a_content_type_and_a_message_expiry_sets_them() {
        let mut h = user_harness();
        h.get_by_label("MQTT 5 properties").click();
        h.run();
        type_into_label(&mut h, "Content type", "text/plain");
        type_into_label(&mut h, "Message expiry", "60s");
        let message = &h.state().doc.file().templates[0].messages[0];
        assert_eq!(message.content_type.as_deref(), Some("text/plain"));
        assert_eq!(message.message_expiry, Some(Duration::from_secs(60)));
        assert!(
            h.state()
                .doc
                .file()
                .to_toml_string()
                .contains(r#"content_type = "text/plain""#)
        );
    }

    #[test]
    fn a_will_delay_is_set_under_availability() {
        let mut h = user_harness_with(&src_replacing(
            "interval = \"10s\"\n",
            "interval = \"10s\"\navailability = { topic = \"s/{{ device_id }}\", online = \"on\", offline = \"off\" }\n",
        ));
        type_into_label(&mut h, "  Will delay", "5s");
        let availability = h.state().doc.file().templates[0]
            .availability
            .clone()
            .unwrap();
        assert_eq!(availability.will_delay, Some(Duration::from_secs(5)));
    }

    #[test]
    fn on_mqtt_3_1_1_a_set_content_type_shows_the_reason_and_can_be_cleared() {
        let mut h = user_harness_with(&src_with_properties(
            "[broker]\nmqtt_version = \"3.1.1\"\n",
            r#", content_type = "text/plain""#,
        ));
        assert!(h.query_by_label(crate::ui::NEEDS_MQTT5).is_some());
        h.get_by_label("Content type").focus();
        h.run();
        for _ in 0.."text/plain".len() {
            h.key_press(egui::Key::Backspace);
            h.run();
        }
        assert_eq!(
            h.state().doc.file().templates[0].messages[0].content_type,
            None
        );
    }

    #[test]
    fn on_mqtt_3_1_1_empty_properties_are_disabled() {
        let mut h = user_harness_with(&format!("[broker]\nmqtt_version = \"3.1.1\"\n{SRC}"));
        h.get_by_label("MQTT 5 properties").click();
        h.run();
        assert!(
            h.get_by_label("Content type")
                .accesskit_node()
                .is_disabled()
        );
        assert!(
            h.get_by_label("Add user property")
                .accesskit_node()
                .is_disabled()
        );
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
            h.state().doc.file().templates[0].fields["co2"].generator,
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
            h.state().doc.file().templates[0].fields["co2"].generator,
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
            h.state().doc.file().templates[0]
                .fields
                .get("field")
                .map(|f| &f.generator),
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
    fn the_preview_warns_about_a_payload_that_looks_like_json_but_is_not() {
        let h = user_harness_with(&src_replacing(
            r#"payload = "{{ co2 }}""#,
            r#"payload = '{"co2": {{ co2 }},}'"#,
        ));
        assert!(
            h.query_by_label_contains("looks like JSON but is not valid JSON")
                .is_some()
        );
    }

    #[test]
    fn a_fields_name_and_unit_are_edited_and_empty_means_none() {
        let mut h = user_harness();
        type_into_label(&mut h, "Unit", "ppm");
        type_into_label(&mut h, "Name", "CO2");
        let field = &h.state().doc.file().templates[0].fields["co2"];
        assert_eq!(field.unit.as_deref(), Some("ppm"));
        assert_eq!(field.name.as_deref(), Some("CO2"));

        h.get_by_label("Unit").focus();
        h.run();
        for _ in 0.."ppm".len() {
            h.key_press(egui::Key::Backspace);
        }
        h.run();
        assert_eq!(h.state().doc.file().templates[0].fields["co2"].unit, None);
    }

    #[test]
    fn a_section_folds_and_unfolds_when_its_title_is_clicked() {
        let mut h = user_harness();
        assert!(h.query_by_label("Topic").is_some());
        h.get_by_label("Messages").click();
        h.run();
        assert!(h.query_by_label("Topic").is_none(), "Messages folded");
        assert!(
            h.query_by_label("Interval").is_some(),
            "Settings still open"
        );
        h.get_by_label("Messages").click();
        h.run();
        assert!(h.query_by_label("Topic").is_some());
    }

    fn payload(h: &Harness<'static, Fixture>) -> String {
        h.state().doc.file().templates[0].messages[0]
            .payload
            .clone()
    }

    #[test]
    fn insert_variable_puts_a_block_into_the_payload() {
        let mut h = user_harness();
        h.get_by_label("Insert variable: Payload").click();
        h.run();
        h.get_by_label("seq").click();
        h.run();
        assert_eq!(payload(&h), "{{ co2 }}{{ seq }}");
    }

    #[test]
    fn tab_completes_a_variable_while_it_is_typed() {
        let mut h = user_harness();
        h.get_by_label("Payload").focus();
        h.run();
        h.key_press(egui::Key::End);
        h.run();
        type_per_frame(&mut h, &[" ", "{", "{", " ", "c"]);
        assert!(h.query_by_label("co2").is_some(), "co2 is suggested");
        h.key_press(egui::Key::Tab);
        h.run();
        assert_eq!(payload(&h), "{{ co2 }} {{ co2 }}");
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
            show_builtin(ui, "co2", &fx.doc, false, &mut fx.ui, &mut fx.actions);
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
        let spec = &h.state().doc.file().templates[0].fields["co2"].generator;
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
