//! Edits the GUI applies to the project model. Plain functions, no egui.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use senzor_core::generator::GeneratorSpec;
use senzor_core::project::builtin::builtin_templates;
use senzor_core::project::edit::template_id_taken;
pub use senzor_core::project::edit::{
    Removal, Rename, copy_builtin, copy_id_suggestion, find_template, unique_id,
};
use senzor_core::project::file::{DEFAULT_INTERVAL, default_instance_id_pattern};
use senzor_core::project::{
    AvailabilitySpec, BrokerSpec, DeviceSpec, FieldSpec, MessageSpec, ProjectFile,
    SubscriptionSpec, TemplateSpec,
};
use senzor_core::script::Script;
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
        home_assistant: false,
        availability: None,
        messages: vec![MessageSpec::new(
            NEW_MESSAGE_TOPIC,
            r#"{"value": {{ value | round(1) }}}"#,
        )],
        fields: BTreeMap::from([(
            "value".to_string(),
            default_generator(GeneratorKind::RandomWalk).into(),
        )]),
        subscriptions: Vec::new(),
    });
    file.templates.len() - 1
}

/// A message to add to a template: field `value` on the topic new templates use.
pub fn new_message() -> MessageSpec {
    MessageSpec::new(NEW_MESSAGE_TOPIC, "{{ value }}")
}

/// The availability topic and payloads a template starts with when availability is turned on.
/// A subscription to add to a template: a command topic per instance, whose script keeps the
/// last command in `state`.
pub fn new_subscription() -> SubscriptionSpec {
    SubscriptionSpec {
        topic: "senzor/{{ device_id }}/set".into(),
        qos: 0,
        script: Script::new("state.last_command = payload;\n"),
    }
}

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
    Script,
}

impl GeneratorKind {
    pub const ALL: [GeneratorKind; 6] = [
        GeneratorKind::Constant,
        GeneratorKind::RandomWalk,
        GeneratorKind::Sine,
        GeneratorKind::Uniform,
        GeneratorKind::Sequence,
        GeneratorKind::Script,
    ];

    pub fn of(spec: &GeneratorSpec) -> Self {
        match spec {
            GeneratorSpec::Constant { .. } => GeneratorKind::Constant,
            GeneratorSpec::RandomWalk { .. } => GeneratorKind::RandomWalk,
            GeneratorSpec::Sine { .. } => GeneratorKind::Sine,
            GeneratorSpec::Uniform { .. } => GeneratorKind::Uniform,
            GeneratorSpec::Sequence { .. } => GeneratorKind::Sequence,
            GeneratorSpec::Script { .. } => GeneratorKind::Script,
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
            GeneratorKind::Script => "script",
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
        GeneratorKind::Script => GeneratorSpec::Script {
            start: Value::Number(0.0),
            noise: None,
            script: Script::new("value + 1"),
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

impl GeneratorKind {
    /// The kind named `generator = "…"` in the project file.
    pub fn from_label(label: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.label() == label)
    }

    /// The parameters of a generator of this kind, as the project file names them.
    pub fn parameters(self) -> &'static [&'static str] {
        match self {
            GeneratorKind::Constant => &["value", "noise"],
            GeneratorKind::RandomWalk => &["start", "min", "max", "step", "noise"],
            GeneratorKind::Sine => &["center", "amplitude", "period", "phase", "noise"],
            GeneratorKind::Uniform => &["min", "max", "noise"],
            GeneratorKind::Sequence => &["values", "loop", "noise"],
            GeneratorKind::Script => &["start", "noise", "script"],
        }
    }
}

/// Parameters a generator can do without (they have a default or are off when missing).
const OPTIONAL_PARAMETERS: [&str; 3] = ["noise", "phase", "loop"];

/// Whether `key` is a parameter of some generator kind.
fn is_parameter(key: &str) -> bool {
    GeneratorKind::ALL
        .iter()
        .any(|kind| kind.parameters().contains(&key))
}

/// A field as one table, the way the project file writes it.
fn field_table(field: &FieldSpec) -> toml::Table {
    toml::Table::try_from(field).expect("a field serializes to a table")
}

/// A device's override of one template field, as the device editor shows it: one setting per
/// row, each either overridden (its key is in the table) or showing what the field uses without
/// it. The table is merged onto the template's field as validation merges it; with `generator`,
/// it replaces the template's generator and parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldOverride {
    /// The generator kind in effect.
    pub kind: GeneratorKind,
    /// The table sets `generator`, so it replaces the template's generator.
    pub replaces_generator: bool,
    /// The field with every readable setting of the table applied.
    pub effective: FieldSpec,
    /// Settings whose value in the table cannot be read, e.g. `start = "high"`; the effective
    /// field has what the setting would be without them.
    pub unreadable: BTreeSet<String>,
    /// Keys of the table that are not settings of [`Self::kind`].
    pub unknown: Vec<String>,
}

impl FieldOverride {
    pub fn new(template: &FieldSpec, table: &toml::Table) -> Self {
        let template_kind = GeneratorKind::of(&template.generator);
        let mut unreadable = BTreeSet::new();
        let (kind, generator) = match table.get("generator") {
            None => (template_kind, template.generator.clone()),
            Some(named) => match named.as_str().and_then(GeneratorKind::from_label) {
                // What a replaced generator uses for the parameters the table leaves out.
                Some(kind) => (kind, default_generator(kind)),
                None => {
                    unreadable.insert("generator".to_string());
                    (template_kind, template.generator.clone())
                }
            },
        };
        let mut merged = field_table(&FieldSpec {
            generator,
            ..template.clone()
        });
        let mut unknown = Vec::new();
        for (key, value) in table.iter().filter(|(key, _)| *key != "generator") {
            let mut candidate = merged.clone();
            candidate.insert(key.clone(), value.clone());
            if toml::Value::Table(candidate.clone())
                .try_into::<FieldSpec>()
                .is_ok()
            {
                merged = candidate;
            } else if override_settings(kind).contains(&key.as_str()) {
                unreadable.insert(key.clone());
            } else {
                unknown.push(key.clone());
            }
        }
        let effective = toml::Value::Table(merged)
            .try_into()
            .expect("only keys that keep the field readable were added");
        Self {
            kind,
            replaces_generator: table.contains_key("generator"),
            effective,
            unreadable,
            unknown,
        }
    }

