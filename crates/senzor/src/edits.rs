//! Edits the GUI applies to the project model. Plain functions, no egui.

use std::collections::BTreeMap;
use std::time::Duration;

use senzor_core::generator::GeneratorSpec;
use senzor_core::project::builtin::{builtin_template, builtin_templates, is_builtin};
use senzor_core::project::file::{DEFAULT_INTERVAL, default_instance_id_pattern};
use senzor_core::project::{
    AvailabilitySpec, BrokerSpec, DeviceSpec, MessageSpec, ProjectFile, TemplateSpec,
};
use senzor_core::value::Value;

/// The topic new messages publish on.
const NEW_MESSAGE_TOPIC: &str = "senzor/{{ device_id }}/value";

/// The noise a generator starts with when noise is turned on.
pub const DEFAULT_NOISE: f64 = 0.1;

/// `base` if `taken` says it is free, otherwise `base-2`, `base-3`, …
pub fn unique_id(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !taken(candidate))
        .expect("an unused id exists")
}

fn template_id_taken(file: &ProjectFile, id: &str) -> bool {
    file.templates.iter().any(|t| t.id == id) || is_builtin(id)
}

/// Why `id` cannot name a new or renamed template: it is empty, a built-in's, or taken.
fn check_new_template_id(file: &ProjectFile, id: &str) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err("the name must not be empty".into());
    }
    if is_builtin(id) {
        return Err(format!("`{id}` is a built-in template id"));
    }
    if file.templates.iter().any(|t| t.id == id) {
        return Err(format!("a template `{id}` already exists"));
    }
    Ok(())
}

/// Appends a device and returns its index. A project's own templates are what its devices most
/// likely use; without any, the first built-in (`co2`) gives a device that runs as it is.
pub fn add_device(file: &mut ProjectFile) -> usize {
    let template = file
        .templates
        .first()
        .unwrap_or(&builtin_templates()[0])
        .id
        .clone();
    let id = unique_id("device", |id| file.devices.iter().any(|d| d.id == id));
    file.devices.push(DeviceSpec::new(id, template));
    file.devices.len() - 1
}

/// Appends a small working template (one message, one field) and returns its index.
pub fn add_template(file: &mut ProjectFile) -> usize {
    let id = unique_id("template", |id| template_id_taken(file, id));
    file.templates.push(TemplateSpec {
        id,
        interval: DEFAULT_INTERVAL,
        qos: 0,
        retain: false,
        availability: None,
        messages: vec![MessageSpec {
            topic: NEW_MESSAGE_TOPIC.into(),
            payload: r#"{"value": {{ value | round(1) }}}"#.into(),
        }],
        fields: BTreeMap::from([(
            "value".to_string(),
            default_generator(GeneratorKind::RandomWalk),
        )]),
    });
    file.templates.len() - 1
}

/// A message to add to a template: field `value` on the topic new templates use.
pub fn new_message() -> MessageSpec {
    MessageSpec {
        topic: NEW_MESSAGE_TOPIC.into(),
        payload: "{{ value }}".into(),
    }
}

/// The availability topic and payloads a template starts with when availability is turned on.
pub fn new_availability() -> AvailabilitySpec {
    AvailabilitySpec {
        topic: "senzor/{{ device_id }}/status".into(),
        online: "online".into(),
        offline: "offline".into(),
    }
}

/// The id offered for a copy of built-in `builtin`: `<builtin>-copy`, numbered when taken.
pub fn copy_id_suggestion(file: &ProjectFile, builtin: &str) -> String {
    unique_id(&format!("{builtin}-copy"), |id| template_id_taken(file, id))
}

/// Copies built-in template `builtin` into the project under `new_id`; returns the new index.
pub fn copy_builtin(file: &mut ProjectFile, builtin: &str, new_id: &str) -> Result<usize, String> {
    let new_id = new_id.trim();
    check_new_template_id(file, new_id)?;
    let mut template = builtin_template(builtin)
        .cloned()
        .ok_or_else(|| format!("unknown built-in template `{builtin}`"))?;
    template.id = new_id.to_string();
    file.templates.push(template);
    Ok(file.templates.len() - 1)
}

/// The template a device naming `id` runs on. Built-ins come first, as in validation: a project
/// template cannot replace one.
pub fn find_template<'a>(file: &'a ProjectFile, id: &str) -> Option<&'a TemplateSpec> {
    builtin_template(id).or_else(|| file.templates.iter().find(|t| t.id == id))
}

