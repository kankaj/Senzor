//! Test helpers: an in-process rumqttd broker with an MQTT 5 and an MQTT 3.1.1 listener, a
//! collecting MQTT 5 subscriber, fake brokers that refuse or delay connections, and the ports of
//! the Docker brokers in `dev/compose.yaml`.

use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use rumqttc::ConnectReturnCode;
use rumqttc::v5::mqttbytes::QoS;
use rumqttc::v5::mqttbytes::v5::Packet;
use rumqttc::v5::{AsyncClient, Event, MqttOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

/// Every port `free_port` has returned in this process.
static HANDED_OUT: Mutex<BTreeSet<u16>> = Mutex::new(BTreeSet::new());

/// A port that was free a moment ago and that no earlier call in this process returned. The
/// kernel may pick a just-released port again: two tests starting brokers at once would then
/// collide, and the second `start_broker` would lose about 2 s before retrying; and a port a test
/// uses as "nothing listens here" could get a broker of another test.
pub fn free_port() -> u16 {
    loop {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind an ephemeral port")
            .local_addr()
            .expect("local address")
            .port();
        let mut handed_out = HANDED_OUT.lock().unwrap_or_else(PoisonError::into_inner);
        if handed_out.insert(port) {
            return port;
        }
    }
}

/// An MQTT protocol version as the CONNECT packet's protocol level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    V311,
    V5,
}

impl Protocol {
    fn level(self) -> u8 {
        match self {
            Protocol::V311 => 4,
            Protocol::V5 => 5,
        }
    }
}

/// The two listeners of one in-process broker. Both reach the same router, so a client on one
/// listener receives what clients on the other publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrokerPorts {
    pub v5: u16,
    pub v311: u16,
}

impl BrokerPorts {
    /// The listener for clients of `protocol`.
    pub fn port(&self, protocol: Protocol) -> u16 {
        match protocol {
            Protocol::V5 => self.v5,
            Protocol::V311 => self.v311,
        }
    }
}

/// Starts rumqttd (no auth) in a background thread and returns its MQTT 5 port, the protocol
/// projects use by default. See [`start_broker_ports`] for the MQTT 3.1.1 listener.
pub fn start_broker() -> u16 {
    start_broker_ports().v5
}

/// Starts rumqttd (no auth) on two free ports and returns both. A free port can be taken before
/// rumqttd binds it, so a failed start retries on new ports.
pub fn start_broker_ports() -> BrokerPorts {
    let mut last_error = String::new();
    for _ in 0..5 {
        let ports = BrokerPorts {
            v5: free_port(),
            v311: free_port(),
        };
        match start(ports) {
            Ok(()) => return ports,
            Err(e) => last_error = e,
        }
    }
    panic!("could not start a broker: {last_error}");
}

/// Starts rumqttd with its MQTT 5 listener on `port` (and its 3.1.1 listener on a free port), and
/// waits until it is ready. Fails when something else answers on either port, another broker
/// included: rumqttd only logs a failed bind.
///
/// rumqttd cannot be stopped, so after a failure the listener that did start keeps running in
/// its thread. The 3.1.1 port is checked just before the start to keep that rare.
pub fn start_broker_on(port: u16) -> Result<BrokerPorts, String> {
    let v311 = loop {
        let candidate = free_port();
        if std::net::TcpListener::bind(("127.0.0.1", candidate)).is_ok() {
            break candidate;
        }
    };
    let ports = BrokerPorts { v5: port, v311 };
    start(ports).map(|()| ports)
}

fn start(ports: BrokerPorts) -> Result<(), String> {
    let listener = |version: &str, port: u16| {
        format!(
            r#"
[{version}.1]
name = "{version}-1"
listen = "127.0.0.1:{port}"
next_connection_delay_ms = 1

[{version}.1.connections]
connection_timeout_ms = 60000
max_payload_size = 20480
max_inflight_count = 100
dynamic_filters = true
"#
        )
    };
    let config = format!(
        r#"
id = 0

[router]
id = 0
max_connections = 10010
max_outgoing_packet_count = 200
max_segment_size = 104857600
max_segment_count = 10
{}{}"#,
        listener("v4", ports.v311),
        listener("v5", ports.v5)
    );
    let config: rumqttd::Config = toml::from_str(&config).expect("valid rumqttd config");
    let mut broker = rumqttd::Broker::new(config);
    // A client inside this broker: a probe's message reaches it only if the listener on the
    // port is this broker's, not another one's.
    let (mut link_tx, mut link_rx) = broker
        .link("testkit-probe-receiver")
        .map_err(|e| format!("in-process link: {e}"))?;
    link_tx
        .subscribe(PROBE_TOPIC)
        .map_err(|e| format!("in-process subscribe: {e}"))?;
    std::thread::spawn(move || {
        // A bind failure is only logged by rumqttd; the probes below detect it.
        let _ = broker.start();
    });
    for (port, protocol) in [(ports.v5, Protocol::V5), (ports.v311, Protocol::V311)] {
        wait_for_own_listener(&mut link_rx, port, protocol)?;
    }
    Ok(())
}

