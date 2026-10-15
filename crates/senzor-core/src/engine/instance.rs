use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use rand::RngExt;
use rand::rngs::ChaCha8Rng;
use rumqttc::{AsyncClient, ClientError, ConnectionError, Event, EventLoop, LastWill, Packet, QoS};
use tokio::sync::mpsc;
use tokio::time::{Instant, Interval, MissedTickBehavior};

use super::mqtt::{MAX_MESSAGE_BYTES, backoff, mqtt_options, qos};
use super::state::{InstanceStatus, LogEntry, Shared};
use crate::generator::{TickContext, ValueSource};
use crate::project::{BrokerConfig, InstanceConfig};
use crate::render::{Context, InstanceTemplates, RenderedMessage, base_context, tick_context};
use crate::rng::instance_rng;
use crate::value::Value;

/// How long a graceful stop may take before the connection is dropped.
pub(crate) const STOP_TIMEOUT: Duration = Duration::from_secs(2);

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
}

impl Availability {
    /// The broker publishes this `offline` when the connection drops without DISCONNECT; a
    /// graceful stop publishes the same message itself.
    fn last_will(&self) -> LastWill {
        LastWill::new(
            self.topic.clone(),
            self.offline.clone(),
            QoS::AtLeastOnce,
            true,
        )
    }

    fn publish_online(&self, client: &AsyncClient) -> Result<(), ClientError> {
        self.publish(client, &self.online)
    }

    fn publish_offline(&self, client: &AsyncClient) -> Result<(), ClientError> {
        self.publish(client, &self.offline)
    }

    fn publish(&self, client: &AsyncClient, payload: &str) -> Result<(), ClientError> {
        client.try_publish(self.topic.clone(), QoS::AtLeastOnce, true, payload)
    }
}

