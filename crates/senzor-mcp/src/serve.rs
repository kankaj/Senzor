//! Running the server over a transport.

use std::io;
use std::sync::Arc;

use axum::extract::Request;
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use rand::RngExt;
use rmcp::ServiceExt;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use crate::{SenzorServer, Session};

/// The port the GUI serves MCP on unless told otherwise.
pub const DEFAULT_HTTP_PORT: u16 = 18830;
/// The path of the MCP endpoint: `http://127.0.0.1:<port>/mcp`.
pub const HTTP_PATH: &str = "/mcp";

/// Serves `server` over Streamable HTTP at [`HTTP_PATH`] on `listener` until the returned future
/// is dropped (abort its task to stop).
///
/// With a `token`, only requests with `Authorization: Bearer <token>` are answered; the others
/// get 401. On a loopback address, only requests whose `Host` is a loopback name are answered,
/// so a web page cannot reach the endpoint through DNS rebinding. On any other address, which
/// other machines can reach, a token is required (an [`io::ErrorKind::InvalidInput`] error
/// without one) and any `Host` is answered: the token is what keeps others out.
pub async fn serve_http(
    listener: TcpListener,
    server: SenzorServer,
    token: Option<String>,
) -> io::Result<()> {
    let loopback = listener.local_addr()?.ip().is_loopback();
    if !loopback && token.is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a token is required to let other machines connect",
        ));
    }
    let config = if loopback {
        StreamableHttpServerConfig::default().with_allowed_hosts(["localhost", "127.0.0.1", "::1"])
    } else {
        StreamableHttpServerConfig::default().disable_allowed_hosts()
    };
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let mut router = axum::Router::new().nest_service(HTTP_PATH, service);
    if let Some(token) = token {
        let expected: Arc<str> = format!("Bearer {token}").into();
        router = router.layer(middleware::from_fn(move |request: Request, next: Next| {
            let expected = expected.clone();
            async move { authorize(request, next, &expected).await }
        }));
    }
    axum::serve(listener, router).await
}

/// Passes `request` on when its `Authorization` header is `expected`; answers 401 otherwise.
async fn authorize(request: Request, next: Next, expected: &str) -> Response {
    let given = request
        .headers()
        .get(header::AUTHORIZATION)
        .map(|value| value.as_bytes())
        .unwrap_or_default();
    if same_bytes(given, expected.as_bytes()) {
        next.run(request).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            "missing or wrong token",
        )
            .into_response()
    }
}

/// Compares without stopping at the first difference, so timing does not reveal the token.
fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

/// A new random token for [`serve_http`]: 32 letters and digits (about 190 bits).
pub fn new_token() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rng();
    (0..32)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect()
}

/// Serves `session` on stdin and stdout until the client disconnects or `stop` completes, then
/// returns the session (still running; call [`Session::shutdown`]).
pub async fn serve_stdio(session: Session, stop: impl Future<Output = ()>) -> Session {
    let view = session.view();
    let (requests, received) = mpsc::unbounded_channel();
    let owner = tokio::spawn(session.run(received));
    let server = SenzorServer::new(view, requests, Arc::new(|| {}), "Senzor");
    tokio::pin!(stop);
    // Dropping the server (or the running service, which owns it) closes the request channel,
    // which ends the owner task.
    tokio::select! {
        served = server.serve(rmcp::transport::stdio()) => match served {
            Ok(running) => {
                tokio::select! {
                    _ = running.waiting() => {}
                    () = &mut stop => {}
                }
            }
            Err(error) => tracing::error!("MCP client did not initialize: {error}"),
        },
        () = &mut stop => {}
    }
    owner.await.expect("the session task does not panic")
}
