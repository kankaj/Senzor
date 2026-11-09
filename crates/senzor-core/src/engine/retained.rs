//! Clears the retained messages a project's instances leave on the broker.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, SystemTime};

use super::mqtt::{ConnError, ConnEvent, connect};
use super::probe::{connect_failure, no_answer};
use crate::project::{BrokerConfig, InstanceConfig, Project};
use crate::render::{MessageProperties, base_context, tick_context};
use crate::value::Value;

/// How long clearing retained messages waits for the broker.
pub const CLEAR_RETAINED_TIMEOUT: Duration = Duration::from_secs(10);

/// The client ID that clears, after the broker's client ID prefix.
const CLIENT_ID: &str = "clear-retained";

/// How long the DISCONNECT after the last acknowledgement may take.
const DISCONNECT_TIMEOUT: Duration = Duration::from_secs(1);

/// Every topic on which the instances of `devices` (all devices when `None`) can leave a retained
/// message, sorted and without duplicates: the availability topic of each instance whose template
/// has availability, the message topics of each instance whose template has `retain = true`, and
/// the Home Assistant discovery topics of each instance whose template has `home_assistant = true`.
/// Device IDs that are not in the project are ignored.
///
/// Topics are rendered as validation renders them: with each field's initial value, `seq` 0 and
/// the current time. A topic that uses a field, `seq` or the time therefore lists only that one
/// rendering, not the topics other values lead to. A topic that cannot be rendered is left out.
/// Devices blocked by validation problems are not in a [`Project`], so their topics are not listed.
pub fn retained_topics(project: &Project, devices: Option<&[String]>) -> Vec<String> {
    let mut topics = BTreeSet::new();
    let selected = project
        .devices
        .iter()
        .filter(|device| devices.is_none_or(|ids| ids.contains(&device.id)));
    for instance in selected.flat_map(|device| &device.instances) {
        topics.extend(instance_topics(instance));
    }
    topics.into_iter().collect()
}

/// The topics on which `instance` can leave a retained message.
fn instance_topics(instance: &InstanceConfig) -> Vec<String> {
    let base = base_context(
        &instance.id,
        instance.index,
        &instance.template_id,
        &instance.params,
    );
    let mut topics: Vec<String> = instance
        .templates
        .availability_topic(&base)
        .ok()
        .flatten()
        .into_iter()
        .collect();
    if instance.retain {
        let values: BTreeMap<String, Value> = instance
            .fields
            .iter()
            .map(|(name, field)| (name.clone(), field.generator.initial_value()))
            .collect();
        let ctx = tick_context(&base, &values, SystemTime::now(), 0);
        topics.extend(instance.templates.render_topics(&ctx).into_iter().flatten());
    }
    topics.extend(super::discovery::topics(instance));
    topics
}

/// How far clearing got.
#[derive(Default)]
struct Progress {
    /// Whether the broker accepted the connection.
    accepted: bool,
    /// Publishes the broker acknowledged.
    acknowledged: usize,
    /// Acknowledged publishes the broker did not accept (MQTT 5 only).
    refused: usize,
    /// The broker's reasons for those, e.g. `NotAuthorized`.
    reasons: BTreeSet<String>,
}

/// Deletes the retained message on each of `topics`: connects once with `broker` (without a Last
/// Will), publishes an empty retained QoS 1 message to each, which tells the broker to delete the
/// retained message there, waits until the broker acknowledged every one, then disconnects.
/// A topic without a retained message is not an error. With no topics it does not connect.
///
/// `timeout` covers the whole run. `Err` says why it failed: a connection error in the words a
/// failed instance uses; `no answer from <host>:<port> within <n> s` when the broker did not
/// accept the connection in time; how many topics the broker acknowledged when it did not
/// acknowledge every one in time or the connection dropped; on MQTT 5, the reasons the broker
/// gave for the publishes it refused, e.g. `NotAuthorized`.
pub async fn clear_retained(
    broker: &BrokerConfig,
    topics: &[String],
    timeout: Duration,
) -> Result<(), String> {
    if topics.is_empty() {
        return Ok(());
    }
    let total = topics.len();
    // Room for every publish and the DISCONNECT, queued without waiting.
    let (client, mut connection) = connect(broker, CLIENT_ID, None, total + 1);
    let mut progress = Progress::default();
    let attempt = async {
        loop {
            match connection.poll().await {
                Ok(ConnEvent::ConnAck { .. }) => {
                    progress.accepted = true;
                    for topic in topics {
                        client
                            .try_publish(topic, 1, true, "", &MessageProperties::default())
                            .map_err(|e| format!("publish: {e}"))?;
                    }
                }
                Ok(ConnEvent::PubAck { refused }) => {
                    progress.acknowledged += 1;
                    if let Some(reason) = refused {
                        progress.refused += 1;
                        progress.reasons.insert(reason);
                    }
                    if progress.acknowledged == total {
                        client.try_disconnect();
                        // Sends the DISCONNECT; the broker then closes the connection.
                        let _ = tokio::time::timeout(DISCONNECT_TIMEOUT, connection.poll()).await;
                        return Ok(());
                    }
                }
                Ok(ConnEvent::Other) => {}
                Err(error) if !progress.accepted => {
                    return Err(connect_failure(broker, CLIENT_ID, error).await);
                }
                Err(error) => {
                    return Err(format!(
                        "the connection dropped after the broker acknowledged {} of {}: {}",
                        progress.acknowledged,
                        count_topics(total),
                        error_text(error)
                    ));
                }
            }
        }
    };
    let result = tokio::time::timeout(timeout, attempt).await;
    match result {
        Ok(Ok(())) if progress.refused == 0 => Ok(()),
        Ok(Ok(())) => Err(format!(
            "the broker refused to clear {} of {}: {}",
            progress.refused,
            count_topics(total),
            progress.reasons.into_iter().collect::<Vec<_>>().join(", ")
        )),
        Ok(Err(error)) => Err(error),
        Err(_) if !progress.accepted => Err(no_answer(broker, timeout)),
        Err(_) => Err(format!(
            "the broker at {}:{} acknowledged {} of {} within {} s",
            broker.host,
            broker.port,
            progress.acknowledged,
            count_topics(total),
            timeout.as_secs_f32()
        )),
    }
}

