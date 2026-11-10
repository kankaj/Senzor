use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::error::{Location, ProjectError, line_column};
use super::field::FieldSpec;
use crate::i18n::text;
use crate::script::Script;
use crate::value::Value;

/// The project file as written on disk. The GUI edits this; `validate` turns it into a `Project`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct ProjectFile {
    #[serde(default)]
    pub broker: BrokerSpec,
    #[serde(default, skip_serializing_if = "SimulationSpec::is_empty")]
    pub simulation: SimulationSpec,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub templates: Vec<TemplateSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<DeviceSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct BrokerSpec {
    pub host: String,
    pub port: u16,
    pub client_id_prefix: String,
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    #[serde(with = "humantime_serde")]
    pub keep_alive: Duration,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tls: Option<TlsSpec>,
    /// The MQTT protocol version every instance of the project connects with.
    pub mqtt_version: MqttVersion,
    /// How instances reach the broker: plain MQTT over TCP, or MQTT over WebSocket.
    #[serde(skip_serializing_if = "Transport::is_tcp")]
    pub transport: Transport,
    /// The WebSocket path, such as `/mqtt`; used only with `transport = "websocket"`.
    #[serde(skip_serializing_if = "is_default_path")]
    pub path: String,
    /// MQTT 5 session expiry interval. Above zero, instances reconnect without a clean start, so
    /// a reconnect within `availability.will_delay` cancels the Last Will.
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    #[serde(with = "humantime_serde", skip_serializing_if = "Duration::is_zero")]
    pub session_expiry: Duration,
}

/// The MQTT protocol version of a project's connections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub enum MqttVersion {
    #[default]
    #[serde(rename = "5")]
    V5,
    #[serde(rename = "3.1.1")]
    V311,
}

/// How a project's instances reach the broker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    /// MQTT over TCP (with `tls`, over TLS).
    #[default]
    Tcp,
    /// MQTT over WebSocket, `ws://` (with `tls`, `wss://`), at the broker's `path`.
    WebSocket,
}

impl Transport {
    pub const ALL: [Transport; 2] = [Transport::Tcp, Transport::WebSocket];

    fn is_tcp(&self) -> bool {
        *self == Transport::Tcp
    }

    /// The transport as written in the project file: `tcp` or `websocket`.
    pub fn label(self) -> &'static str {
        match self {
            Transport::Tcp => "tcp",
            Transport::WebSocket => "websocket",
        }
    }
}

/// The WebSocket path brokers commonly serve MQTT on, used when the file sets none.
pub const DEFAULT_WEBSOCKET_PATH: &str = "/mqtt";

fn is_default_path(path: &String) -> bool {
    path == DEFAULT_WEBSOCKET_PATH
}

impl MqttVersion {
    pub const ALL: [MqttVersion; 2] = [MqttVersion::V5, MqttVersion::V311];

    /// The version as written in the project file and shown to people: `5` or `3.1.1`.
    pub fn label(self) -> &'static str {
        match self {
            MqttVersion::V5 => "5",
            MqttVersion::V311 => "3.1.1",
        }
    }
}

