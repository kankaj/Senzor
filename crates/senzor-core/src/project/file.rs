use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::error::{Location, ProjectError, line_column};
use crate::generator::GeneratorSpec;
use crate::value::Value;

/// The project file as written on disk. The GUI edits this; `validate` turns it into a `Project`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
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
#[serde(default, deny_unknown_fields)]
pub struct BrokerSpec {
    pub host: String,
    pub port: u16,
    pub client_id_prefix: String,
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
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
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
#[serde(deny_unknown_fields)]
pub struct TemplateSpec {
    pub id: String,
    #[serde(default = "default_interval", with = "humantime_serde")]
    pub interval: Duration,
    #[serde(default)]
    pub qos: u8,
    #[serde(default)]
    pub retain: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability: Option<AvailabilitySpec>,
    pub messages: Vec<MessageSpec>,
    pub fields: BTreeMap<String, GeneratorSpec>,
}

/// The publish interval of a template that sets none.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(10);

fn default_interval() -> Duration {
    DEFAULT_INTERVAL
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageSpec {
    pub topic: String,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AvailabilitySpec {
    pub topic: String,
    pub online: String,
    pub offline: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "humantime_serde"
    )]
    pub interval: Option<Duration>,
    /// Partial generator tables merged onto the template's fields (see `validate`).
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
            vec![ProjectError::new(location, err.message())]
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
            template.fields["co2"],
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
}
