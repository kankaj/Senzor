//! Edits the GUI applies to the project model. Plain functions, no egui.

use std::collections::BTreeMap;
use std::time::Duration;

use senzor_core::generator::GeneratorSpec;
use senzor_core::project::builtin::builtin_templates;
use senzor_core::project::edit::template_id_taken;
pub use senzor_core::project::edit::{
    Removal, Rename, copy_builtin, copy_id_suggestion, find_template, unique_id,
};
use senzor_core::project::file::{DEFAULT_INTERVAL, default_instance_id_pattern};
use senzor_core::project::{
    AvailabilitySpec, BrokerSpec, DeviceSpec, MessageSpec, ProjectFile, TemplateSpec,
};
use senzor_core::value::Value;

/// The topic new messages publish on.
const NEW_MESSAGE_TOPIC: &str = "senzor/{{ device_id }}/value";

/// The noise a generator starts with when noise is turned on.
pub const DEFAULT_NOISE: f64 = 0.1;

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
        messages: vec![MessageSpec::new(
            NEW_MESSAGE_TOPIC,
            r#"{"value": {{ value | round(1) }}}"#,
        )],
        fields: BTreeMap::from([(
            "value".to_string(),
            default_generator(GeneratorKind::RandomWalk).into(),
        )]),
    });
    file.templates.len() - 1
}

/// A message to add to a template: field `value` on the topic new templates use.
pub fn new_message() -> MessageSpec {
    MessageSpec::new(NEW_MESSAGE_TOPIC, "{{ value }}")
}

/// The availability topic and payloads a template starts with when availability is turned on.
pub fn new_availability() -> AvailabilitySpec {
    AvailabilitySpec {
        topic: "senzor/{{ device_id }}/status".into(),
        online: "online".into(),
        offline: "offline".into(),
        will_delay: None,
    }
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
            }
            .into(),
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
}
