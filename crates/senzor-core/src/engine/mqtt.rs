//! The MQTT connection of one instance, over rumqttc's 3.1.1 client or its MQTT 5 client. The
//! instance loop in `instance.rs` sees only this module's version-neutral types.

use std::time::Duration;

use rumqttc::v5::mqttbytes::v5::{
    ConnectReturnCode as Reason, LastWill as LastWill5, LastWillProperties, PubAckReason,
    PublishProperties,
};
use rumqttc::{LastWill, MqttOptions, TlsConfiguration, Transport};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::project::{BrokerConfig, MqttVersion, TlsConfig, mqtt_seconds};
use crate::render::MessageProperties;

const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Largest message (topic + payload) an instance publishes; larger ones are counted as errors.
pub(crate) const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// How many QoS 1 and 2 messages a connection may have waiting for the broker's acknowledgement.
/// rumqttc keeps a slot for each up front; its MQTT 5 client has no limit of its own and would
/// make 65,536 of them, about 14 MB per instance. An instance publishes a few messages per tick, so
/// 100, the 3.1.1 client's default, is plenty.
const MAX_INFLIGHT: u16 = 100;

/// rumqttc treats an over-limit packet as a connection error, so its limit is the message plus header room.
const MAX_PACKET_BYTES: usize = MAX_MESSAGE_BYTES + 1024;

/// Room for a PUBLISH packet's fixed header, topic length, packet ID and property-list length.
pub(crate) const PUBLISH_HEADER_BYTES: usize = 16;

/// How long the check for a 3.1.1-only broker may take.
const MQTT311_CHECK_TIMEOUT: Duration = Duration::from_secs(2);

/// The Last Will: the retained availability `offline`, QoS 1, with an optional MQTT 5 delay.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Will {
    pub topic: String,
    pub payload: String,
    /// MQTT 5 will delay in seconds.
    pub delay: Option<u32>,
}

/// The sending half of an instance's connection.
pub(crate) enum Client {
    V311(rumqttc::AsyncClient),
    V5(rumqttc::v5::AsyncClient),
}

/// The receiving half: drives the connection and reports what happened.
pub(crate) enum Connection {
    // Boxed: the two event loops differ a lot in size.
    V311(Box<rumqttc::EventLoop>),
    V5(Box<rumqttc::v5::EventLoop>),
}

/// What a successful `Connection::poll` saw.
#[derive(Debug, PartialEq)]
pub(crate) enum ConnEvent {
    /// The broker accepted the connection. On MQTT 5 it may limit the size of the packets it
    /// accepts; rumqttc treats a larger publish as a connection error.
    ConnAck {
        max_packet_size: Option<u32>,
    },
    /// The broker acknowledged a QoS 1 publish. On MQTT 5, `refused` is its reason when it did
    /// not accept the message, e.g. `NotAuthorized`; MQTT 3.1.1 has no way to say so.
    PubAck {
        refused: Option<String>,
    },
    Other,
}

/// Why the connection failed.
#[derive(Debug, PartialEq)]
pub(crate) enum ConnError {
    /// The broker refused the connection; the text is its reason.
    Refused(String),
    /// An MQTT 5 broker refused the connection because it does not support MQTT 5.
    UnsupportedVersion,
    /// MQTT 5 only: the broker closed the connection abruptly. Before the first CONNACK this is
    /// also how a 3.1.1-only broker's refusal looks, because the MQTT 5 client cannot read it;
    /// [`answers_mqtt311`] tells the two apart.
    Aborted(String),
    Other(String),
}

/// Creates the client and connection for instance `instance_id`, in the broker's MQTT version.
pub(crate) fn connect(
    broker: &BrokerConfig,
    instance_id: &str,
    will: Option<Will>,
    queue_capacity: usize,
) -> (Client, Connection) {
    match broker.mqtt_version {
        MqttVersion::V311 => {
            let (client, eventloop) =
                rumqttc::AsyncClient::new(v311_options(broker, instance_id, will), queue_capacity);
            (Client::V311(client), Connection::V311(Box::new(eventloop)))
        }
        MqttVersion::V5 => {
            let (client, eventloop) = rumqttc::v5::AsyncClient::new(
                v5_options(broker, instance_id, will),
                queue_capacity,
            );
            (Client::V5(client), Connection::V5(Box::new(eventloop)))
        }
    }
}

