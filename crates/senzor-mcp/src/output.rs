//! What the tools return, as JSON with a schema.

use std::collections::BTreeMap;
use std::time::SystemTime;

use schemars::JsonSchema;
use senzor_core::engine::{InstanceState, InstanceStatus, LogEntry};
use senzor_core::project::builtin::builtin_templates;
use senzor_core::project::{ProjectError, ProjectFile, TemplateSpec, Validated};
use senzor_core::value::Value;
use serde::Serialize;

/// An instance's status. `state` is `stopped`, `connecting`, `online`, `reconnecting` or `failed`.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    Stopped,
    Connecting,
    Online,
    /// Retrying after a network error.
    Reconnecting {
        attempt: u32,
        last_error: String,
    },
    /// The broker refused the connection, or the instance crashed. Start it again to retry.
    Failed {
        reason: String,
    },
}

impl From<&InstanceStatus> for Status {
    fn from(status: &InstanceStatus) -> Self {
        match status {
            InstanceStatus::Stopped => Status::Stopped,
            InstanceStatus::Connecting => Status::Connecting,
            InstanceStatus::Online => Status::Online,
            InstanceStatus::Reconnecting {
                attempt,
                last_error,
            } => Status::Reconnecting {
                attempt: *attempt,
                last_error: last_error.clone(),
            },
            InstanceStatus::Failed { reason } => Status::Failed {
                reason: reason.clone(),
            },
        }
    }
}

