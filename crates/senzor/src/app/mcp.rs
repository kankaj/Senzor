//! The window as an MCP host: an agent's requests act on the window's document and engine
//! through the same paths as the user's own actions, one at a time, at the start of a frame.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use senzor_core::engine::{Command, EngineObserver, Target};
use senzor_core::project::{Document, ProjectFile};
use senzor_mcp::{Envelope, Host, Request, SenzorServer, View, apply, serve_http};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::SenzorApp;
use crate::selection::Selection;
use crate::ui::Modal;

/// At most this many requests are applied in one frame; the rest wait for the next.
const REQUESTS_PER_FRAME: usize = 16;

/// Whether the window serves MCP, as the top bar's toggle shows it.
#[derive(Debug, Clone, PartialEq)]
pub enum McpStatus {
    /// AI agents can connect at this address.
    Listening(SocketAddr),
    /// Not serving. Turning MCP on uses `port`; `error` says why the last attempt failed.
    Off { port: u16, error: Option<String> },
}

/// A running endpoint. The window has one only while MCP is on.
pub(super) struct Mcp {
    requests: mpsc::UnboundedReceiver<Envelope>,
    view: View,
    server: SenzorServer,
    task: JoinHandle<std::io::Result<()>>,
    address: SocketAddr,
    /// What the view last showed, so it is only republished after a change.
    published: Option<Published>,
}

#[derive(PartialEq)]
struct Published {
    file: ProjectFile,
    path: Option<PathBuf>,
    dirty: bool,
    engine: u64,
    open_error: Option<String>,
}

/// The selected device or template by ID, so an agent's edit that moves it keeps it selected.
enum Selected {
    Device(String),
    Template(String),
    Unchanged,
}

impl SenzorApp {
    /// Turns MCP on at launch, on `port` (the `--mcp` and `--mcp-port` flags), also when the
    /// settings had already turned it on at another port. The settings keep their port.
    pub fn with_mcp(mut self, port: u16) -> Self {
        if self
            .mcp
            .as_ref()
            .is_some_and(|mcp| mcp.address.port() != port)
        {
            self.stop_mcp();
        }
        self.start_mcp(port);
        self
    }

    /// Turns MCP on at launch on the port in the settings (the `--mcp` flag).
    pub fn with_mcp_on(mut self) -> Self {
        self.start_mcp(self.mcp_port);
        self
    }

    /// The port the MCP toggle uses, without turning MCP on.
    pub fn with_mcp_port(mut self, port: u16) -> Self {
        self.mcp_port = port;
        self
    }

    /// Serves MCP at `http://127.0.0.1:<port>/mcp` until [`SenzorApp::stop_mcp`]. If the port
    /// cannot be used, MCP stays off and a dialog says why. Does nothing while MCP is on.
    pub fn start_mcp(&mut self, port: u16) {
        if self.mcp.is_some() {
            return;
        }
        self.mcp_port = port;
        let bound = self
            .runtime
            .block_on(TcpListener::bind((Ipv4Addr::LOCALHOST, port)))
            .and_then(|listener| Ok((listener.local_addr()?, listener)));
        let (address, listener) = match bound {
            Ok(bound) => bound,
            Err(e) => {
                let reason = format!("cannot listen on 127.0.0.1:{port}: {e}");
                if self.ui.modal.is_none() {
                    self.ui.modal =
                        Some(Modal::Error(format!("AI agents cannot connect: {reason}")));
                }
                self.mcp_error = Some(reason);
                return;
            }
        };
        let (requests, received) = mpsc::unbounded_channel();
        let view = View::default();
        let ctx = self.ctx.clone();
        let server = SenzorServer::new(
            view.clone(),
            requests,
            Arc::new(move || ctx.request_repaint()),
            "the Senzor window",
        );
        let task = self.runtime.spawn(serve_http(listener, server.clone()));
        self.mcp_error = None;
        self.mcp = Some(Mcp {
            requests: received,
            view,
            server,
            task,
            address,
            published: None,
        });
        self.publish_view();
    }

    /// Closes the endpoint. Requests still waiting fail; connected agents lose their session.
    pub fn stop_mcp(&mut self) {
        if let Some(mcp) = self.mcp.take() {
            mcp.task.abort();
        }
    }

