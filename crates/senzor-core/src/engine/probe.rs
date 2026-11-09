//! A one-off connection that tells whether the broker settings work, before any device runs.

use std::time::Duration;

use super::instance::{MAYBE_MQTT311_ONLY, MQTT5_UNSUPPORTED};
use super::mqtt::{ConnError, ConnEvent, answers_mqtt311, connect};
use crate::project::{BrokerConfig, MqttVersion};

/// The client ID the test connects with, after the broker's client ID prefix.
const CLIENT_ID: &str = "connection-test";

/// How long the GUI's connection test waits for the broker.
pub const CONNECTION_TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Connects once with `broker`, as an instance would but without a Last Will and without
/// retrying, then disconnects. `Err` says why it failed, in the words a failed instance uses.
pub async fn test_connection(broker: &BrokerConfig, timeout: Duration) -> Result<(), String> {
    let (client, mut connection) = connect(broker, CLIENT_ID, None, 4);
    let attempt = async {
        loop {
            match connection.poll().await {
                Ok(ConnEvent::ConnAck { .. }) => {
                    client.try_disconnect();
                    // Sends the DISCONNECT; the broker then closes the connection.
                    let _ = tokio::time::timeout(Duration::from_secs(1), connection.poll()).await;
                    return Ok(());
                }
                Ok(_) => {}
                Err(error) => return Err(connect_failure(broker, CLIENT_ID, error).await),
            }
        }
    };
    tokio::time::timeout(timeout, attempt)
        .await
        .unwrap_or_else(|_| Err(no_answer(broker, timeout)))
}

/// Why a one-off connection with client ID `client_id` failed before the broker accepted it, in
/// the words a failed instance uses.
pub(super) async fn connect_failure(
    broker: &BrokerConfig,
    client_id: &str,
    error: ConnError,
) -> String {
    match error {
        ConnError::Refused(reason) => format!("broker refused the connection: {reason}"),
        ConnError::UnsupportedVersion => MQTT5_UNSUPPORTED.to_string(),
        ConnError::Aborted(text) if broker.mqtt_version == MqttVersion::V5 => {
            match answers_mqtt311(broker, client_id).await {
                Some(true) => MQTT5_UNSUPPORTED.to_string(),
                Some(false) => text,
                None => format!("{text}; {MAYBE_MQTT311_ONLY}"),
            }
        }
        ConnError::Aborted(error) | ConnError::Other(error) => error,
    }
}

/// The broker did not accept a connection within `timeout`.
pub(super) fn no_answer(broker: &BrokerConfig, timeout: Duration) -> String {
    format!(
        "no answer from {}:{} within {} s",
        broker.host,
        broker.port,
        timeout.as_secs_f32()
    )
}

#[cfg(test)]
mod tests {
    use rumqttc::ConnectReturnCode;
    use senzor_testkit::{free_port, refusing_broker, slow_broker, start_broker};

    use super::*;
    use crate::project::MqttVersion;

    fn broker(port: u16) -> BrokerConfig {
        BrokerConfig {
            host: "127.0.0.1".into(),
            port,
            client_id_prefix: "senzor-".into(),
            keep_alive: Duration::from_secs(30),
            credentials: None,
            tls: None,
            mqtt_version: MqttVersion::V5,
            transport: crate::project::Transport::Tcp,
            path: "/mqtt".into(),
            session_expiry: Duration::ZERO,
        }
    }

    #[tokio::test]
    async fn a_running_broker_accepts_the_test() {
        let port = start_broker();
        assert_eq!(
            test_connection(&broker(port), CONNECTION_TEST_TIMEOUT).await,
            Ok(())
        );
    }

    #[tokio::test]
    async fn a_refusal_says_why() {
        let port = refusing_broker(ConnectReturnCode::BadUserNamePassword).await;
        let error = test_connection(&broker(port), CONNECTION_TEST_TIMEOUT)
            .await
            .unwrap_err();
        assert!(
            error.starts_with("broker refused the connection: "),
            "{error}"
        );
    }

    #[tokio::test]
    async fn nothing_listening_fails_at_once() {
        let error = test_connection(&broker(free_port()), CONNECTION_TEST_TIMEOUT)
            .await
            .unwrap_err();
        assert!(!error.contains("no answer"), "{error}");
    }

    #[tokio::test]
    async fn a_broker_that_does_not_answer_runs_into_the_timeout() {
        let port = slow_broker(Duration::from_secs(5)).await;
        let error = test_connection(&broker(port), Duration::from_millis(300))
            .await
            .unwrap_err();
        assert_eq!(
            error,
            format!("no answer from 127.0.0.1:{port} within 0.3 s")
        );
    }
}
