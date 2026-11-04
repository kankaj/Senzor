//! Tool arguments. Device, template and broker tools take the project-file types directly.

use schemars::JsonSchema;
use senzor_core::value::Value;
use serde::Deserialize;

use crate::host::Targets;

/// The longest a run action waits for its instances to settle.
pub const MAX_RUN_WAIT_SECONDS: f64 = 60.0;
/// How long a run action waits when the call does not say.
pub const DEFAULT_RUN_WAIT_SECONDS: f64 = 5.0;
/// The longest `get_messages` waits for a matching message.
pub const MAX_MESSAGE_WAIT_SECONDS: f64 = 30.0;
pub const DEFAULT_MESSAGE_LIMIT: usize = 100;
pub const MAX_MESSAGE_LIMIT: usize = 1000;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct InstanceParams {
    /// Instance ID, e.g. `office-3`.
    pub instance: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MessagesParams {
    /// Only messages published after this sequence number: pass the `last_seq` of the previous
    /// call to get only new messages.
    pub after_seq: Option<u64>,
    /// Only messages of this instance.
    pub instance: Option<String>,
    /// Only messages of this device's instances.
    pub device: Option<String>,
    /// Only messages whose topic contains this text.
    pub topic_contains: Option<String>,
    /// At most this many messages, the newest (default 100, at most 1000).
    pub limit: Option<usize>,
    /// When no message matches yet, wait up to this many seconds for one (at most 30).
    pub wait_seconds: Option<f64>,
}

/// Exactly one of `all`, `devices` and `instances`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunParams {
    /// Every instance.
    pub all: Option<bool>,
    /// Device IDs (`[[devices]]` ids): all instances of each.
    pub devices: Option<Vec<String>>,
    /// Instance IDs.
    pub instances: Option<Vec<String>>,
    /// Wait up to this many seconds for the instances to settle (default 5, at most 60, 0 to
    /// return at once).
    pub wait_seconds: Option<f64>,
}

impl RunParams {
    pub fn targets(&self) -> Result<Targets, String> {
        let non_empty = |list: &Option<Vec<String>>| list.as_ref().is_some_and(|l| !l.is_empty());
        let chosen = [
            self.all == Some(true),
            non_empty(&self.devices),
            non_empty(&self.instances),
        ];
        if chosen.iter().filter(|c| **c).count() != 1 {
            return Err(
                "a target is required: give exactly one of all = true, devices or instances".into(),
            );
        }
        Ok(if self.all == Some(true) {
            Targets::All
        } else if let Some(devices) = self.devices.clone().filter(|l| !l.is_empty()) {
            Targets::Devices(devices)
        } else {
            Targets::Instances(self.instances.clone().unwrap_or_default())
        })
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PinParams {
    pub instance: String,
    /// Field name, e.g. `co2`.
    pub field: String,
    /// The value to publish: a number, `true`/`false` or a string.
    pub value: Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UnpinParams {
    pub instance: String,
    /// The field to unpin; without it, every pinned field of the instance.
    pub field: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetProjectParams {
    /// The whole project file as TOML.
    pub toml: String,
    /// Only check the text and report its problems; change nothing.
    pub dry_run: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RenameParams {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct IdParams {
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CopyBuiltinParams {
    /// Built-in template ID: `co2`, `temperature` or `humidity`.
    pub builtin: String,
    /// The copy's ID in the project.
    pub new_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NewProjectParams {
    /// Throw away unsaved changes.
    pub discard_unsaved: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenProjectParams {
    /// Project file to open; relative paths start at Senzor's working directory.
    pub path: String,
    /// Throw away unsaved changes.
    pub discard_unsaved: Option<bool>,
    /// Start the devices with `autostart = true` (default true).
    pub autostart: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SaveProjectParams {
    /// Save as this file (`.toml` is added when missing), which becomes the project's file.
    /// Without it, save to the project's file.
    pub path: Option<String>,
    /// Replace an existing file other than the project's own.
    pub overwrite: Option<bool>,
}

/// `seconds` as a wait, clamped to `0..=max`; `None` gives `default`.
pub fn wait(seconds: Option<f64>, default: f64, max: f64) -> std::time::Duration {
    let seconds = seconds.unwrap_or(default);
    let seconds = if seconds.is_finite() {
        seconds
    } else {
        default
    };
    std::time::Duration::from_secs_f64(seconds.clamp(0.0, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(all: Option<bool>, devices: &[&str], instances: &[&str]) -> RunParams {
        let list =
            |ids: &[&str]| (!ids.is_empty()).then(|| ids.iter().map(|s| s.to_string()).collect());
        RunParams {
            all,
            devices: list(devices),
            instances: list(instances),
            wait_seconds: None,
        }
    }

    #[test]
    fn exactly_one_target_is_required() {
        assert_eq!(params(Some(true), &[], &[]).targets(), Ok(Targets::All));
        assert_eq!(
            params(None, &["a"], &[]).targets(),
            Ok(Targets::Devices(vec!["a".into()]))
        );
        assert_eq!(
            params(Some(false), &[], &["a-0"]).targets(),
            Ok(Targets::Instances(vec!["a-0".into()]))
        );
        for bad in [
            params(None, &[], &[]),
            params(Some(false), &[], &[]),
            params(Some(true), &["a"], &[]),
            params(None, &["a"], &["a-0"]),
        ] {
            assert!(
                bad.targets()
                    .unwrap_err()
                    .starts_with("a target is required")
            );
        }
    }

    #[test]
    fn waits_are_clamped() {
        use std::time::Duration;
        assert_eq!(wait(None, 5.0, 60.0), Duration::from_secs(5));
        assert_eq!(wait(Some(600.0), 5.0, 60.0), Duration::from_secs(60));
        assert_eq!(wait(Some(-1.0), 5.0, 60.0), Duration::ZERO);
        assert_eq!(wait(Some(f64::NAN), 5.0, 60.0), Duration::from_secs(5));
    }
}
