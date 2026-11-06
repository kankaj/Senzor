mod support;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use assert_cmd::Command;
use rumqttc::ConnectReturnCode;
use senzor_testkit::{Subscriber, free_port, refusing_broker, start_broker};

const BIN: &str = "senzor-cli";

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
    support::TempProject::new("senzor-cli", name, content)
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
