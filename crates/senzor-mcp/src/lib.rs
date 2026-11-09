//! The Senzor MCP server: tools that let an AI agent observe and control a simulation and edit
//! its project, served over stdio (`senzor-cli mcp`) or HTTP (the GUI, `senzor-cli run --mcp-port`).
//!
//! The program that owns the project implements [`Host`] and applies [`Envelope`]s with
//! [`apply`]; [`Session`] is that program for the headless CLI.

mod host;
mod key_path;
mod live;
mod output;
mod params;
mod prompts;
mod redact;
mod resources;
mod serve;
mod server;
mod session;
mod subscriptions;
mod view;

pub use host::{Envelope, Host, Reply, Request, RunAction, Targets, apply};
pub use redact::REDACTED;
pub use serve::{DEFAULT_HTTP_PORT, HTTP_PATH, new_token, serve_http, serve_stdio};
pub use server::{REQUEST_TIMEOUT, SenzorServer, Wake};
pub use session::{Session, open_error_text};
pub use view::View;