impl Default for BrokerSpec {
    fn default() -> Self {
        Self {
            host: "localhost".into(),
            port: 1883,
            client_id_prefix: "senzor-".into(),
            keep_alive: Duration::from_secs(30),
            username: None,
            password: None,
            password_env: None,
            tls: None,
            mqtt_version: MqttVersion::default(),
            transport: Transport::default(),
            path: DEFAULT_WEBSOCKET_PATH.into(),
            session_expiry: Duration::ZERO,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TlsSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_file: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_cert: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_key: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SimulationSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

impl SimulationSpec {
    fn is_empty(&self) -> bool {
        self.seed.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct TemplateSpec {
    pub id: String,
    #[cfg_attr(feature = "schema", schemars(with = "String"))]
    #[serde(default = "default_interval", with = "humantime_serde")]
    pub interval: Duration,
    #[serde(default)]
    pub qos: u8,
    #[serde(default)]
    pub retain: bool,
    /// Announce each instance and its fields to Home Assistant over MQTT Discovery.
    #[serde(default, skip_serializing_if = "is_false")]
    pub home_assistant: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability: Option<AvailabilitySpec>,
    pub messages: Vec<MessageSpec>,
    pub fields: BTreeMap<String, FieldSpec>,
    /// Topic filters each instance subscribes to, each with a script that runs for every
    /// message that arrives on it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscriptions: Vec<SubscriptionSpec>,
}

/// One `[[templates.subscriptions]]` entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct SubscriptionSpec {
    /// The topic filter, a template with the variables of the availability topic; it may
    /// contain the MQTT wildcards `+` and `#`.
    pub topic: String,
    #[serde(default)]
    pub qos: u8,
    /// Runs for every message that arrives on the topic.
    pub script: Script,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// The publish interval of a template that sets none.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(10);

fn default_interval() -> Duration {
    DEFAULT_INTERVAL
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct MessageSpec {
    pub topic: String,
    pub payload: String,
    /// How the rendered payload becomes the bytes that are published.
    #[serde(default, skip_serializing_if = "Encoding::is_text")]
    pub encoding: Encoding,
    /// With `encoding = "protobuf"`: the `.proto` file that defines `message`, relative to the
    /// project file's folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proto: Option<PathBuf>,
    /// With `encoding = "protobuf"`: the fully qualified message type, such as
    /// `sensors.v1.Reading`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// MQTT 5 message expiry interval, in whole seconds.
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "humantime_serde"
    )]
    pub message_expiry: Option<Duration>,
    /// MQTT 5 content type, sent as written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// MQTT 5 user properties. Values are templates with the variables of the topic and payload.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub user_properties: BTreeMap<String, String>,
}

impl MessageSpec {
    /// A message without MQTT 5 properties.
    pub fn new(topic: impl Into<String>, payload: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
            payload: payload.into(),
            encoding: Encoding::Text,
            proto: None,
            message: None,
            message_expiry: None,
            content_type: None,
            user_properties: BTreeMap::new(),
        }
    }

    /// Whether the message sets any MQTT 5 property.
    pub fn has_properties(&self) -> bool {
        self.message_expiry.is_some()
            || self.content_type.is_some()
            || !self.user_properties.is_empty()
    }
}

/// How a message's rendered payload becomes the bytes that are published.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    /// The text as it is rendered, in UTF-8.
    #[default]
    Text,
    /// The text is hex digits, two per byte; whitespace is ignored.
    Hex,
    /// The text is standard base64 with padding; whitespace is ignored.
    Base64,
    /// The text is JSON, published as CBOR.
    Cbor,
    /// The text is JSON in protobuf's JSON mapping, published as the protobuf message `message`
    /// of the `proto` file.
    Protobuf,
}

impl Encoding {
    pub const ALL: [Encoding; 5] = [
        Encoding::Text,
        Encoding::Hex,
        Encoding::Base64,
        Encoding::Cbor,
        Encoding::Protobuf,
    ];

    pub fn is_text(&self) -> bool {
        *self == Encoding::Text
    }

    /// The encoding as written in the project file: `text`, `hex`, `base64`, `cbor` or
    /// `protobuf`.
    pub fn label(self) -> &'static str {
        match self {
            Encoding::Text => "text",
            Encoding::Hex => "hex",
            Encoding::Base64 => "base64",
            Encoding::Cbor => "cbor",
            Encoding::Protobuf => "protobuf",
        }
    }

    /// The encoding as the window shows it: `text`, `hex`, `base64`, `CBOR` or `protobuf`.
    pub fn name(self) -> &'static str {
        match self {
            Encoding::Cbor => "CBOR",
            other => other.label(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct AvailabilitySpec {
    pub topic: String,
    pub online: String,
    pub offline: String,
    /// MQTT 5 Last Will delay, in whole seconds; needs a broker `session_expiry` at least as long.
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "humantime_serde"
    )]
    pub will_delay: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct DeviceSpec {
    pub id: String,
    pub template: String,
    #[serde(default = "default_count")]
    pub count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Value>,
    #[serde(default = "default_autostart")]
    pub autostart: bool,
    #[cfg_attr(feature = "schema", schemars(with = "Option<String>"))]
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "humantime_serde"
    )]
    pub interval: Option<Duration>,
    /// Partial field tables (generator keys, `unit`, `name`) merged onto the template's fields
    /// (see `validate`).
    #[cfg_attr(
        feature = "schema",
        schemars(with = "BTreeMap<String, crate::schema::TomlTable>")
    )]
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, toml::Table>,
}

