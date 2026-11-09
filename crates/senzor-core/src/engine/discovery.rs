//! Home Assistant MQTT Discovery: the retained config messages that make an instance appear in
//! Home Assistant as a device with one entity per field.
//!
//! Where a field sits in a payload is found by rendering each message twice, with only that
//! field's value changed: the one spot that changes is where Home Assistant reads it from.

use std::collections::BTreeMap;
use std::time::SystemTime;

use serde_json::{Map, Value as Json, json};

use crate::project::InstanceConfig;
use crate::render::{Context, RenderedMessage, base_context, tick_context};
use crate::value::Value;

/// The topic prefix Home Assistant subscribes to for discovery, its default.
pub const DISCOVERY_PREFIX: &str = "homeassistant";

/// One retained discovery message.
#[derive(Debug, Clone, PartialEq)]
pub struct Announcement {
    pub topic: String,
    pub payload: String,
}

/// The discovery messages of `config`, one per field Senzor finds in exactly one place of one
/// message; none when its template does not turn `home_assistant` on.
pub fn announcements(config: &InstanceConfig, base: &Context) -> Vec<Announcement> {
    if !config.home_assistant {
        return Vec::new();
    }
    let node = object_id(&config.id);
    let common = common_config(config, base, &node);
    config
        .fields
        .keys()
        .filter_map(|field| {
            let kind = Kind::of(&config.fields[field].generator.initial_value());
            let found = locate(config, base, field, kind)?;
            Some(announcement(config, &common, &node, field, kind, found))
        })
        .collect()
}

/// The fields of `config` that are not announced, because Senzor does not find them in exactly
/// one place of one message, whether or not its template turns `home_assistant` on.
pub fn unannounced_fields(config: &InstanceConfig) -> Vec<String> {
    let base = base_context(
        &config.id,
        config.index,
        &config.template_id,
        &config.params,
    );
    config
        .fields
        .iter()
        .filter(|(field, spec)| {
            let kind = Kind::of(&spec.generator.initial_value());
            locate(config, &base, field, kind).is_none()
        })
        .map(|(field, _)| field.clone())
        .collect()
}

/// The discovery topics `config` may have left retained, for clearing them.
pub fn topics(config: &InstanceConfig) -> Vec<String> {
    if !config.home_assistant {
        return Vec::new();
    }
    let node = object_id(&config.id);
    config
        .fields
        .iter()
        .map(|(field, spec)| {
            let kind = Kind::of(&spec.generator.initial_value());
            topic(kind, &node, field)
        })
        .collect()
}

/// What Home Assistant makes of a field.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Number,
    Text,
    /// A `binary_sensor`: on or off.
    Switch,
}

impl Kind {
    fn of(value: &Value) -> Self {
        match value {
            Value::Number(_) => Kind::Number,
            Value::String(_) => Kind::Text,
            Value::Bool(_) => Kind::Switch,
        }
    }

    /// Two values for a field of this kind that render differently.
    fn probes(self) -> (Value, Value) {
        match self {
            Kind::Number => (Value::Number(1234.5678), Value::Number(8765.4321)),
            Kind::Text => (
                Value::String("senzor-probe-a".into()),
                Value::String("senzor-probe-b".into()),
            ),
            Kind::Switch => (Value::Bool(true), Value::Bool(false)),
        }
    }

    fn component(self) -> &'static str {
        match self {
            Kind::Switch => "binary_sensor",
            Kind::Number | Kind::Text => "sensor",
        }
    }
}

/// One step into a JSON payload.
#[derive(Debug, Clone, PartialEq)]
enum Step {
    Key(String),
    Index(usize),
}

/// Where a field is in a message.
#[derive(Debug, Clone, PartialEq)]
struct Found {
    state_topic: String,
    /// The path into the JSON payload, or `None` when the payload is the value alone.
    path: Option<Vec<Step>>,
    /// What the payload holds there when the field is the first probe value (`true` for a
    /// switch), as JSON.
    first: Json,
}

