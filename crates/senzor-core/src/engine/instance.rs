use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use rand::RngExt;
use rand::rngs::ChaCha8Rng;
use tokio::sync::mpsc;
use tokio::time::{Instant, Interval, MissedTickBehavior};

use super::mqtt::{
    Client, ConnError, ConnEvent, Connection, MAX_MESSAGE_BYTES, PUBLISH_HEADER_BYTES, Will,
    answers_mqtt311, backoff, connect,
};
use super::state::{InstanceStatus, LogEntry, Shared};
use crate::generator::{TickContext, ValueSource};
use crate::project::{BrokerConfig, InstanceConfig, MqttVersion, mqtt_seconds};
use crate::render::{Context, MessageProperties, RenderedMessage, base_context, tick_context};
use crate::rng::instance_rng;
use crate::value::Value;

/// How long a graceful stop may take before the connection is dropped.
pub(crate) const STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// Why an MQTT 5 instance ends **Failed** when the broker refuses MQTT 5.
pub(super) const MQTT5_UNSUPPORTED: &str = "the broker does not support MQTT 5; set mqtt_version = \"3.1.1\" \
    in [broker], or choose 3.1.1 under MQTT version in the broker settings";

/// Appended to an MQTT 5 connection over TLS that the broker closed before accepting it: that may
/// be a 3.1.1-only broker, which [`answers_mqtt311`] cannot check over TLS.
pub(super) const MAYBE_MQTT311_ONLY: &str = "the broker closed the connection before accepting it; if it \
    supports only MQTT 3.1.1, set mqtt_version = \"3.1.1\" in [broker], or choose 3.1.1 under MQTT \
    version in the broker settings";

/// Requests rumqttc queues for the event loop. A tick never waits for room: once the queue is
/// full, `try_publish` fails and the message counts as an error.
const REQUEST_QUEUE_CAPACITY: usize = 64;

#[derive(Debug)]
pub(crate) enum InstanceCmd {
    /// Publish `offline`, DISCONNECT: the broker does not send the Last Will.
    Stop,
    /// Drop the socket: the broker sends the Last Will.
    Kill,
    Pin(String, Value),
    Unpin(String),
    #[cfg(test)]
    Panic,
}

/// Runs one instance until it is stopped, killed or fails. Leaves the final status in `shared`.
pub(crate) async fn run_instance(
    config: InstanceConfig,
    broker: BrokerConfig,
    mut commands: mpsc::UnboundedReceiver<InstanceCmd>,
    shared: Shared,
) {
    shared.update_instance(&config.id, |instance| {
        instance.values.clear();
        instance.pinned.clear();
    });
    let status = match run(&config, &broker, &mut commands, &shared).await {
        Ok(()) => InstanceStatus::Stopped,
        Err(reason) => InstanceStatus::Failed { reason },
    };
    shared.update_instance(&config.id, |instance| instance.pinned.clear());
    shared.set_status(&config.id, status);
}

/// An instance's availability topic. Its messages, the Last Will included, are QoS 1 and retained.
struct Availability {
    topic: String,
    online: String,
    offline: String,
    /// MQTT 5 will delay in seconds.
    will_delay: Option<u32>,
}

impl Availability {
    /// The broker publishes this `offline` when the connection drops without DISCONNECT (after
    /// `will_delay` on MQTT 5); a graceful stop publishes the same message itself.
    fn last_will(&self) -> Will {
        Will {
            topic: self.topic.clone(),
            payload: self.offline.clone(),
            delay: self.will_delay,
        }
    }

    fn publish_online(&self, client: &Client) -> Result<(), String> {
        self.publish(client, &self.online)
    }

    fn publish_offline(&self, client: &Client) -> Result<(), String> {
        self.publish(client, &self.offline)
    }

    fn publish(&self, client: &Client, payload: &str) -> Result<(), String> {
        client.try_publish(&self.topic, 1, true, payload, &MessageProperties::default())
    }
}

