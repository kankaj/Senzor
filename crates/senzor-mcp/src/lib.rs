//! The Senzor MCP server: tools that let an AI agent observe and control a simulation and edit
//! its project, served over stdio (`senzor-cli mcp`) or local HTTP (the GUI).
//!
//! The program that owns the project implements [`Host`] and applies [`Envelope`]s with
//! [`apply`]; [`Session`] is that program for the headless CLI.

mod host;
mod output;
mod params;
mod redact;
mod resources;
mod serve;
mod server;
mod session;
mod view;

pub use host::{Envelope, Host, Reply, Request, RunAction, Targets, apply};
pub use redact::REDACTED;
pub use serve::{DEFAULT_HTTP_PORT, HTTP_PATH, serve_http, serve_stdio};
pub use server::{REQUEST_TIMEOUT, SenzorServer, Wake};
pub use session::{Session, open_error_text};
pub use view::View;