fn client_id(broker: &BrokerConfig, instance_id: &str) -> String {
    format!("{}{}", broker.client_id_prefix, instance_id)
}

/// MQTT 3.1.1 options: client ID, keep-alive, clean session, packet size, credentials, Last Will, TLS.
fn v311_options(broker: &BrokerConfig, instance_id: &str, will: Option<Will>) -> MqttOptions {
    let mut options = MqttOptions::new(
        client_id(broker, instance_id),
        broker.host.clone(),
        broker.port,
    );
    options.set_keep_alive(broker.keep_alive);
    options.set_clean_session(true);
    options.set_max_packet_size(MAX_PACKET_BYTES, MAX_PACKET_BYTES);
    options.set_inflight(MAX_INFLIGHT);
    if let Some(credentials) = &broker.credentials {
        options.set_credentials(credentials.username.clone(), credentials.password.clone());
    }
    if let Some(will) = will {
        options.set_last_will(LastWill::new(
            will.topic,
            will.payload,
            rumqttc::QoS::AtLeastOnce,
            true,
        ));
    }
    if let Some(tls) = &broker.tls {
        options.set_transport(transport(tls));
    }
    options
}

/// MQTT 5 options: as 3.1.1, plus session expiry and the will delay. A clean start only when the
/// session expires at once: with a session expiry, a reconnect continues the session, so a
/// reconnect within the will delay cancels the Last Will.
fn v5_options(
    broker: &BrokerConfig,
    instance_id: &str,
    will: Option<Will>,
) -> rumqttc::v5::MqttOptions {
    let mut options = rumqttc::v5::MqttOptions::new(
        client_id(broker, instance_id),
        broker.host.clone(),
        broker.port,
    );
    options.set_keep_alive(broker.keep_alive);
    let session_expiry = mqtt_seconds(broker.session_expiry);
    options.set_clean_start(session_expiry == 0);
    options.set_session_expiry_interval((session_expiry > 0).then_some(session_expiry));
    options.set_max_packet_size(Some(MAX_PACKET_BYTES as u32));
    options.set_outgoing_inflight_upper_limit(MAX_INFLIGHT);
    if let Some(credentials) = &broker.credentials {
        options.set_credentials(credentials.username.clone(), credentials.password.clone());
    }
    if let Some(will) = will {
        let properties = will.delay.map(|delay| LastWillProperties {
            delay_interval: Some(delay),
            payload_format_indicator: None,
            message_expiry_interval: None,
            content_type: None,
            response_topic: None,
            correlation_data: None,
            user_properties: Vec::new(),
        });
        options.set_last_will(LastWill5::new(
            will.topic,
            will.payload,
            rumqttc::v5::mqttbytes::QoS::AtLeastOnce,
            true,
            properties,
        ));
    }
    if let Some(tls) = &broker.tls {
        options.set_transport(transport(tls));
    }
    options
}

fn transport(tls: &TlsConfig) -> Transport {
    let configuration = match &tls.ca {
        Some(ca) => TlsConfiguration::Simple {
            ca: ca.clone(),
            alpn: None,
            client_auth: tls.client_auth.clone(),
        },
        None => TlsConfiguration::default(),
    };
    Transport::Tls(configuration)
}

impl Client {
    /// Queues a publish without waiting; fails when the request queue is full.
    pub(crate) fn try_publish(
        &self,
        topic: &str,
        qos: u8,
        retain: bool,
        payload: &str,
        properties: &MessageProperties,
    ) -> Result<(), String> {
        match self {
            Client::V311(client) => client
                .try_publish(topic, qos_v311(qos), retain, payload)
                .map_err(|e| e.to_string()),
            Client::V5(client) => client
                .try_publish_with_properties(
                    topic,
                    qos_v5(qos),
                    retain,
                    payload.to_string(),
                    publish_properties(properties),
                )
                .map_err(|e| e.to_string()),
        }
    }

    /// Queues a DISCONNECT without waiting; the broker then discards the Last Will. A full queue
    /// is ignored: the caller drops the connection soon after anyway.
    pub(crate) fn try_disconnect(&self) {
        match self {
            Client::V311(client) => {
                let _ = client.try_disconnect();
            }
            Client::V5(client) => {
                let _ = client.try_disconnect();
            }
        }
    }
}

