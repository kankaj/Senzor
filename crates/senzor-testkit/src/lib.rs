//! Test helpers: an in-process rumqttd broker, a collecting subscriber, and a broker that refuses connections.

use std::collections::BTreeSet;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rumqttc::{AsyncClient, ConnectReturnCode, Event, MqttOptions, Packet, QoS};
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
        let mut handed_out = HANDED_OUT
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if handed_out.insert(port) {
            return port;
        }
    }
}

/// Starts rumqttd (MQTT 3.1.1, no auth) on a free port in a background thread and returns the port.
/// The free port can be taken before rumqttd binds it, so a failed start retries on a new port.
pub fn start_broker() -> u16 {
    let mut last_error = String::new();
    for _ in 0..5 {
        let port = free_port();
        match start_broker_on(port) {
            Ok(()) => return port,
            Err(e) => last_error = e,
        }
    }
    panic!("could not start a broker: {last_error}");
}

/// Starts rumqttd on `port` and waits until it is ready. Fails when something else answers on
/// `port`, another broker included: rumqttd only logs a failed bind.
pub fn start_broker_on(port: u16) -> Result<(), String> {
    let config = format!(
        r#"
id = 0

[router]
id = 0
max_connections = 10010
max_outgoing_packet_count = 200
max_segment_size = 104857600
max_segment_count = 10

[v4.1]
name = "v4-1"
listen = "127.0.0.1:{port}"
next_connection_delay_ms = 1

[v4.1.connections]
connection_timeout_ms = 60000
max_payload_size = 20480
max_inflight_count = 100
dynamic_filters = true
"#
    );
    let config: rumqttd::Config = toml::from_str(&config).expect("valid rumqttd config");
    let mut broker = rumqttd::Broker::new(config);
    // A client inside this broker: the probe's message reaches it only if the listener on `port`
    // is this broker's, not another one's.
    let (mut link_tx, mut link_rx) = broker
        .link("testkit-probe-receiver")
        .map_err(|e| format!("in-process link: {e}"))?;
    link_tx
        .subscribe(PROBE_TOPIC)
        .map_err(|e| format!("in-process subscribe: {e}"))?;
    std::thread::spawn(move || {
        // A bind failure is only logged by rumqttd; the probe below detects it.
        let _ = broker.start();
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if mqtt_probe(port).is_ok() {
            // Its own budget: the probe may have succeeded just before (or, with its read
            // timeout, after) `deadline`.
            let arrival = std::time::Instant::now() + PROBE_ARRIVAL;
            return if probe_arrived(&mut link_rx, arrival) {
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

/// Whether `mqtt_probe`'s message reaches `link` before `deadline`.
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

/// Whether an MQTT broker on 127.0.0.1:`port` accepts an anonymous MQTT 3.1.1 connection.
/// It only connects and disconnects: nothing is published to the broker.
pub fn broker_answers(port: u16) -> bool {
    use std::io::Write;

    mqtt_connect(port)
        .and_then(|mut stream| stream.write_all(&DISCONNECT))
        .is_ok()
}

/// Sends a raw MQTT 3.1.1 CONNECT, expects an accepting CONNACK, then publishes an empty QoS 0
/// message on [`PROBE_TOPIC`]. A plain TCP connect is not enough: another listener, or a socket
/// connected to itself, would also accept it.
fn mqtt_probe(port: u16) -> std::io::Result<()> {
    use std::io::Write;

    let mut stream = mqtt_connect(port)?;
    // The remaining length counts the length-prefixed topic; the payload is empty.
    let mut publish = vec![0x30, (2 + PROBE_TOPIC.len()) as u8];
    publish.extend_from_slice(&[0x00, PROBE_TOPIC.len() as u8]);
    publish.extend_from_slice(PROBE_TOPIC.as_bytes());
    publish.extend_from_slice(&DISCONNECT);
    stream.write_all(&publish)
}

/// Sends a raw MQTT 3.1.1 CONNECT and returns the connection once an accepting CONNACK arrived.
fn mqtt_connect(port: u16) -> std::io::Result<std::net::TcpStream> {
    use std::io::{Read, Write};

    const CLIENT_ID: &[u8] = b"testkit-probe";
    // Protocol name "MQTT", level 4 (3.1.1), connect flags: clean session, keep-alive 60 s.
    const VARIABLE_HEADER: [u8; 10] = [0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3c];
    // The remaining length counts the variable header and the length-prefixed client ID.
    let mut connect = vec![0x10, (VARIABLE_HEADER.len() + 2 + CLIENT_ID.len()) as u8];
    connect.extend_from_slice(&VARIABLE_HEADER);
    connect.extend_from_slice(&[0x00, CLIENT_ID.len() as u8]);
    connect.extend_from_slice(CLIENT_ID);

    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.write_all(&connect)?;
    let mut reply = [0u8; 4];
    stream.read_exact(&mut reply)?;
    if reply != connack(ConnectReturnCode::Success) {
        let _ = stream.write_all(&DISCONNECT);
        return Err(std::io::Error::other(format!(
            "unexpected reply {reply:02x?}"
        )));
    }
    Ok(stream)
}

const DISCONNECT: [u8; 2] = [0xe0, 0x00];

/// A CONNACK carrying `code`, with the session-present flag clear.
fn connack(code: ConnectReturnCode) -> [u8; 4] {
    [0x20, 0x02, 0x00, code as u8]
}

/// One message as seen by a subscriber.
#[derive(Debug, Clone, PartialEq)]
pub struct Received {
    pub topic: String,
    pub payload: String,
    pub retain: bool,
}

/// An MQTT client that records every message on its subscription until it is dropped.
pub struct Subscriber {
    rx: mpsc::UnboundedReceiver<Received>,
    _client: AsyncClient,
}

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

impl Subscriber {
    /// Connects, subscribes with QoS 1 and returns once the broker confirmed the subscription.
    pub async fn connect(port: u16, filter: &str) -> Subscriber {
        let id = format!("testkit-sub-{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));
        let mut options = MqttOptions::new(id, "127.0.0.1", port);
        options.set_max_packet_size(2 * 1024 * 1024, 2 * 1024 * 1024);
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
                        let received = Received {
                            topic: p.topic.clone(),
                            payload: String::from_utf8_lossy(&p.payload).into_owned(),
                            retain: p.retain,
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
        Subscriber {
            rx,
            _client: client,
        }
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

/// A fake broker that answers every CONNECT with a CONNACK carrying `code`.
pub async fn refusing_broker(code: ConnectReturnCode) -> u16 {
    fake_broker(Duration::ZERO, code).await
}

/// A fake broker that accepts every CONNECT, but only after `delay` (like a slow remote TLS broker).
pub async fn slow_broker(delay: Duration) -> u16 {
    fake_broker(delay, ConnectReturnCode::Success).await
}

async fn fake_broker(delay: Duration, code: ConnectReturnCode) -> u16 {
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
                if socket.read(&mut buffer).await.unwrap_or(0) == 0 {
                    return;
                }
                tokio::time::sleep(delay).await;
                let _ = socket.write_all(&connack(code)).await;
                while socket.read(&mut buffer).await.unwrap_or(0) > 0 {}
            });
        }
    });
    port
}
