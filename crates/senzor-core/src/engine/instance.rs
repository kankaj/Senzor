use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use rand::RngExt;
use rand::rngs::ChaCha8Rng;
use tokio::sync::mpsc;
use tokio::time::{Instant, Interval, MissedTickBehavior};

use super::discovery::{self, Announcement};
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
    /// Run with this configuration from now on, without reconnecting. Only for a configuration
    /// that [`Apply::between`](super::Apply::between) finds can change in place.
    Update(Box<InstanceConfig>),
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

/// The Home Assistant discovery messages of a run.
struct Discovery {
    announced: Vec<Announcement>,
    /// Topics announced earlier in the run but not any more, which the broker may still hold.
    withdrawn: Vec<String>,
}

impl Discovery {
    fn new(announced: Vec<Announcement>) -> Self {
        Self {
            announced,
            withdrawn: Vec::new(),
        }
    }

    /// Clears the withdrawn topics and publishes every announcement: on each connect.
    fn publish_all(&mut self, client: &Client, shared: &Shared, id: &str) {
        let clear = self.withdrawn.drain(..).map(|topic| (topic, String::new()));
        let announce_all = self
            .announced
            .iter()
            .map(|a| (a.topic.clone(), a.payload.clone()));
        publish_announcements(client, shared, id, clear.chain(announce_all));
    }

    /// Takes the announcements of an updated configuration. While `online`, it publishes the ones
    /// that changed and clears the ones that are gone at once; otherwise the next connect does.
    fn update(
        &mut self,
        fresh: Vec<Announcement>,
        client: &Client,
        online: bool,
        shared: &Shared,
        id: &str,
    ) {
        let gone = self
            .announced
            .iter()
            .filter(|old| !fresh.iter().any(|new| new.topic == old.topic))
            .map(|old| old.topic.clone());
        self.withdrawn.extend(gone);
        self.withdrawn
            .retain(|topic| !fresh.iter().any(|new| new.topic == *topic));
        let changed: Vec<(String, String)> = fresh
            .iter()
            .filter(|new| !self.announced.contains(new))
            .map(|a| (a.topic.clone(), a.payload.clone()))
            .collect();
        self.announced = fresh;
        if online {
            let clear = self.withdrawn.drain(..).map(|topic| (topic, String::new()));
            publish_announcements(client, shared, id, clear.chain(changed));
        }
    }
}

/// Publishes `(topic, payload)` discovery messages, retained with QoS 1 like the availability
/// messages; a failure counts as an error of `id`. An empty payload clears the retained message,
/// and Home Assistant drops the entity.
fn publish_announcements(
    client: &Client,
    shared: &Shared,
    id: &str,
    messages: impl Iterator<Item = (String, String)>,
) {
    for (topic, payload) in messages {
        let published =
            client.try_publish(&topic, 1, true, &payload, &MessageProperties::default());
        if let Err(e) = published {
            shared.record_error(id, &format!("announce to Home Assistant: {e}"));
        }
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
    let mut discovery = Discovery::new(discovery::announcements(config, &base));
    let (client, mut connection) = connect(broker, id, will, REQUEST_QUEUE_CAPACITY);
    let mut ticker = Ticker::new(config.clone(), base);
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
                    // Before `online`, so Home Assistant knows the entities the status is for.
                    discovery.publish_all(&client, shared, id);
                    if let Some(a) = &availability
                        && let Err(e) = a.publish_online(&client)
                    {
                        shared.record_error(id, &format!("publish online: {e}"));
                    }
                }
                Ok(ConnEvent::PubAck { .. } | ConnEvent::Other) => {}
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
                Some(InstanceCmd::Update(config)) => {
                    if ticker.update(*config, shared) {
                        timer = ticker.next_timer();
                    }
                    let fresh = discovery::announcements(&ticker.config, &ticker.base);
                    discovery.update(fresh, &client, online, shared, id);
                }
                #[cfg(test)]
                Some(InstanceCmd::Panic) => panic!("test panic"),
            },
        }
    }
}