fn publish_properties(properties: &MessageProperties) -> PublishProperties {
    PublishProperties {
        message_expiry_interval: properties.message_expiry,
        content_type: properties.content_type.clone(),
        user_properties: properties.user_properties.clone(),
        ..PublishProperties::default()
    }
}

impl Connection {
    /// Drives the connection until the next event; connects or reconnects first when needed.
    pub(crate) async fn poll(&mut self) -> Result<ConnEvent, ConnError> {
        match self {
            Connection::V311(eventloop) => match eventloop.poll().await {
                Ok(rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(_))) => {
                    Ok(ConnEvent::ConnAck {
                        max_packet_size: None,
                    })
                }
                Ok(rumqttc::Event::Incoming(rumqttc::Packet::PubAck(_))) => {
                    Ok(ConnEvent::PubAck { refused: None })
                }
                Ok(_) => Ok(ConnEvent::Other),
                Err(rumqttc::ConnectionError::ConnectionRefused(code)) => {
                    Err(ConnError::Refused(format!("{code:?}")))
                }
                Err(e) => Err(ConnError::Other(e.to_string())),
            },
            Connection::V5(eventloop) => match eventloop.poll().await {
                Ok(rumqttc::v5::Event::Incoming(rumqttc::v5::mqttbytes::v5::Packet::ConnAck(
                    ack,
                ))) => Ok(ConnEvent::ConnAck {
                    max_packet_size: ack.properties.and_then(|p| p.max_packet_size),
                }),
                Ok(rumqttc::v5::Event::Incoming(rumqttc::v5::mqttbytes::v5::Packet::PubAck(
                    ack,
                ))) => Ok(ConnEvent::PubAck {
                    refused: v5_puback_refusal(ack.reason),
                }),
                Ok(_) => Ok(ConnEvent::Other),
                Err(rumqttc::v5::ConnectionError::ConnectionRefused(reason)) => {
                    Err(v5_refusal(reason))
                }
                Err(
                    e @ rumqttc::v5::ConnectionError::MqttState(
                        rumqttc::v5::StateError::ConnectionAborted,
                    ),
                ) => Err(ConnError::Aborted(e.to_string())),
                Err(e) => Err(ConnError::Other(e.to_string())),
            },
        }
    }
}

/// Whether the broker answers an MQTT 3.1.1 handshake with a 3.1.1 CONNACK, accepting or refusing
/// it. Asked after the broker closed an MQTT 5 connection before accepting it: then yes means it
/// supports only 3.1.1. `None` over TLS, which this check does not speak.
pub(crate) async fn answers_mqtt311(broker: &BrokerConfig, instance_id: &str) -> Option<bool> {
    if broker.tls.is_some() {
        return None;
    }
    let check = async {
        let mut stream = tokio::net::TcpStream::connect((broker.host.as_str(), broker.port))
            .await
            .ok()?;
        let client_id = format!("{}-check", client_id(broker, instance_id));
        stream.write_all(&mqtt311_connect(&client_id)).await.ok()?;
        let mut reply = [0u8; 4];
        stream.read_exact(&mut reply).await.ok()?;
        // A 3.1.1 CONNACK: type 2, remaining length 2. Leave cleanly if it was accepted.
        let connack = reply[..2] == [0x20, 0x02];
        if connack && reply[3] == 0 {
            let _ = stream.write_all(&[0xe0, 0x00]).await;
        }
        Some(connack)
    };
    Some(
        tokio::time::timeout(MQTT311_CHECK_TIMEOUT, check)
            .await
            .ok()
            .flatten()
            .unwrap_or(false),
    )
}

/// An MQTT 3.1.1 CONNECT with a clean session, keep-alive 60 s and no credentials.
fn mqtt311_connect(client_id: &str) -> Vec<u8> {
    let id = client_id.as_bytes();
    let mut body = vec![0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x3c];
    body.extend_from_slice(&(id.len() as u16).to_be_bytes());
    body.extend_from_slice(id);
    let mut packet = vec![0x10];
    // The remaining length as a variable-length integer.
    let mut length = body.len();
    loop {
        let mut byte = (length % 128) as u8;
        length /= 128;
        if length > 0 {
            byte |= 0x80;
        }
        packet.push(byte);
        if length == 0 {
            break;
        }
    }
    packet.extend_from_slice(&body);
    packet
}