    pub fn mcp_status(&self) -> McpStatus {
        match &self.mcp {
            Some(mcp) => McpStatus::Listening(mcp.address),
            None => McpStatus::Off {
                port: self.mcp_port,
                error: self.mcp_error.clone(),
            },
        }
    }

    /// The server behind the endpoint while MCP is on, for connecting to it without HTTP (tests).
    pub fn mcp_server(&self) -> Option<SenzorServer> {
        self.mcp.as_ref().map(|mcp| mcp.server.clone())
    }

    /// Applies the requests that arrived since the last frame.
    pub(super) fn handle_mcp_requests(&mut self) {
        let Some(mcp) = self.mcp.as_mut() else {
            return;
        };
        let mut envelopes = Vec::new();
        while envelopes.len() < REQUESTS_PER_FRAME {
            match mcp.requests.try_recv() {
                Ok(envelope) => envelopes.push(envelope),
                Err(_) => break,
            }
        }
        let more = !mcp.requests.is_empty();
        for envelope in envelopes {
            if envelope.is_cancelled() {
                continue;
            }
            let selected = self.selected();
            let renamed = renamed(&envelope.request);
            let file_before = self.doc.file().clone();
            let engine_before = self.engine_generation;
            let result = apply(self, envelope.request);
            // Opening a project resets the panels itself.
            if self.engine_generation == engine_before && *self.doc.file() != file_before {
                // Text still being typed would show (and later commit) the old values.
                self.ui.buffers.clear();
                self.ui.pending_rename = None;
                self.reselect(selected, renamed);
            }
            self.publish_view();
            let _ = envelope.reply.send(result);
        }
        if more {
            self.ctx.request_repaint();
        }
    }

    fn selected(&self) -> Selected {
        let file = self.doc.file();
        match self.ui.selection {
            Some(Selection::Device(i)) => file
                .devices
                .get(i)
                .map_or(Selected::Unchanged, |d| Selected::Device(d.id.clone())),
            Some(Selection::Template(i)) => file
                .templates
                .get(i)
                .map_or(Selected::Unchanged, |t| Selected::Template(t.id.clone())),
            _ => Selected::Unchanged,
        }
    }

    /// Points the selection at the item it showed before the edit, wherever it is now.
    fn reselect(&mut self, selected: Selected, renamed: Option<(Kind, String, String)>) {
        let follow = |kind: Kind, id: String| match &renamed {
            Some((k, from, to)) if *k == kind && *from == id => to.clone(),
            _ => id,
        };
        let file = self.doc.file();
        self.ui.selection = match selected {
            Selected::Device(id) => {
                let id = follow(Kind::Device, id);
                file.devices
                    .iter()
                    .position(|d| d.id == id)
                    .map(Selection::Device)
            }
            Selected::Template(id) => {
                let id = follow(Kind::Template, id);
                file.templates
                    .iter()
                    .position(|t| t.id == id)
                    .map(Selection::Template)
            }
            Selected::Unchanged => return,
        };
    }

    /// Hands the current document and engine to the server's readers, if anything changed.
    pub(super) fn publish_view(&mut self) {
        let Some(mcp) = self.mcp.as_mut() else {
            return;
        };
        let unchanged = mcp.published.as_ref().is_some_and(|p| {
            p.file == *self.doc.file()
                && p.path.as_deref() == self.doc.path()
                && p.dirty == self.doc.is_dirty()
                && p.engine == self.engine_generation
                && p.open_error == self.open_error
        });
        if unchanged {
            return;
        }
        let Some(engine) = self.engine.as_ref() else {
            return;
        };
        mcp.view
            .publish(&self.doc, engine.observer(), self.open_error.clone());
        mcp.published = Some(Published {
            file: self.doc.file().clone(),
            path: self.doc.path().map(PathBuf::from),
            dirty: self.doc.is_dirty(),
            engine: self.engine_generation,
            open_error: self.open_error.clone(),
        });
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Device,
    Template,
}

fn renamed(request: &Request) -> Option<(Kind, String, String)> {
    match request {
        Request::RenameDevice { from, to } => Some((Kind::Device, from.clone(), to.clone())),
        Request::RenameTemplate { from, to } => Some((Kind::Template, from.clone(), to.clone())),
        _ => None,
    }
}

impl Host for SenzorApp {
    fn document(&self) -> &Document {
        &self.doc
    }