async fn run(
    config: &InstanceConfig,
    broker: &BrokerConfig,
    commands: &mut mpsc::UnboundedReceiver<InstanceCmd>,
    shared: &Shared,
) -> Result<(), String> {
    let id = config.id.as_str();
    let base = base_context(id, config.index, &config.template_id, &config.params);
    let availability = config
        .templates
        .availability_topic(&base)?
        .zip(config.availability.as_ref())
        .map(|(topic, spec)| Availability {
            topic,
            online: spec.online.clone(),
            offline: spec.offline.clone(),
            will_delay: spec.will_delay.map(mqtt_seconds),
        });
    let will = availability.as_ref().map(Availability::last_will);
    let (client, mut connection) = connect(broker, id, will, REQUEST_QUEUE_CAPACITY);
    let mut ticker = Ticker::new(config, base);
    let mut timer = ticker.timer();

    shared.set_status(id, InstanceStatus::Connecting);
    let mut online = false;
    // Whether the broker has accepted a connection since this run started.
    let mut accepted_once = false;
    let mut attempt = 0u32;
    let mut retry_at: Option<Instant> = None;
    loop {
        tokio::select! {
            event = connection.poll(), if retry_at.is_none() => match event {
                Ok(ConnEvent::ConnAck { max_packet_size }) => {
                    online = true;
                    accepted_once = true;
                    attempt = 0;
                    ticker.broker_max_packet_size = max_packet_size;
                    shared.set_status(id, InstanceStatus::Online);
                    if let Some(a) = &availability
                        && let Err(e) = a.publish_online(&client)
                    {
                        shared.record_error(id, &format!("publish online: {e}"));
                    }
                }
                Ok(ConnEvent::Other) => {}
                Err(ConnError::Refused(reason)) => {
                    return Err(format!("broker refused the connection: {reason}"));
                }
                Err(ConnError::UnsupportedVersion) => return Err(MQTT5_UNSUPPORTED.to_string()),
                Err(ConnError::Aborted(text)) if !accepted_once && broker.mqtt_version == MqttVersion::V5 => {
                    let last_error = match answers_mqtt311(broker, id).await {
                        Some(true) => return Err(MQTT5_UNSUPPORTED.to_string()),
                        Some(false) => text,
                        None => format!("{text}; {MAYBE_MQTT311_ONLY}"),
                    };
                    online = false;
                    attempt += 1;
                    shared.set_status(id, InstanceStatus::Reconnecting { attempt, last_error });
                    retry_at = Some(Instant::now() + backoff(attempt));
                }
                Err(ConnError::Aborted(last_error) | ConnError::Other(last_error)) => {
                    online = false;
                    attempt += 1;
                    shared.set_status(id, InstanceStatus::Reconnecting { attempt, last_error });
                    retry_at = Some(Instant::now() + backoff(attempt));
                }
            },
            // `select!` evaluates a disabled branch's expression too, so it needs some deadline.
            _ = tokio::time::sleep_until(retry_at.unwrap_or_else(Instant::now)), if retry_at.is_some() => {
                retry_at = None;
            }
            // Only while online: a tick firing mid-connect would drop the `poll()` future,
            // which restarts rumqttc's connect from scratch.
            _ = timer.tick(), if online => {
                ticker.tick(&client, shared);
            }
            command = commands.recv() => match command {
                Some(InstanceCmd::Stop) | None => {
                    if online {
                        stop_gracefully(&client, &mut connection, availability.as_ref()).await;
                    }
                    return Ok(());
                }
                Some(InstanceCmd::Kill) => return Ok(()),
                Some(InstanceCmd::Pin(field, value)) => ticker.pin(shared, field, value),
                Some(InstanceCmd::Unpin(field)) => ticker.unpin(shared, field),
                #[cfg(test)]
                Some(InstanceCmd::Panic) => panic!("test panic"),
            },
        }
    }
}