fn default_count() -> u32 {
    1
}

fn default_autostart() -> bool {
    true
}

/// The `instance_id` pattern used when a device sets no `instance_id`: a single instance takes
/// the device ID, several get the index appended.
pub fn default_instance_id_pattern(count: u32) -> &'static str {
    if count == 1 {
        "{{ id }}"
    } else {
        "{{ id }}-{{ index }}"
    }
}

impl DeviceSpec {
    /// A device of `template` with every other setting at its default, as if the file gave only
    /// `id` and `template`.
    pub fn new(id: impl Into<String>, template: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            template: template.into(),
            count: default_count(),
            instance_id: None,
            params: BTreeMap::new(),
            autostart: default_autostart(),
            interval: None,
            fields: BTreeMap::new(),
        }
    }
}

impl ProjectFile {
    /// Parses TOML text. Syntax and schema errors (wrong types, unknown keys) end up here.
    pub fn parse(text: &str) -> Result<ProjectFile, Vec<ProjectError>> {
        toml::from_str(text).map_err(|err| {
            let location = match err.span() {
                Some(span) => {
                    let (line, column) = line_column(text, span.start);
                    Location::Toml { line, column }
                }
                None => Location::File,
            };
            // The TOML parser's own text, in English.
            vec![ProjectError::new(
                location,
                text!("verbatim", text = err.message()),
            )]
        })
    }