/// The first message that holds `field` in exactly one place, and where.
fn locate(config: &InstanceConfig, base: &Context, field: &str, kind: Kind) -> Option<Found> {
    let mut first = BTreeMap::new();
    for (name, spec) in &config.fields {
        first.insert(
            name.clone(),
            Kind::of(&spec.generator.initial_value()).probes().0,
        );
    }
    let mut second = first.clone();
    second.insert(field.to_string(), kind.probes().1);
    let render = |values: &BTreeMap<String, Value>| {
        let ctx = tick_context(base, values, SystemTime::UNIX_EPOCH, 0);
        config.templates.render_messages(&ctx)
    };
    render(&first)
        .into_iter()
        .zip(render(&second))
        .find_map(|pair| match pair {
            (Ok(a), Ok(b)) => place(&a, &b),
            _ => None,
        })
}

/// Where `a` and `b`, the same message with one field changed, differ, when that is one spot
/// of a JSON payload or the whole of a payload that is a single value.
fn place(a: &RenderedMessage, b: &RenderedMessage) -> Option<Found> {
    if a.topic != b.topic || a.payload == b.payload {
        return None;
    }
    // Home Assistant reads values out of text, not out of encoded bytes.
    let (a_text, b_text) = (a.payload.as_text()?, b.payload.as_text()?);
    let found = |path, first| Found {
        state_topic: a.topic.clone(),
        path,
        first,
    };
    match (
        serde_json::from_str::<Json>(a_text),
        serde_json::from_str::<Json>(b_text),
    ) {
        (Ok(ja), Ok(jb)) if ja.is_object() || ja.is_array() => {
            let mut changed = Vec::new();
            differences(&ja, &jb, &mut Vec::new(), &mut changed).then_some(())?;
            let [path] = <[Vec<Step>; 1]>::try_from(changed).ok()?;
            let first = lookup(&ja, &path)?.clone();
            Some(found(Some(path), first))
        }
        // A quoted string: Home Assistant reads it through `value_json`.
        (Ok(Json::String(text)), Ok(_)) => Some(found(Some(Vec::new()), Json::String(text))),
        _ => single_token(a_text)
            .zip(single_token(b_text))
            .map(|(text, _)| found(None, Json::String(text.to_string()))),
    }
}

/// `payload` trimmed, when it is one value and nothing else.
fn single_token(payload: &str) -> Option<&str> {
    let text = payload.trim();
    let separator = |c: char| c.is_whitespace() || "{}[],;=&\"".contains(c);
    (!text.is_empty() && !text.contains(separator)).then_some(text)
}

/// Collects into `changed` the paths of the values that differ between `a` and `b`; false when
/// their shapes differ, so no single path can be named.
fn differences(a: &Json, b: &Json, path: &mut Vec<Step>, changed: &mut Vec<Vec<Step>>) -> bool {
    match (a, b) {
        (Json::Object(a), Json::Object(b)) => {
            a.len() == b.len()
                && a.iter().all(|(key, value)| {
                    let Some(other) = b.get(key) else {
                        return false;
                    };
                    path.push(Step::Key(key.clone()));
                    let same_shape = differences(value, other, path, changed);
                    path.pop();
                    same_shape
                })
        }
        (Json::Array(a), Json::Array(b)) => {
            a.len() == b.len()
                && a.iter().zip(b).enumerate().all(|(index, (value, other))| {
                    path.push(Step::Index(index));
                    let same_shape = differences(value, other, path, changed);
                    path.pop();
                    same_shape
                })
        }
        (Json::Object(_) | Json::Array(_), _) | (_, Json::Object(_) | Json::Array(_)) => false,
        _ => {
            if a != b {
                changed.push(path.clone());
            }
            true
        }
    }
}

fn lookup<'a>(json: &'a Json, path: &[Step]) -> Option<&'a Json> {
    path.iter().try_fold(json, |json, step| match step {
        Step::Key(key) => json.get(key),
        Step::Index(index) => json.get(index),
    })
}

