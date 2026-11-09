//! Resource subscriptions: the server tells a subscribed client when the project, the instances
//! or the message log change, at most once per resource every 250 ms.

#![allow(deprecated)] // `resources/subscribe` is how clients before 2026-07-28 subscribe.

mod support;

use std::time::{Duration, Instant};

use rmcp::model::{
    ReadResourceRequestParams, ResourceContents, ServerNotification, SubscribeRequestParams,
    SubscriptionFilter, UnsubscribeRequestParams,
};
use serde_json::{Value, json};
use support::Agent;

const PROJECT: &str = "senzor://project";
const INSTANCES: &str = "senzor://instances";
const MESSAGES: &str = "senzor://messages";

/// The shortest time between two updates of one resource.
const INTERVAL: Duration = Duration::from_millis(250);
/// Long enough for an update that is coming to arrive.
const SETTLE: Duration = Duration::from_millis(700);

async fn subscribe(agent: &Agent, uri: &str) {
    agent
        .client
        .subscribe(SubscribeRequestParams::new(uri))
        .await
        .unwrap();
}

async fn read(agent: &Agent, uri: &str) -> Value {
    let result = agent
        .client
        .read_resource(ReadResourceRequestParams::new(uri))
        .await
        .unwrap();
    let ResourceContents::TextResourceContents { text, .. } = &result.contents[0] else {
        panic!("{uri} is not text");
    };
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{uri} is not JSON: {e}: {text}"))
}