    /// Writes canonical TOML. Comments and custom formatting are not preserved.
    pub fn to_toml_string(&self) -> String {
        toml::to_string_pretty(self).expect("project model always serializes to TOML")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::GeneratorSpec;

    const EXAMPLE: &str = include_str!("../../tests/fixtures/example.toml");

    #[test]
    fn parses_the_spec_example() {
        let file = ProjectFile::parse(EXAMPLE).unwrap();
        assert_eq!(file.broker.host, "localhost");
        assert_eq!(file.broker.password_env.as_deref(), Some("MQTT_PASSWORD"));
        assert_eq!(file.simulation.seed, Some(42));
        let template = &file.templates[0];
        assert_eq!(template.id, "office-air");
        assert_eq!(template.interval, Duration::from_secs(10));
        assert_eq!(template.messages.len(), 1);
        assert!(matches!(
            template.fields["co2"].generator,
            GeneratorSpec::RandomWalk { start: 600.0, .. }
        ));
        let device = &file.devices[0];
        assert_eq!(device.count, 5);
        assert_eq!(device.params["room"], Value::String("office".into()));
        assert_eq!(device.fields["co2"]["start"].as_integer(), Some(1200));
    }

    #[test]
    fn applies_defaults() {
        let file = ProjectFile::parse(
            r#"
[[templates]]
id = "t"
messages = [{ topic = "a", payload = "b" }]
fields.v = { generator = "constant", value = 1 }

[[devices]]
id = "d"
template = "t"
"#,
        )
        .unwrap();
        assert_eq!(file.broker, BrokerSpec::default());
        assert_eq!(file.templates[0].interval, Duration::from_secs(10));
        assert_eq!(file.templates[0].qos, 0);
        assert!(!file.templates[0].retain);
        assert_eq!(file.devices[0].count, 1);
        assert!(file.devices[0].autostart);
    }

    /// `DeviceSpec::new` stands for an entry with only `id` and `template`, so it must not
    /// drift from the serde defaults.
    #[test]
    fn a_new_device_equals_one_read_with_only_id_and_template() {
        let file = ProjectFile::parse("[[devices]]\nid = \"d\"\ntemplate = \"t\"\n").unwrap();
        assert_eq!(file.devices[0], DeviceSpec::new("d", "t"));
    }

    #[test]
    fn unknown_key_reports_line_and_column() {
        let errors = ProjectFile::parse("[broker]\nhost = \"x\"\nhots = \"y\"\n").unwrap_err();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].location, Location::Toml { line: 3, column: 1 });
        assert!(
            errors[0].message.contains("unknown field `hots`"),
            "{}",
            errors[0].message
        );
    }

    #[test]
    fn syntax_error_is_reported() {
        let errors = ProjectFile::parse("[broker\n").unwrap_err();
        assert!(matches!(errors[0].location, Location::Toml { line: 1, .. }));
    }

    #[test]
    fn round_trips_through_toml() {
        let file = ProjectFile::parse(EXAMPLE).unwrap();
        let written = file.to_toml_string();
        assert_eq!(ProjectFile::parse(&written).unwrap(), file);
    }

    #[test]
    fn a_list_or_table_as_a_value_says_which_values_are_allowed() {
        for value in ["[1, 2]", "{ a = 1 }"] {
            let src = format!(
                "[[devices]]\nid = \"d\"\ntemplate = \"co2\"\nparams = {{ room = {value} }}\n"
            );
            let errors = ProjectFile::parse(&src).unwrap_err();
            let message = errors[0].to_string();
            assert!(
                message.contains("expected a number, a boolean or a string"),
                "{value}: {message}"
            );
        }
    }

    #[test]
    fn a_sine_without_a_phase_is_written_without_one() {
        let src = r#"
[[templates]]
id = "t"
messages = [{ topic = "a", payload = "b" }]
fields.flat = { generator = "sine", center = 20, amplitude = 2, period = "1h" }
fields.late = { generator = "sine", center = 20, amplitude = 2, period = "1h", phase = "15m" }
"#;
        let file = ProjectFile::parse(src).unwrap();
        let written = file.to_toml_string();
        assert_eq!(written.matches("phase").count(), 1, "{written}");
        assert!(written.contains("phase = \"15m\""), "{written}");
        assert_eq!(ProjectFile::parse(&written).unwrap(), file);
    }

    #[test]
    fn field_unit_and_name_round_trip_and_are_omitted_when_unset() {
        let src = r#"
[[templates]]
id = "t"
messages = [{ topic = "a", payload = "b" }]
fields.co2 = { generator = "random_walk", start = 600, min = 400, max = 2000, step = 15, unit = "ppm", name = "CO2" }
fields.v = { generator = "constant", value = 1 }
"#;
        let file = ProjectFile::parse(src).unwrap();
        let co2 = &file.templates[0].fields["co2"];
        assert_eq!(co2.unit.as_deref(), Some("ppm"));
        assert_eq!(co2.name.as_deref(), Some("CO2"));
        let written = file.to_toml_string();
        assert!(
            written.contains("step = 15.0\nunit = \"ppm\"\nname = \"CO2\"\n"),
            "{written}"
        );
        assert_eq!(written.matches("unit").count(), 1, "{written}");
        assert_eq!(ProjectFile::parse(&written).unwrap(), file);
    }

    /// A field's problems point where they did before fields took a unit and a name.
    #[test]
    fn field_problems_point_at_the_field_or_its_value() {
        let error = |field: &str| {
            let src = format!("[[templates]]\nid = \"t\"\nmessages = []\nfields.co2 = {field}\n");
            let errors = ProjectFile::parse(&src).unwrap_err();
            assert_eq!(errors.len(), 1);
            (errors[0].location.clone(), errors[0].message.clone())
        };
        let at = |column| Location::Toml { line: 4, column };
        assert_eq!(
            error("{ generator = \"uniform\", min = 1, max = 2, stepp = 1 }"),
            (
                at(14),
                "unknown field `stepp`, expected one of `min`, `max`, `noise`".into()
            )
        );
        assert_eq!(
            error("{ min = 1, unit = \"ppm\" }"),
            (at(14), "missing field `generator`".into())
        );
        assert_eq!(
            error("{ generator = \"foo\" }"),
            (
                at(28),
                "unknown variant `foo`, expected one of `constant`, `random_walk`, `sine`, \
                 `uniform`, `sequence`, `script`"
                    .into()
            )
        );
        assert_eq!(
            error("{ generator = \"uniform\", min = 1, max = 2, unit = 5 }"),
            (
                at(64),
                "invalid type: integer `5`, expected a string".into()
            )
        );
    }

    #[test]
    fn mqtt_version_defaults_to_5_and_reads_3_1_1() {
        assert_eq!(
            ProjectFile::parse("").unwrap().broker.mqtt_version,
            MqttVersion::V5
        );
        let file = ProjectFile::parse("[broker]\nmqtt_version = \"3.1.1\"\n").unwrap();
        assert_eq!(file.broker.mqtt_version, MqttVersion::V311);
    }

    #[test]
    fn unknown_mqtt_version_names_the_accepted_values() {
        let errors = ProjectFile::parse("[broker]\nmqtt_version = \"4\"\n").unwrap_err();
        assert_eq!(
            errors[0].location,
            Location::Toml {
                line: 2,
                column: 16
            }
        );
        assert!(
            errors[0].message.contains("`5`") && errors[0].message.contains("`3.1.1`"),
            "{}",
            errors[0].message
        );
    }

    #[test]
    fn mqtt_version_is_written_and_zero_session_expiry_is_not() {
        let written = ProjectFile::default().to_toml_string();
        assert!(written.contains("mqtt_version = \"5\""), "{written}");
        assert!(!written.contains("session_expiry"), "{written}");
    }

    #[test]
    fn mqtt5_properties_round_trip_and_are_omitted_when_unset() {
        let src = r#"
[broker]
session_expiry = "1m"

[[templates]]
id = "t"
availability = { topic = "s", online = "on", offline = "off", will_delay = "5s" }
messages = [
    { topic = "a", payload = "p", message_expiry = "60s", content_type = "application/json", user_properties = { room = "{{ room }}" } },
    { topic = "b", payload = "p" },
]
fields.v = { generator = "constant", value = 1 }
"#;
        let file = ProjectFile::parse(src).unwrap();
        let message = &file.templates[0].messages[0];
        assert_eq!(message.message_expiry, Some(Duration::from_secs(60)));
        assert_eq!(message.user_properties["room"], "{{ room }}");
        assert!(!file.templates[0].messages[1].has_properties());
        let written = file.to_toml_string();
        assert_eq!(ProjectFile::parse(&written).unwrap(), file);
        assert_eq!(written.matches("message_expiry").count(), 1, "{written}");
    }

    const ENCODED: &str = r#"
