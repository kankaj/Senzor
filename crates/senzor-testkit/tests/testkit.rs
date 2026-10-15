use std::time::Duration;

use rumqttc::{AsyncClient, ConnectReturnCode, ConnectionError, MqttOptions, QoS};
use senzor_testkit::{
    Received, Subscriber, broker_answers, free_port, refusing_broker, start_broker, start_broker_on,
};

#[tokio::test(flavor = "multi_thread")]
async fn subscriber_receives_messages_from_the_broker() {
    let port = start_broker();
    let mut subscriber = Subscriber::connect(port, "kit/#").await;
    let (client, mut eventloop) =
        AsyncClient::new(MqttOptions::new("kit-pub", "127.0.0.1", port), 10);
    client
        .publish("kit/a", QoS::AtLeastOnce, true, "hello")
        .await
        .unwrap();
    tokio::spawn(async move { while eventloop.poll().await.is_ok() {} });

    let got = subscriber
        .wait_for(Duration::from_secs(5), |m| m.topic == "kit/a")
        .await;
    assert_eq!(
        got,
        Some(Received {
            topic: "kit/a".into(),
            payload: "hello".into(),
            retain: false
        })
    );
}

#[tokio::test]
async fn a_dropped_subscriber_stops_its_task() {
    let port = start_broker();
    let metrics = tokio::runtime::Handle::current().metrics();
    let before = metrics.num_alive_tasks();
    let subscriber = Subscriber::connect(port, "kit/#").await;
    assert_eq!(metrics.num_alive_tasks(), before + 1, "one event-loop task");

    drop(subscriber);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    while metrics.num_alive_tasks() > before {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the event-loop task outlived its subscriber"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn refusing_broker_returns_the_given_code() {
    let port = refusing_broker(ConnectReturnCode::BadUserNamePassword).await;
    let (_client, mut eventloop) =
        AsyncClient::new(MqttOptions::new("kit-refused", "127.0.0.1", port), 10);
    match eventloop.poll().await {
        Err(ConnectionError::ConnectionRefused(code)) => {
            assert_eq!(code, ConnectReturnCode::BadUserNamePassword)
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn free_port_is_bindable() {
    let port = free_port();
    std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
}

#[test]
fn free_port_never_hands_out_a_port_twice() {
    // The kernel picks from a few thousand ports, so a few hundred draws repeat one.
    let ports: Vec<u16> = (0..500).map(|_| free_port()).collect();
    let distinct: std::collections::HashSet<u16> = ports.iter().copied().collect();
    assert_eq!(distinct.len(), ports.len(), "a port was handed out twice");
}

/// A port whose listener accepts connections and never answers.
fn non_mqtt_listener() -> u16 {
    let squatter = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = squatter.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in squatter.incoming() {
            std::mem::forget(stream);
        }
    });
    port
}

#[test]
fn a_port_held_by_a_non_mqtt_listener_is_not_reported_as_a_broker() {
    assert!(start_broker_on(non_mqtt_listener()).is_err());
}

#[test]
fn a_started_broker_answers() {
    assert!(broker_answers(start_broker()));
}

#[test]
fn nothing_answers_on_a_free_port() {
    assert!(!broker_answers(free_port()));
}

#[test]
fn a_non_mqtt_listener_does_not_answer() {
    assert!(!broker_answers(non_mqtt_listener()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_broker_that_refuses_the_connection_does_not_answer() {
    let port = refusing_broker(ConnectReturnCode::NotAuthorized).await;
    let answers = tokio::task::spawn_blocking(move || broker_answers(port))
        .await
        .unwrap();
    assert!(!answers);
}

#[test]
fn a_port_served_by_another_broker_is_not_reported_as_this_broker() {
    let port = start_broker();
    let error = start_broker_on(port).unwrap_err();
    assert!(error.contains("another MQTT broker"), "{error}");
}

#[test]
fn started_broker_answers_an_mqtt_handshake() {
    let port = start_broker();
    let (_client, mut connection) =
        rumqttc::Client::new(MqttOptions::new("kit-handshake", "127.0.0.1", port), 10);
    let first = connection
        .recv_timeout(Duration::from_secs(5))
        .expect("an event in time");
    assert!(
        matches!(
            first,
            Ok(rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(_)))
        ),
        "{first:?}"
    );
}
