use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use minijinja::value::ValueKind;
use minijinja::{AutoEscape, Environment, UndefinedBehavior};

use crate::project::file::{AvailabilitySpec, MessageSpec};
use crate::value::Value;

/// Variable names that params and fields must not use, because the context builders below set them.
const RESERVED_NAMES: [&str; 7] = [
    "device_id",
    "index",
    "id",
    "template_id",
    "now_ms",
    "now_iso",
    "seq",
];

/// Whether `name` is one of the variable names that params and fields must not use.
pub fn is_reserved(name: &str) -> bool {
    RESERVED_NAMES.contains(&name)
}

pub type Context = BTreeMap<String, minijinja::Value>;

const AVAILABILITY_TOPIC: &str = "availability.topic";

/// A rendered message, ready to publish.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedMessage {
    pub topic: String,
    pub payload: String,
}

/// The compiled topic/payload templates of one template.
pub struct InstanceTemplates {
    env: Environment<'static>,
    message_count: usize,
    has_availability: bool,
}

fn strict_environment() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_auto_escape_callback(|_| AutoEscape::None);
    // Payloads are mostly JSON: render booleans as `true`/`false`, not Python-style `True`.
    env.set_formatter(|out, state, value| {
        if value.kind() == ValueKind::Bool {
            out.write_str(if value.is_true() { "true" } else { "false" })?;
            Ok(())
        } else {
            minijinja::escape_formatter(out, state, value)
        }
    });
    env
}

fn topic_name(index: usize) -> String {
    format!("messages[{}].topic", index + 1)
}

fn payload_name(index: usize) -> String {
    format!("messages[{}].payload", index + 1)
}

impl InstanceTemplates {
    /// Compiles all templates; the error names the first broken one.
    pub fn compile(
        messages: &[MessageSpec],
        availability: Option<&AvailabilitySpec>,
    ) -> Result<Self, String> {
        let mut env = strict_environment();
        for (i, message) in messages.iter().enumerate() {
            env.add_template_owned(topic_name(i), message.topic.clone())
                .map_err(|e| e.to_string())?;
            env.add_template_owned(payload_name(i), message.payload.clone())
                .map_err(|e| e.to_string())?;
        }
        if let Some(availability) = availability {
            env.add_template_owned(AVAILABILITY_TOPIC, availability.topic.clone())
                .map_err(|e| e.to_string())?;
        }
        Ok(Self {
            env,
            message_count: messages.len(),
            has_availability: availability.is_some(),
        })
    }

    /// Renders the availability topic, or `None` when the template has no availability.
    pub fn availability_topic(&self, ctx: &Context) -> Result<Option<String>, String> {
        if !self.has_availability {
            return Ok(None);
        }
        let topic = render_named(&self.env, AVAILABILITY_TOPIC, ctx)?;
        check_topic(&topic).map_err(|e| format!("availability topic: {e}"))?;
        Ok(Some(topic))
    }

    /// Renders every message. A failure affects only that message.
    pub fn render_messages(&self, ctx: &Context) -> Vec<Result<RenderedMessage, String>> {
        (0..self.message_count)
            .map(|i| {
                let topic = render_named(&self.env, &topic_name(i), ctx)?;
                check_topic(&topic).map_err(|e| format!("message {} topic: {e}", i + 1))?;
                let payload = render_named(&self.env, &payload_name(i), ctx)?;
                Ok(RenderedMessage { topic, payload })
            })
            .collect()
    }
}

fn render_named(env: &Environment<'static>, name: &str, ctx: &Context) -> Result<String, String> {
    let template = env.get_template(name).map_err(|e| e.to_string())?;
    template
        .render(minijinja::Value::from(ctx.clone()))
        .map_err(|e| e.to_string())
}

/// The name minijinja uses for string templates; kept so `instance_id` errors read
/// `in <string>:1`.
const STRING_TEMPLATE: &str = "<string>";

/// A template string compiled once and rendered many times, such as `instance_id` for every
/// index of a device. Building the environment and parsing the source for each render made
/// validating a project with thousands of instances slow.
pub struct CompiledTemplate {
    env: Environment<'static>,
}

impl CompiledTemplate {
    /// Parses `source`; a syntax error is reported here rather than on the first render.
    pub fn compile(source: &str) -> Result<Self, String> {
        let mut env = strict_environment();
        env.add_template_owned(STRING_TEMPLATE, source.to_string())
            .map_err(|e| e.to_string())?;
        Ok(Self { env })
    }

    pub fn render(&self, ctx: &Context) -> Result<String, String> {
        render_named(&self.env, STRING_TEMPLATE, ctx)
    }
}

