//! An agent talking to a headless session over an in-memory MCP connection.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::{RoleClient, ServiceExt};
use senzor_mcp::{SenzorServer, Session};
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub const WAIT: Duration = Duration::from_secs(5);

/// A project on broker `port`: device `office` (2 instances) and `lab`, neither autostarting,
/// publishing `<v>/<w>` on `t/<id>/state` every 100 ms, with availability on `t/<id>/status`.
pub fn project(port: u16) -> String {
    format!(
        r#"[broker]
host = "127.0.0.1"
port = {port}

[[templates]]
id = "air"
interval = "100ms"
availability = {{ topic = "t/{{{{ device_id }}}}/status", online = "online", offline = "offline" }}
messages = [{{ topic = "t/{{{{ device_id }}}}/state", payload = "{{{{ v }}}}/{{{{ w }}}}" }}]
fields.v = {{ generator = "constant", value = 1 }}
fields.w = {{ generator = "constant", value = 2 }}

[[devices]]
id = "office"
template = "air"
count = 2
autostart = false

[[devices]]
id = "lab"
template = "air"
autostart = false
"#
    )
}

/// An empty folder for one test.
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("senzor-mcp-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub struct Agent {
    pub client: RunningService<RoleClient, ()>,
    owner: JoinHandle<Session>,
    pub port: u16,
    pub dir: PathBuf,
}

impl Agent {
    /// A session on a fresh broker with [`project`] saved as `project.toml` and opened.
    pub async fn with_project(name: &str) -> Agent {
        let port = senzor_testkit::start_broker();
        let dir = temp_dir(name);
        let path = dir.join("project.toml");
        std::fs::write(&path, project(port)).unwrap();
        Self::open(Some(&path), true, port, dir).await
    }

    /// A session that opens `path` (or starts empty).
    pub async fn open(path: Option<&Path>, autostart: bool, port: u16, dir: PathBuf) -> Agent {
        let session = Session::new(&Handle::current(), path, autostart);
        let view = session.view();
        let (tx, rx) = mpsc::unbounded_channel();
        let owner = tokio::spawn(session.run(rx));
        let server = SenzorServer::new(view, tx, Arc::new(|| {}), "Senzor");
        let (server_end, client_end) = tokio::io::duplex(1 << 20);
        tokio::spawn(async move {
            if let Ok(running) = server.serve(server_end).await {
                let _ = running.waiting().await;
            }
        });
        let client = ().serve(client_end).await.unwrap();
        Agent {
            client,
            owner,
            port,
            dir,
        }
    }

    pub async fn call_raw(&self, tool: &str, args: Value) -> Result<CallToolResult, String> {
        let mut params = CallToolRequestParams::new(tool.to_string());
        if let Value::Object(map) = args {
            params = params.with_arguments(map);
        }
        self.client
            .call_tool(params)
            .await
            .map_err(|e| e.to_string())
    }

    /// The structured result of a call that must succeed.
    pub async fn ok(&self, tool: &str, args: Value) -> Value {
        let result = self.call_raw(tool, args).await.unwrap();
        assert_ne!(
            result.is_error,
            Some(true),
            "{tool} failed: {}",
            text(&result)
        );
        result.structured_content.expect("structured content")
    }

    /// The message of a call that must fail as a tool error.
    pub async fn err(&self, tool: &str, args: Value) -> String {
        let result = self.call_raw(tool, args).await.unwrap();
        assert_eq!(
            result.is_error,
            Some(true),
            "{tool} succeeded: {:?}",
            result.structured_content
        );
        text(&result)
    }

    pub async fn status_of(&self, id: &str) -> Value {
        let status = self.ok("get_status", json!({})).await;
        status["instances"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == id)
            .unwrap_or_else(|| panic!("no instance {id} in {status:#}"))["status"]
            .clone()
    }

    /// Disconnects and shuts the session down.
    pub async fn finish(self) {
        let _ = self.client.cancel().await;
        let session = self.owner.await.unwrap();
        session.shutdown().await;
    }
}

pub fn text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| {
            serde_json::to_value(c)
                .ok()?
                .get("text")?
                .as_str()
                .map(String::from)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn state(status: &Value) -> &str {
    status["state"].as_str().unwrap_or_default()
}