/// `1 topic`, `2 topics`.
fn count_topics(count: usize) -> String {
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} topic{plural}")
}

/// The text of a connection error after the broker had accepted the connection.
fn error_text(error: ConnError) -> String {
    match error {
        ConnError::Refused(text) | ConnError::Aborted(text) | ConnError::Other(text) => text,
        ConnError::UnsupportedVersion => "unsupported protocol version".into(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rumqttc::ConnectReturnCode;
    use senzor_testkit::{
        Received, Subscriber, free_port, refusing_broker, slow_broker, start_broker_ports,
    };
    use tokio::runtime::Handle;

    use super::super::test_support::valid_project;
    use super::super::{Command, Engine, Target};
    use super::*;

    /// Template `air` publishes retained data and has availability; `plain` publishes data that is
    /// not retained, without availability; `shared` is retained and every instance publishes on
    /// the same topic.
    fn project() -> Project {
        valid_project(
            r#"
[broker]
host = "127.0.0.1"
port = 1883

[[templates]]
id = "air"
interval = "100ms"
retain = true
availability = { topic = "rt/{{ device_id }}/status", online = "online", offline = "offline" }
messages = [
  { topic = "rt/{{ device_id }}/state", payload = "{{ v }}" },
  { topic = "rt/{{ room }}/{{ v }}/level", payload = "{{ v }}" },
]
fields.v = { generator = "constant", value = 7 }

[[templates]]
id = "plain"
interval = "100ms"
messages = [{ topic = "rt/plain/{{ device_id }}", payload = "1" }]
fields.v = { generator = "constant", value = 1 }

[[templates]]
id = "shared"
interval = "100ms"
retain = true
messages = [{ topic = "rt/shared", payload = "1" }]
fields.v = { generator = "constant", value = 1 }

[[devices]]
id = "a"
template = "air"
count = 2
params = { room = "lab" }

[[devices]]
id = "p"
template = "plain"

[[devices]]
id = "s"
template = "shared"
count = 3
"#,
        )
    }

    fn strings(topics: &[&str]) -> Vec<String> {
        topics.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn home_assistant_discovery_topics_are_listed() {
        let mut project = project();
        project.devices[1].instances[0].home_assistant = true;
        let topics = retained_topics(&project, Some(&["p".to_string()]));
        assert_eq!(topics, strings(&["homeassistant/sensor/p/v/config"]));
    }

    #[test]
    fn availability_and_retained_data_topics_are_listed_sorted_and_once() {
        let topics = retained_topics(&project(), None);
        assert_eq!(
            topics,
            strings(&[
                "rt/a-0/state",
                "rt/a-0/status",
                "rt/a-1/state",
                "rt/a-1/status",
                // Rendered with the field's initial value.
                "rt/lab/7/level",
                "rt/shared",
            ])
        );
    }

    #[test]
    fn only_the_named_devices_are_listed() {
        let project = project();
        assert_eq!(
            retained_topics(&project, Some(&strings(&["s"]))),
            strings(&["rt/shared"])
        );
        assert_eq!(
            retained_topics(&project, Some(&strings(&["p", "ghost"]))),
            Vec::<String>::new()
        );
        assert_eq!(retained_topics(&project, Some(&[])), Vec::<String>::new());
    }

    /// A project whose only device `a` publishes retained data and availability on `<prefix>/…`.
    fn small_project(port: u16, prefix: &str, broker: &str) -> Project {
        valid_project(&format!(
            r#"
[broker]
host = "127.0.0.1"
port = {port}
{broker}

[[templates]]
id = "t"
interval = "100ms"
retain = true
availability = {{ topic = "{prefix}/{{{{ device_id }}}}/status", online = "online", offline = "offline" }}
messages = [{{ topic = "{prefix}/{{{{ device_id }}}}/state", payload = "1" }}]
fields.v = {{ generator = "constant", value = 1 }}

[[devices]]
id = "a"
template = "t"
"#
        ))
    }

    /// The retained messages a new subscriber to `filter` gets.
    async fn retained(port: u16, filter: &str) -> Vec<Received> {
        let mut subscriber = Subscriber::connect(port, filter).await;
        let mut messages = subscriber.collect(Duration::from_millis(500)).await;
        messages.retain(|m| m.retain);
        messages
    }

    /// Runs device `a` until both its messages arrived, then stops it: the broker keeps its
    /// state and its `offline`.
    async fn leave_retained_messages(project: Project, subscriber_port: u16, prefix: &str) {
        let mut subscriber = Subscriber::connect(subscriber_port, &format!("{prefix}/#")).await;
        let engine = Engine::spawn(project, &Handle::current(), Arc::new(|| {}));
        engine.send(Command::Start(Target::Device("a".into())));
        subscriber
            .wait_for(Duration::from_secs(5), |m| m.topic.ends_with("/state"))
            .await
            .expect("state published");
        engine.shutdown().await;
        let mut topics: Vec<String> = retained(subscriber_port, &format!("{prefix}/#"))
            .await
            .into_iter()
            .map(|m| m.topic)
            .collect();
        topics.sort();
        assert_eq!(
            topics,
            [format!("{prefix}/a/state"), format!("{prefix}/a/status")]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn clearing_deletes_the_retained_messages_on_mqtt_5() {
        let ports = start_broker_ports();
        let project = small_project(ports.v5, "c5", "");
        leave_retained_messages(project.clone(), ports.v5, "c5").await;

        let topics = retained_topics(&project, None);
        clear_retained(&project.broker, &topics, CLEAR_RETAINED_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(retained(ports.v5, "c5/#").await, []);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn clearing_deletes_the_retained_messages_on_mqtt_3_1_1() {
        let ports = start_broker_ports();
        let project = small_project(ports.v311, "c3", "mqtt_version = \"3.1.1\"");
        leave_retained_messages(project.clone(), ports.v5, "c3").await;

        let topics = retained_topics(&project, None);
        clear_retained(&project.broker, &topics, CLEAR_RETAINED_TIMEOUT)
            .await
            .unwrap();
        assert_eq!(retained(ports.v5, "c3/#").await, []);
    }

    #[tokio::test]
    async fn nothing_to_clear_does_not_connect() {
        let project = small_project(free_port(), "none", "");
        assert_eq!(
            clear_retained(&project.broker, &[], CLEAR_RETAINED_TIMEOUT).await,
            Ok(())
        );
    }

    #[tokio::test]
    async fn a_refusal_says_why() {
        let port = refusing_broker(ConnectReturnCode::NotAuthorized).await;
        let project = small_project(port, "r", "");
        let error = clear_retained(&project.broker, &strings(&["r/a"]), CLEAR_RETAINED_TIMEOUT)
            .await
            .unwrap_err();
        assert_eq!(error, "broker refused the connection: NotAuthorized");
    }

    #[tokio::test]
    async fn nothing_listening_fails_at_once() {
        let project = small_project(free_port(), "u", "");
        let error = clear_retained(&project.broker, &strings(&["u/a"]), CLEAR_RETAINED_TIMEOUT)
            .await
            .unwrap_err();
        assert!(!error.contains("no answer"), "{error}");
    }

    #[tokio::test]
    async fn a_broker_that_does_not_answer_runs_into_the_timeout() {
        let port = slow_broker(Duration::from_secs(5)).await;
        let project = small_project(port, "t", "");
        let error = clear_retained(
            &project.broker,
            &strings(&["t/a"]),
            Duration::from_millis(300),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            format!("no answer from 127.0.0.1:{port} within 0.3 s")
        );
    }

    #[tokio::test]
    async fn a_broker_that_accepts_but_does_not_acknowledge_says_how_far_it_got() {
        // The slow broker accepts at once when the delay is zero, then ignores every publish.
        let port = slow_broker(Duration::ZERO).await;
        let project = small_project(port, "n", "");
        let error = clear_retained(
            &project.broker,
            &strings(&["n/a", "n/b"]),
            Duration::from_millis(300),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            format!("the broker at 127.0.0.1:{port} acknowledged 0 of 2 topics within 0.3 s")
        );
    }
}