    fn edit(&mut self, f: &mut dyn FnMut(&mut ProjectFile)) -> bool {
        let changed = self.doc.edit(f);
        if changed {
            self.document_changed();
        }
        changed
    }

    fn send(&mut self, command: Command) {
        match command {
            // As the Stop all button: nothing waiting for a replaced engine starts afterwards.
            Command::Stop(Target::All) => {
                self.autostart_pending = false;
                self.queued.clear();
                SenzorApp::send(self, command);
            }
            command => self.send_when_ready(command),
        }
    }

    fn replace(&mut self, doc: Document, autostart: bool) {
        self.replace_document(doc);
        if !autostart {
            self.autostart_pending = false;
        }
    }

    fn save(&mut self, path: Option<PathBuf>) -> Result<(), String> {
        match path {
            None => self.doc.save()?,
            Some(path) => self.doc.save_as(path)?,
        }
        // Saving elsewhere can change how relative TLS paths resolve.
        SenzorApp::send(
            self,
            Command::ReplaceProject(self.doc.validated().project.clone()),
        );
        Ok(())
    }

    fn observer(&self) -> EngineObserver {
        self.engine().observer()
    }

    fn commands_sent(&self) -> u64 {
        self.engine().commands_sent()
    }

    fn busy(&self) -> Option<String> {
        self.ui.modal.as_ref().map(|_| {
            "the Senzor window is waiting for the user to answer a dialog; try again once it is closed"
                .to_string()
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use rmcp::model::{CallToolRequestParams, CallToolResult};
    use rmcp::service::RunningService;
    use rmcp::{RoleClient, ServiceExt};
    use serde_json::{Value, json};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;
    use crate::testing::{App, FRAME_PAUSE, FakeDialogs, TIMEOUT, wait_online, write_project};
    use crate::ui::Pending;

    /// Devices `a`, `b` and `c` (built-in `co2`, not autostarting) on the broker at `port`.
    fn three_devices(port: u16) -> String {
        let mut src = format!("[broker]\nhost = \"127.0.0.1\"\nport = {port}\n");
        for id in ["a", "b", "c"] {
            src.push_str(&format!(
                "\n[[devices]]\nid = \"{id}\"\ntemplate = \"co2\"\nautostart = false\n"
            ));
        }
        src
    }

    /// An MCP client of the window's server, on a runtime of its own.
    struct Agent {
        runtime: tokio::runtime::Runtime,
        client: RunningService<RoleClient, ()>,
    }

    impl Agent {
        fn connect(server: SenzorServer) -> Agent {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let client = runtime.block_on(async {
                let (server_end, client_end) = tokio::io::duplex(1 << 20);
                tokio::spawn(async move {
                    if let Ok(running) = server.serve(server_end).await {
                        let _ = running.waiting().await;
                    }
                });
                ().serve(client_end).await.unwrap()
            });
            Agent { runtime, client }
        }

        fn params(tool: &str, args: Value) -> CallToolRequestParams {
            let mut params = CallToolRequestParams::new(tool.to_string());
            if let Value::Object(map) = args {
                params = params.with_arguments(map);
            }
            params
        }

        /// Calls `tool` while the window keeps drawing frames.
        fn call(&self, h: &mut App, tool: &str, args: Value) -> CallToolResult {
            let peer = self.client.peer().clone();
            let params = Self::params(tool, args);
            let task = self
                .runtime
                .spawn(async move { peer.call_tool(params).await });
            let deadline = Instant::now() + TIMEOUT;
            while !task.is_finished() {
                assert!(Instant::now() < deadline, "{tool} did not return");
                h.step();
                std::thread::sleep(FRAME_PAUSE);
            }
            self.runtime.block_on(task).unwrap().unwrap()
        }

        /// Calls `tool` without the window drawing a single frame.
        fn call_without_frames(&self, tool: &str, args: Value) -> CallToolResult {
            self.runtime
                .block_on(self.client.call_tool(Self::params(tool, args)))
                .unwrap()
        }
    }

    fn text(result: &CallToolResult) -> String {
        serde_json::to_value(&result.content).unwrap().to_string()
    }

    /// The window with `src` opened and MCP on a free port.
    fn window(name: &str, src: &str) -> App {
        let path = write_project(name, src);
        let mut h = Harness::builder().build_eframe(move |cc| {
            SenzorApp::new(cc, Some(path), Box::new(FakeDialogs::default())).with_mcp(0)
        });
        h.run_steps(2);
        h
    }

    fn connect(h: &App) -> Agent {
        Agent::connect(h.state().mcp_server().unwrap())
    }

    #[test]
    fn a_device_the_agent_adds_appears_and_marks_the_project_unsaved() {
        let mut h = window("mcp-add", &three_devices(1));
        let agent = connect(&h);
        let result = agent.call(
            &mut h,
            "put_device",
            json!({"id": "lab", "template": "temperature", "autostart": false}),
        );
        assert_ne!(result.is_error, Some(true), "{}", text(&result));
        h.run_steps(2);
        assert!(h.query_by_label("lab ×1").is_some());
        assert_eq!(h.state().document().title(), "project.toml*");
    }

    #[test]
    fn a_pin_by_the_agent_shows_in_the_instance_view() {
        let port = senzor_testkit::start_broker();
        let src = format!(
            "[broker]\nhost = \"127.0.0.1\"\nport = {port}\n\n[[devices]]\nid = \"d\"\ntemplate = \"co2\"\n"
        );
        let mut h = window("mcp-pin", &src);
        wait_online(&mut h, "d");
        h.state_mut().ui.selection = Some(Selection::Instance("d".into()));
        let agent = connect(&h);
        let result = agent.call(
            &mut h,
            "pin",
            json!({"instance": "d", "field": "co2", "value": 2500}),
        );
        assert_ne!(result.is_error, Some(true), "{}", text(&result));
        let deadline = Instant::now() + TIMEOUT;
        while h.query_by_label_contains("(pinned)").is_none() {
            assert!(Instant::now() < deadline, "the pin never showed");
            h.step();
            std::thread::sleep(FRAME_PAUSE);
        }
        assert!(h.query_by_label("Unpin").is_some());
    }

    #[test]
    fn the_selection_follows_its_device_and_text_being_typed_is_dropped() {
        let mut h = window("mcp-select", &three_devices(1));
        h.state_mut().ui.selection = Some(Selection::Device(2));
        h.state_mut().ui.buffers.get("probe", "model").text = "typing".into();
        let agent = connect(&h);

        agent.call(&mut h, "remove_device", json!({"id": "a"}));
        assert_eq!(h.state().ui.selection, Some(Selection::Device(1)));
        assert_eq!(h.state().document().file().devices[1].id, "c");
        assert_eq!(h.state_mut().ui.buffers.get("probe", "model").text, "model");

        agent.call(&mut h, "rename_device", json!({"from": "c", "to": "c2"}));
        assert_eq!(h.state().ui.selection, Some(Selection::Device(1)));

        agent.call(&mut h, "remove_device", json!({"id": "c2"}));
        assert_eq!(
            h.state().ui.selection,
            None,
            "no other device takes its place"
        );
    }

    #[test]
    fn saving_is_refused_while_a_dialog_waits_for_the_user() {
        let mut h = window("mcp-busy", &three_devices(1));
        h.state_mut().ui.modal = Some(Modal::Unsaved(Pending::Close));
        let agent = connect(&h);
        let result = agent.call(&mut h, "save_project", json!({}));
        assert_eq!(result.is_error, Some(true));
        assert!(
            text(&result).contains("waiting for the user"),
            "{}",
            text(&result)
        );
        assert_eq!(h.state().ui.modal, Some(Modal::Unsaved(Pending::Close)));

        let result = agent.call(&mut h, "put_device", json!({"id": "e", "template": "co2"}));
        assert_ne!(
            result.is_error,
            Some(true),
            "edits still work: {}",
            text(&result)
        );
    }

    #[test]
    fn reads_answer_while_no_frames_are_drawn() {
        let h = window("mcp-reads", &three_devices(1));
        let agent = connect(&h);
        for tool in [
            "get_status",
            "get_project",
            "list_templates",
            "get_messages",
        ] {
            let result = agent.call_without_frames(tool, json!({}));
            assert_ne!(result.is_error, Some(true), "{tool}: {}", text(&result));
        }
    }

    #[test]
    fn a_change_the_window_never_handled_is_cancelled() {
        let mut h = window("mcp-timeout", &three_devices(1));
        let server = h
            .state()
            .mcp_server()
            .unwrap()
            .with_request_timeout(Duration::from_millis(200));
        let agent = Agent::connect(server);
        let result = agent.call_without_frames("remove_device", json!({"id": "a"}));
        assert_eq!(result.is_error, Some(true));
        assert!(
            text(&result).contains("did not respond"),
            "{}",
            text(&result)
        );
        h.run_steps(3);
        assert_eq!(
            h.state().document().file().devices.len(),
            3,
            "not applied later"
        );
    }

    /// POSTs an MCP `initialize` with `host` as the Host header; returns the HTTP status.
    fn initialize_over_http(address: SocketAddr, host: &str) -> u16 {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
            let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
            let request = format!(
                "POST /mcp HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = Vec::new();
            let _ = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut response))
                .await;
            String::from_utf8_lossy(&response)
                .split_whitespace()
                .nth(1)
                .and_then(|code| code.parse().ok())
                .unwrap_or(0)
        })
    }

