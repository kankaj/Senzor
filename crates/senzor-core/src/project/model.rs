use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::Duration;

use super::error::ProjectError;
use super::file::{AvailabilitySpec, MessageSpec};
use crate::generator::GeneratorSpec;
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
    pub qos: u8,
    pub retain: bool,
    pub interval: Duration,
    /// Template fields with the device's overrides applied.
    pub fields: BTreeMap<String, GeneratorSpec>,
    pub params: BTreeMap<String, Value>,
    pub seed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BrokerConfig {
    pub host: String,
    pub port: u16,
    pub client_id_prefix: String,
    pub keep_alive: Duration,
    pub credentials: Option<Credentials>,
    pub tls: Option<TlsConfig>,
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