async fn add_device(agent: &Agent, id: &str) {
    agent
        .ok(
            "put_device",
            json!({"id": id, "template": "air", "autostart": false}),
        )
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_server_offers_subscriptions_and_prompts() {
    let agent = Agent::with_project("sub-capabilities").await;
    let info = agent.client.peer_info().unwrap();
    let resources = info.capabilities.resources.clone().unwrap();
    assert_eq!(resources.subscribe, Some(true));
    assert_ne!(
        resources.list_changed,
        Some(true),
        "the resource list is fixed"
    );
    assert!(info.capabilities.prompts.is_some());
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_live_resources_are_listed_and_read_as_json() {
    let agent = Agent::with_project("sub-read").await;
    let listed: Vec<String> = agent
        .client
        .list_all_resources()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.uri.clone())
        .collect();
    for uri in [PROJECT, INSTANCES, MESSAGES] {
        assert!(listed.iter().any(|u| u == uri), "{uri} in {listed:?}");
    }

    let project = read(&agent, PROJECT).await;
    assert!(project["toml"].as_str().unwrap().contains("[[devices]]"));
    assert_eq!(project["dirty"], false);
    let instances = read(&agent, INSTANCES).await;
    let ids: Vec<&str> = instances["instances"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["lab", "office-0", "office-1"]);
    assert_eq!(instances["instances"][0]["status"]["state"], "stopped");
    let messages = read(&agent, MESSAGES).await;
    assert_eq!(messages["messages"], json!([]));
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_sends_one_update_to_a_project_subscriber() {
    let agent = Agent::with_project("sub-edit").await;
    subscribe(&agent, PROJECT).await;
    subscribe(&agent, MESSAGES).await;
    tokio::time::sleep(SETTLE).await;
    assert!(
        agent.updates.updates(PROJECT).is_empty(),
        "no change, no update"
    );

    add_device(&agent, "porch").await;
    assert_eq!(agent.updates.wait_for(PROJECT, 1).await, 1);
    tokio::time::sleep(SETTLE).await;
    assert_eq!(agent.updates.updates(PROJECT).len(), 1, "exactly one");
    assert!(
        agent.updates.updates(MESSAGES).is_empty(),
        "an edit publishes nothing"
    );
    assert!(
        agent.updates.updates(INSTANCES).is_empty(),
        "not subscribed"
    );
    assert!(
        read(&agent, PROJECT).await["toml"]
            .as_str()
            .unwrap()
            .contains("porch")
    );
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_burst_of_edits_is_coalesced_and_its_last_edit_is_notified() {
    let agent = Agent::with_project("sub-burst").await;
    subscribe(&agent, PROJECT).await;

    let started = Instant::now();
    let mut last_edit = started;
    for n in 0..20 {
        last_edit = Instant::now();
        add_device(&agent, &format!("d{n}")).await;
    }
    let burst = started.elapsed();
    tokio::time::sleep(SETTLE).await;

    let updates = agent.updates.updates(PROJECT);
    let most = (burst.as_millis() / INTERVAL.as_millis()) as usize + 2;
    assert!(
        !updates.is_empty() && updates.len() <= most,
        "{} updates for 20 edits in {burst:?}",
        updates.len()
    );
    assert!(
        updates
            .windows(2)
            .all(|w| w[1] - w[0] >= INTERVAL - Duration::from_millis(30)),
        "updates closer than {INTERVAL:?}: {updates:?}"
    );
    assert!(
        *updates.last().unwrap() > last_edit,
        "the last edit was never notified"
    );
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unsubscribing_stops_the_updates() {
    let agent = Agent::with_project("sub-unsubscribe").await;
    subscribe(&agent, PROJECT).await;
    add_device(&agent, "first").await;
    assert_eq!(agent.updates.wait_for(PROJECT, 1).await, 1);

    agent
        .client
        .unsubscribe(UnsubscribeRequestParams::new(PROJECT))
        .await
        .unwrap();
    tokio::time::sleep(INTERVAL).await;
    add_device(&agent, "second").await;
    tokio::time::sleep(SETTLE).await;
    assert_eq!(agent.updates.updates(PROJECT).len(), 1);
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn running_instances_update_the_instances_and_throttle_the_messages() {
    let agent = Agent::with_project("sub-run").await;
    subscribe(&agent, INSTANCES).await;
    subscribe(&agent, MESSAGES).await;

    agent.ok("start", json!({"all": true})).await;
    assert!(agent.updates.wait_for(INSTANCES, 1).await >= 1);
    assert!(agent.updates.wait_for(MESSAGES, 1).await >= 1);

    // Three instances publish every 100 ms: about 30 messages a second.
    let before = agent.updates.updates(MESSAGES).len();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let during = agent.updates.updates(MESSAGES).len() - before;
    assert!((1..=5).contains(&during), "{during} updates in a second");
    let messages = read(&agent, MESSAGES).await;
    assert!(!messages["messages"].as_array().unwrap().is_empty());
    assert!(messages["last_seq"].as_u64().unwrap() > 0);

    let online = read(&agent, INSTANCES).await;
    assert!(
        online["instances"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["status"]["state"] == "online"),
        "{online:#}"
    );
    let seen = agent.updates.updates(INSTANCES).len();
    agent.ok("stop", json!({"instances": ["lab"]})).await;
    assert!(agent.updates.wait_for(INSTANCES, seen + 1).await > seen);
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_resource_cannot_be_subscribed() {
    let agent = Agent::with_project("sub-unknown").await;
    let error = agent
        .client
        .subscribe(SubscribeRequestParams::new("senzor://nothing"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("senzor://nothing"), "{error}");
    // A reference page never changes, but subscribing to it is allowed.
    agent
        .client
        .subscribe(SubscribeRequestParams::new("senzor://reference/generators"))
        .await
        .unwrap();
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_2026_client_listens_for_updates() {
    let agent = Agent::with_project_discovered("sub-listen").await;
    let filter = SubscriptionFilter::builder()
        .resource_subscription(PROJECT)
        .resource_subscription("senzor://nothing")
        .build();
    let mut subscription = agent.client.peer().listen(filter).await.unwrap();
    assert_eq!(
        subscription.acknowledged().resource_subscriptions,
        Some(vec![PROJECT.to_string()]),
        "an unknown URI is not accepted"
    );

    add_device(&agent, "porch").await;
    let notification = tokio::time::timeout(support::WAIT, subscription.next())
        .await
        .expect("an update")
        .unwrap()
        .unwrap();
    let ServerNotification::ResourceUpdatedNotification(update) = notification else {
        panic!("{notification:?}");
    };
    assert_eq!(update.params.uri, PROJECT);
    assert!(
        tokio::time::timeout(SETTLE, subscription.next())
            .await
            .is_err(),
        "exactly one"
    );

    // Cancelling the listen request is how a 2026-07-28 client unsubscribes.
    subscription.cancel().await.unwrap();
    add_device(&agent, "garage").await;
    tokio::time::sleep(SETTLE).await;
    assert!(
        matches!(subscription.next().await, Ok(None) | Err(_)),
        "no update after cancelling"
    );
    agent.finish().await;
}