    #[test]
    fn the_endpoint_listens_on_loopback_and_shows_in_the_settings_window() {
        let mut h = window("mcp-endpoint", &three_devices(1));
        let McpStatus::Listening(address) = h.state().mcp_status() else {
            panic!("not listening: {:?}", h.state().mcp_status());
        };
        assert!(address.ip().is_loopback());
        assert!(h.query_by_label("MCP").is_none(), "not in the top bar");
        crate::testing::click(&mut h, "Settings");
        assert!(h.query_by_label("MCP").is_some());
        assert!(h.query_by_label(&address.to_string()).is_some());
        assert_eq!(initialize_over_http(address, &address.to_string()), 200);
        let foreign = initialize_over_http(address, &format!("evil.example:{}", address.port()));
        assert!((400..500).contains(&foreign), "{foreign}");
    }

    /// The window with `src` opened, MCP off, and its toggle set to `port`.
    fn window_with_mcp_off(name: &str, port: u16) -> App {
        let path = write_project(name, &three_devices(1));
        let mut h = Harness::builder().build_eframe(move |cc| {
            SenzorApp::new(cc, Some(path), Box::new(FakeDialogs::default())).with_mcp_port(port)
        });
        h.run_steps(2);
        h
    }

    /// Clicks the MCP toggle in the settings window, opening the window first if needed.
    fn toggle_mcp(h: &mut App) {
        if h.query_by_label("MCP").is_none() {
            crate::testing::click(h, "Settings");
        }
        // The window re-centres a frame after it opens or its rows change; click once it is there.
        h.run();
        crate::testing::click(h, "MCP");
    }

