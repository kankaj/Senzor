//! Running the server over a transport.

use std::sync::Arc;

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
/// is dropped (abort its task to stop). Only requests whose `Host` is a loopback name are
/// answered, so a web page cannot reach the endpoint through DNS rebinding.
pub async fn serve_http(listener: TcpListener, server: SenzorServer) -> std::io::Result<()> {
    let config =
        StreamableHttpServerConfig::default().with_allowed_hosts(["localhost", "127.0.0.1", "::1"]);
    let service = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let router = axum::Router::new().nest_service(HTTP_PATH, service);
    axum::serve(listener, router).await
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
