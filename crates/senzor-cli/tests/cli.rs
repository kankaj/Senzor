use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::Duration;

use assert_cmd::Command;
use senzor_testkit::{Subscriber, free_port, start_broker};

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

/// Writes `content` to a fresh file under the system temp dir and returns its path.
fn write_project(name: &str, content: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("senzor-cli-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("project.toml");
    std::fs::write(&path, content).unwrap();
    path
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
fn validate_accepts_a_valid_project() {
    let path = write_project("valid", &project_toml(1883, true));
    let output = run_cli("validate", &path, &[]);
    assert_exit(&output, 0);
    assert!(String::from_utf8_lossy(&output.stdout).contains("OK (2 devices, 3 instances)"));
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

    let output = run_cli_async("run", path, &["--duration", "1s"]).await;
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

    let output = run_cli_async("run", path, &["--only", "other, dev", "--duration", "1s"]).await;
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
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn sigterm_shuts_down_gracefully_and_exits_0() {
    let port = start_broker();
    let mut sub = Subscriber::connect(port, "cli/+/status").await;
    let path = write_project("sigterm", &project_toml(port, true));
    let mut child = std::process::Command::new(assert_cmd::cargo::cargo_bin(BIN))
        .arg("run")
        .arg(&path)
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    sub.wait_for(Duration::from_secs(5), |m| m.payload == "online")
        .await
        .expect("instances came online");

    let sent = std::process::Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(sent.success());
    let pid = child.id();
    let waited = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::task::spawn_blocking(move || child.wait().unwrap()),
    )
    .await;
    let Ok(status) = waited else {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status();
        panic!("senzor-cli did not exit within 5 s of SIGTERM");
    };
    let status = status.unwrap();
    assert_eq!(status.code(), Some(0), "exit status: {status:?}");
}