    /// Whether the row of `setting` must stay overridden: a replaced generator needs its
    /// required parameters.
    pub fn is_required(&self, setting: &str) -> bool {
        // A script starts at 0 without `start`; a random walk needs it.
        let optional = OPTIONAL_PARAMETERS.contains(&setting)
            || (self.kind == GeneratorKind::Script && setting == "start");
        self.replaces_generator && self.kind.parameters().contains(&setting) && !optional
    }
}

/// The settings a device can override on a field whose generator is `kind`, in the order the
/// device editor shows them.
pub fn override_settings(kind: GeneratorKind) -> Vec<&'static str> {
    std::iter::once("generator")
        .chain(kind.parameters().iter().copied())
        .chain(["unit", "name"])
        .collect()
}

/// Starts overriding `setting` of field `field` with the value it has now, so the field does not
/// change until the value is edited. A noise the field does not have starts at
/// [`DEFAULT_NOISE`], a missing unit empty and a missing name as the field's key. Overriding
/// the generator also writes its parameters, which the generator then replaces.
pub fn override_setting(table: &mut toml::Table, view: &FieldOverride, field: &str, setting: &str) {
    let current = field_table(&view.effective);
    if setting == "generator" {
        for (key, value) in current {
            if key == "generator" || view.kind.parameters().contains(&key.as_str()) {
                table.entry(key).or_insert(value);
            }
        }
        return;
    }
    let value = current
        .get(setting)
        .cloned()
        .or_else(|| unwritten_default(setting))
        .unwrap_or_else(|| match setting {
            "noise" => toml::Value::Float(DEFAULT_NOISE),
            "name" => toml::Value::String(field.to_string()),
            _ => toml::Value::String(String::new()),
        });
    table.insert(setting.to_string(), value);
}

/// The value of a parameter the file leaves out when it has it: a sine's `phase` of zero.
fn unwritten_default(key: &str) -> Option<toml::Value> {
    (key == "phase").then(|| toml::Value::String("0s".to_string()))
}

/// Stops overriding `setting`. For the generator, that also drops the parameters the template's
/// generator does not have, and those the table sets to the template's own value.
pub fn clear_setting(table: &mut toml::Table, template: &FieldSpec, setting: &str) {
    table.remove(setting);
    if setting == "generator" {
        let own = GeneratorKind::of(&template.generator).parameters();
        let template = field_table(template);
        table.retain(|key, value| {
            !is_parameter(key) || (own.contains(&key) && template.get(key) != Some(value))
        });
    }
}

/// Replaces the overridden generator with a valid one of `kind`; unit and name stay.
pub fn set_override_kind(table: &mut toml::Table, kind: GeneratorKind) {
    table.retain(|key, _| key != "generator" && !is_parameter(key));
    let generator = toml::Table::try_from(default_generator(kind)).expect("a generator serializes");
    table.extend(generator);
}