/// The interval of the template a device names, so overriding it starts from what the device
/// already uses.
pub fn template_interval(file: &ProjectFile, id: &str) -> Option<Duration> {
    find_template(file, id).map(|t| t.interval)
}

/// Sets how many instances the device runs. Ticking "custom" seeds the instance ID pattern with
/// the default, so a pattern still equal to the old count's default behaves like the default and
/// follows to the new count's; an edited pattern is kept.
pub fn set_count(device: &mut DeviceSpec, count: u32) {
    if device.instance_id.as_deref() == Some(default_instance_id_pattern(device.count)) {
        device.instance_id = Some(default_instance_id_pattern(count).to_string());
    }
    device.count = count;
}

/// A name for a new field or param that does not clash with `existing`.
pub fn unique_name<'a>(base: &str, existing: impl IntoIterator<Item = &'a String>) -> String {
    let existing: Vec<&String> = existing.into_iter().collect();
    unique_id(base, |name| existing.iter().any(|e| e.as_str() == name))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneratorKind {
    Constant,
    RandomWalk,
    Sine,
    Uniform,
    Sequence,
}

impl GeneratorKind {
    pub const ALL: [GeneratorKind; 5] = [
        GeneratorKind::Constant,
        GeneratorKind::RandomWalk,
        GeneratorKind::Sine,
        GeneratorKind::Uniform,
        GeneratorKind::Sequence,
    ];

    pub fn of(spec: &GeneratorSpec) -> Self {
        match spec {
            GeneratorSpec::Constant { .. } => GeneratorKind::Constant,
            GeneratorSpec::RandomWalk { .. } => GeneratorKind::RandomWalk,
            GeneratorSpec::Sine { .. } => GeneratorKind::Sine,
            GeneratorSpec::Uniform { .. } => GeneratorKind::Uniform,
            GeneratorSpec::Sequence { .. } => GeneratorKind::Sequence,
        }
    }

    /// The `generator = "…"` name used in the project file.
    pub fn label(self) -> &'static str {
        match self {
            GeneratorKind::Constant => "constant",
            GeneratorKind::RandomWalk => "random_walk",
            GeneratorKind::Sine => "sine",
            GeneratorKind::Uniform => "uniform",
            GeneratorKind::Sequence => "sequence",
        }
    }
}

/// A valid generator of `kind` to start editing from.
pub fn default_generator(kind: GeneratorKind) -> GeneratorSpec {
    match kind {
        GeneratorKind::Constant => GeneratorSpec::Constant {
            value: Value::Number(0.0),
            noise: None,
        },
        GeneratorKind::RandomWalk => GeneratorSpec::RandomWalk {
            start: 50.0,
            min: 0.0,
            max: 100.0,
            step: 1.0,
            noise: None,
        },
        GeneratorKind::Sine => GeneratorSpec::Sine {
            center: 0.0,
            amplitude: 1.0,
            period: Duration::from_secs(60),
            phase: Duration::ZERO,
            noise: None,
        },
        GeneratorKind::Uniform => GeneratorSpec::Uniform {
            min: 0.0,
            max: 1.0,
            noise: None,
        },
        GeneratorKind::Sequence => GeneratorSpec::Sequence {
            values: vec![Value::String("on".into()), Value::String("off".into())],
            looping: true,
            noise: None,
        },
    }
}

/// Switches `spec` to `kind`; keeps it unchanged when it already is that kind.
pub fn set_generator_kind(spec: &mut GeneratorSpec, kind: GeneratorKind) {
    if GeneratorKind::of(spec) != kind {
        *spec = default_generator(kind);
    }
}

/// The type of a param or `constant` value. The user picks it next to the value box, so typing
/// `007` or `true` into a text value does not turn it into a number or bool (and change topics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Text,
    Number,
    Bool,
}

impl ValueKind {
    pub const ALL: [ValueKind; 3] = [ValueKind::Text, ValueKind::Number, ValueKind::Bool];

    pub fn of(value: &Value) -> Self {
        match value {
            Value::String(_) => ValueKind::Text,
            Value::Number(_) => ValueKind::Number,
            Value::Bool(_) => ValueKind::Bool,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ValueKind::Text => "text",
            ValueKind::Number => "number",
            ValueKind::Bool => "bool",
        }
    }
}