/// Per-run state that produces and publishes one tick.
struct Ticker {
    config: InstanceConfig,
    base: Context,
    sources: BTreeMap<String, Box<dyn ValueSource>>,
    pins: BTreeMap<String, Value>,
    rng: ChaCha8Rng,
    started: Instant,
    seq: u64,
    /// The largest packet the broker accepts, from its MQTT 5 CONNACK.
    broker_max_packet_size: Option<u32>,
}

impl Ticker {
    fn new(config: InstanceConfig, base: Context) -> Self {
        Self {
            sources: config
                .fields
                .iter()
                .map(|(name, field)| (name.clone(), field.generator.build()))
                .collect(),
            pins: BTreeMap::new(),
            rng: instance_rng(config.seed, &config.id),
            config,
            base,
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

    /// A tick timer for a changed interval: its first tick comes one interval from now.
    fn next_timer(&self) -> Interval {
        let interval = self.config.interval;
        let mut timer = tokio::time::interval_at(Instant::now() + interval, interval);
        timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
        timer
    }

    /// Runs with `config` from the next tick on; returns whether the interval changed. A field
    /// whose generator is the same keeps its value source, so a random walk carries on where it
    /// is; a changed or added field starts afresh. A removed field loses its value and its pin.
    /// `seq` and the run's start time carry on; a changed seed reseeds the instance RNG.
    fn update(&mut self, config: InstanceConfig, shared: &Shared) -> bool {
        let old = std::mem::replace(&mut self.config, config);
        let config = &self.config;
        self.base = base_context(
            &config.id,
            config.index,
            &config.template_id,
            &config.params,
        );
        if config.seed != old.seed {
            self.rng = instance_rng(config.seed, &config.id);
        }
        let mut sources = std::mem::take(&mut self.sources);
        self.sources = config
            .fields
            .iter()
            .map(|(name, field)| {
                let same = old
                    .fields
                    .get(name)
                    .is_some_and(|f| f.generator == field.generator);
                let source = sources
                    .remove(name)
                    .filter(|_| same)
                    .unwrap_or_else(|| field.generator.build());
                (name.clone(), source)
            })
            .collect();
        self.pins.retain(|name, _| config.fields.contains_key(name));
        shared.update_instance(&config.id, |instance| {
            instance
                .values
                .retain(|name, _| config.fields.contains_key(name));
            instance
                .pinned
                .retain(|name| config.fields.contains_key(name));
        });
        old.interval != config.interval
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
        let now = SystemTime::now();
        let values = self.next_values(now);
        let id = self.config.id.as_str();
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
        BrokerPorts, Protocol, Received, Subscriber, closing_broker, free_port, refusing_broker,
        slow_broker, start_broker_ports,
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

    async fn announces_itself_to_home_assistant_before_it_goes_online(version: MqttVersion) {
        let (ports, port) = broker(version);
        let mut sub = Subscriber::connect(ports.v5, "#").await;
        let src = project(port, version, DEVICE);
        let mut config = src.instance("d").unwrap().clone();
        config.home_assistant = true;
        let project = Project {
            devices: vec![crate::project::DeviceConfig {
                id: "d".into(),
                autostart: true,
                instances: vec![config],
            }],
            ..src
        };
        let _running = spawn(&project, "d");

        let first = sub.wait_for(WAIT, |_| true).await.unwrap();
        assert_eq!(first.topic, "homeassistant/sensor/d/co2/config");
        let config: serde_json::Value = serde_json::from_str(&first.payload).unwrap();
        assert_eq!(config["state_topic"], "it/d/state");
        let online = sub.wait_for(WAIT, |_| true).await.unwrap();
        assert_eq!(
            (online.topic.as_str(), online.payload.as_str()),
            ("it/d/status", "online")
        );

        // Retained, so Home Assistant finds it after a restart.
        let late = Subscriber::connect(ports.v5, "homeassistant/#")
            .await
            .wait_for(WAIT, |_| true)
            .await
            .unwrap();
        assert_eq!((late.payload, late.retain), (first.payload, true));
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

    /// A project for device `d` of template `t`, whose keys besides its ID, interval and
    /// availability are `template`.
    fn live_project(port: u16, version: MqttVersion, template: &str) -> Project {
        let version = version.label();
        valid_project(&format!(
            r#"
[broker]
host = "127.0.0.1"
port = {port}
mqtt_version = "{version}"

[[templates]]
id = "t"
interval = "100ms"
availability = {{ topic = "lv/{{{{ device_id }}}}/status", online = "online", offline = "offline" }}
{template}

[[devices]]
id = "d"
template = "t"
"#
        ))
    }

    /// Hands `d`'s configuration in `project` to the running instance.
    fn update(running: &Running, project: &Project) {
        let config = project.instance("d").unwrap().clone();
        running
            .commands
            .send(InstanceCmd::Update(Box::new(config)))
            .unwrap();
    }

    /// Adds messages to `seen` until one in it satisfies `done`, and returns the first such;
    /// fails after [`WAIT`].
    async fn record_until(
        sub: &mut Subscriber,
        seen: &mut Vec<Received>,
        done: impl Fn(&Received) -> bool,
    ) -> Received {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(found) = seen.iter().find(|m| done(m)) {
                return found.clone();
            }
            assert!(Instant::now() < deadline, "timed out: {seen:#?}");
            seen.extend(sub.collect(Duration::from_millis(50)).await);
        }
    }

    fn on<'a>(seen: &'a [Received], topic: &str) -> Vec<&'a str> {
        seen.iter()
            .filter(|m| m.topic == topic)
            .map(|m| m.payload.as_str())
            .collect()
    }

    async fn an_update_changes_what_is_published_without_reconnecting(version: MqttVersion) {
        let (ports, port) = broker(version);
        let mut sub = Subscriber::connect(ports.v5, "lv/#").await;
        let old = live_project(
            port,
            version,
            r#"messages = [{ topic = "lv/{{ device_id }}/state", payload = "{{ v }}" }]
fields.v = { generator = "constant", value = 1 }"#,
        );
        let running = spawn(&old, "d");
        let mut before = Vec::new();
        record_until(&mut sub, &mut before, |m| m.topic == "lv/d/state").await;
        before.extend(sub.collect(Duration::from_millis(350)).await);
        assert_eq!(on(&before, "lv/d/status"), ["online"]);

        let new = live_project(
            port,
            version,
            r#"qos = 1
retain = true
messages = [{ topic = "lv/{{ device_id }}/data", payload = "{{ v }}/{{ seq }}" }]
fields.v = { generator = "constant", value = 2 }"#,
        );
        update(&running, &new);
        let mut after = Vec::new();
        let first = record_until(&mut sub, &mut after, |m| m.topic == "lv/d/data").await;
        let (value, seq) = first.payload.split_once('/').unwrap();
        assert_eq!(value, "2");
        assert!(seq.parse::<u64>().unwrap() >= 3, "seq continues: {seq}");
        after.extend(sub.collect(Duration::from_millis(300)).await);
        assert_eq!(
            on(&after, "lv/d/status"),
            Vec::<&str>::new(),
            "no offline, Last Will or online: the connection stays"
        );
        let state = running.shared.snapshot();
        assert_eq!(state.instances["d"].status, InstanceStatus::Online);
        let logged = state.log.back().unwrap();
        assert_eq!((logged.qos, logged.retain), (1, true));
    }

    async fn an_update_keeps_the_state_of_unchanged_generators(version: MqttVersion) {
        let (ports, port) = broker(version);
        let mut sub = Subscriber::connect(ports.v5, "lv/d/state").await;
        let numbers: Vec<String> = (1..=200).map(|n| n.to_string()).collect();
        let numbers = numbers.join(", ");
        let template = |payload: &str, more: &str| {
            format!(
                "messages = [{{ topic = \"lv/{{{{ device_id }}}}/state\", payload = \"{payload}\" }}]\n\
                 fields.n = {{ generator = \"sequence\", values = [{numbers}], loop = false }}\n{more}"
            )
        };
        let old = live_project(
            port,
            version,
            &template(
                "{{ n }} {{ c }}",
                "fields.c = { generator = \"constant\", value = \"a\" }",
            ),
        );
        let running = spawn(&old, "d");
        let mut seen = Vec::new();
        record_until(&mut sub, &mut seen, |m| m.payload == "4 a").await;

        let new = live_project(
            port,
            version,
            &template(
                "{{ n }} {{ c }} {{ m }}",
                &format!(
                    "fields.c = {{ generator = \"constant\", value = \"b\" }}\n\
                     fields.m = {{ generator = \"sequence\", values = [{numbers}], loop = false }}"
                ),
            ),
        );
        update(&running, &new);
        let first = record_until(&mut sub, &mut seen, |m| m.payload.contains(" b ")).await;
        let parts: Vec<u32> = first
            .payload
            .split(" b ")
            .map(|n| n.parse().unwrap())
            .collect();
        assert!(parts[0] > 4, "n carries on: {}", first.payload);
        assert_eq!(
            parts[1], 1,
            "the added field starts afresh: {}",
            first.payload
        );
    }

    async fn an_update_drops_the_pin_of_a_removed_field(version: MqttVersion) {
        let (ports, port) = broker(version);
        let mut sub = Subscriber::connect(ports.v5, "lv/d/state").await;
        let old = live_project(
            port,
            version,
            r#"messages = [{ topic = "lv/{{ device_id }}/state", payload = "{{ a }} {{ b }}" }]
fields.a = { generator = "constant", value = 1 }
fields.b = { generator = "constant", value = 1 }"#,
        );
        let running = spawn(&old, "d");
        let mut seen = Vec::new();
        record_until(&mut sub, &mut seen, |m| m.payload == "1 1").await;
        for field in ["a", "b"] {
            running
                .commands
                .send(InstanceCmd::Pin(field.into(), Value::Number(5.0)))
                .unwrap();
        }
        record_until(&mut sub, &mut seen, |m| m.payload == "5 5").await;

        let new = live_project(
            port,
            version,
            r#"messages = [{ topic = "lv/{{ device_id }}/state", payload = "a={{ a }}" }]
fields.a = { generator = "constant", value = 1 }"#,
        );
        update(&running, &new);
        record_until(&mut sub, &mut seen, |m| m.payload == "a=5").await;
        let instance = running.shared.snapshot().instances["d"].clone();
        assert_eq!(
            instance.pinned.iter().collect::<Vec<_>>(),
            ["a"],
            "the pin of the kept field stays"
        );
        assert_eq!(
            instance.values.keys().collect::<Vec<_>>(),
            ["a"],
            "the removed field has no value"
        );
        running
            .commands
            .send(InstanceCmd::Pin("b".into(), Value::Number(7.0)))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !running.shared.snapshot().instances["d"]
                .pinned
                .contains("b")
        );
    }

