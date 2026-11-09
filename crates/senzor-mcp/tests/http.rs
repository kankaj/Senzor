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

fn server() -> SenzorServer {
    let session = Session::new(&Handle::current(), None, false);
    let view = session.view();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(session.run(rx));
    SenzorServer::new(view, tx, Arc::new(|| {}), "Senzor")
}

/// An endpoint on `bind` (port 0), answering with `token` when set; the address to connect to.
async fn endpoint_on(bind: &str, token: Option<&str>) -> SocketAddr {
    let listener = TcpListener::bind(format!("{bind}:0")).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(serve_http(listener, server(), token.map(str::to_string)));
    SocketAddr::from(([127, 0, 0, 1], port))
}

async fn endpoint() -> SocketAddr {
    endpoint_on("127.0.0.1", None).await
}

/// POSTs `body` with `host` as the Host header; returns the status code and the response text.
async fn post(address: SocketAddr, host: &str, body: &str) -> (u16, String) {
    post_with(address, host, "", body).await
}

/// As [`post`], with `headers` (each ending in CRLF) added to the request.
async fn post_with(address: SocketAddr, host: &str, headers: &str, body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let request = format!(
        "POST {HTTP_PATH} HTTP/1.1\r\nHost: {host}\r\n{headers}Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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

#[tokio::test(flavor = "multi_thread")]
async fn with_a_token_only_requests_that_carry_it_are_answered() {
    let address = endpoint_on("127.0.0.1", Some("s3cret")).await;
    let host = address.to_string();
    for headers in [
        "",
        "Authorization: Bearer wrong\r\n",
        "Authorization: s3cret\r\n",
    ] {
        let (status, response) = post_with(address, &host, headers, INITIALIZE).await;
        assert_eq!(status, 401, "{headers:?}: {response}");
        assert!(response.contains("WWW-Authenticate") || response.contains("www-authenticate"));
        assert!(!response.contains("senzor"), "{response}");
    }
    let (status, response) = post_with(
        address,
        &host,
        "Authorization: Bearer s3cret\r\n",
        INITIALIZE,
    )
    .await;
    assert_eq!(status, 200, "{response}");
}

#[tokio::test(flavor = "multi_thread")]
async fn beyond_this_machine_any_host_name_is_answered_with_the_token() {
    let address = endpoint_on("0.0.0.0", Some("s3cret")).await;
    let host = format!("senzor.lan:{}", address.port());
    let (status, _) = post(address, &host, INITIALIZE).await;
    assert_eq!(status, 401);
    let (status, response) = post_with(
        address,
        &host,
        "Authorization: Bearer s3cret\r\n",
        INITIALIZE,
    )
    .await;
    assert_eq!(status, 200, "{response}");
}

#[tokio::test(flavor = "multi_thread")]
async fn listening_beyond_this_machine_without_a_token_is_refused() {
    let listener = TcpListener::bind("0.0.0.0:0").await.unwrap();
    let error = serve_http(listener, server(), None).await.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(error.to_string().contains("token"), "{error}");
}

#[test]
fn new_tokens_are_long_and_differ() {
    let (a, b) = (senzor_mcp::new_token(), senzor_mcp::new_token());
    assert_ne!(a, b);
    assert!(
        a.len() >= 32 && a.chars().all(|c| c.is_ascii_alphanumeric()),
        "{a}"
    );
}