/// Waits until `port` answers a `protocol` handshake and the probe's message reaches `link`.
fn wait_for_own_listener(
    link: &mut rumqttd::local::LinkRx,
    port: u16,
    protocol: Protocol,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if mqtt_probe(port, protocol).is_ok() {
            // Its own budget: the probe may have succeeded just before (or, with its read
            // timeout, after) `deadline`.
            let arrival = std::time::Instant::now() + PROBE_ARRIVAL;
            return if probe_arrived(link, arrival) {
                Ok(())
            } else {
                Err(format!("another MQTT broker answered on port {port}"))
            };
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Err(format!("no MQTT broker answered on port {port}"))
}

/// The topic `mqtt_probe` publishes on.
const PROBE_TOPIC: &str = "testkit/probe";

/// How long the probe's message may take to reach the broker's in-process link.
const PROBE_ARRIVAL: Duration = Duration::from_secs(1);

/// Whether a probe's message reaches `link` before `deadline`.
fn probe_arrived(link: &mut rumqttd::local::LinkRx, deadline: std::time::Instant) -> bool {
    loop {
        match link.recv_deadline(deadline) {
            Ok(Some(rumqttd::Notification::Forward(forward)))
                if forward.publish.topic == PROBE_TOPIC =>
            {
                return true;
            }
            Ok(_) => {}
            Err(_) => return false,
        }
    }
}

/// Whether an MQTT broker on 127.0.0.1:`port` accepts an anonymous connection with MQTT 5 or,
/// failing that, MQTT 3.1.1. It only connects and disconnects: nothing is published.
pub fn broker_answers(port: u16) -> bool {
    [Protocol::V5, Protocol::V311]
        .into_iter()
        .any(|protocol| broker_accepts(port, protocol))
}

/// Whether an MQTT broker on 127.0.0.1:`port` accepts an anonymous `protocol` connection, as an
/// MQTT 5 project needs. It only connects and disconnects: nothing is published.
pub fn broker_accepts(port: u16, protocol: Protocol) -> bool {
    mqtt_connect(port, protocol)
        .and_then(|mut stream| stream.write_all(&DISCONNECT))
        .is_ok()
}

/// Sends a raw CONNECT, expects an accepting CONNACK, then publishes an empty QoS 0 message on
/// [`PROBE_TOPIC`]. A plain TCP connect is not enough: another listener, or a socket connected to
/// itself, would also accept it.
fn mqtt_probe(port: u16, protocol: Protocol) -> std::io::Result<()> {
    let mut stream = mqtt_connect(port, protocol)?;
    // The remaining length counts the length-prefixed topic, the MQTT 5 properties length (an
    // empty property list) and the empty payload.
    let properties: &[u8] = match protocol {
        Protocol::V311 => &[],
        Protocol::V5 => &[0x00],
    };
    let mut publish = vec![0x30, (2 + PROBE_TOPIC.len() + properties.len()) as u8];
    publish.extend_from_slice(&[0x00, PROBE_TOPIC.len() as u8]);
    publish.extend_from_slice(PROBE_TOPIC.as_bytes());
    publish.extend_from_slice(properties);
    publish.extend_from_slice(&DISCONNECT);
    stream.write_all(&publish)
}

/// Sends a raw CONNECT and returns the connection once an accepting CONNACK arrived.
fn mqtt_connect(port: u16, protocol: Protocol) -> std::io::Result<std::net::TcpStream> {
    // A client ID of its own for every probe: probes that follow each other with one ID make the
    // broker take the previous session over, which can drop the new connection.
    static NEXT_PROBE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let client_id = format!(
        "testkit-probe-{}-{}",
        std::process::id(),
        NEXT_PROBE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let client_id = client_id.as_bytes();
    // Protocol name "MQTT", the level, connect flags: clean session, keep-alive 60 s; MQTT 5
    // adds an empty property list.
    let mut variable_header = vec![0x00, 0x04, b'M', b'Q', b'T', b'T', protocol.level(), 0x02];
    variable_header.extend_from_slice(&[0x00, 0x3c]);
    if protocol == Protocol::V5 {
        variable_header.push(0x00);
    }
    // The remaining length counts the variable header and the length-prefixed client ID.
    let mut connect = vec![0x10, (variable_header.len() + 2 + client_id.len()) as u8];
    connect.extend_from_slice(&variable_header);
    connect.extend_from_slice(&[0x00, client_id.len() as u8]);
    connect.extend_from_slice(client_id);

    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.write_all(&connect)?;
    let mut header = [0u8; 2];
    stream.read_exact(&mut header)?;
    let mut body = vec![0u8; usize::from(header[1])];
    stream.read_exact(&mut body)?;
    // Both versions: flags, then the return code (3.1.1) or reason code (5); 0 accepts.
    if header[0] != 0x20 || body.get(1) != Some(&0) {
        let _ = stream.write_all(&DISCONNECT);
        return Err(std::io::Error::other(format!(
            "unexpected reply {header:02x?} {body:02x?}"
        )));
    }
    Ok(stream)
}

const DISCONNECT: [u8; 2] = [0xe0, 0x00];

/// A CONNACK carrying `code` for a client of `protocol`, with the session-present flag clear.
/// MQTT 5 clients get the reason code that means the same as the 3.1.1 return code.
fn connack(protocol: Protocol, code: ConnectReturnCode) -> Vec<u8> {
    match protocol {
        Protocol::V311 => vec![0x20, 0x02, 0x00, code as u8],
        Protocol::V5 => vec![0x20, 0x03, 0x00, v5_reason(code), 0x00],
    }
}

/// The MQTT 5 CONNACK reason code for a 3.1.1 return code.
fn v5_reason(code: ConnectReturnCode) -> u8 {
    match code {
        ConnectReturnCode::Success => 0x00,
        ConnectReturnCode::RefusedProtocolVersion => 0x84,
        ConnectReturnCode::BadClientId => 0x85,
        ConnectReturnCode::ServiceUnavailable => 0x88,
        ConnectReturnCode::BadUserNamePassword => 0x86,
        ConnectReturnCode::NotAuthorized => 0x87,
    }
}

/// One message as seen by a subscriber, with its MQTT 5 properties.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Received {
    pub topic: String,
    pub payload: String,
    pub retain: bool,
    pub message_expiry: Option<u32>,
    pub content_type: Option<String>,
    pub user_properties: Vec<(String, String)>,
}

/// An MQTT 5 client that records every message on its subscription until it is dropped.
pub struct Subscriber {
    rx: mpsc::UnboundedReceiver<Received>,
    client: AsyncClient,
}

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

impl Subscriber {
    /// Connects with MQTT 5, subscribes with QoS 1 and returns once the broker confirmed the
    /// subscription.
    pub async fn connect(port: u16, filter: &str) -> Subscriber {
        let id = format!("testkit-sub-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
        let mut options = MqttOptions::new(id, "127.0.0.1", port);
        options.set_max_packet_size(Some(2 * 1024 * 1024));
        let (client, mut eventloop) = AsyncClient::new(options, 100);
        client
            .subscribe(filter, QoS::AtLeastOnce)
            .await
            .expect("queue subscribe");
        let (tx, rx) = mpsc::unbounded_channel();
        let (subscribed_tx, subscribed_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let mut subscribed_tx = Some(subscribed_tx);
            loop {
                // Dropping the client does not end `poll()` (the event loop holds a request
                // sender of its own), so stop once the Subscriber has dropped its receiver.
                let event = tokio::select! {
                    () = tx.closed() => return,
                    event = eventloop.poll() => event,
                };
                match event {
                    Ok(Event::Incoming(Packet::SubAck(_))) => {
                        if let Some(done) = subscribed_tx.take() {
                            let _ = done.send(());
                        }
                    }
                    Ok(Event::Incoming(Packet::Publish(p))) => {
                        let properties = p.properties.unwrap_or_default();
                        let received = Received {
                            topic: String::from_utf8_lossy(&p.topic).into_owned(),
                            payload: String::from_utf8_lossy(&p.payload).into_owned(),
                            retain: p.retain,
                            message_expiry: properties.message_expiry_interval,
                            content_type: properties.content_type,
                            user_properties: properties.user_properties,
                        };
                        if tx.send(received).is_err() {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                }
            }
        });
        tokio::time::timeout(Duration::from_secs(5), subscribed_rx)
            .await
            .expect("subscription confirmed in time")
            .expect("subscriber task alive");
        Subscriber { rx, client }
    }

    /// The first message matching `pred` within `timeout`; earlier non-matching messages are dropped.
    pub async fn wait_for(
        &mut self,
        timeout: Duration,
        pred: impl Fn(&Received) -> bool,
    ) -> Option<Received> {
        tokio::time::timeout(timeout, async {
            while let Some(message) = self.rx.recv().await {
                if pred(&message) {
                    return Some(message);
                }
            }
            None
        })
        .await
        .ok()
        .flatten()
    }

    /// Publishes `payload` on `topic` with QoS 1, as any other client of the broker would.
    pub async fn publish(&self, topic: &str, payload: &str) {
        self.client
            .publish(topic, QoS::AtLeastOnce, false, payload.to_string())
            .await
            .expect("queue publish");
    }

    /// Every message that arrives within `during`.
    pub async fn collect(&mut self, during: Duration) -> Vec<Received> {
        let mut messages = Vec::new();
        let _ = tokio::time::timeout(during, async {
            while let Some(message) = self.rx.recv().await {
                messages.push(message);
            }
        })
        .await;
        messages
    }
}

/// A fake broker that answers every CONNECT with a CONNACK carrying `code`, in the client's
/// protocol version (MQTT 5 clients get the equivalent reason code).
pub async fn refusing_broker(code: ConnectReturnCode) -> u16 {
    fake_broker(Duration::ZERO, code, None).await
}

/// A fake broker that accepts every CONNECT, but only after `delay` (like a slow remote TLS
/// broker), in the client's protocol version.
pub async fn slow_broker(delay: Duration) -> u16 {
    fake_broker(delay, ConnectReturnCode::Success, None).await
}

/// A fake broker that accepts every CONNECT in the client's protocol version, then closes the
/// connection `after` that, like a broker that restarts.
pub async fn closing_broker(after: Duration) -> u16 {
    fake_broker(Duration::ZERO, ConnectReturnCode::Success, Some(after)).await
}

/// Answers each CONNECT after `delay` with `code`, then keeps the connection open until the
/// client closes it, or for `close_after` at most.
async fn fake_broker(
    delay: Duration,
    code: ConnectReturnCode,
    close_after: Option<Duration>,
) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("local address").port();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buffer = [0u8; 1024];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                if read == 0 {
                    return;
                }
                let protocol = connect_protocol(&buffer[..read]);
                tokio::time::sleep(delay).await;
                let _ = socket.write_all(&connack(protocol, code)).await;
                let drain = async { while socket.read(&mut buffer).await.unwrap_or(0) > 0 {} };
                match close_after {
                    Some(after) => {
                        let _ = tokio::time::timeout(after, drain).await;
                    }
                    None => drain.await,
                }
            });
        }
    });
    port
}