    /// Whether anything accepts connections on `address` within a second.
    fn refused(address: SocketAddr) -> bool {
        let deadline = Instant::now() + Duration::from_secs(1);
        while std::net::TcpStream::connect(address).is_ok() {
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(FRAME_PAUSE);
        }
        true
    }

    #[test]
    fn the_toggle_turns_the_endpoint_on_and_off() {
        let mut h = window_with_mcp_off("mcp-toggle", 0);
        assert_eq!(
            h.state().mcp_status(),
            McpStatus::Off {
                port: 0,
                error: None
            }
        );
        assert!(h.state().mcp_server().is_none());

        toggle_mcp(&mut h);
        let McpStatus::Listening(address) = h.state().mcp_status() else {
            panic!("not listening: {:?}", h.state().mcp_status());
        };
        assert!(h.query_by_label(&address.to_string()).is_some());
        assert_eq!(initialize_over_http(address, &address.to_string()), 200);

        toggle_mcp(&mut h);
        assert!(matches!(
            h.state().mcp_status(),
            McpStatus::Off { error: None, .. }
        ));
        assert!(h.query_by_label(&address.to_string()).is_none());
        assert!(refused(address), "the listener is closed");
    }

    /// The top bar's sign that MCP is on.
    const MCP_ON: &str = "MCP on";

