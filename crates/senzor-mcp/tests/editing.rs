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
async fn edits_reach_running_instances_or_flag_those_that_need_a_new_connection() {
    let agent = Agent::with_project("needs-restart").await;
    let mut sub = Subscriber::connect(agent.port, "t/lab/#").await;
    agent.ok("start", json!({"instances": ["lab"]})).await;
    sub.wait_for(Duration::from_secs(5), |m| m.payload == "1/2")
        .await
        .unwrap();
    let out = agent
        .ok(
            "put_device",
            json!({"id": "lab", "template": "air", "autostart": false, "interval": "200ms",
                   "fields": {"v": {"value": 7}}}),
        )
        .await;
    assert_eq!(names(&out["needs_restart"]), Vec::<&str>::new());
    let seen = sub
        .wait_for(Duration::from_secs(5), |m| m.payload == "7/2")
        .await;
    assert!(
        seen.is_some(),
        "the running instance publishes the new value"
    );
    assert_eq!(state(&agent.status_of("lab").await), "online");

    let air = json!({
        "id": "air",
        "interval": "100ms",
        "availability": {"topic": "t/{{ device_id }}/status", "online": "up", "offline": "offline"},
        "messages": [{"topic": "t/{{ device_id }}/state", "payload": "{{ v }}/{{ w }}"}],
        "fields": {
            "v": {"generator": "constant", "value": 1},
            "w": {"generator": "constant", "value": 2}
        }
    });
    let out = agent.ok("put_template", air).await;
    assert_eq!(
        names(&out["needs_restart"]),
        ["lab"],
        "the Last Will changed"
    );
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
async fn set_broker_sets_the_mqtt_version_and_defaults_it_to_5() {
    let agent = Agent::with_project("mqtt-version").await;
    let broker = |extra: Value| {
        let mut args = json!({"host": "127.0.0.1", "port": agent.port});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        args
    };
    agent
        .ok("set_broker", broker(json!({"mqtt_version": "3.1.1"})))
        .await;
    let toml = agent.ok("get_project", json!({})).await["toml"].clone();
    assert!(
        toml.as_str().unwrap().contains(r#"mqtt_version = "3.1.1""#),
        "{toml}"
    );

    agent.ok("set_broker", broker(json!({}))).await;
    let toml = agent.ok("get_project", json!({})).await["toml"].clone();
    assert!(
        toml.as_str().unwrap().contains(r#"mqtt_version = "5""#),
        "{toml}"
    );

    let error = agent
        .err("set_broker", broker(json!({"mqtt_version": "4"})))
        .await;
    assert!(error.contains("3.1.1"), "{error}");
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn put_template_keeps_field_units_and_names() {
    let agent = Agent::with_project("field-labels").await;
    let tools = agent.client.list_all_tools().await.unwrap();
    let put_template = tools.iter().find(|t| t.name == "put_template").unwrap();
    let schema = serde_json::to_string(&put_template.input_schema).unwrap();
    assert!(schema.contains("\"unit\""), "unit missing in: {schema}");
    let description = put_template.description.as_deref().unwrap_or_default();
    assert!(description.contains("unit"), "{description}");

    let out = agent
        .ok(
            "put_template",
            json!({
                "id": "labelled",
                "messages": [{"topic": "a/{{ device_id }}", "payload": "{{ co2 }}"}],
                "fields": {"co2": {
                    "generator": "uniform", "min": 400, "max": 800, "unit": "ppm", "name": "CO2"
                }}
            }),
        )
        .await;
    assert_eq!(out["problems"], json!([]));
    let toml = agent.ok("get_project", json!({})).await["toml"].clone();
    let file = ProjectFile::parse(toml.as_str().unwrap()).unwrap();
    let field = &file
        .templates
        .iter()
        .find(|t| t.id == "labelled")
        .unwrap()
        .fields["co2"];
    assert_eq!(field.unit.as_deref(), Some("ppm"));
    assert_eq!(field.name.as_deref(), Some("CO2"));

    let out = agent
        .ok(
            "put_template",
            json!({
                "id": "labelled",
                "messages": [{"topic": "a/{{ device_id }}", "payload": "{{ co2 }}"}],
                "fields": {"co2": {"generator": "uniform", "min": 400, "max": 800, "unit": ""}}
            }),
        )
        .await;
    let problems = out["problems"].to_string();
    assert!(problems.contains("unit must not be empty"), "{problems}");
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn put_template_stores_mqtt5_properties_and_reports_them_on_3_1_1() {
    let agent = Agent::with_project("template-properties").await;
    let template = json!({
        "id": "props",
        "interval": "5s",
        "availability": {"topic": "s/{{ device_id }}", "online": "on", "offline": "off", "will_delay": "5s"},
        "messages": [{
            "topic": "a/{{ device_id }}",
            "payload": "{{ v }}",
            "message_expiry": "60s",
            "content_type": "application/json",
            "user_properties": {"room": "{{ device_id }}"}
        }],
        "fields": {"v": {"generator": "constant", "value": 1}}
    });
    let out = agent.ok("put_template", template).await;
    let toml = agent.ok("get_project", json!({})).await["toml"].clone();
    let toml = toml.as_str().unwrap();
    for key in [
        "message_expiry",
        "content_type",
        "user_properties",
        "will_delay",
    ] {
        assert!(toml.contains(key), "{key} missing in {toml}");
    }
    let problems = out["problems"].to_string();
    assert!(problems.contains("session_expiry"), "{problems}");

    let out = agent
        .ok(
            "set_broker",
            json!({"host": "127.0.0.1", "port": agent.port, "mqtt_version": "3.1.1"}),
        )
        .await;
    let problems = out["problems"].to_string();
    assert!(problems.contains("content_type needs MQTT 5"), "{problems}");
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_input_schemas_describe_the_mqtt5_keys() {
    let agent = Agent::with_project("mqtt5-schemas").await;
    let tools = agent.client.list_all_tools().await.unwrap();
    let schema = |name: &str| {
        let tool = tools.iter().find(|t| t.name == name).unwrap();
        serde_json::to_string(&tool.input_schema).unwrap()
    };
    let description = tools
        .iter()
        .find(|t| t.name == "set_broker")
        .and_then(|t| t.description.as_deref())
        .unwrap_or_default();
    for key in ["mqtt_version", "session_expiry"] {
        assert!(description.contains(key), "{key} missing in: {description}");
    }
    let broker = schema("set_broker");
    for key in ["mqtt_version", "\"3.1.1\"", "\"5\"", "session_expiry"] {
        assert!(
            broker.contains(key),
            "{key} missing in set_broker: {broker}"
        );
    }
    let template = schema("put_template");
    for key in [
        "message_expiry",
        "content_type",
        "user_properties",
        "will_delay",
    ] {
        assert!(
            template.contains(key),
            "{key} missing in put_template: {template}"
        );
    }
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
            "senzor://project",
            "senzor://instances",
            "senzor://messages",
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

/// The project as `get_project` shows it.
async fn project_file(agent: &Agent) -> ProjectFile {
    let out = agent.ok("get_project", json!({})).await;
    ProjectFile::parse(out["toml"].as_str().unwrap()).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn set_changes_one_value_deep_in_the_project() {
    let agent = Agent::with_project("set").await;
    let out = agent
        .ok("set", json!({"path": "devices.office.count", "value": 3}))
        .await;
    assert_eq!(out["changed"], true);
    assert_eq!(state(&agent.status_of("office-2").await), "stopped");

    let mut sub = Subscriber::connect(agent.port, "t/lab/state").await;
    agent.ok("start", json!({"instances": ["lab"]})).await;
    let out = agent
        .ok(
            "set",
            json!({"path": "templates.air.messages[1].payload", "value": "v={{ v }}"}),
        )
        .await;
    assert_eq!(names(&out["needs_restart"]), Vec::<&str>::new());
    let seen = sub
        .wait_for(Duration::from_secs(5), |m| m.payload == "v=1")
        .await;
    assert!(seen.is_some(), "the running instance takes the new payload");

    let out = agent
        .ok(
            "set",
            json!({"path": "devices.lab.params.room", "value": "lab"}),
        )
        .await;
    assert_eq!(out["changed"], true);
    let out = agent
        .ok("set", json!({"path": "devices.lab.count", "value": 1}))
        .await;
    assert_eq!(out["changed"], false, "the value it already had");

    let before = project_file(&agent).await;
    let message = agent
        .err(
            "set",
            json!({"path": "devices.office.count", "value": "many"}),
        )
        .await;
    assert!(message.contains("`devices.office.count`"), "{message}");
    assert!(message.contains("u32"), "{message}");
    let message = agent
        .err("set", json!({"path": "devices.nope.count", "value": 1}))
        .await;
    assert!(message.contains("no device `nope`"), "{message}");
    let message = agent
        .err("set", json!({"path": "devices..count", "value": 1}))
        .await;
    assert!(message.contains("messages numbered from 1"), "{message}");
    assert_eq!(
        project_file(&agent).await,
        before,
        "a refused set changes nothing"
    );

    let out = agent
        .ok(
            "set",
            json!({"path": "devices.lab.template", "value": "missing"}),
        )
        .await;
    assert!(out["problems"].to_string().contains("missing"), "{out}");
    assert_eq!(names(&out["blocked"]), ["lab"]);
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unset_lets_the_default_apply() {
    let agent = Agent::with_project("unset").await;
    let out = agent
        .ok("unset", json!({"path": "devices.office.count"}))
        .await;
    assert_eq!(out["changed"], true);
    assert_eq!(project_file(&agent).await.devices[0].count, 1);
    let status = agent.ok("get_status", json!({})).await;
    assert!(!status.to_string().contains("office-1"), "{status}");

    let message = agent
        .err("unset", json!({"path": "devices.office.template"}))
        .await;
    assert!(message.contains("missing field `template`"), "{message}");
    let message = agent
        .err("unset", json!({"path": "devices.office.interval"}))
        .await;
    assert!(message.contains("not set"), "{message}");
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn undo_and_redo_step_through_the_edits() {
    let agent = Agent::with_project("undo").await;
    let message = agent.err("undo", json!({})).await;
    assert!(message.contains("nothing to undo"), "{message}");
    let message = agent.err("redo", json!({})).await;
    assert!(message.contains("nothing to redo"), "{message}");
    let original = project_file(&agent).await;

    agent
        .ok("set", json!({"path": "devices.office.count", "value": 3}))
        .await;
    agent
        .ok("set", json!({"path": "broker.port", "value": 1}))
        .await;
    agent
        .err("set", json!({"path": "broker.port", "value": "x"}))
        .await;

    let out = agent.ok("undo", json!({})).await;
    assert_eq!(out["broker"], true, "{out}");
    assert_eq!(out["devices"], json!([]), "{out}");
    assert_eq!(out["can_undo"], true);
    assert_eq!(out["can_redo"], true);
    assert_eq!(project_file(&agent).await.broker.port, agent.port);

    let out = agent.ok("undo", json!({})).await;
    assert_eq!(out["broker"], false, "{out}");
    assert_eq!(names(&out["devices"]), ["office"]);
    assert_eq!(out["can_undo"], false);
    assert_eq!(project_file(&agent).await, original);
    let status = agent.ok("get_status", json!({})).await;
    assert_eq!(status["dirty"], false, "back to the file as opened");
    assert!(
        !status.to_string().contains("office-2"),
        "the engine follows"
    );
    let message = agent.err("undo", json!({})).await;
    assert!(message.contains("nothing to undo"), "{message}");

    let out = agent.ok("redo", json!({})).await;
    assert_eq!(names(&out["devices"]), ["office"]);
    assert_eq!(state(&agent.status_of("office-2").await), "stopped");
    let out = agent.ok("redo", json!({})).await;
    assert_eq!(out["broker"], true);
    assert_eq!(out["can_redo"], false);
    assert_eq!(project_file(&agent).await.broker.port, 1);

    agent.ok("undo", json!({})).await;
    agent
        .ok("put_device", json!({"id": "probe", "template": "co2"}))
        .await;
    let message = agent.err("redo", json!({})).await;
    assert!(
        message.contains("nothing to redo"),
        "a new edit drops the redo: {message}"
    );
    let out = agent.ok("undo", json!({})).await;
    assert_eq!(names(&out["devices"]), ["probe"]);
    agent.finish().await;
}