    async fn an_interval_change_takes_effect_from_the_next_tick(version: MqttVersion) {
        let (ports, port) = broker(version);
        let mut sub = Subscriber::connect(ports.v5, "lv/d/state").await;
        let body = r#"messages = [{ topic = "lv/{{ device_id }}/state", payload = "{{ v }}" }]
fields.v = { generator = "constant", value = 1 }"#;
        let running = spawn(&live_project(port, version, body), "d");
        let mut seen = Vec::new();
        record_until(&mut sub, &mut seen, |_| true).await;

        let slow = live_project(port, version, body);
        let mut slow_config = slow.instance("d").unwrap().clone();
        slow_config.interval = Duration::from_secs(10);
        running
            .commands
            .send(InstanceCmd::Update(Box::new(slow_config)))
            .unwrap();
        // A tick already on its way may still arrive.
        sub.collect(Duration::from_millis(150)).await;
        let later = sub.collect(Duration::from_millis(700)).await;
        assert!(later.is_empty(), "{later:#?}");
    }

    async fn an_update_announces_changed_entities_and_clears_dropped_ones(version: MqttVersion) {
        let (ports, port) = broker(version);
        let mut sub = Subscriber::connect(ports.v5, "#").await;
        let ha = |payload: &str, fields: &str| {
            live_project(
                port,
                version,
                &format!(
                    "home_assistant = true\n\
                     messages = [{{ topic = \"lv/{{{{ device_id }}}}/state\", payload = '{payload}' }}]\n\
                     {fields}"
                ),
            )
        };
        const CO2: &str = "homeassistant/sensor/d/co2/config";
        const TEMP: &str = "homeassistant/sensor/d/temp/config";
        let old = ha(
            r#"{"co2": {{ co2 }}}"#,
            r#"fields.co2 = { generator = "constant", value = 600, unit = "ppm" }"#,
        );
        let running = spawn(&old, "d");
        let mut seen = Vec::new();
        record_until(&mut sub, &mut seen, |m| m.topic == "lv/d/state").await;
        assert_eq!(on(&seen, CO2).len(), 1, "{seen:#?}");

        let more = ha(
            r#"{"co2": {{ co2 }}, "temp": {{ temp }}}"#,
            "fields.co2 = { generator = \"constant\", value = 600, unit = \"PPM\" }\n\
             fields.temp = { generator = \"constant\", value = 21 }",
        );
        update(&running, &more);
        let mut after = Vec::new();
        record_until(&mut sub, &mut after, |m| m.topic == TEMP).await;
        let co2 = record_until(&mut sub, &mut after, |m| m.topic == CO2).await;
        let co2: serde_json::Value = serde_json::from_str(&co2.payload).unwrap();
        assert_eq!(co2["unit_of_measurement"], "PPM");

        let off = live_project(
            port,
            version,
            r#"messages = [{ topic = "lv/{{ device_id }}/state", payload = '{"co2": {{ co2 }}}' }]
fields.co2 = { generator = "constant", value = 600 }"#,
        );
        update(&running, &off);
        record_until(&mut sub, &mut after, |m| {
            m.topic == TEMP && m.payload.is_empty()
        })
        .await;
        record_until(&mut sub, &mut after, |m| {
            m.topic == CO2 && m.payload.is_empty()
        })
        .await;
        assert_eq!(
            on(&after, "lv/d/status"),
            Vec::<&str>::new(),
            "no reconnect"
        );
        let retained = Subscriber::connect(ports.v5, "homeassistant/#")
            .await
            .collect(Duration::from_millis(300))
            .await;
        assert!(
            retained.is_empty(),
            "Home Assistant forgets them: {retained:#?}"
        );
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
        announces_itself_to_home_assistant_before_it_goes_online,
        stop_publishes_offline_without_last_will,
        kill_makes_the_broker_send_the_last_will,
        pin_overrides_value_and_unpin_restores_generator,
        pin_of_unknown_field_is_ignored,
        refused_connection_fails_without_retrying,
        payload_over_ten_kib_is_delivered_without_reconnecting,
        oversized_payload_is_counted_as_an_error_not_a_reconnect,
        connect_slower_than_the_interval_still_completes,
        unreachable_broker_reconnects_and_stop_returns_promptly,
        an_update_changes_what_is_published_without_reconnecting,
        an_update_keeps_the_state_of_unchanged_generators,
        an_update_drops_the_pin_of_a_removed_field,
        an_interval_change_takes_effect_from_the_next_tick,
        an_update_announces_changed_entities_and_clears_dropped_ones
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