    #[test]
    fn the_top_bar_shows_while_agents_can_control_the_window_and_opens_the_settings() {
        let mut h = window_with_mcp_off("mcp-indicator", 0);
        assert!(h.query_by_label(MCP_ON).is_none());

        toggle_mcp(&mut h);
        h.state_mut().ui.settings_open = false;
        h.run();
        assert!(h.query_by_label(MCP_ON).is_some());

        crate::testing::click(&mut h, MCP_ON);
        assert!(h.state().ui.settings_open);

        toggle_mcp(&mut h);
        assert!(h.query_by_label(MCP_ON).is_none());
    }

    /// An initialize request on a kept-alive connection.
    fn initialize_request(host: &str) -> String {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
        format!(
            "POST /mcp HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    /// Everything `stream` sends within `quiet` of silence, or until it closes.
    async fn read_for(stream: &mut tokio::net::TcpStream, quiet: Duration) -> Vec<u8> {
        let mut received = Vec::new();
        let mut buffer = [0u8; 4096];
        while let Ok(Ok(n)) = tokio::time::timeout(quiet, stream.read(&mut buffer)).await {
            if n == 0 {
                break;
            }
            received.extend_from_slice(&buffer[..n]);
        }
        received
    }

    #[test]
    fn turning_mcp_off_closes_connections_that_are_already_open() {
        let mut h = window_with_mcp_off("mcp-open-connection", 0);
        toggle_mcp(&mut h);
        let McpStatus::Listening(address) = h.state().mcp_status() else {
            panic!("not listening: {:?}", h.state().mcp_status());
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let mut stream = runtime
            .block_on(tokio::net::TcpStream::connect(address))
            .unwrap();
        let request = initialize_request(&address.to_string());
        let first = runtime.block_on(async {
            stream.write_all(request.as_bytes()).await.unwrap();
            read_for(&mut stream, Duration::from_millis(500)).await
        });
        assert!(
            String::from_utf8_lossy(&first).starts_with("HTTP/1.1 200"),
            "{}",
            String::from_utf8_lossy(&first)
        );

        toggle_mcp(&mut h);
        let second = runtime.block_on(async {
            // Writing may already fail on a closed connection; either way nothing may come back.
            let _ = stream.write_all(request.as_bytes()).await;
            read_for(&mut stream, Duration::from_secs(1)).await
        });
        assert!(
            second.is_empty(),
            "the open connection still answers: {}",
            String::from_utf8_lossy(&second)
        );
    }

    #[test]
    fn a_port_in_use_turns_mcp_off_and_says_why() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = taken.local_addr().unwrap().port();
        let path = write_project("mcp-taken", &three_devices(1));
        let mut h = Harness::builder().build_eframe(move |cc| {
            SenzorApp::new(cc, Some(path), Box::new(FakeDialogs::default())).with_mcp(port)
        });
        h.run_steps(2);
        let McpStatus::Off {
            error: Some(error), ..
        } = h.state().mcp_status()
        else {
            panic!("not off with an error: {:?}", h.state().mcp_status());
        };
        assert!(error.contains(&port.to_string()), "{error}");
        assert!(matches!(h.state().ui.modal, Some(Modal::Error(_))));
        assert_eq!(
            h.state().document().file().devices.len(),
            3,
            "the project opened"
        );
    }

    #[test]
    fn turning_mcp_on_with_the_port_in_use_leaves_it_off() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = taken.local_addr().unwrap().port();
        let mut h = window_with_mcp_off("mcp-toggle-taken", port);
        toggle_mcp(&mut h);
        assert!(matches!(
            h.state().mcp_status(),
            McpStatus::Off { error: Some(_), .. }
        ));
        assert!(matches!(h.state().ui.modal, Some(Modal::Error(_))));
    }
}
