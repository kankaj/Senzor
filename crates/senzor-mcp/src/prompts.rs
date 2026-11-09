//! Prompts: ready-made first messages that set an agent to work with the tools. A client shows
//! them as commands, such as `/mcp__senzor__simulate-device` in Claude Code.

use rmcp::ErrorData;
use rmcp::model::{GetPromptResult, JsonObject, Prompt, PromptArgument, PromptMessage, Role};

struct Spec {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    /// Each required argument, named `{name}` in the text, with its description.
    arguments: &'static [(&'static str, &'static str)],
    text: &'static str,
}

const PROMPTS: [Spec; 3] = [
    Spec {
        name: "simulate-device",
        title: "Simulate a real device",
        description: "Create a template and a device that imitate a real device, and start it.",
        arguments: &[(
            "device",
            "The real device to imitate: model, what it measures, how often it reports, and its topics and payload format if you know them.",
        )],
        text: "Make Senzor imitate this real device: {device}

1. Read the resources senzor://reference/project-file, senzor://reference/generators and senzor://reference/template-variables. Call `list_templates`: if a built-in template comes close, `copy_builtin_template` copies it to change.
2. Create a template with `put_template` whose topics, payload format, fields (with units), interval and availability match the device as closely as you know them. Tell me which details you guessed.
3. Add a device that uses it with `put_device`, fix any problems the result lists, then `start` it.
4. Check with `get_messages` that the topics and payloads look like the real device's. Do not save the project unless I ask.",
    },
    Spec {
        name: "test-consumer",
        title: "Test a consumer",
        description: "Drive values with pins, kill and stop, and check how the program that consumes the messages reacts.",
        arguments: &[(
            "expectation",
            "What the consumer should do in reaction to the values, e.g. \"the fan turns on above 1000 ppm CO2 and an alarm fires when the sensor drops off\".",
        )],
        text: "Test how the program that consumes Senzor's messages reacts. It should: {expectation}

1. Call `get_status` for the instances, and `get_instance` for the fields an instance publishes. If no device fits, ask me before adding one.
2. Start the instances you need with `start`.
3. For each case, force a field with `pin` and confirm with `get_messages` (pass the previous last_seq as after_seq) that the value went out. Then check the consumer's reaction where it shows it, such as its log, its UI, its API or the topics it publishes: `get_messages` only shows what Senzor publishes. To test a device dropping off, `kill` it (the broker publishes its Last Will) or `stop` it (it sends its offline message); `restart` brings it back.
4. When done, `unpin` the fields and report each case: what you sent, what the consumer should have done and what it did.",
    },
    Spec {
        name: "explain-problems",
        title: "Explain and fix problems",
        description: "Read the project's validation problems, explain them and fix them.",
        arguments: &[],
        text: "Call `get_project` and read the validation problems of the Senzor project and the devices they block. For each problem, explain in one sentence what is wrong and where, then fix it: `put_template`, `put_device` and `set_broker` replace one entry, `rename_device`, `rename_template`, `remove_device` and `remove_template` do what they say, and `set_project` replaces the whole TOML (with dry_run it only reports the problems the text would have). The resource senzor://reference/project-file lists every key. Call `get_project` again to confirm that no problems are left, and tell me what you changed. Do not save the project unless I ask.",
    },
];

pub(crate) fn list() -> Vec<Prompt> {
    PROMPTS
        .iter()
        .map(|spec| {
            let arguments = spec
                .arguments
                .iter()
                .map(|(name, description)| {
                    PromptArgument::new(*name)
                        .with_description(*description)
                        .with_required(true)
                })
                .collect();
            Prompt::new(spec.name, Some(spec.description), Some(arguments)).with_title(spec.title)
        })
        .collect()
}

/// The prompt `name` with `arguments` in its text: one user message.
pub(crate) fn get(
    name: &str,
    arguments: Option<&JsonObject>,
) -> Result<GetPromptResult, ErrorData> {
    let spec = PROMPTS
        .iter()
        .find(|spec| spec.name == name)
        .ok_or_else(|| ErrorData::invalid_params(format!("there is no prompt `{name}`"), None))?;
    let mut text = spec.text.to_string();
    for (argument, _) in spec.arguments {
        let value = arguments
            .and_then(|args| args.get(*argument))
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ErrorData::invalid_params(
                    format!("prompt `{name}` needs the argument `{argument}`"),
                    None,
                )
            })?;
        text = text.replace(&format!("{{{argument}}}"), value);
    }
    Ok(
        GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)])
            .with_description(spec.description),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_argument_appears_in_its_text_once() {
        for spec in &PROMPTS {
            for (argument, _) in spec.arguments {
                let placeholder = format!("{{{argument}}}");
                assert_eq!(spec.text.matches(&placeholder).count(), 1, "{}", spec.name);
            }
            assert!(!spec.text.contains('{') || !spec.arguments.is_empty());
        }
    }

    #[test]
    fn a_value_is_inserted_as_written_once() {
        let args = JsonObject::from_iter([("device".into(), "a {device} meter".into())]);
        let result = get("simulate-device", Some(&args)).unwrap();
        let text = serde_json::to_value(&result.messages[0].content).unwrap()["text"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(text.contains("device: a {device} meter\n"), "{text}");
    }
}