fn v5_refusal(reason: Reason) -> ConnError {
    match reason {
        Reason::UnsupportedProtocolVersion => ConnError::UnsupportedVersion,
        other => ConnError::Refused(format!("{other:?}")),
    }
}

/// Why an MQTT 5 broker did not accept a publish, or `None` when it did.
fn v5_puback_refusal(reason: PubAckReason) -> Option<String> {
    match reason {
        PubAckReason::Success | PubAckReason::NoMatchingSubscribers => None,
        other => Some(format!("{other:?}")),
    }
}

fn qos_v311(level: u8) -> rumqttc::QoS {
    match level {
        0 => rumqttc::QoS::AtMostOnce,
        1 => rumqttc::QoS::AtLeastOnce,
        _ => rumqttc::QoS::ExactlyOnce,
    }
}

fn qos_v5(level: u8) -> rumqttc::v5::mqttbytes::QoS {
    match level {
        0 => rumqttc::v5::mqttbytes::QoS::AtMostOnce,
        1 => rumqttc::v5::mqttbytes::QoS::AtLeastOnce,
        _ => rumqttc::v5::mqttbytes::QoS::ExactlyOnce,
    }
}

/// Delay before reconnect attempt `attempt` (1-based): 1 s, 2 s, 4 s, … capped at 30 s.
pub(crate) fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(2u64.saturating_pow(attempt.saturating_sub(1))).min(MAX_BACKOFF)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Credentials;

    fn broker(mqtt_version: MqttVersion) -> BrokerConfig {
        BrokerConfig {
            host: "mqtt.local".into(),
            port: 1884,
            client_id_prefix: "senzor-".into(),
            keep_alive: Duration::from_secs(30),
            credentials: Some(Credentials {
                username: "dev".into(),
                password: "secret".into(),
            }),
            tls: None,
            mqtt_version,
            session_expiry: Duration::ZERO,
        }
    }

    /// rumqttc's MQTT 5 client keeps a slot for every packet ID it may have in flight, made up
    /// front: without a limit that is 65,536 slots, about 14 MB for each instance.
    #[test]
    fn both_versions_keep_at_most_a_hundred_messages_in_flight() {
        let v5 = v5_options(&broker(MqttVersion::V5), "d", None);
        assert_eq!(v5.get_outgoing_inflight_upper_limit(), Some(MAX_INFLIGHT));
        let v311 = v311_options(&broker(MqttVersion::V311), "d", None);
        assert_eq!(v311.inflight(), MAX_INFLIGHT);
    }

    fn will(delay: Option<u32>) -> Will {
        Will {
            topic: "s/status".into(),
            payload: "offline".into(),
            delay,
        }
    }

    #[test]
    fn v311_options_carry_client_id_credentials_and_will() {
        let options = v311_options(&broker(MqttVersion::V311), "office-0", Some(will(None)));
        assert_eq!(options.client_id(), "senzor-office-0");
        assert_eq!(options.broker_address(), ("mqtt.local".to_string(), 1884));
        assert_eq!(options.keep_alive(), Duration::from_secs(30));
        assert!(options.clean_session());
        let login = options.credentials().unwrap();
        assert_eq!(
            (login.username.as_str(), login.password.as_str()),
            ("dev", "secret")
        );
        assert_eq!(
            options.last_will(),
            Some(LastWill::new(
                "s/status",
                "offline",
                rumqttc::QoS::AtLeastOnce,
                true
            ))
        );
        assert!(matches!(options.transport(), Transport::Tcp));
    }

    #[test]
    fn v5_options_carry_client_id_credentials_will_and_its_delay() {
        let options = v5_options(&broker(MqttVersion::V5), "office-0", Some(will(Some(5))));
        assert_eq!(options.client_id(), "senzor-office-0");
        assert_eq!(options.broker_address(), ("mqtt.local".to_string(), 1884));
        assert_eq!(options.keep_alive(), Duration::from_secs(30));
        let login = options.credentials().unwrap();
        assert_eq!(
            (login.username.as_str(), login.password.as_str()),
            ("dev", "secret")
        );
        let last_will = options.last_will().unwrap();
        assert_eq!(last_will.topic, "s/status");
        assert_eq!(&last_will.message[..], b"offline");
        assert!(last_will.retain);
        assert_eq!(last_will.properties.unwrap().delay_interval, Some(5));
        assert_eq!(options.max_packet_size(), Some(MAX_PACKET_BYTES as u32));
    }

    #[test]
    fn v5_clean_start_only_without_session_expiry() {
        let mut config = broker(MqttVersion::V5);
        let options = v5_options(&config, "x", None);
        assert!(options.clean_start());
        assert_eq!(options.session_expiry_interval(), None);

        config.session_expiry = Duration::from_secs(60);
        let options = v5_options(&config, "x", None);
        assert!(!options.clean_start());
        assert_eq!(options.session_expiry_interval(), Some(60));
    }

    #[test]
    fn tls_with_ca_uses_simple_configuration_on_both_versions() {
        let tls = TlsConfig {
            ca: Some(b"CA".to_vec()),
            client_auth: None,
        };
        let is_simple = |transport: Transport| {
            matches!(
                transport,
                Transport::Tls(TlsConfiguration::Simple { ca, client_auth: None, .. }) if ca == b"CA".to_vec()
            )
        };
        let mut config = broker(MqttVersion::V311);
        config.tls = Some(tls);
        assert!(is_simple(v311_options(&config, "x", None).transport()));
        config.mqtt_version = MqttVersion::V5;
        assert!(is_simple(v5_options(&config, "x", None).transport()));
    }

    #[test]
    fn mqtt5_refusals_map_to_reasons_and_unsupported_version() {
        assert_eq!(
            v5_refusal(Reason::BadUserNamePassword),
            ConnError::Refused("BadUserNamePassword".into())
        );
        assert_eq!(
            v5_refusal(Reason::UnsupportedProtocolVersion),
            ConnError::UnsupportedVersion
        );
    }

    #[test]
    fn publish_properties_carry_expiry_content_type_and_user_properties() {
        let properties = publish_properties(&MessageProperties {
            message_expiry: Some(60),
            content_type: Some("application/json".into()),
            user_properties: vec![("room".into(), "office".into())],
        });
        assert_eq!(properties.message_expiry_interval, Some(60));
        assert_eq!(properties.content_type.as_deref(), Some("application/json"));
        assert_eq!(
            properties.user_properties,
            [("room".into(), "office".into())]
        );
        assert_eq!(
            publish_properties(&MessageProperties::default()),
            PublishProperties::default()
        );
    }

    #[test]
    fn mqtt5_pubacks_name_the_reason_only_when_the_broker_did_not_accept() {
        assert_eq!(v5_puback_refusal(PubAckReason::Success), None);
        assert_eq!(v5_puback_refusal(PubAckReason::NoMatchingSubscribers), None);
        assert_eq!(
            v5_puback_refusal(PubAckReason::NotAuthorized),
            Some("NotAuthorized".into())
        );
    }

    #[test]
    fn the_3_1_1_check_encodes_long_client_ids() {
        let short = mqtt311_connect("abc");
        assert_eq!(short[..2], [0x10, 10 + 2 + 3]);
        let long = mqtt311_connect(&"x".repeat(200));
        // 212 bytes of body: two length bytes, 0xd4 0x01.
        assert_eq!(long[..3], [0x10, 0xd4, 0x01]);
        assert_eq!(long.len(), 3 + 212);
    }

    #[test]
    fn backoff_doubles_up_to_thirty_seconds() {
        let delays: Vec<u64> = (1..=8).map(|a| backoff(a).as_secs()).collect();
        assert_eq!(delays, [1, 2, 4, 8, 16, 30, 30, 30]);
    }

    #[test]
    fn qos_levels_map() {
        assert_eq!(qos_v311(0), rumqttc::QoS::AtMostOnce);
        assert_eq!(qos_v311(1), rumqttc::QoS::AtLeastOnce);
        assert_eq!(qos_v311(2), rumqttc::QoS::ExactlyOnce);
        assert_eq!(qos_v5(2), rumqttc::v5::mqttbytes::QoS::ExactlyOnce);
    }
}
