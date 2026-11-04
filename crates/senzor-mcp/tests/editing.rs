//! Project editing and files over MCP.

mod support;

use std::path::Path;
use std::time::{Duration, Instant};

use senzor_core::project::ProjectFile;
use senzor_testkit::Subscriber;
use serde_json::{Value, json};
use support::{Agent, project, state, temp_dir};

fn names(list: &Value) -> Vec<&str> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn get_project_returns_the_file_as_saved() {
    let agent = Agent::with_project("get-project").await;
    let out = agent.ok("get_project", json!({})).await;
    let path = agent.dir.join("project.toml");
    let on_disk = ProjectFile::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        ProjectFile::parse(out["toml"].as_str().unwrap()).unwrap(),
        on_disk
    );
    assert_eq!(out["dirty"], false);
    assert_eq!(out["path"], path.display().to_string());
    assert_eq!(out["problems"], json!([]));
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn set_project_checks_before_it_replaces() {
    let agent = Agent::with_project("set-project").await;
    let before = agent.ok("get_project", json!({})).await["toml"].clone();

    let message = agent
        .err("set_project", json!({"toml": "[broker]\nhots = 1\n"}))
        .await;
    assert!(message.contains("line 2"), "{message}");
    assert_eq!(agent.ok("get_project", json!({})).await["toml"], before);

    let broken = format!(
        "{}\n[[devices]]\nid = \"x\"\ntemplate = \"missing\"\n",
        project(agent.port)
    );
    let out = agent
        .ok("set_project", json!({"toml": broken, "dry_run": true}))
        .await;
    assert_eq!(out["changed"], false);
    assert!(out["problems"].to_string().contains("missing"), "{out}");
    assert_eq!(names(&out["blocked"]), ["x"]);
    let after = agent.ok("get_project", json!({})).await;
    assert_eq!(after["toml"], before);
    assert_eq!(after["dirty"], false);

    let out = agent.ok("set_project", json!({"toml": broken})).await;
    assert_eq!(out["changed"], true);
    assert_eq!(names(&out["blocked"]), ["x"]);
    assert_eq!(agent.ok("get_status", json!({})).await["dirty"], true);
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn devices_are_added_replaced_renamed_and_removed() {
    let agent = Agent::with_project("devices").await;
    let out = agent
        .ok(
            "put_device",
            json!({"id": "probe", "template": "temperature", "count": 3, "autostart": false}),
        )
        .await;
    assert_eq!(out["changed"], true);
    for id in ["probe-0", "probe-1", "probe-2"] {
        assert_eq!(state(&agent.status_of(id).await), "stopped");
    }

    let out = agent
        .ok(
            "put_device",
            json!({"id": "probe", "template": "temperature"}),
        )
        .await;
    assert_eq!(out["changed"], true);
    let project = agent.ok("get_project", json!({})).await;
    let file = ProjectFile::parse(project["toml"].as_str().unwrap()).unwrap();
    assert_eq!(file.devices.iter().filter(|d| d.id == "probe").count(), 1);
    assert_eq!(file.devices.last().unwrap().count, 1, "replaced in place");

    let message = agent
        .err(
            "put_device",
            json!({"id": "lab", "template": "co2", "cnt": 3}),
        )
        .await;
    assert!(
        message.contains("failed to deserialize parameters"),
        "{message}"
    );
    assert!(message.contains("cnt"), "{message}");

    let message = agent
        .err("rename_device", json!({"from": "lab", "to": "office"}))
        .await;
    assert!(message.contains("already exists"), "{message}");
    agent
        .ok("rename_device", json!({"from": "lab", "to": "bench"}))
        .await;
    let message = agent.err("remove_device", json!({"id": "lab"})).await;
    assert!(message.contains("no device `lab`"), "{message}");
    agent.ok("remove_device", json!({"id": "bench"})).await;
    let status = agent.ok("get_status", json!({})).await;
    assert!(!status.to_string().contains("bench"));
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn edits_flag_running_instances_instead_of_restarting_them() {
    let agent = Agent::with_project("needs-restart").await;
    agent.ok("start", json!({"instances": ["lab"]})).await;
    let out = agent
        .ok(
            "put_device",
            json!({"id": "lab", "template": "air", "autostart": false, "interval": "200ms"}),
        )
        .await;
    assert_eq!(names(&out["needs_restart"]), ["lab"]);
    assert_eq!(state(&agent.status_of("lab").await), "online");

    let out = agent.ok("remove_device", json!({"id": "lab"})).await;
    assert_eq!(names(&out["orphaned"]), ["lab"]);
    agent.ok("stop", json!({"instances": ["lab"]})).await;
    let status = agent.ok("get_status", json!({})).await;
    assert!(!status.to_string().contains("\"lab\""), "the orphan left");
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn templates_follow_the_rules_for_built_ins() {
    let agent = Agent::with_project("templates").await;
    let message = agent
        .err(
            "put_template",
            json!({"id": "co2", "messages": [], "fields": {}}),
        )
        .await;
    assert!(message.contains("built-in"), "{message}");

    agent
        .ok("rename_template", json!({"from": "air", "to": "air-v2"}))
        .await;
    let templates = agent.ok("list_templates", json!({})).await;
    let air = templates["templates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "air-v2")
        .unwrap();
    assert_eq!(names(&air["used_by"]), ["office", "lab"]);
    assert_eq!(air["fields"]["v"], "constant");
    assert_eq!(air["interval"], "100ms");

    let out = agent
        .ok(
            "copy_builtin_template",
            json!({"builtin": "co2", "new_id": "my-co2"}),
        )
        .await;
    assert_eq!(out["problems"], json!([]));
    let out = agent
        .ok(
            "put_template",
            json!({
                "id": "my-co2",
                "interval": "5s",
                "messages": [{"topic": "a/{{ device_id }}", "payload": "{{ co2 }}"}],
                "fields": {"co2": {"generator": "uniform", "min": 400, "max": 800}}
            }),
        )
        .await;
    assert_eq!(out["problems"], json!([]));
    let out = agent.ok("remove_template", json!({"id": "air-v2"})).await;
    assert!(out["problems"].to_string().contains("air-v2"), "{out}");
    let message = agent.err("remove_template", json!({"id": "co2"})).await;
    assert!(message.contains("built-in"), "{message}");
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn built_ins_are_listed_on_an_empty_project() {
    let agent = Agent::open(None, false, 1, temp_dir("builtins")).await;
    let templates = agent.ok("list_templates", json!({})).await;
    let builtins: Vec<&str> = templates["templates"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["builtin"] == true)
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(builtins, ["co2", "temperature", "humidity"]);
    let status = agent.ok("get_status", json!({})).await;
    assert_eq!(status["path"], Value::Null);
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_broker_change_flags_every_running_instance() {
    let agent = Agent::with_project("broker").await;
    agent.ok("start", json!({"all": true})).await;
    let out = agent
        .ok("set_broker", json!({"host": "localhost", "port": 1884}))
        .await;
    assert_eq!(
        names(&out["needs_restart"]),
        ["lab", "office-0", "office-1"]
    );
    assert_eq!(agent.ok("get_status", json!({})).await["broker_port"], 1884);
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn passwords_never_leave_and_survive_a_round_trip() {
    let agent = Agent::with_project("password").await;
    agent
        .ok(
            "set_broker",
            json!({"host": "127.0.0.1", "port": agent.port, "username": "u", "password": "s3cret"}),
        )
        .await;
    let out = agent.ok("get_project", json!({})).await;
    let toml = out["toml"].as_str().unwrap();
    assert!(!out.to_string().contains("s3cret"));
    assert!(toml.contains("<redacted>"));

    let changed = toml.replace(&format!("port = {}", agent.port), "port = 1999");
    agent.ok("set_project", json!({"toml": changed})).await;
    agent.ok("save_project", json!({})).await;
    let saved = std::fs::read_to_string(agent.dir.join("project.toml")).unwrap();
    let saved = ProjectFile::parse(&saved).unwrap();
    assert_eq!(saved.broker.password.as_deref(), Some("s3cret"));
    assert_eq!(saved.broker.port, 1999);
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unsaved_changes_protect_new_and_open() {
    let agent = Agent::with_project("unsaved").await;
    agent.ok("remove_device", json!({"id": "lab"})).await;
    let other = agent.dir.join("other.toml");
    std::fs::write(&other, project(agent.port)).unwrap();

    for (tool, args) in [
        ("new_project", json!({})),
        ("open_project", json!({"path": other.display().to_string()})),
    ] {
        let message = agent.err(tool, args).await;
        assert!(message.contains("unsaved changes"), "{tool}: {message}");
    }
    assert_eq!(agent.ok("get_status", json!({})).await["dirty"], true);

    let out = agent
        .ok(
            "open_project",
            json!({"path": other.display().to_string(), "discard_unsaved": true, "autostart": false}),
        )
        .await;
    assert_eq!(out["path"], other.display().to_string());
    assert_eq!(names(&out["instances"]), ["office-0", "office-1", "lab"]);
    assert_eq!(agent.ok("get_status", json!({})).await["dirty"], false);
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_broken_file_leaves_the_running_project_alone() {
    let agent = Agent::with_project("broken-open").await;
    agent.ok("start", json!({"instances": ["lab"]})).await;
    let broken = agent.dir.join("broken.toml");
    std::fs::write(&broken, "[broker\n").unwrap();
    let message = agent
        .err(
            "open_project",
            json!({"path": broken.display().to_string()}),
        )
        .await;
    assert!(message.contains("line 1"), "{message}");
    assert_eq!(state(&agent.status_of("lab").await), "online");
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_starts_the_autostart_devices_after_the_old_ones_stopped() {
    let agent = Agent::with_project("autostart").await;
    agent.ok("start", json!({"instances": ["lab"]})).await;
    let mut sub = Subscriber::connect(agent.port, "t/lab/status").await;
    sub.collect(Duration::from_millis(200)).await;
    let other = agent.dir.join("auto.toml");
    std::fs::write(
        &other,
        project(agent.port).replace("autostart = false", "autostart = true"),
    )
    .unwrap();
    agent
        .ok("open_project", json!({"path": other.display().to_string()}))
        .await;

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = agent.ok("get_status", json!({})).await;
        let instances = status["instances"].as_array().unwrap();
        if instances.len() == 3 && instances.iter().all(|i| state(&i["status"]) == "online") {
            break;
        }
        assert!(Instant::now() < deadline, "autostart: {status:#}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let seen: Vec<String> = sub
        .collect(Duration::from_millis(200))
        .await
        .into_iter()
        .map(|m| m.payload)
        .collect();
    assert_eq!(
        seen.first().map(String::as_str),
        Some("offline"),
        "{seen:?}"
    );
    assert!(seen.iter().any(|p| p == "online"), "{seen:?}");
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn save_and_save_as_guard_other_files() {
    let agent = Agent::open(None, false, 1, temp_dir("save")).await;
    let message = agent.err("save_project", json!({})).await;
    assert!(message.contains("no file yet"), "{message}");

    let taken = agent.dir.join("taken.toml");
    std::fs::write(&taken, "# keep me\n").unwrap();
    let message = agent
        .err("save_project", json!({"path": taken.display().to_string()}))
        .await;
    assert!(message.contains("already exists"), "{message}");
    assert_eq!(std::fs::read_to_string(&taken).unwrap(), "# keep me\n");

    agent
        .ok(
            "put_device",
            json!({"id": "d", "template": "co2", "autostart": false}),
        )
        .await;
    let out = agent
        .ok(
            "save_project",
            json!({"path": agent.dir.join("mine").display().to_string()}),
        )
        .await;
    let written = agent.dir.join("mine.toml");
    assert_eq!(out["path"], written.display().to_string());
    assert!(Path::new(&written).exists());
    assert_eq!(agent.ok("get_status", json!({})).await["dirty"], false);

    agent.ok("remove_device", json!({"id": "d"})).await;
    agent.ok("save_project", json!({})).await;
    let saved = ProjectFile::parse(&std::fs::read_to_string(&written).unwrap()).unwrap();
    assert!(saved.devices.is_empty());

    agent
        .ok(
            "save_project",
            json!({"path": taken.display().to_string(), "overwrite": true}),
        )
        .await;
    assert_ne!(std::fs::read_to_string(&taken).unwrap(), "# keep me\n");
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_reference_pages_are_resources() {
    let agent = Agent::open(None, false, 1, temp_dir("resources")).await;
    let resources = agent.client.list_all_resources().await.unwrap();
    let uris: Vec<&str> = resources.iter().map(|r| r.uri.as_str()).collect();
    assert_eq!(
        uris,
        [
            "senzor://reference/project-file",
            "senzor://reference/generators",
            "senzor://reference/template-variables",
            "senzor://reference/builtin-templates"
        ]
    );
    let read = agent
        .client
        .read_resource(rmcp::model::ReadResourceRequestParams::new(
            "senzor://reference/generators",
        ))
        .await
        .unwrap();
    let text = serde_json::to_value(&read.contents[0]).unwrap()["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(text.contains("random_walk"));
    assert!(
        agent
            .client
            .read_resource(rmcp::model::ReadResourceRequestParams::new("senzor://nope"))
            .await
            .is_err()
    );
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn save_reports_the_absolute_path_of_a_project_opened_by_a_relative_one() {
    let dir = temp_dir("relative");
    let absolute = dir.join("project.toml");
    std::fs::write(&absolute, project(1)).unwrap();
    // The same file, reached from the working directory without changing it.
    let cwd = std::env::current_dir().unwrap();
    let mut relative = std::path::PathBuf::new();
    for _ in cwd.components().skip(1) {
        relative.push("..");
    }
    relative.push(absolute.strip_prefix("/").unwrap());
    assert!(relative.is_relative());

    let agent = Agent::open(Some(&relative), false, 1, dir.clone()).await;
    agent.ok("remove_device", json!({"id": "lab"})).await;
    let out = agent.ok("save_project", json!({})).await;
    let saved = Path::new(out["path"].as_str().unwrap());
    assert!(saved.is_absolute(), "{out}");
    assert_eq!(
        std::fs::canonicalize(saved).unwrap(),
        std::fs::canonicalize(&absolute).unwrap()
    );
    agent.finish().await;
}
