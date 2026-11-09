//! Runtime control over MCP: tool list, status, messages, start/stop/kill/restart, pins.

mod support;

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use senzor_testkit::Subscriber;
use serde_json::json;
use support::{Agent, WAIT, project, state, temp_dir};

const TOOLS: [&str; 27] = [
    "copy_builtin_template",
    "get_instance",
    "get_messages",
    "get_project",
    "get_status",
    "kill",
    "list_templates",
    "new_project",
    "open_project",
    "pin",
    "put_device",
    "put_template",
    "remove_device",
    "remove_template",
    "rename_device",
    "redo",
    "rename_template",
    "restart",
    "save_project",
    "set",
    "set_broker",
    "set_project",
    "start",
    "stop",
    "undo",
    "unpin",
    "unset",
];

#[tokio::test(flavor = "multi_thread")]
async fn the_server_names_itself_and_lists_every_tool_with_a_schema() {
    let agent = Agent::with_project("tools").await;
    let info = agent.client.peer_info().unwrap();
    let server = info.server_info.as_ref().unwrap();
    assert_eq!(server.name, "senzor");
    assert_eq!(server.version, env!("CARGO_PKG_VERSION"));
    assert!(info.instructions.as_deref().unwrap_or("").contains("pin"));

    let tools = agent.client.list_all_tools().await.unwrap();
    let names: BTreeSet<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    let expected: BTreeSet<&str> = TOOLS.into_iter().collect();
    assert_eq!(names, expected);
    for tool in &tools {
        assert!(
            tool.description.as_deref().is_some_and(|d| !d.is_empty()),
            "{}",
            tool.name
        );
        assert_eq!(
            tool.input_schema.get("type"),
            Some(&json!("object")),
            "{}",
            tool.name
        );
    }
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_instance_is_a_tool_error() {
    let agent = Agent::with_project("unknown-instance").await;
    let message = agent.err("get_instance", json!({"instance": "nope"})).await;
    assert!(message.contains("nope"), "{message}");
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn start_waits_for_online_and_status_shows_the_fleet() {
    let agent = Agent::with_project("start").await;
    let out = agent.ok("start", json!({"devices": ["office"]})).await;
    assert_eq!(out["timed_out"], false);
    let ids: Vec<&str> = out["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            assert_eq!(state(&i["status"]), "online");
            i["id"].as_str().unwrap()
        })
        .collect();
    assert_eq!(ids, ["office-0", "office-1"]);

    let status = agent.ok("get_status", json!({})).await;
    assert_eq!(status["broker_port"], agent.port);
    assert_eq!(status["dirty"], false);
    assert!(status["path"].as_str().unwrap().ends_with("project.toml"));
    let office: Vec<_> = status["instances"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["device"] == "office")
        .collect();
    assert_eq!(office.len(), 2);
    assert_eq!(state(&agent.status_of("lab").await), "stopped");
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_targets_are_refused_and_nothing_starts() {
    let agent = Agent::with_project("unknown-target").await;
    let message = agent
        .err("start", json!({"devices": ["office", "ghost"]}))
        .await;
    assert!(message.contains("ghost"), "{message}");
    assert!(!message.contains("office"), "{message}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(state(&agent.status_of("office-0").await), "stopped");

    let message = agent.err("stop", json!({})).await;
    assert!(message.contains("target is required"), "{message}");
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn start_without_waiting_returns_at_once() {
    let agent = Agent::with_project("no-wait").await;
    let started = Instant::now();
    let out = agent
        .ok("start", json!({"all": true, "wait_seconds": 0}))
        .await;
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(out["timed_out"], false);
    assert_eq!(out["instances"].as_array().unwrap().len(), 3);
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn kill_drops_the_connection_and_the_broker_sends_the_last_will() {
    let agent = Agent::with_project("kill").await;
    let mut sub = Subscriber::connect(agent.port, "t/lab/status").await;
    agent.ok("start", json!({"instances": ["lab"]})).await;
    sub.wait_for(WAIT, |m| m.payload == "online").await.unwrap();

    let out = agent.ok("kill", json!({"instances": ["lab"]})).await;
    assert_eq!(state(&out["instances"][0]["status"]), "stopped");
    sub.wait_for(WAIT, |m| m.payload == "offline")
        .await
        .expect("Last Will");
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn restart_returns_once_the_new_run_is_online() {
    let agent = Agent::with_project("restart").await;
    agent.ok("start", json!({"instances": ["lab"]})).await;
    let mut sub = Subscriber::connect(agent.port, "t/lab/status").await;
    sub.collect(Duration::from_millis(200)).await;

    let out = agent.ok("restart", json!({"instances": ["lab"]})).await;
    assert_eq!(out["timed_out"], false);
    assert_eq!(state(&out["instances"][0]["status"]), "online");
    let seen: Vec<String> = sub
        .collect(Duration::from_millis(300))
        .await
        .into_iter()
        .map(|m| m.payload)
        .collect();
    let offline = seen
        .iter()
        .position(|p| p == "offline")
        .expect("went offline");
    assert!(
        seen[offline..].iter().any(|p| p == "online"),
        "back online before restart returned: {seen:?}"
    );
    agent.finish().await;
}

/// With an `instance_id` that does not use `{{ id }}`, a renamed device keeps its running
/// instance, and restart and stop by the new device name reach it.
#[tokio::test(flavor = "multi_thread")]
async fn device_targets_reach_a_running_instance_after_its_device_is_renamed() {
    let agent = Agent::with_project("rename-running").await;
    agent
        .ok(
            "put_device",
            json!({"id": "lab", "template": "air", "autostart": false, "instance_id": "room"}),
        )
        .await;
    agent.ok("start", json!({"instances": ["room"]})).await;
    agent
        .ok("rename_device", json!({"from": "lab", "to": "bench"}))
        .await;

    let out = agent
        .ok("restart", json!({"devices": ["bench"], "wait_seconds": 2}))
        .await;
    assert_eq!(out["timed_out"], false, "{out:#}");
    assert_eq!(out["instances"][0]["id"], "room");
    assert_eq!(state(&out["instances"][0]["status"]), "online");

    let out = agent
        .ok("stop", json!({"devices": ["bench"], "wait_seconds": 2}))
        .await;
    assert_eq!(out["timed_out"], false, "{out:#}");
    assert_eq!(state(&out["instances"][0]["status"]), "stopped");
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pin_is_published_and_unpinning_all_restores_the_generators() {
    let agent = Agent::with_project("pin").await;
    let mut sub = Subscriber::connect(agent.port, "t/lab/state").await;
    agent.ok("start", json!({"instances": ["lab"]})).await;
    sub.wait_for(WAIT, |m| m.payload == "1/2").await.unwrap();

    let out = agent
        .ok(
            "pin",
            json!({"instance": "lab", "field": "v", "value": 2500}),
        )
        .await;
    assert_eq!(out["pinned"], json!(["v"]));
    sub.wait_for(WAIT, |m| m.payload == "2500/2").await.unwrap();
    agent
        .ok(
            "pin",
            json!({"instance": "lab", "field": "w", "value": "x"}),
        )
        .await;
    sub.wait_for(WAIT, |m| m.payload == "2500/x").await.unwrap();
    let detail = agent.ok("get_instance", json!({"instance": "lab"})).await;
    assert_eq!(detail["pinned"], json!(["v", "w"]));
    assert_eq!(detail["values"]["v"], 2500.0);
    assert_eq!(detail["template"], "air");
    assert_eq!(detail["fields"], json!(["v", "w"]));

    let out = agent.ok("unpin", json!({"instance": "lab"})).await;
    assert_eq!(out["pinned"], json!([]));
    sub.wait_for(WAIT, |m| m.payload == "1/2").await.unwrap();
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn pins_are_checked() {
    let agent = Agent::with_project("pin-checks").await;
    let message = agent
        .err("pin", json!({"instance": "lab", "field": "v", "value": 1}))
        .await;
    assert!(message.contains("not running"), "{message}");

    agent.ok("start", json!({"instances": ["lab"]})).await;
    let message = agent
        .err(
            "pin",
            json!({"instance": "lab", "field": "nope", "value": 1}),
        )
        .await;
    assert!(message.contains("no field `nope`"), "{message}");
    let message = agent
        .err(
            "pin",
            json!({"instance": "lab", "field": "v", "value": null}),
        )
        .await;
    assert!(
        message.contains("failed to deserialize parameters"),
        "{message}"
    );
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn messages_carry_their_mqtt5_properties_and_omit_unset_ones() {
    let port = senzor_testkit::start_broker();
    let dir = temp_dir("properties");
    let path = dir.join("project.toml");
    let content = project(port).replace(
        r#"payload = "{{ v }}/{{ w }}" }]"#,
        r#"payload = "{{ v }}/{{ w }}", content_type = "text/plain", user_properties = { id = "{{ device_id }}" } }]"#,
    );
    assert!(content.contains("user_properties"));
    std::fs::write(&path, content).unwrap();
    let agent = Agent::open(Some(&path), true, port, dir).await;

    agent.ok("start", json!({"instances": ["lab"]})).await;
    let out = agent
        .ok("get_messages", json!({"device": "lab", "wait_seconds": 5}))
        .await;
    let message = &out["messages"][0];
    assert_eq!(message["content_type"], "text/plain", "{message}");
    assert_eq!(
        message["user_properties"],
        json!({"id": "lab"}),
        "{message}"
    );
    assert!(message.get("message_expiry").is_none(), "{message}");
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn messages_without_properties_have_no_property_keys() {
    let agent = Agent::with_project("no-properties").await;
    agent.ok("start", json!({"instances": ["lab"]})).await;
    let out = agent
        .ok("get_messages", json!({"device": "lab", "wait_seconds": 5}))
        .await;
    let message = out["messages"][0].as_object().unwrap();
    for key in ["message_expiry", "content_type", "user_properties"] {
        assert!(!message.contains_key(key), "{key} in {message:?}");
    }
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn messages_come_with_a_cursor_filters_and_a_wait() {
    let agent = Agent::with_project("messages").await;
    let first = agent.ok("get_messages", json!({})).await;
    let cursor = first["last_seq"].as_u64().unwrap();

    agent.ok("start", json!({"instances": ["lab"]})).await;
    let out = agent
        .ok(
            "get_messages",
            json!({"after_seq": cursor, "wait_seconds": 5}),
        )
        .await;
    let messages = out["messages"].as_array().unwrap();
    assert!(!messages.is_empty());
    let seqs: Vec<u64> = messages
        .iter()
        .map(|m| m["seq"].as_u64().unwrap())
        .collect();
    assert!(seqs.iter().all(|s| *s > cursor), "{seqs:?}");
    assert!(seqs.windows(2).all(|w| w[0] < w[1]), "{seqs:?}");
    assert_eq!(messages[0]["topic"], "t/lab/state");
    assert!(messages[0]["time"].as_str().unwrap().ends_with('Z'));
    assert_eq!(out["truncated"], false);

    // Waiting for a topic nobody publishes on yet, then starting it.
    let cursor = out["last_seq"].as_u64().unwrap();
    agent
        .ok(
            "start",
            json!({"instances": ["office-1"], "wait_seconds": 0}),
        )
        .await;
    let started = Instant::now();
    let out = agent
        .ok(
            "get_messages",
            json!({"after_seq": cursor, "topic_contains": "office-1", "wait_seconds": 10}),
        )
        .await;
    assert!(started.elapsed() < Duration::from_secs(5));
    let messages = out["messages"].as_array().unwrap();
    assert!(!messages.is_empty());
    assert!(messages.iter().all(|m| m["instance"] == "office-1"));

    let by_device = agent
        .ok("get_messages", json!({"device": "office", "limit": 2}))
        .await;
    let messages = by_device["messages"].as_array().unwrap();
    assert!(!messages.is_empty() && messages.len() <= 2);
    assert!(messages.iter().all(|m| m["instance"] == "office-1"));
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_is_counted_as_received_and_its_script_runs() {
    let agent = Agent::with_project("received").await;
    agent
        .ok(
            "put_template",
            json!({
                "id": "relay",
                "interval": "100ms",
                "availability": {"topic": "t/{{ device_id }}/status", "online": "online", "offline": "offline"},
                "messages": [{"topic": "t/{{ device_id }}/state", "payload": "{{ on }}"}],
                "fields": {"on": {"generator": "script", "start": false, "script": "state.on == true"}},
                "subscriptions": [{"topic": "cmd/{{ device_id }}/set", "qos": 1, "script": "state.on = payload == \"ON\";"}]
            }),
        )
        .await;
    agent
        .ok(
            "put_device",
            json!({"id": "relay", "template": "relay", "autostart": false}),
        )
        .await;
    let mut sub = Subscriber::connect(agent.port, "t/relay/#").await;
    agent.ok("start", json!({"devices": ["relay"]})).await;
    let relay = agent.ok("get_instance", json!({"instance": "relay"})).await;
    assert_eq!(relay["received"], 0);

    sub.publish("cmd/relay/set", "ON").await;
    sub.wait_for(WAIT, |m| m.topic == "t/relay/state" && m.payload == "true")
        .await
        .expect("the script turned it on");
    let relay = agent.ok("get_instance", json!({"instance": "relay"})).await;
    assert_eq!(relay["received"], 1);
    assert_eq!(relay["errors"], 0);
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn get_messages_without_a_wait_returns_at_once() {
    let agent = Agent::with_project("messages-now").await;
    let started = Instant::now();
    let out = agent
        .ok("get_messages", json!({"topic_contains": "nothing"}))
        .await;
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(out["messages"], json!([]));
    agent.finish().await;
}
