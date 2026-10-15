use std::time::Duration;

use rumqttc::{LastWill, MqttOptions, QoS, TlsConfiguration, Transport};

use crate::project::BrokerConfig;

const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// Largest message (topic + payload) an instance publishes; larger ones are counted as errors.
pub(crate) const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// rumqttc treats an over-limit packet as a connection error, so its limit is the message plus header room.
const MAX_PACKET_BYTES: usize = MAX_MESSAGE_BYTES + 1024;

/// Connection options for one instance: client ID, keep-alive, credentials, Last Will, TLS.
pub(crate) fn mqtt_options(
    broker: &BrokerConfig,
    instance_id: &str,
    will: Option<LastWill>,
) -> MqttOptions {
    let client_id = format!("{}{}", broker.client_id_prefix, instance_id);
    let mut options = MqttOptions::new(client_id, broker.host.clone(), broker.port);
    options.set_keep_alive(broker.keep_alive);
    options.set_clean_session(true);
    options.set_max_packet_size(MAX_PACKET_BYTES, MAX_PACKET_BYTES);
    if let Some(credentials) = &broker.credentials {
        options.set_credentials(credentials.username.clone(), credentials.password.clone());
    }
    if let Some(will) = will {
        options.set_last_will(will);
    }
    if let Some(tls) = &broker.tls {
        let configuration = match &tls.ca {
            Some(ca) => TlsConfiguration::Simple {
                ca: ca.clone(),
                alpn: None,
                client_auth: tls.client_auth.clone(),
            },
            None => TlsConfiguration::default(),
        };
        options.set_transport(Transport::Tls(configuration));
    }
    options
}

pub(crate) fn qos(level: u8) -> QoS {
    match level {
        0 => QoS::AtMostOnce,
        1 => QoS::AtLeastOnce,
        _ => QoS::ExactlyOnce,
    }
}

/// Delay before reconnect attempt `attempt` (1-based): 1 s, 2 s, 4 s, … capped at 30 s.
pub(crate) fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(2u64.saturating_pow(attempt.saturating_sub(1))).min(MAX_BACKOFF)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Credentials, TlsConfig};

    fn broker() -> BrokerConfig {
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
        }
    }

    #[test]
    fn options_carry_client_id_credentials_and_will() {
        let will = LastWill::new("s/status", "offline", QoS::AtLeastOnce, true);
        let options = mqtt_options(&broker(), "office-0", Some(will.clone()));
        assert_eq!(options.client_id(), "senzor-office-0");
        assert_eq!(options.broker_address(), ("mqtt.local".to_string(), 1884));
        assert_eq!(options.keep_alive(), Duration::from_secs(30));
        let login = options.credentials().unwrap();
        assert_eq!(
            (login.username.as_str(), login.password.as_str()),
            ("dev", "secret")
        );
        assert_eq!(options.last_will(), Some(will));
        assert!(matches!(options.transport(), Transport::Tcp));
    }

    #[test]
    fn tls_with_ca_uses_simple_configuration() {
        let mut config = broker();
        config.tls = Some(TlsConfig {
            ca: Some(b"CA".to_vec()),
            client_auth: None,
        });
        let options = mqtt_options(&config, "x", None);
        assert!(matches!(
            options.transport(),
            Transport::Tls(TlsConfiguration::Simple { ca, client_auth: None, .. }) if ca == b"CA".to_vec()
        ));
    }

    #[test]
    fn backoff_doubles_up_to_thirty_seconds() {
        let delays: Vec<u64> = (1..=8).map(|a| backoff(a).as_secs()).collect();
        assert_eq!(delays, [1, 2, 4, 8, 16, 30, 30, 30]);
    }

    #[test]
    fn qos_levels_map() {
        assert_eq!(qos(0), QoS::AtMostOnce);
        assert_eq!(qos(1), QoS::AtLeastOnce);
        assert_eq!(qos(2), QoS::ExactlyOnce);
    }
}