[[templates]]
id = "t"
messages = [
    { topic = "a", payload = "{{ v | u8 }}", encoding = "hex" },
    { topic = "b", payload = '{"v": {{ v }}}', encoding = "protobuf", proto = "proto/sensor.proto", message = "sensors.v1.Reading" },
    { topic = "c", payload = "plain" },
]
fields.v = { generator = "constant", value = 1 }
"#;

    #[test]
    fn encodings_round_trip_and_text_is_not_written() {
        let file = ProjectFile::parse(ENCODED).unwrap();
        let messages = &file.templates[0].messages;
        assert_eq!(messages[0].encoding, Encoding::Hex);
        assert_eq!(messages[1].encoding, Encoding::Protobuf);
        assert_eq!(
            messages[1].proto.as_deref(),
            Some(std::path::Path::new("proto/sensor.proto"))
        );
        assert_eq!(messages[1].message.as_deref(), Some("sensors.v1.Reading"));
        assert_eq!(messages[2].encoding, Encoding::Text, "the default");
        let written = file.to_toml_string();
        assert_eq!(written.matches("encoding").count(), 2, "{written}");
        assert!(written.contains("encoding = \"protobuf\""), "{written}");
        assert_eq!(ProjectFile::parse(&written).unwrap(), file);
        let json = serde_json::to_value(&file).unwrap();
        assert_eq!(serde_json::from_value::<ProjectFile>(json).unwrap(), file);
    }

    #[test]
    fn an_unknown_encoding_names_the_accepted_ones() {
        let src = ENCODED.replace("encoding = \"hex\"", "encoding = \"bson\"");
        let errors = ProjectFile::parse(&src).unwrap_err();
        assert!(
            errors[0]
                .message
                .contains("expected one of `text`, `hex`, `base64`, `cbor`, `protobuf`"),
            "{}",
            errors[0].message
        );
    }

    const SCRIPTED: &str = r#"