fn params_context(params: &BTreeMap<String, Value>) -> Context {
    params
        .iter()
        .map(|(k, v)| (k.clone(), v.to_jinja()))
        .collect()
}

/// Variables available to `instance_id`, except `index`, which changes per instance.
pub fn instance_id_context(device_id: &str, params: &BTreeMap<String, Value>) -> Context {
    let mut ctx = params_context(params);
    ctx.insert("id".into(), device_id.into());
    ctx
}

/// Variables available to the availability topic.
pub fn base_context(
    device_id: &str,
    index: u32,
    template_id: &str,
    params: &BTreeMap<String, Value>,
) -> Context {
    let mut ctx = params_context(params);
    ctx.insert("device_id".into(), device_id.into());
    ctx.insert("index".into(), index.into());
    ctx.insert("template_id".into(), template_id.into());
    ctx
}

/// Variables available to message topics and payloads: base + fields + time + seq.
pub fn tick_context(
    base: &Context,
    values: &BTreeMap<String, Value>,
    now: SystemTime,
    seq: u64,
) -> Context {
    let mut ctx = base.clone();
    for (name, value) in values {
        ctx.insert(name.clone(), value.to_jinja());
    }
    let millis = now
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    ctx.insert("now_ms".into(), millis.into());
    ctx.insert(
        "now_iso".into(),
        humantime::format_rfc3339_millis(now).to_string().into(),
    );
    ctx.insert("seq".into(), seq.into());
    ctx
}

