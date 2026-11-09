//! Prompts: ready-made first messages that set an agent to work with the tools.

mod support;

use std::collections::BTreeSet;

use rmcp::model::{GetPromptRequestParams, GetPromptResult, Role};
use serde_json::{Map, Value, json};
use support::{Agent, temp_dir};

async fn agent(name: &str) -> Agent {
    Agent::open(None, false, 1, temp_dir(name)).await
}

async fn get(agent: &Agent, name: &str, args: Value) -> Result<GetPromptResult, String> {
    let mut params = GetPromptRequestParams::new(name);
    if let Value::Object(map) = args {
        params = params.with_arguments(map);
    }
    agent
        .client
        .get_prompt(params)
        .await
        .map_err(|e| e.to_string())
}

fn text(result: &GetPromptResult) -> String {
    assert_eq!(result.messages.len(), 1, "{result:?}");
    let message = &result.messages[0];
    assert_eq!(message.role, Role::User);
    serde_json::to_value(&message.content).unwrap()["text"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_prompts_are_listed_with_their_arguments() {
    let agent = agent("prompts-list").await;
    let prompts = agent.client.list_all_prompts().await.unwrap();
    let names: Vec<&str> = prompts.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(
        names,
        ["simulate-device", "test-consumer", "explain-problems"]
    );
    for prompt in &prompts {
        assert!(prompt.description.is_some(), "{}", prompt.name);
        for argument in prompt.arguments.iter().flatten() {
            assert!(argument.description.is_some(), "{}", argument.name);
        }
    }
    let required = |name: &str| -> Vec<String> {
        prompts
            .iter()
            .find(|p| p.name == name)
            .unwrap()
            .arguments
            .iter()
            .flatten()
            .filter(|a| a.required == Some(true))
            .map(|a| a.name.clone())
            .collect()
    };
    assert_eq!(required("simulate-device"), ["device"]);
    assert_eq!(required("test-consumer"), ["expectation"]);
    assert!(required("explain-problems").is_empty());
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_puts_its_arguments_into_the_message() {
    let agent = agent("prompts-get").await;
    let device = "Shelly H&T: temperature and humidity every 5 minutes as JSON";
    let message = text(
        &get(&agent, "simulate-device", json!({"device": device}))
            .await
            .unwrap(),
    );
    assert!(message.contains(device), "{message}");
    assert!(message.contains("put_template"), "{message}");

    let expectation = "the fan turns on above 1000 ppm";
    let message = text(
        &get(&agent, "test-consumer", json!({"expectation": expectation}))
            .await
            .unwrap(),
    );
    assert!(message.contains(expectation), "{message}");
    assert!(message.contains("pin"), "{message}");

    let message = text(&get(&agent, "explain-problems", json!({})).await.unwrap());
    assert!(message.contains("get_project"), "{message}");
    agent.finish().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_required_argument_or_an_unknown_prompt_is_an_error() {
    let agent = agent("prompts-errors").await;
    let error = get(&agent, "simulate-device", json!({})).await.unwrap_err();
    assert!(error.contains("device"), "{error}");
    let error = get(&agent, "test-consumer", json!({"expectation": "  "}))
        .await
        .unwrap_err();
    assert!(error.contains("expectation"), "{error}");
    let error = get(&agent, "nothing", json!({})).await.unwrap_err();
    assert!(error.contains("nothing"), "{error}");
    agent.finish().await;
}

/// Every `snake_case` word in backticks in a prompt is a tool the server has.
#[tokio::test(flavor = "multi_thread")]
async fn the_prompts_name_only_tools_that_exist() {
    let agent = agent("prompts-tools").await;
    let tools: BTreeSet<String> = agent
        .client
        .list_all_tools()
        .await
        .unwrap()
        .iter()
        .map(|t| t.name.to_string())
        .collect();
    let args = Map::from_iter([
        ("device".to_string(), json!("a thermometer")),
        ("expectation".to_string(), json!("it logs the value")),
    ]);
    for prompt in agent.client.list_all_prompts().await.unwrap() {
        let mut params = GetPromptRequestParams::new(prompt.name.clone());
        let wanted: Map<String, Value> = args
            .iter()
            .filter(|(k, _)| prompt.arguments.iter().flatten().any(|a| &a.name == *k))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        params = params.with_arguments(wanted);
        let message = text(&agent.client.get_prompt(params).await.unwrap());
        let named: Vec<&str> = message
            .split('`')
            .skip(1)
            .step_by(2)
            .filter(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_lowercase() || c == '_'))
            .collect();
        assert!(!named.is_empty(), "{}", prompt.name);
        for tool in named {
            assert!(tools.contains(tool), "{} names `{tool}`", prompt.name);
        }
    }
    agent.finish().await;
}