[[templates]]
id = "thermostat"
messages = [{ topic = "t/{{ device_id }}", payload = "{{ temp }}" }]

[templates.fields.temp]
generator = "script"
start = 20.0
script = '''
if state.heating == true { value + 0.2 } else { value - 0.1 }
'''

[[templates.subscriptions]]
topic = "cmd/{{ device_id }}/heating"
qos = 1
script = '''
state.heating = payload == "ON";
'''

[[templates.subscriptions]]
topic = "cmd/all"
script = "1"
"#;

    #[test]
    fn scripts_and_subscriptions_round_trip() {
        let file = ProjectFile::parse(SCRIPTED).unwrap();
        let template = &file.templates[0];
        assert_eq!(template.subscriptions.len(), 2);
        let first = &template.subscriptions[0];
        assert_eq!(first.topic, "cmd/{{ device_id }}/heating");
        assert_eq!(first.qos, 1);
        assert_eq!(
            first.script.source(),
            "state.heating = payload == \"ON\";\n"
        );
        assert_eq!(template.subscriptions[1].qos, 0, "qos defaults to 0");
        assert!(matches!(
            &template.fields["temp"].generator,
            GeneratorSpec::Script {
                start: Value::Number(20.0),
                ..
            }
        ));
        let written = file.to_toml_string();
        assert_eq!(ProjectFile::parse(&written).unwrap(), file, "{written}");
        let json = serde_json::to_value(&file).unwrap();
        assert_eq!(
            serde_json::from_value::<ProjectFile>(json).unwrap(),
            file,
            "JSON, as the MCP server reads and writes it"
        );
    }

    #[test]
    fn a_template_without_subscriptions_writes_none() {
        let file = ProjectFile::parse(EXAMPLE).unwrap();
        assert!(file.templates[0].subscriptions.is_empty());
        assert!(!file.to_toml_string().contains("subscriptions"));
    }

    #[test]
    fn a_subscription_needs_a_topic_and_a_script_and_knows_no_other_keys() {
        for (table, message) in [
            ("script = \"1\"", "missing field `topic`"),
            ("topic = \"a\"", "missing field `script`"),
            (
                "topic = \"a\"\nscript = \"1\"\nretain = true",
                "unknown field `retain`",
            ),
        ] {
            let src = format!(
                "[[templates]]\nid = \"t\"\nmessages = []\nfields = {{}}\n\
                 [[templates.subscriptions]]\n{table}\n"
            );
            let errors = ProjectFile::parse(&src).unwrap_err();
            assert!(errors[0].message.contains(message), "{errors:?}");
        }
    }
}