async fn run(
    config: &InstanceConfig,
    broker: &BrokerConfig,
    commands: &mut mpsc::UnboundedReceiver<InstanceCmd>,
    shared: &Shared,
) -> Result<(), String> {
    let id = config.id.as_str();
    let templates = InstanceTemplates::compile(&config.messages, config.availability.as_ref())?;
    let base = base_context(id, config.index, &config.template_id, &config.params);
    let availability = templates
        .availability_topic(&base)?
        .zip(config.availability.as_ref())
        .map(|(topic, spec)| Availability {
            topic,
            online: spec.online.clone(),
            offline: spec.offline.clone(),
        });
    let will = availability.as_ref().map(Availability::last_will);
    let (client, mut eventloop) =
        AsyncClient::new(mqtt_options(broker, id, will), REQUEST_QUEUE_CAPACITY);
    let mut ticker = Ticker::new(config, templates, base);
    let mut timer = ticker.timer();

    shared.set_status(id, InstanceStatus::Connecting);
    let mut online = false;
    let mut attempt = 0u32;
    let mut retry_at: Option<Instant> = None;
    loop {
        tokio::select! {
            event = eventloop.poll(), if retry_at.is_none() => match event {
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    online = true;
                    attempt = 0;
                    shared.set_status(id, InstanceStatus::Online);
                    if let Some(a) = &availability
                        && let Err(e) = a.publish_online(&client)
                    {
                        shared.record_error(id, &format!("publish online: {e}"));
                    }
                }
                Ok(_) => {}
                Err(ConnectionError::ConnectionRefused(code)) => {
                    return Err(format!("broker refused the connection: {code:?}"));
                }
                Err(e) => {
                    online = false;
                    attempt += 1;
                    shared.set_status(id, InstanceStatus::Reconnecting { attempt, last_error: e.to_string() });
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
                        stop_gracefully(&client, &mut eventloop, availability.as_ref()).await;
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
    templates: InstanceTemplates,
    base: Context,
    sources: BTreeMap<String, Box<dyn ValueSource>>,
    pins: BTreeMap<String, Value>,
    rng: ChaCha8Rng,
    started: Instant,
    seq: u64,
}

impl<'a> Ticker<'a> {
    fn new(config: &'a InstanceConfig, templates: InstanceTemplates, base: Context) -> Self {
        Self {
            config,
            templates,
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

    fn tick(&mut self, client: &AsyncClient, shared: &Shared) {
        let id = self.config.id.as_str();
        let now = SystemTime::now();
        let values = self.next_values(now);
        let render_ctx = tick_context(&self.base, &values, now, self.seq);
        shared.update_instance(id, |instance| instance.values = values);
        self.seq += 1;

        for rendered in self.templates.render_messages(&render_ctx) {
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
        client: &AsyncClient,
        message: RenderedMessage,
        now: SystemTime,
    ) -> Result<LogEntry, String> {
        let id = self.config.id.as_str();
        let size = message.topic.len() + message.payload.len();
        if size > MAX_MESSAGE_BYTES {
            return Err(format!(
                "message on `{}` is {size} bytes; the limit is {MAX_MESSAGE_BYTES}",
                message.topic
            ));
        }
        client
            .try_publish(
                message.topic.clone(),
                qos(self.config.qos),
                self.config.retain,
                message.payload.clone(),
            )
            .map_err(|e| format!("publish: {e}"))?;
        tracing::debug!(instance = %id, topic = %message.topic, payload = %message.payload, "publish");
        Ok(LogEntry {
            time: now,
            instance: id.to_string(),
            topic: message.topic,
            payload: message.payload,
            qos: self.config.qos,
            retain: self.config.retain,
        })
    }
}

/// Publishes `offline`, sends DISCONNECT and keeps polling until the broker closes the connection.
async fn stop_gracefully(
    client: &AsyncClient,
    eventloop: &mut EventLoop,
    availability: Option<&Availability>,
) {
    if let Some(a) = availability {
        let _ = a.publish_offline(client);
    }
    let _ = client.try_disconnect();
    let _ = tokio::time::timeout(STOP_TIMEOUT, async {
        while eventloop.poll().await.is_ok() {}
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::state::InstanceState;
    use crate::engine::test_support::{WAIT, eventually, valid_project};
    use crate::project::Project;
    use rumqttc::ConnectReturnCode;
    use senzor_testkit::{Subscriber, free_port, refusing_broker, slow_broker, start_broker};

    fn project(port: u16, extra_device_toml: &str) -> Project {
        let src = format!(
            r#"
[broker]
host = "127.0.0.1"
port = {port}

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

    #[tokio::test(flavor = "multi_thread")]
    async fn publishes_online_retained_and_data_on_interval() {
        let port = start_broker();
        let mut sub = Subscriber::connect(port, "it/#").await;
        let running = spawn(&project(port, DEVICE), "d");

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

        let late = Subscriber::connect(port, "it/d/status")
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

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_publishes_offline_without_last_will() {
        let port = start_broker();
        let mut sub = Subscriber::connect(port, "it/d/status").await;
        let running = spawn(&project(port, DEVICE), "d");
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

    #[tokio::test(flavor = "multi_thread")]
    async fn kill_makes_the_broker_send_the_last_will() {
        let port = start_broker();
        let mut sub = Subscriber::connect(port, "it/d/status").await;
        let running = spawn(&project(port, DEVICE), "d");
        sub.wait_for(WAIT, |m| m.payload == "online").await.unwrap();

        running.commands.send(InstanceCmd::Kill).unwrap();
        running.task.await.unwrap();

        let will = sub.wait_for(WAIT, |m| m.payload == "offline").await;
        assert!(will.is_some(), "broker should deliver the Last Will");
        assert_eq!(status(&running.shared, "d"), InstanceStatus::Stopped);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pin_overrides_value_and_unpin_restores_generator() {
        let port = start_broker();
        let mut sub = Subscriber::connect(port, "it/d/state").await;
        let running = spawn(&project(port, DEVICE), "d");
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

    #[tokio::test(flavor = "multi_thread")]
    async fn pin_of_unknown_field_is_ignored() {
        let port = start_broker();
        let mut sub = Subscriber::connect(port, "it/d/state").await;
        let running = spawn(&project(port, DEVICE), "d");
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

    #[tokio::test(flavor = "multi_thread")]
    async fn refused_connection_fails_without_retrying() {
        let port = refusing_broker(ConnectReturnCode::BadUserNamePassword).await;
        let running = spawn(&project(port, DEVICE), "d");
        running.task.await.unwrap();
        assert_eq!(
            status(&running.shared, "d"),
            InstanceStatus::Failed {
                reason: "broker refused the connection: BadUserNamePassword".into()
            }
        );
    }

    fn big_payload_project(port: u16, payload_len: usize) -> Project {
        let blob = "x".repeat(payload_len);
        let src = format!(
            r#"
[broker]
host = "127.0.0.1"
port = {port}

[[templates]]
id = "big"
interval = "100ms"
messages = [{{ topic = "big/{{{{ device_id }}}}/state", payload = "{{{{ blob }}}}" }}]
fields.blob = {{ generator = "constant", value = "{blob}" }}

[[devices]]
id = "d"
template = "big"
"#
        );
        valid_project(&src)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn payload_over_ten_kib_is_delivered_without_reconnecting() {
        let port = start_broker();
        let mut sub = Subscriber::connect(port, "big/#").await;
        let running = spawn(&big_payload_project(port, 12_000), "d");

        let data = sub.wait_for(WAIT, |m| m.topic == "big/d/state").await;
        assert_eq!(data.map(|m| m.payload.len()), Some(12_000));
        let state = running.shared.snapshot();
        assert_eq!(state.instances["d"].status, InstanceStatus::Online);
        assert_eq!(state.instances["d"].errors, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn oversized_payload_is_counted_as_an_error_not_a_reconnect() {
        let port = start_broker();
        let running = spawn(&big_payload_project(port, MAX_MESSAGE_BYTES + 1), "d");
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

    #[tokio::test(flavor = "multi_thread")]
    async fn connect_slower_than_the_interval_still_completes() {
        let port = slow_broker(Duration::from_millis(300)).await;
        let running = spawn(&project(port, DEVICE), "d");
        wait_status(&running, "d", |s| *s == InstanceStatus::Online).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unreachable_broker_reconnects_and_stop_returns_promptly() {
        let running = spawn(&project(free_port(), DEVICE), "d");
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
}