/// Reads typed text as a value of `kind`. Text is kept exactly as typed (spaces included);
/// numbers and bools are trimmed, and anything else is an error shown under the box.
pub fn parse_value(kind: ValueKind, text: &str) -> Result<Value, String> {
    match kind {
        ValueKind::Text => Ok(Value::String(text.to_string())),
        ValueKind::Number => text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite())
            .map(Value::Number)
            .ok_or_else(|| "use a number, e.g. 42 or 22.5".to_string()),
        ValueKind::Bool => match text.trim() {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err("use true or false".to_string()),
        },
    }
}

/// A `sequence` typed as `open, closed, 3`. Unlike params, its items may mix types, so each is
/// read like a pinned value (number, then bool, then text); empty items are skipped.
pub fn parse_sequence(text: &str) -> Vec<Value> {
    text.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(Value::parse_input)
        .collect()
}

/// The text [`parse_sequence`] reads back as `values`.
pub fn sequence_text(values: &[Value]) -> String {
    values
        .iter()
        .map(Value::to_input)
        .collect::<Vec<_>>()
        .join(", ")
}

/// A rename typed into a name box. It names what it renames (`from`), so a rename that arrives
/// after the item changed or moved is refused instead of hitting another item.
#[derive(Debug, Clone, PartialEq)]
pub enum Rename {
    Device {
        index: usize,
        from: String,
        to: String,
    },
    Template {
        index: usize,
        from: String,
        to: String,
    },
    Param {
        device: usize,
        from: String,
        to: String,
    },
    Field {
        template: usize,
        from: String,
        to: String,
    },
}

fn stale_rename() -> String {
    "it was changed or removed in the meantime".to_string()
}

impl Rename {
    fn names(&self) -> (&str, &str) {
        match self {
            Rename::Device { from, to, .. }
            | Rename::Template { from, to, .. }
            | Rename::Param { from, to, .. }
            | Rename::Field { from, to, .. } => (from, to),
        }
    }

    /// Why the rename cannot be applied; shown under the name box.
    pub fn check(&self, file: &ProjectFile) -> Result<(), String> {
        let (from, to) = self.names();
        if to.trim().is_empty() {
            return Err("the name must not be empty".into());
        }
        match self {
            Rename::Device { index, .. } => {
                file.devices
                    .get(*index)
                    .filter(|d| d.id == from)
                    .ok_or_else(stale_rename)?;
                if to != from && file.devices.iter().any(|d| d.id == to) {
                    return Err(format!("a device `{to}` already exists"));
                }
            }
            Rename::Template { index, .. } => {
                file.templates
                    .get(*index)
                    .filter(|t| t.id == from)
                    .ok_or_else(stale_rename)?;
                if to != from {
                    check_new_template_id(file, to)?;
                }
            }
            Rename::Param { device, .. } => {
                let device = file
                    .devices
                    .get(*device)
                    .filter(|d| d.params.contains_key(from))
                    .ok_or_else(stale_rename)?;
                if to != from && device.params.contains_key(to) {
                    return Err(format!("the device already has a param `{to}`"));
                }
            }
            Rename::Field { template, .. } => {
                let template = file
                    .templates
                    .get(*template)
                    .filter(|t| t.fields.contains_key(from))
                    .ok_or_else(stale_rename)?;
                if to != from && template.fields.contains_key(to) {
                    return Err(format!("the template already has a field `{to}`"));
                }
            }
        }
        Ok(())
    }

    /// Applies the rename and carries it to what refers to the old name: devices follow a
    /// renamed template, device overrides follow a renamed field.
    pub fn apply(&self, file: &mut ProjectFile) -> Result<(), String> {
        self.check(file)?;
        let (from, to) = self.names();
        let (from, to) = (from.to_string(), to.to_string());
        match self {
            Rename::Device { index, .. } => file.devices[*index].id = to,
            Rename::Template { index, .. } => {
                file.templates[*index].id = to.clone();
                for device in file.devices.iter_mut().filter(|d| d.template == from) {
                    device.template = to.clone();
                }
            }
            Rename::Param { device, .. } => {
                let params = &mut file.devices[*device].params;
                if let Some(value) = params.remove(&from) {
                    params.insert(to, value);
                }
            }
            Rename::Field { template, .. } => {
                let template = &mut file.templates[*template];
                if let Some(spec) = template.fields.remove(&from) {
                    template.fields.insert(to.clone(), spec);
                }
                let template_id = template.id.clone();
                for device in file
                    .devices
                    .iter_mut()
                    .filter(|d| d.template == template_id)
                {
                    if let Some(table) = device.fields.remove(&from) {
                        device.fields.insert(to.clone(), table);
                    }
                }
            }
        }
        Ok(())
    }
}

