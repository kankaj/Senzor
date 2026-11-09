//! Whether a running instance can take a changed configuration while it stays connected.

use std::time::Duration;

use crate::project::{BrokerConfig, InstanceConfig};
use crate::render::base_context;

/// How a running instance takes the configuration it should now run with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Apply {
    /// The configuration is the same.
    Unchanged,
    /// The running instance takes the new configuration at once and stays connected: messages,
    /// fields and their scripts, interval, QoS, retain, params, seed, Home Assistant and
    /// subscriptions (it unsubscribes from the filters that are gone and subscribes to new ones).
    InPlace,
    /// The new configuration needs a new connection, because the broker settings or the Last Will
    /// (the availability topic and its payloads) changed: the instance needs a restart.
    Restart,
}

impl Apply {
    /// Compares what an instance runs with (`running`, connected with `running_broker`) with what
    /// it should run with (`config` and `broker`).
    pub fn between(
        running_broker: &BrokerConfig,
        running: &InstanceConfig,
        broker: &BrokerConfig,
        config: &InstanceConfig,
    ) -> Self {
        if running_broker != broker || LastWill::of(running) != LastWill::of(config) {
            Apply::Restart
        } else if running != config {
            Apply::InPlace
        } else {
            Apply::Unchanged
        }
    }
}

/// What an instance's Last Will is made of, fixed when it connects.
#[derive(Debug, PartialEq)]
struct LastWill<'a> {
    topic: Result<Option<String>, String>,
    online: Option<&'a str>,
    offline: Option<&'a str>,
    will_delay: Option<Duration>,
}

impl<'a> LastWill<'a> {
    fn of(config: &'a InstanceConfig) -> Self {
        let base = base_context(
            &config.id,
            config.index,
            &config.template_id,
            &config.params,
        );
        let spec = config.availability.as_ref();
        Self {
            topic: config.templates.availability_topic(&base),
            online: spec.map(|a| a.online.as_str()),
            offline: spec.map(|a| a.offline.as_str()),
            will_delay: spec.and_then(|a| a.will_delay),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::valid_project;
    use super::*;
    use crate::project::Project;

    const BASE: &str = r#"
[broker]
host = "127.0.0.1"
port = 1883
session_expiry = "60s"

[[templates]]
id = "air"
interval = "1s"
availability = { topic = "a/{{ device_id }}/status", online = "on", offline = "off" }
messages = [{ topic = "a/{{ device_id }}/state", payload = "{{ v }}" }]
fields.v = { generator = "constant", value = 1 }

[[templates]]
id = "other"
interval = "1s"
availability = { topic = "a/{{ device_id }}/status", online = "on", offline = "off" }
messages = [{ topic = "b/{{ device_id }}", payload = "{{ w }}" }]
fields.w = { generator = "constant", value = 2 }

[[devices]]
id = "d"
template = "air"
"#;

    fn project(src: &str) -> Project {
        valid_project(src)
    }

    /// How instance `d` of `BASE` takes `BASE` with `from` replaced by `to`.
    fn apply(from: &str, to: &str) -> Apply {
        assert!(BASE.contains(from), "{from}");
        let old = project(BASE);
        let new = project(&BASE.replacen(from, to, 1));
        Apply::between(
            &old.broker,
            old.instance("d").unwrap(),
            &new.broker,
            new.instance("d").unwrap(),
        )
    }

    #[test]
    fn the_same_configuration_is_unchanged() {
        assert_eq!(apply("id = \"d\"", "id = \"d\""), Apply::Unchanged);
    }

    #[test]
    fn what_is_published_and_how_often_changes_in_place() {
        for (from, to) in [
            ("payload = \"{{ v }}\"", "payload = \"v={{ v }}\""),
            ("a/{{ device_id }}/state", "a/{{ device_id }}/data"),
            ("value = 1 }", "value = 5 }"),
            (
                "fields.v = { generator = \"constant\", value = 1 }",
                "fields.v = { generator = \"constant\", value = 1, unit = \"ppm\", name = \"V\" }",
            ),
            ("interval = \"1s\"", "interval = \"5s\""),
            (
                "interval = \"1s\"",
                "interval = \"1s\"\nqos = 1\nretain = true",
            ),
            (
                "interval = \"1s\"",
                "interval = \"1s\"\nhome_assistant = true",
            ),
            (
                "template = \"air\"",
                "template = \"air\"\nparams = { room = \"x\" }",
            ),
            ("template = \"air\"", "template = \"other\""),
            ("[[templates]]", "[simulation]\nseed = 7\n\n[[templates]]"),
            (
                "fields.v = { generator = \"constant\", value = 1 }",
                "fields.v = { generator = \"script\", script = \"1\" }",
            ),
            (
                "[[templates]]\nid = \"other\"",
                "[[templates.subscriptions]]\ntopic = \"c/{{ device_id }}\"\nscript = \"1\"\n\n\
                 [[templates]]\nid = \"other\"",
            ),
        ] {
            assert_eq!(apply(from, to), Apply::InPlace, "{from} -> {to}");
        }
    }

    #[test]
    fn a_broker_change_needs_a_restart() {
        for to in [
            "port = 1884",
            "port = 1883\nkeep_alive = \"10s\"",
            "port = 1883\nclient_id_prefix = \"x-\"",
            "port = 1883\nusername = \"u\"\npassword = \"p\"",
        ] {
            assert_eq!(apply("port = 1883", to), Apply::Restart, "{to}");
        }
        for to in ["session_expiry = \"90s\"", "mqtt_version = \"3.1.1\""] {
            assert_eq!(
                apply("session_expiry = \"60s\"", to),
                Apply::Restart,
                "{to}"
            );
        }
    }

    #[test]
    fn a_last_will_change_needs_a_restart() {
        for (from, to) in [
            ("a/{{ device_id }}/status", "a/{{ device_id }}/up"),
            ("online = \"on\"", "online = \"yes\""),
            ("offline = \"off\"", "offline = \"no\""),
            (
                "offline = \"off\" }",
                "offline = \"off\", will_delay = \"5s\" }",
            ),
            (
                "availability = { topic = \"a/{{ device_id }}/status\", online = \"on\", offline = \"off\" }\nmessages = [{ topic = \"a/",
                "messages = [{ topic = \"a/",
            ),
        ] {
            assert_eq!(apply(from, to), Apply::Restart, "{from} -> {to}");
        }
    }

    #[test]
    fn a_param_in_the_availability_topic_needs_a_restart() {
        let src = BASE
            .replace("a/{{ device_id }}/status", "a/{{ room }}/status")
            .replace(
                "template = \"air\"",
                "template = \"air\"\nparams = { room = \"x\" }",
            );
        let old = project(&src);
        let new = project(&src.replace("room = \"x\"", "room = \"y\""));
        let between = |new: &Project| {
            Apply::between(
                &old.broker,
                old.instance("d").unwrap(),
                &new.broker,
                new.instance("d").unwrap(),
            )
        };
        assert_eq!(between(&new), Apply::Restart);
        let unused = project(&src.replace("room = \"x\"", "room = \"x\", floor = 1"));
        assert_eq!(between(&unused), Apply::InPlace, "a param it does not use");
    }

    #[test]
    fn a_change_of_both_kinds_needs_a_restart() {
        let old = project(BASE);
        let new = project(
            &BASE
                .replace("value = 1 }", "value = 5 }")
                .replace("online = \"on\"", "online = \"yes\""),
        );
        assert_eq!(
            Apply::between(
                &old.broker,
                old.instance("d").unwrap(),
                &new.broker,
                new.instance("d").unwrap(),
            ),
            Apply::Restart
        );
    }
}
