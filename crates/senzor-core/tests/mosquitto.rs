//! Tests against the real Mosquitto brokers of `dev/compose.yaml`. They are ignored by default:
//!
//! ```text
//! docker compose -f dev/compose.yaml up -d
//! cargo test --workspace -- --ignored
//! ```
//!
//! The brokers are shared by every test, so each test uses its own client-ID prefix and topics.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use senzor_core::engine::{Command, Engine, EngineHandle, EngineState, InstanceStatus, Target};
use senzor_core::project::{Project, ProjectFile};
use senzor_testkit::{Subscriber, mosquitto_15_port, mosquitto_port, mosquitto_ws_port};
use tokio::runtime::Handle;

const WAIT: Duration = Duration::from_secs(10);

/// A topic prefix no other test run uses: retained messages from earlier runs stay on the broker.
fn unique(name: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{name}-{nanos}-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}

/// Project with device `d` of a template publishing `{{ v }}` on `<prefix>/state` every 200 ms,
/// with availability on `<prefix>/status`. `broker`, `message` and `availability` add keys.
fn project(port: u16, prefix: &str, broker: &str, message: &str, availability: &str) -> Project {
    let src = format!(
        r#"
[broker]
host = "127.0.0.1"
port = {port}
client_id_prefix = "{prefix}-"
{broker}

[[templates]]
id = "t"
interval = "200ms"
availability = {{ topic = "{prefix}/status", online = "online", offline = "offline"{availability} }}
messages = [{{ topic = "{prefix}/state", payload = "{{{{ v }}}}"{message} }}]
fields.v = {{ generator = "constant", value = 1 }}

[[devices]]
id = "d"
template = "t"
autostart = false
params = {{ room = "office" }}
"#
    );
    let validated = ProjectFile::parse(&src)
        .unwrap()
        .validate(std::path::Path::new("."));
    assert!(validated.errors.is_empty(), "{:?}", validated.errors);
    validated.project
}

fn spawn(project: Project) -> EngineHandle {
    Engine::spawn(project, &Handle::current(), Arc::new(|| {}))
}