/// Writes the settings the editor changed from `before` to `after` into the table.
pub fn store_override_edits(table: &mut toml::Table, before: &FieldSpec, after: &FieldSpec) {
    let before = field_table(before);
    let after = field_table(after);
    for (key, value) in &after {
        if key != "generator" && before.get(key) != Some(value) {
            table.insert(key.clone(), value.clone());
        }
    }
    // A value edited back to one the file leaves out, such as a zero phase, must still be
    // written, or the template's value would show through.
    for key in before.keys().filter(|key| !after.contains_key(*key)) {
        if let Some(value) = unwritten_default(key) {
            table.insert(key.clone(), value);
        }
    }
}

/// What a field's override changes, e.g. `start, max overridden`.
pub fn override_summary(table: &toml::Table, kind: GeneratorKind) -> String {
    if table.is_empty() {
        return "not overridden".to_string();
    }
    let settings = override_settings(kind);
    let mut keys: Vec<&str> = settings
        .iter()
        .copied()
        .filter(|s| table.contains_key(*s))
        .collect();
    keys.extend(
        table
            .keys()
            .map(String::as_str)
            .filter(|k| !settings.contains(k)),
    );
    format!("{} overridden", keys.join(", "))
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

    fn co2_field() -> FieldSpec {
        builtin_templates()[0].fields["co2"].clone()
    }

    fn table(src: &str) -> toml::Table {
        toml::from_str(src).unwrap()
    }

    #[test]
    fn every_kind_is_found_by_its_label_and_lists_the_parameters_it_writes() {
        for kind in GeneratorKind::ALL {
            assert_eq!(GeneratorKind::from_label(kind.label()), Some(kind));
            // Every kind has noise, and a sine a phase, which are written only when set.
            let written = toml::Table::try_from(default_generator(kind)).unwrap();
            let optional = match kind {
                GeneratorKind::Sine => &["noise", "phase"][..],
                GeneratorKind::Script => &["noise", "start"][..],
                _ => &["noise"][..],
            };
            let keys: BTreeSet<&str> = written
                .keys()
                .map(String::as_str)
                .filter(|k| *k != "generator")
                .chain(optional.iter().copied())
                .collect();
            let parameters: BTreeSet<&str> = kind.parameters().iter().copied().collect();
            assert_eq!(keys, parameters, "{kind:?}");
        }
        assert_eq!(GeneratorKind::from_label("square"), None);
    }

    #[test]
    fn an_override_is_merged_onto_the_template_field() {
        let view = FieldOverride::new(&co2_field(), &table("start = 1200\nunit = \"ppmv\""));
        assert_eq!(view.kind, GeneratorKind::RandomWalk);
        assert!(!view.replaces_generator);
        assert_eq!(
            view.effective.generator,
            GeneratorSpec::RandomWalk {
                start: 1200.0,
                min: 400.0,
                max: 2000.0,
                step: 15.0,
                noise: None
            }
        );
        assert_eq!(view.effective.unit.as_deref(), Some("ppmv"));
        assert_eq!(view.effective.name.as_deref(), Some("CO2"));
    }

    #[test]
    fn a_replaced_generator_starts_from_the_kinds_defaults_and_keeps_the_labels() {
        let view = FieldOverride::new(&co2_field(), &table("generator = \"uniform\"\nmax = 9"));
        assert_eq!(view.kind, GeneratorKind::Uniform);
        assert!(view.replaces_generator);
        assert_eq!(
            view.effective.generator,
            GeneratorSpec::Uniform {
                min: 0.0,
                max: 9.0,
                noise: None
            }
        );
        assert_eq!(view.effective.unit.as_deref(), Some("ppm"));
        assert!(view.is_required("min"));
        assert!(!view.is_required("noise"));
        assert!(!view.is_required("unit"));
    }

    #[test]
    fn values_that_cannot_be_read_and_unknown_keys_are_set_apart() {
        let view = FieldOverride::new(
            &co2_field(),
            &table("start = \"high\"\nstrat = 5\ncenter = 1\nmax = 2500"),
        );
        assert_eq!(view.unreadable, BTreeSet::from(["start".to_string()]));
        assert_eq!(view.unknown, ["center", "strat"]);
        assert_eq!(
            view.effective.generator,
            GeneratorSpec::RandomWalk {
                start: 600.0,
                min: 400.0,
                max: 2500.0,
                step: 15.0,
                noise: None
            }
        );
        let view = FieldOverride::new(&co2_field(), &table("generator = \"square\""));
        assert_eq!(view.kind, GeneratorKind::RandomWalk);
        assert_eq!(view.unreadable, BTreeSet::from(["generator".to_string()]));
    }

    /// A sine field whose phase is `phase` (zero is not written to the file).
    fn sine_field(phase: Duration) -> FieldSpec {
        GeneratorSpec::Sine {
            center: 20.0,
            amplitude: 2.0,
            period: Duration::from_secs(3600),
            phase,
            noise: None,
        }
        .into()
    }

    #[test]
    fn overriding_a_phase_that_is_not_written_starts_at_zero() {
        let template = sine_field(Duration::ZERO);
        let view = FieldOverride::new(&template, &toml::Table::new());
        let mut overrides = toml::Table::new();
        override_setting(&mut overrides, &view, "t", "phase");
        assert_eq!(overrides, table("phase = \"0s\""));
    }

    #[test]
    fn a_phase_changed_back_to_zero_is_stored() {
        let template = sine_field(Duration::from_secs(900));
        let mut overrides = table("phase = \"15m\"");
        let before = FieldOverride::new(&template, &overrides).effective;
        store_override_edits(&mut overrides, &before, &sine_field(Duration::ZERO));
        assert_eq!(overrides, table("phase = \"0s\""));
        let view = FieldOverride::new(&template, &overrides);
        assert_eq!(view.effective, sine_field(Duration::ZERO));
    }

    #[test]
    fn overriding_a_setting_writes_its_current_value() {
        let field = co2_field();
        let mut overrides = table("max = 2500");
        let view = FieldOverride::new(&field, &overrides);
        for setting in ["start", "max", "noise", "unit"] {
            override_setting(&mut overrides, &view, "co2", setting);
        }
        assert_eq!(
            overrides,
            table("start = 600.0\nmax = 2500.0\nnoise = 0.1\nunit = \"ppm\"")
        );
        let unnamed = FieldSpec::new(default_generator(GeneratorKind::Uniform));
        let mut overrides = toml::Table::new();
        let view = FieldOverride::new(&unnamed, &overrides);
        override_setting(&mut overrides, &view, "v", "name");
        override_setting(&mut overrides, &view, "v", "unit");
        assert_eq!(overrides, table("name = \"v\"\nunit = \"\""));
    }

    #[test]
    fn overriding_the_generator_keeps_the_field_as_it_is() {
        let field = co2_field();
        let mut overrides = table("start = 1200\nunit = \"ppmv\"");
        let view = FieldOverride::new(&field, &overrides);
        override_setting(&mut overrides, &view, "co2", "generator");
        assert_eq!(
            overrides,
            table(
                "generator = \"random_walk\"\nstart = 1200\nmin = 400.0\nmax = 2000.0\n\
                 step = 15.0\nunit = \"ppmv\""
            )
        );
        assert_eq!(
            FieldOverride::new(&field, &overrides).effective,
            view.effective
        );
    }

    #[test]
    fn clearing_the_generator_drops_parameters_the_template_does_not_need() {
        let field = co2_field();
        let mut overrides = table(
            "generator = \"sine\"\ncenter = 1\nperiod = \"1m\"\nnoise = 0.5\nmin = 400.0\n\
             step = 2\nunit = \"ppb\"",
        );
        clear_setting(&mut overrides, &field, "generator");
        assert_eq!(overrides, table("noise = 0.5\nstep = 2\nunit = \"ppb\""));
        clear_setting(&mut overrides, &field, "step");
        assert_eq!(overrides, table("noise = 0.5\nunit = \"ppb\""));
    }

    #[test]
    fn choosing_a_kind_writes_a_complete_generator_of_it() {
        let mut overrides = table("generator = \"random_walk\"\nstart = 1\nunit = \"ppb\"");
        set_override_kind(&mut overrides, GeneratorKind::Sequence);
        assert_eq!(
            overrides,
            table(
                "generator = \"sequence\"\nvalues = [\"on\", \"off\"]\nloop = true\nunit = \"ppb\""
            )
        );
        let view = FieldOverride::new(&co2_field(), &overrides);
        assert!(view.effective.problems().is_empty());
        assert!(view.unreadable.is_empty() && view.unknown.is_empty());
    }

    #[test]
    fn only_edited_settings_are_stored() {
        let before = co2_field();
        let mut after = before.clone();
        if let GeneratorSpec::RandomWalk { step, .. } = &mut after.generator {
            *step = 3.0;
        }
        after.name = Some("Carbon dioxide".into());
        let mut overrides = table("start = 1");
        store_override_edits(&mut overrides, &before, &after);
        assert_eq!(
            overrides,
            table("start = 1\nstep = 3.0\nname = \"Carbon dioxide\"")
        );
    }

    #[test]
    fn the_summary_lists_overridden_settings_in_the_editors_order() {
        let kind = GeneratorKind::RandomWalk;
        assert_eq!(
            override_summary(&toml::Table::new(), kind),
            "not overridden"
        );
        assert_eq!(
            override_summary(&table("unit = \"x\"\nmax = 1\nstart = 2\nzzz = 1"), kind),
            "start, max, unit, zzz overridden"
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
