mod support;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use assert_cmd::Command;
use rumqttc::ConnectReturnCode;
use senzor_testkit::{Received, Subscriber, free_port, refusing_broker, start_broker};
use tokio::io::{AsyncBufReadExt, BufReader};

use support::BIN;

fn project_toml(port: u16, autostart: bool) -> String {
    format!(
        r#"
[broker]
host = "127.0.0.1"
port = {port}

[[templates]]
id = "cli"
interval = "200ms"
availability = {{ topic = "cli/{{{{ device_id }}}}/status", online = "online", offline = "offline" }}
messages = [{{ topic = "cli/{{{{ device_id }}}}/state", payload = "{{{{ v }}}}" }}]
fields.v = {{ generator = "constant", value = 1 }}

[[devices]]
id = "dev"
template = "cli"
count = 2
autostart = {autostart}

[[devices]]
id = "other"
template = "cli"
autostart = false
"#
    )
}

/// Writes `content` to a project file in a fresh temp folder, removed when the result is dropped.
fn write_project(name: &str, content: &str) -> support::TempProject {
    support::TempProject::new(BIN, name, content)
}

#[test]
fn a_test_project_folder_is_removed_when_the_test_is_done() {
    let project = write_project("cleanup", "");
    let dir = project.dir().to_path_buf();
    assert!(project.exists());
    drop(project);
    assert!(!dir.exists(), "{} was left behind", dir.display());
}

/// Runs `senzor-cli <command> <project> <args>` to completion.
fn run_cli(command: &str, project: &Path, args: &[&str]) -> Output {
    Command::cargo_bin(BIN)
        .unwrap()
        .arg(command)
        .arg(project)
        .args(args)
        .output()
        .unwrap()
}

/// [`run_cli`] on the blocking pool, so the test's subscriber keeps receiving meanwhile.
async fn run_cli_async(command: &'static str, project: PathBuf, args: &'static [&str]) -> Output {
    tokio::task::spawn_blocking(move || run_cli(command, &project, args))
        .await
        .unwrap()
}

fn stderr(output: &Output) -> Cow<'_, str> {
    String::from_utf8_lossy(&output.stderr)
}

/// Asserts the exit code; a mismatch shows what the CLI printed to stderr.
fn assert_exit(output: &Output, code: i32) {
    assert_eq!(output.status.code(), Some(code), "{}", stderr(output));
}

#[test]
fn version_flag_prints_the_package_version() {
    let output = Command::cargo_bin(BIN)
        .unwrap()
        .arg("--version")
        .output()
        .unwrap();
    assert_exit(&output, 0);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("senzor-cli {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn validate_accepts_a_valid_project() {
    let path = write_project("valid", &project_toml(1883, true));
    let output = run_cli("validate", &path, &[]);
    assert_exit(&output, 0);
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("OK (2 devices, 3 instances, MQTT 5)")
    );
}

#[test]
fn validate_names_mqtt_3_1_1_when_the_project_uses_it() {
    let content =
        project_toml(1883, true).replace("[broker]\n", "[broker]\nmqtt_version = \"3.1.1\"\n");
    let path = write_project("v311", &content);
    let output = run_cli("validate", &path, &[]);
    assert_exit(&output, 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("OK (2 devices, 3 instances, MQTT 3.1.1)"),
        "{stdout}"
    );
}

#[test]
fn validate_uses_singular_nouns_for_one_device_and_one_instance() {
    let content = r#"
[broker]
host = "127.0.0.1"
port = 1883

[[templates]]
id = "cli"
interval = "200ms"
messages = [{ topic = "cli/{{ device_id }}/state", payload = "{{ v }}" }]
fields.v = { generator = "constant", value = 1 }

[[devices]]
id = "dev"
template = "cli"
"#;
    let path = write_project("singular", content);
    let output = run_cli("validate", &path, &[]);
    assert_exit(&output, 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("OK (1 device, 1 instance, MQTT 5)"),
        "{stdout}"
    );
}