/// RFC 3339 in UTC with milliseconds, e.g. `2026-11-04T14:02:11.123Z`.
pub fn time(time: SystemTime) -> String {
    humantime::format_rfc3339_millis(time).to_string()
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct InstanceSummary {
    pub id: String,
    /// The `[[devices]]` entry the instance belongs to.
    pub device: String,
    pub status: Status,
    /// Messages published since the instance started.
    pub published: u64,
    /// Failed publishes and renders since the instance started.
    pub errors: u64,
    pub last_publish: Option<String>,
    /// Fields forced to a value with `pin`.
    pub pinned: Vec<String>,
    /// Running with an older configuration than the project's: `restart` applies the new one.
    pub needs_restart: bool,
    /// Running, but no longer in the project: it disappears once stopped.
    pub orphaned: bool,
}

impl InstanceSummary {
    pub fn new(id: &str, state: &InstanceState) -> Self {
        Self {
            id: id.to_string(),
            device: state.device.clone(),
            status: (&state.status).into(),
            published: state.published,
            errors: state.errors,
            last_publish: state.last_publish.map(time),
            pinned: state.pinned.iter().cloned().collect(),
            needs_restart: state.needs_restart,
            orphaned: state.orphaned,
        }
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct StatusOut {
    /// The project file, or none for a project that was never saved.
    pub path: Option<String>,
    /// The project has changes that are not saved.
    pub dirty: bool,
    pub broker_host: String,
    pub broker_port: u16,
    /// How many validation problems the project has (`get_project` lists them).
    pub problems: usize,
    /// Why the last attempt to open a project file failed.
    pub open_error: Option<String>,
    pub instances: Vec<InstanceSummary>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct InstanceDetail {
    #[serde(flatten)]
    pub summary: InstanceSummary,
    /// The template the instance runs, when it is in the project.
    pub template: Option<String>,
    /// Every field the instance publishes.
    pub fields: Vec<String>,
    /// Each field's latest value (empty before the first publish).
    pub values: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Message {
    pub seq: u64,
    pub time: String,
    pub instance: String,
    pub topic: String,
    pub payload: String,
    pub qos: u8,
    pub retain: bool,
    /// MQTT 5 message expiry in seconds, when the message set one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_expiry: Option<u32>,
    /// MQTT 5 content type, when the message set one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// MQTT 5 user properties as rendered for this message, when it set any.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub user_properties: BTreeMap<String, String>,
}

impl From<&LogEntry> for Message {
    fn from(entry: &LogEntry) -> Self {
        Self {
            seq: entry.seq,
            time: time(entry.time),
            instance: entry.instance.clone(),
            topic: entry.topic.clone(),
            payload: entry.payload.clone(),
            qos: entry.qos,
            retain: entry.retain,
            message_expiry: entry.properties.message_expiry,
            content_type: entry.properties.content_type.clone(),
            user_properties: entry.properties.user_properties.iter().cloned().collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MessagesOut {
    /// Oldest first.
    pub messages: Vec<Message>,
    /// The highest sequence number so far: pass it as `after_seq` to get only newer messages.
    pub last_seq: u64,
    /// Messages after `after_seq` were dropped from the log (it keeps the last 1,000) before
    /// they could be read.
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct InstanceStatusOut {
    pub id: String,
    /// `None` when the instance left the project once it stopped.
    pub status: Option<Status>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RunOut {
    pub instances: Vec<InstanceStatusOut>,
    /// Some instances had not settled when the wait ran out.
    pub timed_out: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PinOut {
    pub instance: String,
    /// The instance's pinned fields now.
    pub pinned: Vec<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Problem {
    /// Where, e.g. `device \`office\`` or `line 3, column 1`.
    pub location: String,
    pub message: String,
}

impl From<&ProjectError> for Problem {
    fn from(error: &ProjectError) -> Self {
        Self {
            location: error.location.to_string(),
            message: error.message.clone(),
        }
    }
}

pub fn problems(validated: &Validated) -> Vec<Problem> {
    validated.errors.iter().map(Problem::from).collect()
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct EditOut {
    /// The project changed (an edit that changes nothing reports false).
    pub changed: bool,
    /// Every validation problem the project has now.
    pub problems: Vec<Problem>,
    /// Devices that cannot start because of problems.
    pub blocked: Vec<String>,
    /// Running instances whose configuration differs from the project now: `restart` them.
    pub needs_restart: Vec<String>,
    /// Running instances that are no longer in the project.
    pub orphaned: Vec<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProjectOut {
    /// The project as TOML, as Save writes it (passwords redacted).
    pub toml: String,
    pub path: Option<String>,
    pub dirty: bool,
    pub problems: Vec<Problem>,
    /// Devices that cannot start because of problems.
    pub blocked: Vec<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct OpenOut {
    pub path: Option<String>,
    pub problems: Vec<Problem>,
    pub blocked: Vec<String>,
    pub instances: Vec<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SaveOut {
    /// The file written.
    pub path: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TemplateInfo {
    pub id: String,
    /// Ships with Senzor; cannot be changed, but `copy_builtin_template` copies it.
    pub builtin: bool,
    /// Publish interval, e.g. `10s`.
    pub interval: String,
    pub topics: Vec<String>,
    /// Each field's generator, e.g. `random_walk`.
    pub fields: BTreeMap<String, String>,
    /// Devices of the project that use it.
    pub used_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TemplatesOut {
    pub templates: Vec<TemplateInfo>,
}

pub fn templates(file: &ProjectFile) -> TemplatesOut {
    let info = |template: &TemplateSpec, builtin: bool| TemplateInfo {
        id: template.id.clone(),
        builtin,
        interval: humantime::format_duration(template.interval).to_string(),
        topics: template.messages.iter().map(|m| m.topic.clone()).collect(),
        fields: template
            .fields
            .iter()
            .map(|(name, spec)| {
                let kind = serde_json::to_value(spec)
                    .ok()
                    .and_then(|v| v.get("generator")?.as_str().map(String::from))
                    .unwrap_or_default();
                (name.clone(), kind)
            })
            .collect(),
        used_by: file
            .devices
            .iter()
            .filter(|d| d.template == template.id)
            .map(|d| d.id.clone())
            .collect(),
    };
    let builtins = builtin_templates().iter().map(|t| info(t, true));
    let own = file.templates.iter().map(|t| info(t, false));
    TemplatesOut {
        templates: builtins.chain(own).collect(),
    }
}