/// Per-run state that produces and publishes one tick.
struct Ticker<'a> {
    config: &'a InstanceConfig,
    base: Context,
    sources: BTreeMap<String, Box<dyn ValueSource>>,
    pins: BTreeMap<String, Value>,
    rng: ChaCha8Rng,
    started: Instant,
    seq: u64,
    /// The largest packet the broker accepts, from its MQTT 5 CONNACK.
    broker_max_packet_size: Option<u32>,
}

impl<'a> Ticker<'a> {
    fn new(config: &'a InstanceConfig, base: Context) -> Self {
        Self {
            config,
            base,
            sources: config
                .fields
                .iter()
                .map(|(name, spec)| (name.clone(), spec.build()))
                .collect(),
            pins: BTreeMap::new(),
            rng: instance_rng(config.seed, &config.id),
            started: Instant::now(),
            seq: 0,
            broker_max_packet_size: None,
        }
    }

    /// The tick timer. Its first tick comes at a random offset within one interval, so instances
    /// started together do not publish in lockstep. Call it right after `new`: the offset is the
    /// first draw from the instance RNG, which keeps seeded runs repeatable.
    fn timer(&mut self) -> Interval {
        // At least 1 ms, so the offset range is not empty for sub-millisecond intervals.
        let interval_ms = self.config.interval.as_millis().max(1) as u64;
        let offset = Duration::from_millis(self.rng.random_range(0..interval_ms));
        let mut timer = tokio::time::interval_at(self.started + offset, self.config.interval);
        timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
        timer
    }

    /// Publishes `value` for `field` instead of generating it; unknown fields are ignored.
    fn pin(&mut self, shared: &Shared, field: String, value: Value) {
        if !self.config.fields.contains_key(&field) {
            return;
        }
        shared.update_instance(&self.config.id, |instance| {
            instance.pinned.insert(field.clone());
            instance.values.insert(field.clone(), value.clone());
        });
        self.pins.insert(field, value);
    }

    fn unpin(&mut self, shared: &Shared, field: String) {
        shared.update_instance(&self.config.id, |instance| {
            instance.pinned.remove(&field);
        });
        self.pins.remove(&field);
    }

    fn tick(&mut self, client: &Client, shared: &Shared) {
        let id = self.config.id.as_str();
        let now = SystemTime::now();
        let values = self.next_values(now);
        let render_ctx = tick_context(&self.base, &values, now, self.seq);
        shared.update_instance(id, |instance| instance.values = values);
        self.seq += 1;

        for rendered in self.config.templates.render_messages(&render_ctx) {
            let published = rendered
                .map_err(|e| format!("render: {e}"))
                .and_then(|message| self.publish(client, message, now));
            match published {
                Ok(entry) => shared.record_publish(entry),
                Err(e) => shared.record_error(id, &e),
            }
        }
    }

    /// This tick's field values: pinned ones as pinned, the others from their generators.
    fn next_values(&mut self, now: SystemTime) -> BTreeMap<String, Value> {
        let mut ctx = TickContext {
            now,
            elapsed: self.started.elapsed(),
            seq: self.seq,
            rng: &mut self.rng,
        };
        self.sources
            .iter_mut()
            .map(|(name, source)| {
                let value = match self.pins.get(name) {
                    Some(pinned) => pinned.clone(),
                    None => source.next(&mut ctx),
                };
                (name.clone(), value)
            })
            .collect()
    }

