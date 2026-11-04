//! `senzor-cli mcp`: the MCP server on stdin/stdout, driven with raw JSON-RPC.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use senzor_testkit::{Subscriber, start_broker};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const WAIT: Duration = Duration::from_secs(10);

fn project(port: u16) -> String {
    format!(
        r#"
[broker]
host = "127.0.0.1"
port = {port}

[[templates]]
id = "mcp"
interval = "200ms"
availability = {{ topic = "mcp/{{{{ device_id }}}}/status", online = "online", offline = "offline" }}
messages = [{{ topic = "mcp/{{{{ device_id }}}}/state", payload = "{{{{ v }}}}" }}]
fields.v = {{ generator = "constant", value = 1 }}

[[devices]]
id = "dev"
template = "mcp"
count = 2
"#
    )
}

fn write_project(name: &str, content: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("senzor-cli-mcp-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("project.toml");
    std::fs::write(&path, content).unwrap();
    path
}

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Lines<BufReader<ChildStdout>>,
    /// Every line the server wrote to stdout.
    lines: Vec<String>,
    next_id: u64,
}

impl Server {
    async fn start(args: &[&str]) -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_senzor-cli"))
            .arg("mcp")
            .args(args)
            .arg("-v")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut server = Server {
            child,
            stdin,
            stdout,
            lines: Vec::new(),
            next_id: 1,
        };
        let init = server
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-11-25",
                    "capabilities": {},
                    "clientInfo": {"name": "test", "version": "0"}
                }),
            )
            .await;
        assert_eq!(init["serverInfo"]["name"], "senzor", "{init}");
        server
            .send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;
        server
    }

    async fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        stdin
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
        stdin.flush().await.unwrap();
    }

    /// Sends a request and returns its `result`, skipping notifications.
    async fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        loop {
            let line = tokio::time::timeout(WAIT, self.stdout.next_line())
                .await
                .expect("a reply in time")
                .unwrap()
                .expect("stdout open");
            self.lines.push(line.clone());
            let message: Value = serde_json::from_str(&line).expect("stdout carries JSON");
            if message["id"] == id {
                assert!(message.get("error").is_none(), "{message}");
                return message["result"].clone();
            }
        }
    }

    async fn tool(&mut self, name: &str, arguments: Value) -> Value {
        let result = self
            .request("tools/call", json!({"name": name, "arguments": arguments}))
            .await;
        assert_ne!(result["isError"], true, "{name}: {result}");
        result["structuredContent"].clone()
    }

    /// Closes stdin, waits for the exit and returns the exit code and stderr.
    async fn close(mut self) -> (i32, String) {
        drop(self.stdin.take());
        let status = tokio::time::timeout(WAIT, self.child.wait())
            .await
            .expect("exits after stdin closes")
            .unwrap();
        while let Some(line) = self.stdout.next_line().await.unwrap() {
            self.lines.push(line);
        }
        for line in &self.lines {
            let message: Value = serde_json::from_str(line)
                .unwrap_or_else(|_| panic!("not JSON-RPC on stdout: {line}"));
            assert_eq!(message["jsonrpc"], "2.0", "{line}");
        }
        let mut stderr = String::new();
        self.child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .await
            .unwrap();
        (status.code().unwrap_or(-1), stderr)
    }
}

fn states(status: &Value) -> Vec<(String, String)> {
    status["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["id"].as_str().unwrap().to_string(),
                i["status"]["state"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_runs_the_project_and_disconnecting_stops_it_cleanly() {
    let port = start_broker();
    let path = write_project("run", &project(port));
    let mut sub = Subscriber::connect(port, "mcp/+/status").await;
    let mut server = Server::start(&[path.to_str().unwrap()]).await;

    let started = server.tool("start", json!({"all": true})).await;
    assert_eq!(started["timed_out"], false, "{started}");
    let status = server.tool("get_status", json!({})).await;
    assert_eq!(
        states(&status),
        [
            ("dev-0".into(), "online".into()),
            ("dev-1".into(), "online".into())
        ]
    );
    let messages = server
        .tool("get_messages", json!({"wait_seconds": 5}))
        .await;
    assert!(!messages["messages"].as_array().unwrap().is_empty());

    let (code, stderr) = server.close().await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("publish"), "-v logs go to stderr: {stderr}");
    let mut offline: Vec<String> = sub
        .collect(Duration::from_millis(500))
        .await
        .into_iter()
        .filter(|m| m.payload == "offline")
        .map(|m| m.topic)
        .collect();
    offline.sort();
    offline.dedup();
    assert_eq!(offline, ["mcp/dev-0/status", "mcp/dev-1/status"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn no_autostart_leaves_every_instance_stopped() {
    let port = start_broker();
    let path = write_project("no-autostart", &project(port));
    let mut server = Server::start(&[path.to_str().unwrap(), "--no-autostart"]).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let status = server.tool("get_status", json!({})).await;
    assert!(
        states(&status).iter().all(|(_, s)| s == "stopped"),
        "{status}"
    );
    let (code, _) = server.close().await;
    assert_eq!(code, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_broken_project_file_is_reported_through_get_status() {
    let path = write_project("broken", "[broker\n");
    let mut server = Server::start(&[path.to_str().unwrap()]).await;
    let status = server.tool("get_status", json!({})).await;
    let error = status["open_error"].as_str().expect("open_error");
    assert!(error.contains("line 1"), "{error}");
    assert_eq!(status["instances"], json!([]));
    assert_eq!(status["path"], Value::Null);
    let (code, stderr) = server.close().await;
    assert_eq!(code, 0);
    assert!(
        stderr.contains("starting with an empty project"),
        "{stderr}"
    );
}