/// A publish topic must be non-empty and contain no wildcards.
fn check_topic(topic: &str) -> Result<(), String> {
    if topic.is_empty() {
        Err("topic is empty".into())
    } else if topic.contains(['+', '#']) {
        Err(format!("topic `{topic}` contains a wildcard (+ or #)"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::time::Duration;

    fn message(topic: &str, payload: &str) -> MessageSpec {
        MessageSpec {
            topic: topic.into(),
            payload: payload.into(),
        }
    }

    fn params() -> BTreeMap<String, Value> {
        BTreeMap::from([("room".to_string(), Value::String("office".into()))])
    }

    fn values() -> BTreeMap<String, Value> {
        BTreeMap::from([
            ("co2".to_string(), Value::Number(1184.6)),
            ("temp".to_string(), Value::Number(22.6789)),
        ])
    }

    fn now() -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(1_700_000_000_123)
    }

    /// The message context of instance `d` (no params) with these field `values`.
    fn tick_ctx(values: &BTreeMap<String, Value>) -> Context {
        tick_context(
            &base_context("d", 0, "t", &BTreeMap::new()),
            values,
            now(),
            0,
        )
    }

    #[test]
    fn renders_topic_and_payload_with_filters() {
        let templates = InstanceTemplates::compile(
            &[message(
                "b/{{ room }}/{{ device_id }}/state",
                r#"{"co2": {{ co2 | round | int }}, "temp": {{ temp | round(1) }}, "ts": {{ now_ms }}, "seq": {{ seq }}}"#,
            )],
            None,
        )
        .unwrap();
        let base = base_context("office-3", 3, "office-air", &params());
        let ctx = tick_context(&base, &values(), now(), 7);
        let rendered = templates.render_messages(&ctx);
        assert_eq!(
            rendered,
            vec![Ok(RenderedMessage {
                topic: "b/office/office-3/state".into(),
                payload: r#"{"co2": 1185, "temp": 22.7, "ts": 1700000000123, "seq": 7}"#.into(),
            })]
        );
    }

    #[test]
    fn booleans_render_lowercase_for_json() {
        let templates =
            InstanceTemplates::compile(&[message("t", r#"{"open": {{ open }}}"#)], None).unwrap();
        let values = BTreeMap::from([("open".to_string(), Value::Bool(true))]);
        let ctx = tick_ctx(&values);
        assert_eq!(
            templates.render_messages(&ctx).remove(0).unwrap().payload,
            r#"{"open": true}"#
        );
    }

    #[test]
    fn now_iso_is_rfc3339_with_millis() {
        let templates = InstanceTemplates::compile(&[message("t", "{{ now_iso }}")], None).unwrap();
        let ctx = tick_ctx(&BTreeMap::new());
        let rendered = templates.render_messages(&ctx).remove(0).unwrap();
        assert_eq!(rendered.payload, "2023-11-14T22:13:20.123Z");
    }

    #[test]
    fn tojson_escapes_strings() {
        let templates =
            InstanceTemplates::compile(&[message("t", "{{ room | tojson }}")], None).unwrap();
        let mut p = BTreeMap::new();
        p.insert("room".to_string(), Value::String("a\"b".into()));
        let ctx = tick_context(&base_context("d", 0, "t", &p), &BTreeMap::new(), now(), 0);
        assert_eq!(
            templates.render_messages(&ctx).remove(0).unwrap().payload,
            r#""a\"b""#
        );
    }

    #[test]
    fn undefined_variable_is_an_error() {
        let templates = InstanceTemplates::compile(&[message("t", "{{ c02 }}")], None).unwrap();
        let ctx = tick_ctx(&values());
        let err = templates.render_messages(&ctx).remove(0).unwrap_err();
        assert!(err.contains("c02"), "{err}");
    }

    #[test]
    fn syntax_error_fails_compile_and_names_the_template() {
        let err = InstanceTemplates::compile(&[message("t", "{{ x ")], None)
            .err()
            .unwrap();
        assert!(err.contains("messages[1].payload"), "{err}");
    }

    #[test]
    fn one_broken_message_does_not_affect_the_others() {
        let templates =
            InstanceTemplates::compile(&[message("a", "{{ nope }}"), message("b", "ok")], None)
                .unwrap();
        let ctx = tick_ctx(&BTreeMap::new());
        let rendered = templates.render_messages(&ctx);
        assert!(rendered[0].is_err());
        assert_eq!(
            rendered[1],
            Ok(RenderedMessage {
                topic: "b".into(),
                payload: "ok".into()
            })
        );
    }

    #[test]
    fn availability_topic_uses_base_context() {
        let availability = AvailabilitySpec {
            topic: "b/{{ room }}/{{ device_id }}/status".into(),
            online: "online".into(),
            offline: "offline".into(),
        };
        let templates =
            InstanceTemplates::compile(&[message("t", "p")], Some(&availability)).unwrap();
        let base = base_context("office-0", 0, "office-air", &params());
        assert_eq!(
            templates.availability_topic(&base),
            Ok(Some("b/office/office-0/status".into()))
        );
    }

    #[test]
    fn availability_topic_cannot_use_field_values() {
        let availability = AvailabilitySpec {
            topic: "b/{{ co2 }}".into(),
            online: "online".into(),
            offline: "offline".into(),
        };
        let templates =
            InstanceTemplates::compile(&[message("t", "p")], Some(&availability)).unwrap();
        let base = base_context("d", 0, "t", &BTreeMap::new());
        assert!(templates.availability_topic(&base).is_err());
    }

    #[test]
    fn wildcard_and_empty_topics_are_rejected() {
        assert!(check_topic("a/+/b").is_err());
        assert!(check_topic("a/#").is_err());
        assert!(check_topic("").is_err());
        assert!(check_topic("a/b").is_ok());
    }

    fn id_context() -> Context {
        let mut ctx = Context::new();
        ctx.insert("id".into(), "office".into());
        ctx.insert("index".into(), 2.into());
        ctx.insert("room".into(), "a&b".into());
        ctx.insert("spare".into(), true.into());
        ctx
    }

    #[test]
    fn compiled_template_renders_the_context() {
        let template =
            CompiledTemplate::compile("{{ room }}/{{ id }}-{{ index }}-{{ spare }}").unwrap();
        assert_eq!(
            template.render(&id_context()),
            Ok("a&b/office-2-true".into())
        );
    }

    #[test]
    fn compiled_template_renders_again_with_a_new_context() {
        let template = CompiledTemplate::compile("{{ id }}-{{ index }}").unwrap();
        let mut ctx = id_context();
        assert_eq!(template.render(&ctx), Ok("office-2".into()));
        ctx.insert("index".into(), 3.into());
        assert_eq!(template.render(&ctx), Ok("office-3".into()));
    }

    #[test]
    fn compiled_template_rejects_unknown_variables() {
        let template = CompiledTemplate::compile("{{ idx }}").unwrap();
        assert_eq!(
            template.render(&id_context()),
            Err("undefined value: `idx` is undefined (in <string>:1)".into())
        );
    }

    #[test]
    fn compiled_template_reports_syntax_errors() {
        let err = CompiledTemplate::compile("{{ id ").err().unwrap();
        assert_eq!(
            err,
            "syntax error: unexpected end of input, expected end of variable block (in <string>:1)"
        );
    }

    #[test]
    fn reserved_names_cover_every_builtin_variable() {
        let message_ctx = tick_ctx(&BTreeMap::new());
        let id_ctx = instance_id_context("d", &BTreeMap::new());
        let builtins: BTreeSet<&str> = message_ctx
            .keys()
            .chain(id_ctx.keys())
            .map(String::as_str)
            .collect();
        assert_eq!(builtins, BTreeSet::from(RESERVED_NAMES));
    }
}