    /// Publishes one message and returns its log entry.
    fn publish(
        &self,
        client: &Client,
        message: RenderedMessage,
        now: SystemTime,
    ) -> Result<LogEntry, String> {
        let id = self.config.id.as_str();
        let size = message.topic.len() + message.payload.len() + message.properties.encoded_len();
        let limit = message_limit(self.broker_max_packet_size);
        if size > limit {
            return Err(format!(
                "message on `{}` is {size} bytes; the limit is {limit}",
                message.topic
            ));
        }
        client
            .try_publish(
                &message.topic,
                self.config.qos,
                self.config.retain,
                &message.payload,
                &message.properties,
            )
            .map_err(|e| format!("publish: {e}"))?;
        // `Option` fields are logged only when `Some`.
        let properties =
            (!message.properties.is_empty()).then(|| tracing::field::debug(&message.properties));
        tracing::debug!(instance = %id, topic = %message.topic, payload = %message.payload, properties, "publish");
        Ok(LogEntry {
            // `record_publish` numbers it.
            seq: 0,
            time: now,
            instance: id.to_string(),
            topic: message.topic,
            payload: message.payload,
            qos: self.config.qos,
            retain: self.config.retain,
            properties: message.properties,
        })
    }
}

/// The largest message (topic, payload and properties) an instance publishes: 1 MiB, or less when
/// the broker's MQTT 5 CONNACK limits the packet size. A larger message counts as an error;
/// rumqttc would treat the packet as a connection error and reconnect.
fn message_limit(broker_max_packet_size: Option<u32>) -> usize {
    match broker_max_packet_size {
        Some(max) => (max as usize)
            .saturating_sub(PUBLISH_HEADER_BYTES)
            .min(MAX_MESSAGE_BYTES),
        None => MAX_MESSAGE_BYTES,
    }
}