/// The protocol of a CONNECT packet: MQTT 5 when its protocol level is 5, else 3.1.1.
fn connect_protocol(connect: &[u8]) -> Protocol {
    // Skip the packet type and the remaining length (1 to 4 bytes, the last below 0x80).
    let length_bytes = connect[1..]
        .iter()
        .take(4)
        .position(|b| b & 0x80 == 0)
        .map_or(1, |i| i + 1);
    // Then the length-prefixed protocol name "MQTT" (6 bytes) and the protocol level.
    match connect.get(1 + length_bytes + 6) {
        Some(5) => Protocol::V5,
        _ => Protocol::V311,
    }
}

/// The Mosquitto 2 broker of `dev/compose.yaml` (MQTT 5 and 3.1.1, anonymous).
pub const MOSQUITTO_PORT: u16 = 18831;
/// The Mosquitto 1.5 broker of `dev/compose.yaml` (MQTT 3.1.1 only, anonymous).
pub const MOSQUITTO_15_PORT: u16 = 18832;
/// The WebSocket listener of the Mosquitto 2 broker of `dev/compose.yaml`.
pub const MOSQUITTO_WS_PORT: u16 = 18833;

/// The WebSocket port of the Docker Mosquitto 2: `SENZOR_MOSQUITTO_WS_PORT` or
/// [`MOSQUITTO_WS_PORT`]. Call [`mosquitto_port`] first: it checks that the broker is up.
pub fn mosquitto_ws_port() -> u16 {
    std::env::var("SENZOR_MOSQUITTO_WS_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MOSQUITTO_WS_PORT)
}

/// The port of the Docker Mosquitto 2: `SENZOR_MOSQUITTO_PORT` or [`MOSQUITTO_PORT`]. Panics with
/// how to start it when no MQTT 5 broker answers there.
pub fn mosquitto_port() -> u16 {
    docker_broker("SENZOR_MOSQUITTO_PORT", MOSQUITTO_PORT, Protocol::V5)
}

/// The port of the Docker Mosquitto 1.5: `SENZOR_MOSQUITTO15_PORT` or [`MOSQUITTO_15_PORT`].
/// Panics with how to start it when no MQTT 3.1.1 broker answers there.
pub fn mosquitto_15_port() -> u16 {
    docker_broker("SENZOR_MOSQUITTO15_PORT", MOSQUITTO_15_PORT, Protocol::V311)
}

fn docker_broker(variable: &str, default: u16, protocol: Protocol) -> u16 {
    let port = std::env::var(variable)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default);
    assert!(
        broker_accepts(port, protocol),
        "no MQTT {protocol:?} broker answers on 127.0.0.1:{port}; start the Docker brokers with \
         `docker compose -f dev/compose.yaml up -d` (or set {variable})"
    );
    port
}