#[test]
fn validate_rejects_an_invalid_project_with_every_problem() {
    let content = project_toml(0, true).replace(
        "template = \"cli\"\ncount = 2",
        "template = \"nope\"\ncount = 2",
    );
    let path = write_project("invalid", &content);
    let output = run_cli("validate", &path, &[]);
    assert_exit(&output, 2);
    let stderr = stderr(&output);
    assert!(stderr.contains("broker: port must not be 0"), "{stderr}");
    assert!(
        stderr.contains("device `dev`: unknown template `nope`"),
        "{stderr}"
    );
}

#[test]
fn validate_reports_toml_errors_with_position() {
    let path = write_project("syntax", "[broker]\nhots = 1\n");
    let output = run_cli("validate", &path, &[]);
    assert_exit(&output, 2);
    assert!(stderr(&output).contains("line 2, column 1"));
}

#[test]
fn missing_file_is_an_invalid_argument() {
    let output = run_cli("validate", Path::new("/nonexistent/project.toml"), &[]);
    assert_exit(&output, 2);
}

#[test]
fn run_with_unknown_only_id_exits_2() {
    let path = write_project("unknown-only", &project_toml(1883, true));
    let output = run_cli("run", &path, &["--only", "dev,ghost"]);
    assert_exit(&output, 2);
    assert!(stderr(&output).contains("unknown device id(s): ghost"));
}

#[test]
fn run_with_nothing_to_start_exits_2() {
    let path = write_project("nothing", &project_toml(1883, false));
    let output = run_cli("run", &path, &[]);
    assert_exit(&output, 2);
    assert!(stderr(&output).contains("no devices to start"));
}