/// Publishes `offline`, sends DISCONNECT and keeps polling until the broker closes the connection.
async fn stop_gracefully(
    client: &Client,
    connection: &mut Connection,
    availability: Option<&Availability>,
) {
    if let Some(a) = availability {
        let _ = a.publish_offline(client);
    }
    client.try_disconnect();
    let _ = tokio::time::timeout(STOP_TIMEOUT, async {
        while connection.poll().await.is_ok() {}
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::state::InstanceState;
    use crate::engine::test_support::{WAIT, eventually, valid_project};
    use crate::project::{MqttVersion, Project};
    use rumqttc::ConnectReturnCode;
    use senzor_testkit::{
        BrokerPorts, Protocol, Subscriber, closing_broker, free_port, refusing_broker, slow_broker,
        start_broker_ports,
    };

    fn protocol(version: MqttVersion) -> Protocol {
        match version {
            MqttVersion::V311 => Protocol::V311,
            MqttVersion::V5 => Protocol::V5,
        }
    }

    /// A broker with both listeners: instances use the one for `version`, subscribers MQTT 5.
    fn broker(version: MqttVersion) -> (BrokerPorts, u16) {
        let ports = start_broker_ports();
        (ports, ports.port(protocol(version)))
    }

    fn project(port: u16, version: MqttVersion, extra_device_toml: &str) -> Project {
        let version = version.label();
        let src = format!(
            r#"
[broker]
host = "127.0.0.1"
port = {port}
mqtt_version = "{version}"

[simulation]
seed = 1

[[templates]]
id = "air"
interval = "100ms"
availability = {{ topic = "it/{{{{ device_id }}}}/status", online = "online", offline = "offline" }}
messages = [{{ topic = "it/{{{{ device_id }}}}/state", payload = '{{"co2": {{{{ co2 | round | int }}}}}}' }}]
fields.co2 = {{ generator = "constant", value = 600 }}

{extra_device_toml}
"#
        );
        valid_project(&src)
    }

    struct Running {
        commands: mpsc::UnboundedSender<InstanceCmd>,
        shared: Shared,
        task: tokio::task::JoinHandle<()>,
    }

    fn spawn(project: &Project, id: &str) -> Running {
        let config = project.instance(id).unwrap().clone();
        let shared = Shared::default();
        shared.update(|s| {
            s.instances
                .insert(id.to_string(), InstanceState::new(&config.device));
        });
        let (commands, rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(run_instance(
            config,
            project.broker.clone(),
            rx,
            shared.clone(),
        ));
        Running {
            commands,
            shared,
            task,
        }
    }

    fn status(shared: &Shared, id: &str) -> InstanceStatus {
        shared.snapshot().instances[id].status.clone()
    }

    async fn wait_status(running: &Running, id: &str, pred: impl Fn(&InstanceStatus) -> bool) {
        eventually(
            || format!("status stuck at {:?}", status(&running.shared, id)),
            || pred(&status(&running.shared, id)),
        )
        .await;
    }

    const DEVICE: &str = "[[devices]]\nid = \"d\"\ntemplate = \"air\"\n";

    async fn publishes_online_retained_and_data_on_interval(version: MqttVersion) {
        let (ports, port) = broker(version);
        let mut sub = Subscriber::connect(ports.v5, "it/#").await;
        let running = spawn(&project(port, version, DEVICE), "d");

        let online = sub
            .wait_for(WAIT, |m| m.topic == "it/d/status")
            .await
            .unwrap();
        assert_eq!(online.payload, "online");
        let first = sub
            .wait_for(WAIT, |m| m.topic == "it/d/state")
            .await
            .unwrap();
        assert_eq!(first.payload, r#"{"co2": 600}"#);
        let more = sub.collect(Duration::from_millis(550)).await;
        let count = more.iter().filter(|m| m.topic == "it/d/state").count();
        assert!(
            (3..=7).contains(&count),
            "expected ~5 messages at 100ms, got {count}"
        );

        let late = Subscriber::connect(ports.v5, "it/d/status")
            .await
            .wait_for(WAIT, |_| true)
            .await
            .unwrap();
        assert_eq!((late.payload.as_str(), late.retain), ("online", true));

        let state = running.shared.snapshot();
        assert_eq!(state.instances["d"].status, InstanceStatus::Online);
        assert!(state.instances["d"].published >= 4);
        assert_eq!(state.instances["d"].values["co2"], Value::Number(600.0));
        assert_eq!(state.log.back().unwrap().topic, "it/d/state");
    }

    async fn stop_publishes_offline_without_last_will(version: MqttVersion) {
        let (ports, port) = broker(version);
        let mut sub = Subscriber::connect(ports.v5, "it/d/status").await;
        let running = spawn(&project(port, version, DEVICE), "d");
        sub.wait_for(WAIT, |m| m.payload == "online").await.unwrap();

        running.commands.send(InstanceCmd::Stop).unwrap();
        running.task.await.unwrap();

        let after: Vec<String> = sub
            .collect(Duration::from_millis(500))
            .await
            .into_iter()
            .map(|m| m.payload)
            .collect();
        assert_eq!(
            after,
            ["offline"],
            "exactly one offline (ours), no Last Will"
        );
        assert_eq!(status(&running.shared, "d"), InstanceStatus::Stopped);
    }

    async fn kill_makes_the_broker_send_the_last_will(version: MqttVersion) {
        let (ports, port) = broker(version);
        let mut sub = Subscriber::connect(ports.v5, "it/d/status").await;
        let running = spawn(&project(port, version, DEVICE), "d");
        sub.wait_for(WAIT, |m| m.payload == "online").await.unwrap();

        running.commands.send(InstanceCmd::Kill).unwrap();
        running.task.await.unwrap();

        let will = sub.wait_for(WAIT, |m| m.payload == "offline").await;
        assert!(will.is_some(), "broker should deliver the Last Will");
        assert_eq!(status(&running.shared, "d"), InstanceStatus::Stopped);
    }

    async fn pin_overrides_value_and_unpin_restores_generator(version: MqttVersion) {
        let (ports, port) = broker(version);
        let mut sub = Subscriber::connect(ports.v5, "it/d/state").await;
        let running = spawn(&project(port, version, DEVICE), "d");
        sub.wait_for(WAIT, |m| m.payload == r#"{"co2": 600}"#)
            .await
            .unwrap();

        running
            .commands
            .send(InstanceCmd::Pin("co2".into(), Value::Number(2500.0)))
            .unwrap();
        sub.wait_for(WAIT, |m| m.payload == r#"{"co2": 2500}"#)
            .await
            .unwrap();
        assert!(
            running.shared.snapshot().instances["d"]
                .pinned
                .contains("co2")
        );

        running
            .commands
            .send(InstanceCmd::Unpin("co2".into()))
            .unwrap();
        sub.wait_for(WAIT, |m| m.payload == r#"{"co2": 600}"#)
            .await
            .unwrap();
        assert!(running.shared.snapshot().instances["d"].pinned.is_empty());
    }

    async fn pin_of_unknown_field_is_ignored(version: MqttVersion) {
        let (ports, port) = broker(version);
        let mut sub = Subscriber::connect(ports.v5, "it/d/state").await;
        let running = spawn(&project(port, version, DEVICE), "d");
        sub.wait_for(WAIT, |m| m.topic == "it/d/state")
            .await
            .unwrap();

        running
            .commands
            .send(InstanceCmd::Pin("nope".into(), Value::Number(1.0)))
            .unwrap();
        sub.wait_for(WAIT, |m| m.topic == "it/d/state")
            .await
            .unwrap();
        let state = running.shared.snapshot();
        assert!(state.instances["d"].pinned.is_empty());
        assert!(!state.instances["d"].values.contains_key("nope"));
    }

    async fn refused_connection_fails_without_retrying(version: MqttVersion) {
        let port = refusing_broker(ConnectReturnCode::BadUserNamePassword).await;
        let running = spawn(&project(port, version, DEVICE), "d");
        running.task.await.unwrap();
        assert_eq!(
            status(&running.shared, "d"),
            InstanceStatus::Failed {
                reason: "broker refused the connection: BadUserNamePassword".into()
            }
        );
    }

    fn big_payload_project(port: u16, version: MqttVersion, payload_len: usize) -> Project {
        big_payload_project_with(port, version, payload_len, "")
    }

    /// [`big_payload_project`] with `message_keys` added to the message's table.
    fn big_payload_project_with(
        port: u16,
        version: MqttVersion,
        payload_len: usize,
        message_keys: &str,
    ) -> Project {
        let blob = "x".repeat(payload_len);
        let version = version.label();
        let src = format!(
            r#"
[broker]
host = "127.0.0.1"
port = {port}
mqtt_version = "{version}"

[[templates]]
id = "big"
interval = "100ms"
messages = [{{ topic = "big/{{{{ device_id }}}}/state", payload = "{{{{ blob }}}}"{message_keys} }}]
fields.blob = {{ generator = "constant", value = "{blob}" }}

[[devices]]
id = "d"
template = "big"
"#
        );
        valid_project(&src)
    }

    async fn payload_over_ten_kib_is_delivered_without_reconnecting(version: MqttVersion) {
        let (ports, port) = broker(version);
        let mut sub = Subscriber::connect(ports.v5, "big/#").await;
        let running = spawn(&big_payload_project(port, version, 12_000), "d");

        let data = sub.wait_for(WAIT, |m| m.topic == "big/d/state").await;
        assert_eq!(data.map(|m| m.payload.len()), Some(12_000));
        let state = running.shared.snapshot();
        assert_eq!(state.instances["d"].status, InstanceStatus::Online);
        assert_eq!(state.instances["d"].errors, 0);
    }

    async fn oversized_payload_is_counted_as_an_error_not_a_reconnect(version: MqttVersion) {
        let (_, port) = broker(version);
        let running = spawn(
            &big_payload_project(port, version, MAX_MESSAGE_BYTES + 1),
            "d",
        );
        wait_status(&running, "d", |s| *s == InstanceStatus::Online).await;
        tokio::time::sleep(Duration::from_millis(400)).await;

        let state = running.shared.snapshot();
        assert_eq!(state.instances["d"].status, InstanceStatus::Online);
        assert!(
            state.instances["d"].errors >= 1,
            "{:?}",
            state.instances["d"]
        );
        assert_eq!(state.instances["d"].published, 0);
    }

    async fn connect_slower_than_the_interval_still_completes(version: MqttVersion) {
        let port = slow_broker(Duration::from_millis(300)).await;
        let running = spawn(&project(port, version, DEVICE), "d");
        wait_status(&running, "d", |s| *s == InstanceStatus::Online).await;
    }

    async fn unreachable_broker_reconnects_and_stop_returns_promptly(version: MqttVersion) {
        let running = spawn(&project(free_port(), version, DEVICE), "d");
        wait_status(&running, "d", |s| {
            matches!(s, InstanceStatus::Reconnecting { attempt: 1, .. })
        })
        .await;

        running.commands.send(InstanceCmd::Stop).unwrap();
        tokio::time::timeout(Duration::from_millis(500), running.task)
            .await
            .expect("stop is prompt")
            .unwrap();
        assert_eq!(status(&running.shared, "d"), InstanceStatus::Stopped);
    }

    /// Runs each listed test once per MQTT version, as `mqtt311::<name>` and `mqtt5::<name>`.
    macro_rules! on_both_versions {
        ($($test:ident),* $(,)?) => {
            mod mqtt311 {
                $(
                    #[tokio::test(flavor = "multi_thread")]
                    async fn $test() {
                        super::$test(crate::project::MqttVersion::V311).await
                    }
                )*
            }
            mod mqtt5 {
                $(
                    #[tokio::test(flavor = "multi_thread")]
                    async fn $test() {
                        super::$test(crate::project::MqttVersion::V5).await
                    }
                )*
            }
        };
    }

    on_both_versions!(
        publishes_online_retained_and_data_on_interval,
        stop_publishes_offline_without_last_will,
        kill_makes_the_broker_send_the_last_will,
        pin_overrides_value_and_unpin_restores_generator,
        pin_of_unknown_field_is_ignored,
        refused_connection_fails_without_retrying,
        payload_over_ten_kib_is_delivered_without_reconnecting,
        oversized_payload_is_counted_as_an_error_not_a_reconnect,
        connect_slower_than_the_interval_still_completes,
        unreachable_broker_reconnects_and_stop_returns_promptly
    );

    #[tokio::test(flavor = "multi_thread")]
    async fn mqtt5_properties_reach_the_subscriber_and_the_log() {
        let (ports, port) = broker(MqttVersion::V5);
        let mut sub = Subscriber::connect(ports.v5, "props/#").await;
        let project = valid_project(&format!(
            r#"
[broker]
host = "127.0.0.1"
port = {port}

[[templates]]
id = "p"
interval = "100ms"
messages = [{{ topic = "props/{{{{ device_id }}}}", payload = "{{{{ v }}}}", message_expiry = "60s", content_type = "text/plain", user_properties = {{ room = "{{{{ room }}}}", seq = "{{{{ seq }}}}" }} }}]
fields.v = {{ generator = "constant", value = 1 }}

[[devices]]
id = "d"
template = "p"
params = {{ room = "office" }}
"#
        ));
        let running = spawn(&project, "d");

        let third = sub
            .wait_for(WAIT, |m| {
                m.user_properties.contains(&("seq".into(), "2".into()))
            })
            .await
            .unwrap();
        assert_eq!(third.message_expiry, Some(60));
        assert_eq!(third.content_type.as_deref(), Some("text/plain"));
        assert_eq!(
            third.user_properties,
            [("room".into(), "office".into()), ("seq".into(), "2".into())]
        );
        let logged = running.shared.snapshot().log.front().cloned().unwrap();
        assert_eq!(logged.properties.message_expiry, Some(60));
        assert_eq!(
            logged.properties.user_properties,
            [("room".into(), "office".into()), ("seq".into(), "0".into())]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mqtt5_broker_refusing_the_version_fails_with_the_hint() {
        let port = refusing_broker(ConnectReturnCode::RefusedProtocolVersion).await;
        let running = spawn(&project(port, MqttVersion::V5, DEVICE), "d");
        running.task.await.unwrap();
        assert_eq!(
            status(&running.shared, "d"),
            InstanceStatus::Failed {
                reason: MQTT5_UNSUPPORTED.into()
            }
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mqtt_3_1_1_refusal_of_the_version_has_no_mqtt5_hint() {
        let port = refusing_broker(ConnectReturnCode::RefusedProtocolVersion).await;
        let running = spawn(&project(port, MqttVersion::V311, DEVICE), "d");
        running.task.await.unwrap();
        assert_eq!(
            status(&running.shared, "d"),
            InstanceStatus::Failed {
                reason: "broker refused the connection: RefusedProtocolVersion".into()
            }
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mqtt5_against_a_3_1_1_only_listener_fails_with_the_hint() {
        let ports = start_broker_ports();
        let running = spawn(&project(ports.v311, MqttVersion::V5, DEVICE), "d");
        running.task.await.unwrap();
        assert_eq!(
            status(&running.shared, "d"),
            InstanceStatus::Failed {
                reason: MQTT5_UNSUPPORTED.into()
            }
        );
    }

    /// A listener that reads the CONNECT and closes the connection without answering.
    async fn closing_without_connack() -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut buffer = [0u8; 256];
                let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut buffer).await;
            }
        });
        port
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_broker_that_closes_without_answering_gets_no_mqtt_version_hint() {
        let port = closing_without_connack().await;
        let running = spawn(&project(port, MqttVersion::V5, DEVICE), "d");
        wait_status(&running, "d", |s| {
            matches!(s, InstanceStatus::Reconnecting { .. })
        })
        .await;
        let InstanceStatus::Reconnecting { last_error, .. } = status(&running.shared, "d") else {
            unreachable!()
        };
        assert!(!last_error.contains("3.1.1"), "{last_error}");
        running.commands.send(InstanceCmd::Stop).unwrap();
        running.task.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn properties_count_toward_the_size_limit() {
        let (_, port) = broker(MqttVersion::V5);
        // Topic and payload fit; the user property pushes the message over 1 MiB.
        let note = format!(", user_properties = {{ note = \"{}\" }}", "x".repeat(400));
        let project =
            big_payload_project_with(port, MqttVersion::V5, MAX_MESSAGE_BYTES - 200, &note);
        let running = spawn(&project, "d");
        wait_status(&running, "d", |s| *s == InstanceStatus::Online).await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        let state = running.shared.snapshot();
        assert_eq!(state.instances["d"].status, InstanceStatus::Online);
        assert!(
            state.instances["d"].errors >= 1,
            "{:?}",
            state.instances["d"]
        );
        assert_eq!(state.instances["d"].published, 0);
    }

    #[test]
    fn the_broker_packet_limit_lowers_the_message_limit() {
        assert_eq!(message_limit(None), MAX_MESSAGE_BYTES);
        assert_eq!(message_limit(Some(1000)), 1000 - PUBLISH_HEADER_BYTES);
        assert_eq!(message_limit(Some(u32::MAX)), MAX_MESSAGE_BYTES);
        assert_eq!(message_limit(Some(4)), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_connection_lost_after_it_was_accepted_shows_no_hint() {
        let port = closing_broker(Duration::from_millis(300)).await;
        let running = spawn(&project(port, MqttVersion::V5, DEVICE), "d");
        wait_status(&running, "d", |s| *s == InstanceStatus::Online).await;
        wait_status(&running, "d", |s| {
            matches!(s, InstanceStatus::Reconnecting { .. })
        })
        .await;
        let InstanceStatus::Reconnecting { last_error, .. } = status(&running.shared, "d") else {
            unreachable!()
        };
        assert!(!last_error.contains("3.1.1"), "{last_error}");
        running.commands.send(InstanceCmd::Stop).unwrap();
        running.task.await.unwrap();
    }
}