/// Python dict methods: `value_json.items` would name the method, not the key.
const DICT_METHODS: &[&str] = &[
    "clear",
    "copy",
    "fromkeys",
    "get",
    "items",
    "keys",
    "pop",
    "popitem",
    "setdefault",
    "update",
    "values",
];

/// The Jinja expression Home Assistant reads the value with.
fn expression(path: Option<&[Step]>) -> String {
    let Some(path) = path else {
        return "value".to_string();
    };
    let mut expression = String::from("value_json");
    for step in path {
        match step {
            Step::Key(key) if is_name(key) && !DICT_METHODS.contains(&key.as_str()) => {
                expression.push('.');
                expression.push_str(key);
            }
            Step::Key(key) => expression.push_str(&format!("[{}]", Json::String(key.clone()))),
            Step::Index(index) => expression.push_str(&format!("[{index}]")),
        }
    }
    expression
}

fn is_name(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The parts of every entity's config that come from the instance: its device, the origin and
/// its availability.
fn common_config(config: &InstanceConfig, base: &Context, node: &str) -> Map<String, Json> {
    let mut common = Map::new();
    common.insert(
        "device".into(),
        json!({
            "identifiers": [format!("senzor_{node}")],
            "name": config.id,
            "model": config.template_id,
            "manufacturer": "Senzor",
        }),
    );
    common.insert(
        "origin".into(),
        json!({
            "name": "Senzor",
            "sw_version": env!("CARGO_PKG_VERSION"),
            "support_url": "https://senzor.kankaj.cz",
        }),
    );
    let availability = config.templates.availability_topic(base).ok().flatten();
    if let Some((topic, spec)) = availability.zip(config.availability.as_ref()) {
        common.insert("availability_topic".into(), topic.into());
        common.insert("payload_available".into(), spec.online.clone().into());
        common.insert("payload_not_available".into(), spec.offline.clone().into());
    }
    common
}

fn announcement(
    config: &InstanceConfig,
    common: &Map<String, Json>,
    node: &str,
    field: &str,
    kind: Kind,
    found: Found,
) -> Announcement {
    let spec = &config.fields[field];
    let expression = expression(found.path.as_deref());
    let value_template = match kind {
        Kind::Switch => {
            // A payload that is the value alone is text; one inside JSON is compared as JSON.
            format!(
                "{{{{ 'ON' if {expression} == {} else 'OFF' }}}}",
                found.first
            )
        }
        Kind::Number | Kind::Text => format!("{{{{ {expression} }}}}"),
    };
    let mut entity = common.clone();
    entity.insert(
        "name".into(),
        spec.name
            .clone()
            .unwrap_or_else(|| field.to_string())
            .into(),
    );
    entity.insert(
        "unique_id".into(),
        format!("senzor_{node}_{}", object_id(field)).into(),
    );
    entity.insert("state_topic".into(), found.state_topic.into());
    entity.insert("value_template".into(), value_template.into());
    if let Some(unit) = &spec.unit {
        entity.insert("unit_of_measurement".into(), unit.clone().into());
    }
    if kind == Kind::Number {
        entity.insert("state_class".into(), "measurement".into());
    }
    Announcement {
        topic: topic(kind, node, field),
        payload: Json::Object(entity).to_string(),
    }
}

fn topic(kind: Kind, node: &str, field: &str) -> String {
    format!(
        "{DISCOVERY_PREFIX}/{}/{node}/{}/config",
        kind.component(),
        object_id(field)
    )
}

/// `id` with every character Home Assistant does not allow in a discovery ID as `_`.
fn object_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::{Value as Json, json};

    use super::*;
    use crate::engine::test_support::valid_project;
    use crate::render::base_context;

    /// The announcements of the first instance of the project's first device.
    fn announced(src: &str) -> Vec<Announcement> {
        let project = valid_project(src);
        let config = &project.devices[0].instances[0];
        let base = base_context(
            &config.id,
            config.index,
            &config.template_id,
            &config.params,
        );
        announcements(config, &base)
    }

    /// The announcement on `topic`, its payload parsed.
    fn config_on(announcements: &[Announcement], topic: &str) -> Json {
        let found = announcements
            .iter()
            .find(|a| a.topic == topic)
            .unwrap_or_else(|| panic!("no announcement on {topic}: {announcements:#?}"));
        serde_json::from_str(&found.payload).unwrap()
    }

    const OFFICE: &str = r#"
[[templates]]
id = "office-air"
home_assistant = true
availability = { topic = "b/{{ device_id }}/status", online = "up", offline = "down" }
[[templates.messages]]
topic = "b/{{ device_id }}/state"
payload = '{"co2": {{ co2 | round(1) }}, "door": {"open": {{ open }}}, "readings": [{{ temp }}]}'
[templates.fields.co2]
generator = "random_walk"
start = 600
min = 400
max = 2000
step = 25
unit = "ppm"
name = "CO2"
[templates.fields.open]
generator = "constant"
value = false
[templates.fields.temp]
generator = "constant"
value = 21.5

[[devices]]
id = "office"
template = "office-air"
count = 2
"#;

    #[test]
    fn a_number_field_is_a_sensor_read_from_its_json_key() {
        let announcements = announced(OFFICE);
        let co2 = config_on(&announcements, "homeassistant/sensor/office-0/co2/config");
        assert_eq!(co2["name"], "CO2");
        assert_eq!(co2["unique_id"], "senzor_office-0_co2");
        assert_eq!(co2["state_topic"], "b/office-0/state");
        assert_eq!(co2["value_template"], "{{ value_json.co2 }}");
        assert_eq!(co2["unit_of_measurement"], "ppm");
        assert_eq!(co2["state_class"], "measurement");
    }

    #[test]
    fn every_entity_belongs_to_the_instance_as_a_device() {
        let announcements = announced(OFFICE);
        assert_eq!(announcements.len(), 3, "{announcements:#?}");
        let device = json!({
            "identifiers": ["senzor_office-0"],
            "name": "office-0",
            "model": "office-air",
            "manufacturer": "Senzor",
        });
        for a in &announcements {
            let config: Json = serde_json::from_str(&a.payload).unwrap();
            assert_eq!(config["device"], device, "{}", a.topic);
            assert_eq!(config["origin"]["name"], "Senzor");
        }
    }

    #[test]
    fn availability_follows_the_templates_status_topic() {
        let co2 = config_on(
            &announced(OFFICE),
            "homeassistant/sensor/office-0/co2/config",
        );
        assert_eq!(co2["availability_topic"], "b/office-0/status");
        assert_eq!(co2["payload_available"], "up");
        assert_eq!(co2["payload_not_available"], "down");
    }

    #[test]
    fn a_boolean_field_is_a_binary_sensor() {
        let open = config_on(
            &announced(OFFICE),
            "homeassistant/binary_sensor/office-0/open/config",
        );
        assert_eq!(
            open["name"], "open",
            "a field without a name is named by its key"
        );
        assert_eq!(
            open["value_template"],
            "{{ 'ON' if value_json.door.open == true else 'OFF' }}"
        );
        assert!(open.get("state_class").is_none());
        assert!(open.get("unit_of_measurement").is_none());
    }

    #[test]
    fn values_inside_arrays_are_read_by_index() {
        let temp = config_on(
            &announced(OFFICE),
            "homeassistant/sensor/office-0/temp/config",
        );
        assert_eq!(temp["value_template"], "{{ value_json.readings[0] }}");
    }

    #[test]
    fn a_payload_that_is_only_the_value_is_read_whole() {
        let announcements = announced(
            r#"
[[templates]]
id = "plain"
home_assistant = true
[[templates.messages]]
topic = "t/{{ device_id }}/co2"
payload = "{{ co2 }}"
[[templates.messages]]
topic = "t/{{ device_id }}/mode"
payload = "{{ mode }}"
[templates.fields.co2]
generator = "constant"
value = 600
[templates.fields.mode]
generator = "sequence"
values = ["heat", "cool"]

[[devices]]
id = "d"
template = "plain"
"#,
        );
        let co2 = config_on(&announcements, "homeassistant/sensor/d/co2/config");
        assert_eq!(co2["state_topic"], "t/d/co2");
        assert_eq!(co2["value_template"], "{{ value }}");
        let mode = config_on(&announcements, "homeassistant/sensor/d/mode/config");
        assert_eq!(mode["state_topic"], "t/d/mode");
        assert_eq!(mode["value_template"], "{{ value }}");
        assert!(
            mode.get("state_class").is_none(),
            "text is not a measurement"
        );
        assert!(co2.get("availability_topic").is_none(), "no availability");
    }

    #[test]
    fn keys_that_are_not_names_are_quoted() {
        let announcements = announced(
            r#"
[[templates]]
id = "odd"
home_assistant = true
[[templates.messages]]
topic = "t"
payload = '{"CO2 level": {{ co2 }}}'
[templates.fields.co2]
generator = "constant"
value = 600

[[devices]]
id = "d"
template = "odd"
"#,
        );
        let co2 = config_on(&announcements, "homeassistant/sensor/d/co2/config");
        assert_eq!(co2["value_template"], r#"{{ value_json["CO2 level"] }}"#);
    }

    #[test]
    fn a_field_that_is_not_in_one_place_of_a_payload_is_not_announced() {
        let announcements = announced(
            r#"
[[templates]]
id = "odd"
home_assistant = true
[[templates.messages]]
topic = "t/{{ room }}"
payload = 'co2={{ co2 }};twice={{ twice }},{{ twice }}'
[templates.fields.co2]
generator = "constant"
value = 600
[templates.fields.room]
generator = "constant"
value = "a"
[templates.fields.twice]
generator = "constant"
value = 1
[templates.fields.unused]
generator = "constant"
value = 1

[[devices]]
id = "d"
template = "odd"
"#,
        );
        assert_eq!(announcements, [], "not JSON, in the topic, twice, unused");
    }

    /// Home Assistant reads values out of text; the JSON of a CBOR message is not what it gets.
    #[test]
    fn a_field_in_a_binary_payload_is_not_announced() {
        let src = OFFICE.replace(
            "payload = '{\"co2\"",
            "encoding = \"cbor\"\npayload = '{\"co2\"",
        );
        assert_ne!(src, OFFICE);
        assert_eq!(announced(&src), []);
        let project = valid_project(&src);
        assert_eq!(
            unannounced_fields(&project.devices[0].instances[0]),
            ["co2", "open", "temp"]
        );
    }

    #[test]
    fn the_fields_left_out_are_named() {
        let project = valid_project(&OFFICE.replace(
            "\"readings\": [{{ temp }}]",
            "\"readings\": [{{ temp }}, {{ temp }}]",
        ));
        assert_eq!(
            unannounced_fields(&project.devices[0].instances[0]),
            ["temp"]
        );
    }

    #[test]
    fn nothing_is_announced_unless_the_template_asks() {
        let src = OFFICE.replace("home_assistant = true\n", "");
        assert_eq!(announced(&src), []);
    }

    #[test]
    fn characters_home_assistant_does_not_allow_in_ids_become_underscores() {
        let announcements = announced(
            r#"
[[templates]]
id = "t"
home_assistant = true
[[templates.messages]]
topic = "t"
payload = '{"v": {{ v }}}'
[templates.fields."v"]
generator = "constant"
value = 1

[[devices]]
id = "d"
template = "t"
instance_id = "hall.1"
"#,
        );
        let v = config_on(&announcements, "homeassistant/sensor/hall_1/v/config");
        assert_eq!(v["unique_id"], "senzor_hall_1_v");
        assert_eq!(
            v["device"]["name"], "hall.1",
            "the name keeps the instance ID"
        );
    }
}