#[test]
fn run_with_only_naming_no_devices_exits_2() {
    let path = write_project("only-blank", &project_toml(1883, true));
    for only in [",", " ", " , "] {
        let output = run_cli("run", &path, &["--only", only]);
        assert_exit(&output, 2);
        let stderr = stderr(&output);
        assert!(
            stderr.contains("error: --only names no devices"),
            "--only {only:?}: {stderr}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn run_publishes_until_duration_elapses_then_goes_offline() {
    let port = start_broker();
    let mut sub = Subscriber::connect(port, "cli/#").await;
    let path = write_project("run", &project_toml(port, true));

    let output = run_cli_async("run", path.to_path_buf(), &["--duration", "1s"]).await;
    assert_exit(&output, 0);

    let messages = sub.collect(Duration::from_millis(300)).await;
    let has = |topic: &str, payload: &str| {
        messages
            .iter()
            .any(|m| m.topic == topic && m.payload == payload)
    };
    assert!(has("cli/dev-0/state", "1"));
    assert!(has("cli/dev-1/state", "1"));
    assert!(has("cli/dev-0/status", "offline"));
    assert!(
        !messages.iter().any(|m| m.topic.starts_with("cli/other")),
        "other has autostart = false"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn run_only_accepts_ids_with_spaces() {
    let port = start_broker();
    let mut sub = Subscriber::connect(port, "cli/#").await;
    let path = write_project("only-spaces", &project_toml(port, false));

    let output = run_cli_async(
        "run",
        path.to_path_buf(),
        &["--only", "other, dev", "--duration", "1s"],
    )
    .await;
    assert_exit(&output, 0);
    let messages = sub.collect(Duration::from_millis(300)).await;
    assert!(messages.iter().any(|m| m.topic == "cli/other/state"));
    assert!(messages.iter().any(|m| m.topic == "cli/dev-0/state"));
}

#[test]
fn run_against_an_unreachable_broker_keeps_going_and_exits_0() {
    let path = write_project("unreachable", &project_toml(free_port(), true));
    let output = run_cli("run", &path, &["--duration", "1s"]);
    assert_exit(&output, 0);
    assert!(stderr(&output).contains("reconnecting"));
}

#[tokio::test(flavor = "multi_thread")]
async fn run_exits_3_and_names_the_instances_the_broker_refused() {
    let port = refusing_broker(ConnectReturnCode::BadUserNamePassword).await;
    let path = write_project("refused", &project_toml(port, true));
    let output = run_cli_async("run", path.to_path_buf(), &["--duration", "1s"]).await;
    assert_exit(&output, 3);
    let stderr = stderr(&output);
    assert!(
        stderr.contains("failed: broker refused the connection: BadUserNamePassword"),
        "{stderr}"
    );
    assert!(
        stderr.ends_with("error: 2 instances failed: dev-0, dev-1\n"),
        "{stderr}"
    );
}

/// [`project_toml`] with `retain = true`, so the state topics are retained too.
fn retained_project_toml(port: u16) -> String {
    project_toml(port, true).replace(
        "interval = \"200ms\"\n",
        "interval = \"200ms\"\nretain = true\n",
    )
}

/// The topics a new subscriber to `filter` gets a retained message on, sorted.
async fn retained_topics_on(port: u16, filter: &str) -> Vec<String> {
    let mut subscriber = Subscriber::connect(port, filter).await;
    let mut topics: Vec<String> = subscriber
        .collect(Duration::from_millis(500))
        .await
        .into_iter()
        .filter(|m| m.retain)
        .map(|m| m.topic)
        .collect();
    topics.sort();
    topics
}

#[tokio::test(flavor = "multi_thread")]
async fn clear_retained_deletes_the_project_s_retained_messages_and_lists_them() {
    let port = start_broker();
    let path = write_project("clear", &retained_project_toml(port));
    let output = run_cli_async(
        "run",
        path.to_path_buf(),
        &["--only", "dev,other", "--duration", "1s"],
    )
    .await;
    assert_exit(&output, 0);
    assert_eq!(retained_topics_on(port, "cli/#").await.len(), 6);

    let output = run_cli_async("clear-retained", path.to_path_buf(), &[]).await;
    assert_exit(&output, 0);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "cli/dev-0/state\ncli/dev-0/status\ncli/dev-1/state\ncli/dev-1/status\n\
             cli/other/state\ncli/other/status\n\
             cleared 6 retained topics on 127.0.0.1:{port}\n"
        )
    );
    assert_eq!(
        retained_topics_on(port, "cli/#").await,
        Vec::<String>::new()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn clear_retained_only_clears_the_devices_named_in_only() {
    let port = start_broker();
    let path = write_project("clear-only", &retained_project_toml(port));
    let output = run_cli_async(
        "run",
        path.to_path_buf(),
        &["--only", "dev,other", "--duration", "1s"],
    )
    .await;
    assert_exit(&output, 0);

    let output = run_cli_async("clear-retained", path.to_path_buf(), &["--only", " other"]).await;
    assert_exit(&output, 0);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "cli/other/state\ncli/other/status\ncleared 2 retained topics on 127.0.0.1:{port}\n"
        )
    );
    assert_eq!(
        retained_topics_on(port, "cli/#").await,
        [
            "cli/dev-0/state",
            "cli/dev-0/status",
            "cli/dev-1/state",
            "cli/dev-1/status"
        ]
    );
}

#[test]
fn clear_retained_with_nothing_retained_does_not_connect() {
    let content = project_toml(free_port(), true).replace("availability = ", "# availability = ");
    let path = write_project("clear-nothing", &content);
    let output = run_cli("clear-retained", &path, &[]);
    assert_exit(&output, 0);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "no retained topics to clear\n"
    );
}

#[test]
fn clear_retained_with_unknown_only_id_exits_2() {
    let path = write_project("clear-unknown", &project_toml(1883, true));
    let output = run_cli("clear-retained", &path, &["--only", "ghost"]);
    assert_exit(&output, 2);
    assert!(stderr(&output).contains("error: unknown device id(s): ghost"));
}

#[test]
fn clear_retained_refuses_a_project_with_problems() {
    let content = project_toml(0, true);
    let path = write_project("clear-invalid", &content);
    let output = run_cli("clear-retained", &path, &[]);
    assert_exit(&output, 2);
    assert!(stderr(&output).contains("broker: port must not be 0"));
}

#[tokio::test(flavor = "multi_thread")]
async fn clear_retained_exits_1_with_the_reason_when_the_broker_refuses() {
    let port = refusing_broker(ConnectReturnCode::NotAuthorized).await;
    let path = write_project("clear-refused", &project_toml(port, true));
    let output = run_cli_async("clear-retained", path.to_path_buf(), &[]).await;
    assert_exit(&output, 1);
    assert_eq!(
        stderr(&output),
        "error: cannot clear retained topics: broker refused the connection: NotAuthorized\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
}

#[test]
fn clear_retained_exits_1_when_nothing_listens() {
    let path = write_project("clear-unreachable", &project_toml(free_port(), true));
    let output = run_cli("clear-retained", &path, &[]);
    assert_exit(&output, 1);
    assert!(
        stderr(&output).starts_with("error: cannot clear retained topics: "),
        "{}",
        stderr(&output)
    );
}

#[test]
fn the_log_has_no_colour_codes_when_stderr_is_not_a_terminal() {
    let path = write_project("no-colour", &project_toml(free_port(), true));
    let output = Command::cargo_bin(BIN)
        .unwrap()
        .arg("run")
        .arg(&*path)
        .args(["--duration", "1s"])
        .env_remove("NO_COLOR")
        .output()
        .unwrap();
    assert_exit(&output, 0);
    let stderr = stderr(&output);
    assert!(stderr.contains("connecting with MQTT 5"), "{stderr}");
    assert!(!stderr.contains("\x1b["), "{stderr:?}");
}

#[test]
fn verbose_logs_every_publish() {
    let port = start_broker();
    let path = write_project("verbose", &project_toml(port, true));
    let output = run_cli("run", &path, &["--duration", "1s", "-v"]);
    assert_exit(&output, 0);
    let stderr = stderr(&output);
    assert!(
        stderr.contains("publish") && stderr.contains("cli/dev-0/state"),
        "{stderr}"
    );
    assert_eq!(
        stderr.matches("connecting with MQTT 5").count(),
        1,
        "{stderr}"
    );
}

#[test]
fn verbose_logs_the_mqtt5_properties_of_a_publish() {
    let port = start_broker();
    let content = project_toml(port, true).replace(
        r#"payload = "{{ v }}" }]"#,
        r#"payload = "{{ v }}", user_properties = { room = "lab" } }]"#,
    );
    assert!(content.contains("user_properties"));
    let path = write_project("verbose-properties", &content);
    let output = run_cli("run", &path, &["--duration", "1s", "-v"]);
    assert_exit(&output, 0);
    let stderr = stderr(&output);
    assert!(
        stderr.contains("properties") && stderr.contains(r#"("room", "lab")"#),
        "{stderr}"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn sigterm_shuts_down_gracefully_and_exits_0() {
    let port = start_broker();
    let mut sub = Subscriber::connect(port, "cli/+/status").await;
    let path = write_project("sigterm", &project_toml(port, true));
    let mut child = tokio::process::Command::new(assert_cmd::cargo::cargo_bin(BIN))
        .arg("run")
        .arg(&*path)
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    sub.wait_for(Duration::from_secs(5), |m| m.payload == "online")
        .await
        .expect("instances came online");

    let pid = child.id().expect("senzor-cli is running");
    let sent = tokio::process::Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .await
        .unwrap();
    assert!(sent.success());
    let Ok(status) = tokio::time::timeout(Duration::from_secs(5), child.wait()).await else {
        let _ = child.kill().await;
        panic!("senzor-cli did not exit within 5 s of SIGTERM");
    };
    let status = status.unwrap();
    assert_eq!(status.code(), Some(0), "exit status: {status:?}");
}

#[test]
fn schema_prints_the_project_files_json_schema() {
    let output = Command::cargo_bin(BIN)
        .unwrap()
        .arg("schema")
        .output()
        .unwrap();
    assert!(output.status.success());
    let schema: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        schema["$id"],
        "https://senzor.kankaj.cz/schema/project.json"
    );
    assert!(schema["properties"]["devices"].is_object());
}

/// The docs site publishes the schema editors download; it must match the code.
#[test]
fn the_published_schema_is_up_to_date() {
    let output = Command::cargo_bin(BIN)
        .unwrap()
        .arg("schema")
        .output()
        .unwrap();
    let published = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../website/public/schema/project.json");
    let published = std::fs::read_to_string(&published).unwrap_or_default();
    assert!(
        published == String::from_utf8(output.stdout).unwrap(),
        "website/public/schema/project.json is out of date; regenerate it with\n  \
         cargo run -q -p senzor-cli -- schema > website/public/schema/project.json"
    );
}

/// A project for the `--watch` tests: one device per `(id, value)`, each publishing `value` on
/// `w/<id>/state`, with `online`/`offline` on `w/<id>/status`. Template `w-up` is the same, but
/// publishes `up` for online.
fn watch_toml(port: u16, devices: &[(&str, i32)]) -> String {
    let mut toml = format!(
        r#"
[broker]
host = "127.0.0.1"
port = {port}

[[templates]]
id = "w"
interval = "200ms"
availability = {{ topic = "w/{{{{ device_id }}}}/status", online = "online", offline = "offline" }}
messages = [{{ topic = "w/{{{{ device_id }}}}/state", payload = "{{{{ v }}}}" }}]
fields.v = {{ generator = "constant", value = 0 }}

[[templates]]
id = "w-up"
interval = "200ms"
availability = {{ topic = "w/{{{{ device_id }}}}/status", online = "up", offline = "offline" }}
messages = [{{ topic = "w/{{{{ device_id }}}}/state", payload = "{{{{ v }}}}" }}]
fields.v = {{ generator = "constant", value = 0 }}
"#
    );
    for (id, value) in devices {
        toml.push_str(&format!(
            "\n[[devices]]\nid = \"{id}\"\ntemplate = \"w\"\nfields.v = {{ value = {value} }}\n"
        ));
    }
    toml
}

/// How long a `--watch` test waits for something it expects.
const WATCH_WAIT: Duration = Duration::from_secs(10);

/// `senzor-cli run <project> --watch <args>` in the background. Its stderr is collected as it
/// comes; dropping the value kills the process.
struct Watching {
    child: tokio::process::Child,
    stderr: Arc<Mutex<String>>,
}

impl Watching {
    fn start(project: &Path, args: &[&str]) -> Self {
        let mut child = tokio::process::Command::new(assert_cmd::cargo::cargo_bin(BIN))
            .arg("run")
            .arg(project)
            .arg("--watch")
            .args(args)
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stderr = Arc::new(Mutex::new(String::new()));
        let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
        let collected = stderr.clone();
        tokio::spawn(async move {
            while let Ok(Some(line)) = lines.next_line().await {
                let mut text = collected.lock().unwrap();
                text.push_str(&line);
                text.push('\n');
            }
        });
        Self { child, stderr }
    }

    fn stderr(&self) -> String {
        self.stderr.lock().unwrap().clone()
    }

    /// Waits until stderr contains `text`, then returns all of it.
    async fn wait_for_stderr(&self, text: &str) -> String {
        let deadline = tokio::time::Instant::now() + WATCH_WAIT;
        loop {
            let stderr = self.stderr();
            if stderr.contains(text) {
                return stderr;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "stderr never had {text:?}:\n{stderr}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// Waits for the process to end by itself; returns its exit code and stderr.
    async fn exit(mut self) -> (Option<i32>, String) {
        let status = tokio::time::timeout(WATCH_WAIT, self.child.wait())
            .await
            .expect("senzor-cli exited in time")
            .unwrap();
        // The reader task gets the last lines once the pipe closes.
        tokio::time::sleep(Duration::from_millis(100)).await;
        (status.code(), self.stderr())
    }
}

/// Adds messages to `seen` until `done(seen)` holds; fails after [`WATCH_WAIT`].
async fn record_until(
    sub: &mut Subscriber,
    seen: &mut Vec<Received>,
    what: &str,
    done: impl Fn(&[Received]) -> bool,
) {
    let deadline = tokio::time::Instant::now() + WATCH_WAIT;
    while !done(seen) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}: {seen:#?}"
        );
        seen.extend(sub.collect(Duration::from_millis(50)).await);
    }
}

/// How many messages in `seen` have `payload` on `topic`.
fn count(seen: &[Received], topic: &str, payload: &str) -> usize {
    seen.iter()
        .filter(|m| m.topic == topic && m.payload == payload)
        .count()
}

fn has(seen: &[Received], topic: &str, payload: &str) -> bool {
    count(seen, topic, payload) > 0
}

/// Writes `content` over the project file, as an editor's save does.
fn save(path: &Path, content: &str) {
    std::fs::write(path, content).unwrap();
}

/// Ends with `--duration`, so it also shows that `--watch` keeps to it.
#[tokio::test(flavor = "multi_thread")]
async fn watch_updates_and_restarts_changed_devices_starts_added_ones_and_stops_removed_ones() {
    let port = start_broker();
    let mut sub = Subscriber::connect(port, "w/#").await;
    let devices = [("keep", 1), ("change", 1), ("moved", 1), ("gone", 1)];
    let path = write_project("watch", &watch_toml(port, &devices));
    let run = Watching::start(&path, &["--duration", "6s"]);
    let mut seen = Vec::new();
    record_until(&mut sub, &mut seen, "all online", |s| {
        devices
            .iter()
            .all(|(id, _)| has(s, &format!("w/{id}/status"), "online"))
    })
    .await;

    // `change` gets a new value, which it takes while connected; `moved` a template with
    // another online payload, which needs a new connection.
    save(
        &path,
        &watch_toml(
            port,
            &[("keep", 1), ("change", 2), ("moved", 1), ("lab", 1)],
        )
        .replace(
            "id = \"moved\"\ntemplate = \"w\"",
            "id = \"moved\"\ntemplate = \"w-up\"",
        ),
    );
    let mut after = Vec::new();
    record_until(&mut sub, &mut after, "the reload", |s| {
        has(s, "w/change/state", "2")
            && has(s, "w/moved/status", "offline")
            && has(s, "w/moved/status", "up")
            && has(s, "w/lab/status", "online")
            && has(s, "w/gone/status", "offline")
    })
    .await;
    let stderr = run.wait_for_stderr("reloaded").await;
    assert!(
        stderr.contains(&format!(
            "reloaded {}: restarted moved; updated change; started lab; stopped gone",
            path.display()
        )),
        "{stderr}"
    );
    after.extend(sub.collect(Duration::from_millis(500)).await);
    for id in ["keep", "change"] {
        assert!(
            !after.iter().any(|m| m.topic == format!("w/{id}/status")),
            "{id} stays connected: {after:#?}"
        );
    }
    let gone_offline = after
        .iter()
        .position(|m| m.topic == "w/gone/status")
        .unwrap();
    assert!(
        !after[gone_offline..]
            .iter()
            .any(|m| m.topic == "w/gone/state"),
        "gone stopped publishing: {after:#?}"
    );

    let (code, stderr) = run.exit().await;
    assert_eq!(code, Some(0), "{stderr}");
    let rest = sub.collect(Duration::from_millis(300)).await;
    assert_eq!(count(&rest, "w/keep/status", "offline"), 1, "{rest:#?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_keeps_the_last_valid_project_while_a_save_has_problems() {
    let port = start_broker();
    let mut sub = Subscriber::connect(port, "w/#").await;
    let path = write_project("watch-invalid", &watch_toml(port, &[("dev", 1)]));
    let run = Watching::start(&path, &[]);
    let mut seen = Vec::new();
    record_until(&mut sub, &mut seen, "dev online", |s| {
        has(s, "w/dev/status", "online")
    })
    .await;

    let broken = watch_toml(port, &[("dev", 1), ("bad", 1)]).replace(
        "id = \"bad\"\ntemplate = \"w\"",
        "id = \"bad\"\ntemplate = \"nope\"",
    );
    save(&path, &broken);
    let problem = format!("{}: device `bad`: unknown template `nope`", path.display());
    run.wait_for_stderr(&problem).await;
    let mut after = Vec::new();
    record_until(&mut sub, &mut after, "dev publishing on", |s| {
        count(s, "w/dev/state", "1") >= 8
    })
    .await;
    assert!(
        !after.iter().any(|m| m.topic.ends_with("/status")),
        "nothing restarted: {after:#?}"
    );
    assert_eq!(
        run.stderr().matches(&problem).count(),
        1,
        "{}",
        run.stderr()
    );

    save(&path, &watch_toml(port, &[("dev", 1), ("lab", 1)]));
    let stderr = run.wait_for_stderr("reloaded").await;
    assert!(
        stderr.contains(&format!("reloaded {}: started lab", path.display())),
        "{stderr}"
    );
    record_until(&mut sub, &mut after, "lab online", |s| {
        has(s, "w/lab/status", "online")
    })
    .await;
    assert!(!has(&after, "w/dev/status", "offline"), "{after:#?}");
    assert_eq!(
        run.stderr().matches(&problem).count(),
        1,
        "{}",
        run.stderr()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_restarts_every_device_when_the_broker_changes() {
    let port = start_broker();
    let mut sub = Subscriber::connect(port, "w/#").await;
    let path = write_project("watch-broker", &watch_toml(port, &[("a", 1), ("b", 1)]));
    let run = Watching::start(&path, &[]);
    let mut seen = Vec::new();
    record_until(&mut sub, &mut seen, "a and b online", |s| {
        has(s, "w/a/status", "online") && has(s, "w/b/status", "online")
    })
    .await;

    save(
        &path,
        &watch_toml(port, &[("a", 1), ("b", 1)])
            .replace("[broker]\n", "[broker]\nkeep_alive = \"20s\"\n"),
    );
    let stderr = run.wait_for_stderr("reloaded").await;
    assert!(
        stderr.contains(&format!(
            "reloaded {}: broker changed; restarted a, b",
            path.display()
        )),
        "{stderr}"
    );
    let mut after = Vec::new();
    record_until(&mut sub, &mut after, "a and b back online", |s| {
        ["a", "b"].iter().all(|id| {
            let topic = format!("w/{id}/status");
            has(s, &topic, "offline") && has(s, &topic, "online")
        })
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn watch_waits_while_the_file_is_missing_or_empty() {
    let port = start_broker();
    let mut sub = Subscriber::connect(port, "w/#").await;
    let path = write_project("watch-missing", &watch_toml(port, &[("dev", 1)]));
    let run = Watching::start(&path, &[]);
    let mut seen = Vec::new();
    record_until(&mut sub, &mut seen, "dev online", |s| {
        has(s, "w/dev/status", "online")
    })
    .await;

    std::fs::remove_file(&*path).unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    save(&path, "");
    tokio::time::sleep(Duration::from_millis(1500)).await;
    save(&path, &watch_toml(port, &[("dev", 1), ("lab", 1)]));
    let stderr = run.wait_for_stderr("reloaded").await;
    assert!(
        stderr.contains(&format!("reloaded {}: started lab", path.display())),
        "{stderr}"
    );
    assert!(!stderr.contains("cannot read"), "{stderr}");
    assert!(!stderr.contains("stopped"), "{stderr}");
}

#[tokio::test(flavor = "multi_thread")]
async fn run_without_watch_ignores_edits() {
    let port = start_broker();
    let mut sub = Subscriber::connect(port, "w/#").await;
    let path = write_project("no-watch", &watch_toml(port, &[("dev", 1)]));
    let edit_path = path.to_path_buf();
    let content = watch_toml(port, &[("dev", 2), ("lab", 1)]);
    let edit = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        save(&edit_path, &content);
    });
    let output = run_cli_async("run", path.to_path_buf(), &["--duration", "3s"]).await;
    edit.await.unwrap();
    assert_exit(&output, 0);
    assert!(!stderr(&output).contains("reloaded"), "{}", stderr(&output));
    let messages = sub.collect(Duration::from_millis(300)).await;
    assert!(!messages.iter().any(|m| m.topic.starts_with("w/lab")));
    assert!(!has(&messages, "w/dev/state", "2"));
    assert_eq!(count(&messages, "w/dev/status", "online"), 1);
}

/// A port nothing listens on right now.
fn free_tcp_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// The status code of an MCP initialize sent to `127.0.0.1:<port>` with `headers` (each ending
/// in CRLF), retried until something listens or 10 s pass; 0 when nothing answered.
fn mcp_initialize_status(port: u16, headers: &str) -> u16 {
    use std::io::{Read, Write};
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        let Ok(mut stream) = std::net::TcpStream::connect(("127.0.0.1", port)) else {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };
        write!(
            stream,
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{headers}Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        let _ = stream.read_to_string(&mut response);
        return response
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
    }
    0
}

#[test]
fn run_serves_mcp_over_http_while_the_devices_run() {
    let broker = start_broker();
    let project = write_project("run-mcp", &project_toml(broker, true));
    let port = free_tcp_port();
    let child = std::process::Command::new(assert_cmd::cargo::cargo_bin(BIN))
        .args(["run", &project.to_string_lossy(), "--duration", "4s"])
        .args(["--mcp-port", &port.to_string()])
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    assert_eq!(mcp_initialize_status(port, ""), 200);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let log = String::from_utf8_lossy(&output.stderr);
    assert!(
        log.contains(&format!("MCP at http://127.0.0.1:{port}/mcp")),
        "{log}"
    );
}

#[test]
fn run_lets_other_machines_use_mcp_only_with_a_token() {
    let broker = start_broker();
    let project = write_project("run-mcp-remote", &project_toml(broker, false));
    let port = free_tcp_port();
    let output = run_cli(
        "run",
        &project,
        &["--mcp-port", &port.to_string(), "--mcp-bind", "0.0.0.0"],
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("SENZOR_MCP_TOKEN"),
        "{output:?}"
    );

    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin(BIN))
        .args(["run", &project.to_string_lossy(), "--duration", "4s"])
        .args(["--mcp-port", &port.to_string(), "--mcp-bind", "0.0.0.0"])
        .env("SENZOR_MCP_TOKEN", "s3cret")
        .spawn()
        .unwrap();
    assert_eq!(mcp_initialize_status(port, ""), 401);
    assert_eq!(
        mcp_initialize_status(port, "Authorization: Bearer s3cret\r\n"),
        200
    );
    assert!(child.wait().unwrap().success());
}

#[test]
fn run_takes_either_watch_or_an_mcp_port() {
    let project = write_project("run-mcp-watch", &project_toml(1883, false));
    let output = run_cli("run", &project, &["--watch", "--mcp-port", "18999"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cannot be used with"),
        "{output:?}"
    );
}