/// Deleting a device or template. Applied after the frame's edits and renames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    Device(usize),
    Template(usize),
}

impl Removal {
    pub fn apply(&self, file: &mut ProjectFile) {
        match *self {
            Removal::Device(index) if index < file.devices.len() => {
                file.devices.remove(index);
            }
            Removal::Template(index) if index < file.templates.len() => {
                file.templates.remove(index);
            }
            _ => {}
        }
    }
}

/// A device's field override as one line of text, e.g. `{ start = 1200 }`.
pub fn override_to_text(table: &toml::Table) -> String {
    toml::Value::Table(table.clone()).to_string()
}

/// Parses an override line: empty means "no override", otherwise an inline TOML table.
pub fn override_from_text(text: &str) -> Result<Option<toml::Table>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let parsed: toml::Table =
        toml::from_str(&format!("v = {text}")).map_err(|e| e.message().to_string())?;
    match parsed.get("v") {
        Some(toml::Value::Table(table)) => Ok(Some(table.clone())),
        _ => Err("expected an inline table, e.g. { start = 1200 }".into()),
    }
}

/// Where the broker password comes from: `password` or `password_env`, at most one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordSource {
    None,
    Literal,
    Environment,
}

impl PasswordSource {
    pub const ALL: [PasswordSource; 3] = [
        PasswordSource::None,
        PasswordSource::Literal,
        PasswordSource::Environment,
    ];

    /// A literal password wins when a file sets both, so the editor shows the one that is used.
    pub fn of(broker: &BrokerSpec) -> Self {
        match (&broker.password, &broker.password_env) {
            (Some(_), _) => PasswordSource::Literal,
            (None, Some(_)) => PasswordSource::Environment,
            (None, None) => PasswordSource::None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PasswordSource::None => "none",
            PasswordSource::Literal => "in the file",
            PasswordSource::Environment => "from env var",
        }
    }
}