async fn wait_until(engine: &EngineHandle, what: &str, pred: impl Fn(&EngineState) -> bool) {
    let deadline = Instant::now() + WAIT;
    while !pred(&engine.snapshot()) {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {what}: {:#?}",
            engine.snapshot().instances
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn status(state: &EngineState) -> InstanceStatus {
    state.instances["d"].status.clone()
}

async fn start_online(engine: &EngineHandle) {
    engine.send(Command::Start(Target::Instance("d".into())));
    wait_until(engine, "d online", |s| status(s) == InstanceStatus::Online).await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Docker brokers in dev/compose.yaml"]
async fn both_versions_connect_and_publish_on_mosquitto_2() {
    let port = mosquitto_port();
    for version in ["5", "3.1.1"] {
        let prefix = unique("both");
        let mut sub = Subscriber::connect(port, &format!("{prefix}/#")).await;
        let engine = spawn(project(
            port,
            &prefix,
            &format!("mqtt_version = \"{version}\""),
            "",
            "",
        ));
        start_online(&engine).await;
        let state = sub.wait_for(WAIT, |m| m.topic.ends_with("/state")).await;
        assert_eq!(state.map(|m| m.payload), Some("1".into()), "MQTT {version}");
        engine.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Docker brokers in dev/compose.yaml"]
async fn both_versions_publish_over_websocket() {
    let tcp = mosquitto_port();
    let ws = mosquitto_ws_port();
    for version in ["5", "3.1.1"] {
        let prefix = unique("ws");
        let mut sub = Subscriber::connect(tcp, &format!("{prefix}/#")).await;
        let engine = spawn(project(
            ws,
            &prefix,
            &format!("mqtt_version = \"{version}\"\ntransport = \"websocket\"\npath = \"/\""),
            "",
            "",
        ));
        start_online(&engine).await;
        let state = sub.wait_for(WAIT, |m| m.topic.ends_with("/state")).await;
        assert_eq!(state.map(|m| m.payload), Some("1".into()), "MQTT {version}");
        engine.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Docker brokers in dev/compose.yaml"]
async fn mqtt5_properties_arrive_at_a_subscriber() {
    let port = mosquitto_port();
    let prefix = unique("props");
    let mut sub = Subscriber::connect(port, &format!("{prefix}/state")).await;
    let engine = spawn(project(
        port,
        &prefix,
        "",
        r#", message_expiry = "60s", content_type = "application/json", user_properties = { room = "{{ room }}" }"#,
        "",
    ));
    start_online(&engine).await;
    let message = sub.wait_for(WAIT, |_| true).await.unwrap();
    // Mosquitto forwards the remaining lifetime, which may have dropped a second already.
    assert!(
        matches!(message.message_expiry, Some(59..=60)),
        "{:?}",
        message.message_expiry
    );
    assert_eq!(message.content_type.as_deref(), Some("application/json"));
    assert_eq!(message.user_properties, [("room".into(), "office".into())]);
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Docker brokers in dev/compose.yaml"]
async fn an_expired_retained_message_is_not_delivered() {
    let port = mosquitto_port();
    let prefix = unique("expiry");
    let mut first = Subscriber::connect(port, &format!("{prefix}/state")).await;
    let mut file_project = project(port, &prefix, "", r#", message_expiry = "2s""#, "");
    for instance in file_project
        .devices
        .iter_mut()
        .flat_map(|d| d.instances.iter_mut())
    {
        instance.retain = true;
    }
    let engine = spawn(file_project);
    start_online(&engine).await;
    first.wait_for(WAIT, |_| true).await.unwrap();
    engine.send(Command::Stop(Target::All));
    wait_until(&engine, "d stopped", |s| {
        status(s) == InstanceStatus::Stopped
    })
    .await;

    tokio::time::sleep(Duration::from_secs(4)).await;
    let mut late = Subscriber::connect(port, &format!("{prefix}/state")).await;
    assert_eq!(late.collect(Duration::from_secs(1)).await, []);
    engine.shutdown().await;
}

/// Kills `d` and returns the `offline` messages within `during`, with their delay after the kill.
async fn offline_after_kill(
    engine: &EngineHandle,
    sub: &mut Subscriber,
    restart_after: Option<Duration>,
    during: Duration,
) -> Vec<Duration> {
    let killed = Instant::now();
    engine.send(Command::Kill(Target::All));
    if let Some(after) = restart_after {
        wait_until(engine, "d stopped", |s| {
            status(s) == InstanceStatus::Stopped
        })
        .await;
        tokio::time::sleep(after).await;
        start_online(engine).await;
    }
    let mut delays = Vec::new();
    let deadline = killed + during;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        if let Some(message) = sub.wait_for(left, |m| m.payload == "offline").await {
            let _ = message;
            delays.push(killed.elapsed());
        }
    }
    delays
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Docker brokers in dev/compose.yaml"]
async fn the_last_will_waits_for_will_delay() {
    let port = mosquitto_port();
    let prefix = unique("delay");
    let mut sub = Subscriber::connect(port, &format!("{prefix}/status")).await;
    let engine = spawn(project(
        port,
        &prefix,
        r#"session_expiry = "60s""#,
        "",
        r#", will_delay = "5s""#,
    ));
    start_online(&engine).await;
    let delays = offline_after_kill(&engine, &mut sub, None, Duration::from_secs(8)).await;
    assert_eq!(delays.len(), 1, "{delays:?}");
    // Mosquitto counts the delay in whole seconds, so the Last Will may come up to 1 s early;
    // without the delay it comes at once.
    assert!(delays[0] >= Duration::from_secs(3), "{delays:?}");
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Docker brokers in dev/compose.yaml"]
async fn a_restart_within_will_delay_sends_no_last_will() {
    let port = mosquitto_port();
    let prefix = unique("cancel");
    let mut sub = Subscriber::connect(port, &format!("{prefix}/status")).await;
    let engine = spawn(project(
        port,
        &prefix,
        r#"session_expiry = "60s""#,
        "",
        r#", will_delay = "3s""#,
    ));
    start_online(&engine).await;
    let delays = offline_after_kill(
        &engine,
        &mut sub,
        Some(Duration::from_millis(500)),
        Duration::from_secs(6),
    )
    .await;
    assert_eq!(delays, [], "no Last Will after a restart within the delay");
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Docker brokers in dev/compose.yaml"]
async fn without_will_delay_the_last_will_is_sent_at_once() {
    let port = mosquitto_port();
    let prefix = unique("now");
    let mut sub = Subscriber::connect(port, &format!("{prefix}/status")).await;
    let engine = spawn(project(port, &prefix, "", "", ""));
    start_online(&engine).await;
    let delays = offline_after_kill(&engine, &mut sub, None, Duration::from_secs(3)).await;
    assert_eq!(delays.len(), 1, "{delays:?}");
    assert!(delays[0] < Duration::from_secs(1), "{delays:?}");
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Docker brokers in dev/compose.yaml"]
async fn mqtt5_against_mosquitto_1_5_fails_with_the_3_1_1_hint() {
    let port = mosquitto_15_port();
    let engine = spawn(project(port, &unique("old5"), "", "", ""));
    engine.send(Command::Start(Target::All));
    wait_until(&engine, "failed with the hint", |s| {
        matches!(status(s), InstanceStatus::Failed { reason } if reason.contains("does not support MQTT 5") && reason.contains("mqtt_version = \"3.1.1\""))
    })
    .await;
    engine.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Docker brokers in dev/compose.yaml"]
async fn mqtt_3_1_1_against_mosquitto_1_5_goes_online() {
    let port = mosquitto_15_port();
    let engine = spawn(project(
        port,
        &unique("old311"),
        r#"mqtt_version = "3.1.1""#,
        "",
        "",
    ));
    start_online(&engine).await;
    engine.shutdown().await;
}

/// The connection test tells a 3.1.1-only broker apart, as a failed instance does.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Docker brokers in dev/compose.yaml"]
async fn the_connection_test_names_a_broker_without_mqtt_5() {
    let port = mosquitto_15_port();
    let broker = project(port, &unique("probe-"), "", "", "").broker;
    let error =
        senzor_core::engine::test_connection(&broker, senzor_core::engine::CONNECTION_TEST_TIMEOUT)
            .await
            .unwrap_err();
    assert!(
        error.starts_with("the broker does not support MQTT 5"),
        "{error}"
    );

    let accepted = project(mosquitto_port(), &unique("probe-"), "", "", "").broker;
    assert_eq!(
        senzor_core::engine::test_connection(
            &accepted,
            senzor_core::engine::CONNECTION_TEST_TIMEOUT
        )
        .await,
        Ok(())
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the Docker brokers in dev/compose.yaml"]
async fn clearing_deletes_the_retained_messages_on_mosquitto_2() {
    use senzor_core::engine::{CLEAR_RETAINED_TIMEOUT, clear_retained, retained_topics};

    let port = mosquitto_port();
    for version in ["5", "3.1.1"] {
        let prefix = unique("clear");
        let mut project = project(
            port,
            &prefix,
            &format!("mqtt_version = \"{version}\""),
            "",
            "",
        );
        for instance in project
            .devices
            .iter_mut()
            .flat_map(|d| d.instances.iter_mut())
        {
            instance.retain = true;
        }
        let mut sub = Subscriber::connect(port, &format!("{prefix}/state")).await;
        let engine = spawn(project.clone());
        start_online(&engine).await;
        sub.wait_for(WAIT, |_| true).await.unwrap();
        engine.shutdown().await;
        let mut before = Subscriber::connect(port, &format!("{prefix}/#")).await;
        assert_eq!(
            before.collect(Duration::from_millis(500)).await.len(),
            2,
            "MQTT {version}: state and status are retained"
        );

        let topics = retained_topics(&project, None);
        assert_eq!(
            topics,
            [format!("{prefix}/state"), format!("{prefix}/status")]
        );
        clear_retained(&project.broker, &topics, CLEAR_RETAINED_TIMEOUT)
            .await
            .unwrap();
        let mut after = Subscriber::connect(port, &format!("{prefix}/#")).await;
        assert_eq!(
            after.collect(Duration::from_millis(500)).await,
            [],
            "MQTT {version}"
        );
    }
}
