use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use super::error::ProjectError;
use super::file::{AvailabilitySpec, MessageSpec, MqttVersion};
use crate::generator::GeneratorSpec;
use crate::render::{InstanceTemplates, TemplateError};
use crate::value::Value;

/// The result of `ProjectFile::validate`: what can run, what is wrong, and which devices are blocked.
#[derive(Debug, Clone)]
pub struct Validated {
    /// Only devices that passed validation.
    pub project: Project,
    pub errors: Vec<ProjectError>,
    /// IDs of devices that cannot start because of errors.
    pub blocked: BTreeSet<String>,
}

/// A validated project, ready for the engine.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub broker: BrokerConfig,
    pub devices: Vec<DeviceConfig>,
}

impl Project {
    pub fn device(&self, id: &str) -> Option<&DeviceConfig> {
        self.devices.iter().find(|d| d.id == id)
    }

    pub fn instances(&self) -> impl Iterator<Item = &InstanceConfig> {
        self.devices.iter().flat_map(|d| d.instances.iter())
    }

    pub fn instance(&self, id: &str) -> Option<&InstanceConfig> {
        self.instances().find(|i| i.id == id)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceConfig {
    pub id: String,
    pub autostart: bool,
    pub instances: Vec<InstanceConfig>,
}

/// Everything one instance needs to run. Compared to detect `needs_restart`.
#[derive(Debug, Clone, PartialEq)]
pub struct InstanceConfig {
    /// The instance ID; exposed to templates as `device_id`.
    pub id: String,
    /// The `[[devices]]` entry this instance came from.
    pub device: String,
    pub index: u32,
    pub template_id: String,
    pub messages: Vec<MessageSpec>,
    pub availability: Option<AvailabilitySpec>,
    /// `messages` and `availability` compiled at validation; changing those two afterwards does
    /// not recompile them.
    pub templates: SharedTemplates,
    pub qos: u8,
    pub retain: bool,
    pub interval: Duration,
    /// Template fields with the device's overrides applied.
    pub fields: BTreeMap<String, GeneratorSpec>,
    pub params: BTreeMap<String, Value>,
    pub seed: Option<u64>,
}

/// A template's messages and availability topic, compiled once and shared by every instance of
/// the template. It is made from an instance's `messages` and `availability` alone, which the
/// instance compares itself, so it takes no part in the comparison: any two are equal.
#[derive(Clone)]
pub struct SharedTemplates(Arc<InstanceTemplates>);

impl SharedTemplates {
    pub fn compile(
        messages: &[MessageSpec],
        availability: Option<&AvailabilitySpec>,
    ) -> Result<Self, String> {
        InstanceTemplates::compile(messages, availability).map(|t| Self(Arc::new(t)))
    }

    /// [`Self::compile`], with the key path of the broken template.
    pub(crate) fn compile_located(
        messages: &[MessageSpec],
        availability: Option<&AvailabilitySpec>,
    ) -> Result<Self, TemplateError> {
        InstanceTemplates::compile_located(messages, availability).map(|t| Self(Arc::new(t)))
    }

    /// Whether both are the same compiled templates, not just equal ones.
    #[cfg(test)]
    pub(crate) fn is_shared_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Deref for SharedTemplates {
    type Target = InstanceTemplates;

    fn deref(&self) -> &InstanceTemplates {
        &self.0
    }
}

impl PartialEq for SharedTemplates {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl fmt::Debug for SharedTemplates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SharedTemplates")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrokerConfig {
    pub host: String,
    pub port: u16,
    pub client_id_prefix: String,
    pub keep_alive: Duration,
    pub credentials: Option<Credentials>,
    pub tls: Option<TlsConfig>,
    pub mqtt_version: MqttVersion,
    /// MQTT 5 session expiry; zero means a clean start on every connect.
    pub session_expiry: Duration,
}

#[derive(Clone, PartialEq)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("username", &self.username)
            .field("password", &"***")
            .finish()
    }
}

/// TLS material read from disk at validation time.
#[derive(Clone, PartialEq)]
pub struct TlsConfig {
    /// PEM CA certificate; `None` means system roots.
    pub ca: Option<Vec<u8>>,
    /// PEM client certificate and key (mTLS).
    pub client_auth: Option<(Vec<u8>, Vec<u8>)>,
}

impl fmt::Debug for TlsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsConfig")
            .field("ca", &self.ca.as_ref().map(|c| c.len()))
            .field("client_auth", &self.client_auth.is_some())
            .finish()
    }
}