/// Switches the password to `source`, starting from an empty password or `MQTT_PASSWORD`;
/// keeps the broker unchanged when it already uses that source.
pub fn set_password_source(broker: &mut BrokerSpec, source: PasswordSource) {
    if PasswordSource::of(broker) != source {
        broker.password = (source == PasswordSource::Literal).then(String::new);
        broker.password_env =
            (source == PasswordSource::Environment).then(|| "MQTT_PASSWORD".to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn unique_id_appends_a_counter() {
        assert_eq!(unique_id("device", |_| false), "device");
        assert_eq!(
            unique_id("device", |id| id == "device" || id == "device-2"),
            "device-3"
        );
    }

    #[test]
    fn added_device_and_template_are_valid() {
        let mut file = ProjectFile::default();
        let d = add_device(&mut file);
        assert_eq!(file.devices[d].id, "device");
        assert_eq!(file.devices[d].template, "co2", "the first built-in");
        let t = add_template(&mut file);
        let template_id = file.templates[t].id.clone();
        assert_eq!(add_device(&mut file), 1);
        assert_eq!(file.devices[1].id, "device-2");
        assert_eq!(
            file.devices[1].template, template_id,
            "the first project template"
        );
        let validated = file.validate(Path::new("."));
        assert!(validated.errors.is_empty(), "{:?}", validated.errors);
        assert_eq!(validated.project.instances().count(), 2);
    }

    #[test]
    fn a_new_message_availability_and_noise_are_valid() {
        let mut file = ProjectFile::default();
        let t = add_template(&mut file);
        let template = &mut file.templates[t];
        template.messages.push(new_message());
        template.availability = Some(new_availability());
        template.fields.insert(
            "value".into(),
            GeneratorSpec::Uniform {
                min: 0.0,
                max: 1.0,
                noise: Some(DEFAULT_NOISE),
            },
        );
        assert_eq!(template.messages[1].topic, template.messages[0].topic);
        let id = template.id.clone();
        file.devices.push(DeviceSpec::new("d", id));
        let validated = file.validate(Path::new("."));
        assert!(validated.errors.is_empty(), "{:?}", validated.errors);
    }

    #[test]
    fn new_template_ids_avoid_builtins_and_existing_templates() {
        let mut file = ProjectFile::default();
        add_template(&mut file);
        add_template(&mut file);
        let ids: Vec<&str> = file.templates.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["template", "template-2"]);
    }

    #[test]
    fn copy_builtin_copies_under_a_new_id() {
        let mut file = ProjectFile::default();
        let i = copy_builtin(&mut file, "co2", "my-co2").unwrap();
        assert_eq!(file.templates[i].id, "my-co2");
        assert_eq!(
            file.templates[i].messages,
            builtin_templates()[0].messages,
            "content is the built-in's"
        );
        assert!(file.validate(Path::new(".")).errors.is_empty());
    }

    #[test]
    fn the_suggested_copy_id_is_not_taken() {
        let mut file = ProjectFile::default();
        assert_eq!(copy_id_suggestion(&file, "co2"), "co2-copy");
        copy_builtin(&mut file, "co2", "co2-copy").unwrap();
        assert_eq!(copy_id_suggestion(&file, "co2"), "co2-copy-2");
    }

    #[test]
    fn copy_builtin_rejects_bad_ids() {
        let mut file = ProjectFile::default();
        copy_builtin(&mut file, "co2", "mine").unwrap();
        assert_eq!(
            copy_builtin(&mut file, "co2", " "),
            Err("the name must not be empty".into())
        );
        assert_eq!(
            copy_builtin(&mut file, "co2", "humidity"),
            Err("`humidity` is a built-in template id".into())
        );
        assert_eq!(
            copy_builtin(&mut file, "co2", "mine"),
            Err("a template `mine` already exists".into())
        );
        assert_eq!(
            copy_builtin(&mut file, "nope", "x"),
            Err("unknown built-in template `nope`".into())
        );
    }

    #[test]
    fn every_default_generator_validates() {
        for kind in GeneratorKind::ALL {
            let spec = default_generator(kind);
            assert_eq!(spec.validate(), Ok(()), "{kind:?}");
            assert_eq!(GeneratorKind::of(&spec), kind);
        }
    }

    /// The labels are typed here; the names in the file come from core's serde attributes.
    #[test]
    fn generator_kind_labels_are_the_names_in_the_file() {
        for kind in GeneratorKind::ALL {
            let written = toml::Value::try_from(default_generator(kind)).unwrap();
            assert_eq!(written["generator"].as_str(), Some(kind.label()));
        }
    }

    #[test]
    fn switching_kind_resets_parameters_only_on_change() {
        let mut spec = GeneratorSpec::Uniform {
            min: 5.0,
            max: 9.0,
            noise: None,
        };
        set_generator_kind(&mut spec, GeneratorKind::Uniform);
        assert_eq!(
            spec,
            GeneratorSpec::Uniform {
                min: 5.0,
                max: 9.0,
                noise: None
            }
        );
        set_generator_kind(&mut spec, GeneratorKind::Constant);
        assert_eq!(spec, default_generator(GeneratorKind::Constant));
    }

    #[test]
    fn every_value_has_the_kind_it_was_written_as() {
        assert_eq!(ValueKind::of(&Value::String("007".into())), ValueKind::Text);
        assert_eq!(ValueKind::of(&Value::Number(7.0)), ValueKind::Number);
        assert_eq!(ValueKind::of(&Value::Bool(true)), ValueKind::Bool);
        let labels: Vec<&str> = ValueKind::ALL.iter().map(|k| k.label()).collect();
        assert_eq!(labels, ["text", "number", "bool"]);
    }

    #[test]
    fn text_is_kept_exactly_as_typed() {
        for typed in ["007", "true", " 22.5 ", ""] {
            assert_eq!(
                parse_value(ValueKind::Text, typed),
                Ok(Value::String(typed.into()))
            );
        }
    }

    #[test]
    fn numbers_must_be_finite() {
        assert_eq!(
            parse_value(ValueKind::Number, " 22.5 "),
            Ok(Value::Number(22.5))
        );
        assert_eq!(
            parse_value(ValueKind::Number, "007"),
            Ok(Value::Number(7.0))
        );
        for typed in ["x", "", "inf", "nan"] {
            assert_eq!(
                parse_value(ValueKind::Number, typed),
                Err("use a number, e.g. 42 or 22.5".into()),
                "{typed:?}"
            );
        }
    }

    #[test]
    fn bools_are_true_or_false() {
        assert_eq!(parse_value(ValueKind::Bool, "true "), Ok(Value::Bool(true)));
        assert_eq!(
            parse_value(ValueKind::Bool, "false"),
            Ok(Value::Bool(false))
        );
        for typed in ["yes", "1", ""] {
            assert_eq!(
                parse_value(ValueKind::Bool, typed),
                Err("use true or false".into()),
                "{typed:?}"
            );
        }
    }

    #[test]
    fn sequence_text_is_split_on_commas_and_each_item_read_like_a_pin() {
        assert_eq!(
            parse_sequence(" open, 3 ,, true ,"),
            vec![
                Value::String("open".into()),
                Value::Number(3.0),
                Value::Bool(true)
            ]
        );
        assert_eq!(parse_sequence(" "), Vec::<Value>::new());
    }

    #[test]
    fn sequences_round_trip_through_text() {
        let values = vec![
            Value::String("open".into()),
            Value::Number(2.5),
            Value::Bool(false),
        ];
        assert_eq!(sequence_text(&values), "open, 2.5, false");
        assert_eq!(parse_sequence(&sequence_text(&values)), values);
    }

    fn broker(password: Option<&str>, password_env: Option<&str>) -> BrokerSpec {
        BrokerSpec {
            password: password.map(Into::into),
            password_env: password_env.map(Into::into),
            ..BrokerSpec::default()
        }
    }

    #[test]
    fn the_password_source_is_read_from_the_broker() {
        assert_eq!(
            PasswordSource::of(&broker(None, None)),
            PasswordSource::None
        );
        assert_eq!(
            PasswordSource::of(&broker(Some("s"), None)),
            PasswordSource::Literal
        );
        assert_eq!(
            PasswordSource::of(&broker(None, Some("PW"))),
            PasswordSource::Environment
        );
        assert_eq!(
            PasswordSource::of(&broker(Some("s"), Some("PW"))),
            PasswordSource::Literal,
            "a literal password wins"
        );
        let labels: Vec<&str> = PasswordSource::ALL.iter().map(|s| s.label()).collect();
        assert_eq!(labels, ["none", "in the file", "from env var"]);
    }

    #[test]
    fn switching_the_password_source_sets_one_field_and_clears_the_other() {
        let mut spec = broker(Some("secret"), None);
        set_password_source(&mut spec, PasswordSource::Environment);
        assert_eq!(spec, broker(None, Some("MQTT_PASSWORD")));
        set_password_source(&mut spec, PasswordSource::Literal);
        assert_eq!(spec, broker(Some(""), None));
        set_password_source(&mut spec, PasswordSource::None);
        assert_eq!(spec, broker(None, None));
    }

    #[test]
    fn keeping_the_password_source_keeps_the_password() {
        let mut spec = broker(Some("secret"), None);
        set_password_source(&mut spec, PasswordSource::Literal);
        assert_eq!(spec, broker(Some("secret"), None));
    }

    #[test]
    fn unique_name_avoids_existing_names() {
        let existing = ["value".to_string(), "value-2".to_string()];
        assert_eq!(unique_name("value", &existing), "value-3");
        assert_eq!(unique_name("room", &existing), "room");
    }

    #[test]
    fn overrides_round_trip_through_text() {
        let table = override_from_text("{ start = 1200, step = 0.5 }")
            .unwrap()
            .unwrap();
        assert_eq!(table["start"].as_integer(), Some(1200));
        assert_eq!(
            override_from_text(&override_to_text(&table)),
            Ok(Some(table))
        );
    }

    #[test]
    fn empty_override_text_means_no_override() {
        assert_eq!(override_from_text("  "), Ok(None));
    }

    #[test]
    fn invalid_override_text_is_an_error() {
        assert!(override_from_text("{ start = }").is_err());
        assert_eq!(
            override_from_text("1200"),
            Err("expected an inline table, e.g. { start = 1200 }".into())
        );
    }

    fn project() -> ProjectFile {
        ProjectFile::parse(
            r#"
[[templates]]
id = "air"
messages = [{ topic = "a/{{ device_id }}", payload = "{{ co2 }}" }]
fields.co2 = { generator = "constant", value = 1 }

[[devices]]
id = "office"
template = "air"
params = { room = "kitchen" }
fields.co2 = { value = 2 }

[[devices]]
id = "lab"
template = "air"
"#,
        )
        .unwrap()
    }

    fn rename_template(to: &str) -> Rename {
        Rename::Template {
            index: 0,
            from: "air".into(),
            to: to.into(),
        }
    }

    #[test]
    fn renaming_a_template_updates_the_devices_that_use_it() {
        let mut file = project();
        rename_template("air2").apply(&mut file).unwrap();
        assert_eq!(file.templates[0].id, "air2");
        assert!(file.devices.iter().all(|d| d.template == "air2"));
        assert!(file.validate(Path::new(".")).errors.is_empty());
    }

    #[test]
    fn renaming_a_field_carries_the_device_overrides() {
        let mut file = project();
        Rename::Field {
            template: 0,
            from: "co2".into(),
            to: "carbon".into(),
        }
        .apply(&mut file)
        .unwrap();
        assert!(file.templates[0].fields.contains_key("carbon"));
        assert!(file.devices[0].fields.contains_key("carbon"));
        assert!(!file.devices[0].fields.contains_key("co2"));
    }

    #[test]
    fn renaming_a_param_and_a_device() {
        let mut file = project();
        Rename::Param {
            device: 0,
            from: "room".into(),
            to: "floor".into(),
        }
        .apply(&mut file)
        .unwrap();
        Rename::Device {
            index: 1,
            from: "lab".into(),
            to: "lab-2".into(),
        }
        .apply(&mut file)
        .unwrap();
        assert_eq!(
            file.devices[0].params.get("floor"),
            Some(&Value::String("kitchen".into()))
        );
        assert_eq!(file.devices[1].id, "lab-2");
    }

    #[test]
    fn renames_are_checked() {
        let file = project();
        let check = |rename: Rename| rename.check(&file).unwrap_err();
        assert_eq!(check(rename_template(" ")), "the name must not be empty");
        assert_eq!(
            check(rename_template("co2")),
            "`co2` is a built-in template id"
        );
        assert_eq!(
            check(Rename::Device {
                index: 1,
                from: "lab".into(),
                to: "office".into()
            }),
            "a device `office` already exists"
        );
        assert_eq!(
            check(Rename::Param {
                device: 0,
                from: "room".into(),
                to: "".into()
            }),
            "the name must not be empty"
        );
        assert_eq!(
            check(Rename::Device {
                index: 1,
                from: "gone".into(),
                to: "x".into()
            }),
            "it was changed or removed in the meantime"
        );
        assert_eq!(rename_template("air").check(&file), Ok(()));
    }

    #[test]
    fn a_rename_that_fails_its_check_leaves_the_file_alone() {
        let mut file = project();
        let before = file.clone();
        assert!(rename_template("co2").apply(&mut file).is_err());
        assert_eq!(file, before);
    }

    #[test]
    fn template_interval_looks_up_project_and_builtin_templates() {
        let mut file = project();
        file.templates[0].interval = Duration::from_secs(3);
        assert_eq!(
            template_interval(&file, "air"),
            Some(Duration::from_secs(3))
        );
        assert_eq!(
            template_interval(&file, "co2"),
            Some(builtin_templates()[0].interval)
        );
        assert_eq!(template_interval(&file, "gone"), None);
    }

    fn device_with(count: u32, instance_id: Option<&str>) -> DeviceSpec {
        DeviceSpec {
            count,
            instance_id: instance_id.map(Into::into),
            ..DeviceSpec::new("office", "co2")
        }
    }

    #[test]
    fn an_instance_id_equal_to_the_default_follows_the_count() {
        let mut device = device_with(1, Some("{{ id }}"));
        set_count(&mut device, 3);
        assert_eq!(device, device_with(3, Some("{{ id }}-{{ index }}")));
        set_count(&mut device, 1);
        assert_eq!(device, device_with(1, Some("{{ id }}")));
    }

    #[test]
    fn an_edited_instance_id_stays_when_the_count_changes() {
        let mut device = device_with(3, Some("{{ id }}-x{{ index }}"));
        set_count(&mut device, 1);
        assert_eq!(device, device_with(1, Some("{{ id }}-x{{ index }}")));
    }

    #[test]
    fn no_instance_id_stays_none_when_the_count_changes() {
        let mut device = device_with(1, None);
        set_count(&mut device, 3);
        assert_eq!(device, device_with(3, None));
    }

    #[test]
    fn removals_ignore_indices_out_of_range() {
        let mut file = project();
        Removal::Device(5).apply(&mut file);
        Removal::Template(1).apply(&mut file);
        assert_eq!(file, project());
        Removal::Device(0).apply(&mut file);
        assert_eq!(file.devices[0].id, "lab");
    }
}
