//! The Streamable HTTP endpoint, spoken to with plain HTTP/1.1 requests.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use senzor_mcp::{HTTP_PATH, SenzorServer, Session, serve_http};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::runtime::Handle;
use tokio::sync::mpsc;

const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#;

async fn endpoint() -> SocketAddr {
    let session = Session::new(&Handle::current(), None, false);
    let view = session.view();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(session.run(rx));
    let server = SenzorServer::new(view, tx, Arc::new(|| {}), "Senzor");
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(serve_http(listener, server));
    address
}

/// POSTs `body` with `host` as the Host header; returns the status code and the response text.
async fn post(address: SocketAddr, host: &str, body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let request = format!(
        "POST {HTTP_PATH} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response)).await;
    let response = String::from_utf8_lossy(&response).into_owned();
    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    (status, response)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_loopback_client_can_initialize() {
    let address = endpoint().await;
    let (status, response) = post(address, &address.to_string(), INITIALIZE).await;
    assert_eq!(status, 200, "{response}");
    assert!(response.contains(r#""name":"senzor""#), "{response}");
    let (status, response) = post(
        address,
        &format!("localhost:{}", address.port()),
        INITIALIZE,
    )
    .await;
    assert_eq!(status, 200, "{response}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_foreign_host_header_is_rejected() {
    let address = endpoint().await;
    let (status, response) = post(
        address,
        &format!("evil.example:{}", address.port()),
        INITIALIZE,
    )
    .await;
    assert!((400..500).contains(&status), "{status}: {response}");
    assert!(!response.contains("senzor"), "{response}");
}
