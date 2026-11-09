use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use minijinja::value::{Rest, StringInput, ValueKind, ValueOrKwargs};
use minijinja::{AutoEscape, Environment, ErrorKind, UndefinedBehavior};

use crate::encoding::{Encoder, Encoding, message_encoder_located, to_hex};
use crate::project::file::{AvailabilitySpec, MessageSpec};
use crate::project::mqtt_seconds;
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

/// A compile or render problem with the TOML key path of the template that caused it, such as
/// `messages[1].payload`; validation passes the path on as the problem's field.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TemplateError {
    pub field: String,
    pub message: String,
}

impl TemplateError {
    pub(crate) fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

/// A rendered message, ready to publish.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedMessage {
    pub topic: String,
    pub payload: Payload,
    pub properties: MessageProperties,
}

/// A message's payload: the rendered text, or for a message with a binary encoding, the bytes
/// the text was encoded to.
#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    /// Published as the text's UTF-8 bytes.
    Text(String),
    Binary {
        encoding: Encoding,
        /// What is published.
        bytes: Vec<u8>,
        /// The rendered text the bytes were encoded from.
        text: String,
    },
}

impl Payload {
    /// The bytes that are published.
    pub fn bytes(&self) -> &[u8] {
        match self {
            Payload::Text(text) => text.as_bytes(),
            Payload::Binary { bytes, .. } => bytes,
        }
    }

    /// The rendered text: the payload itself, or what its bytes were encoded from.
    pub fn text(&self) -> &str {
        match self {
            Payload::Text(text) | Payload::Binary { text, .. } => text,
        }
    }

    pub fn encoding(&self) -> Encoding {
        match self {
            Payload::Text(_) => Encoding::Text,
            Payload::Binary { encoding, .. } => *encoding,
        }
    }

    /// The text, for a text payload.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Payload::Text(text) => Some(text),
            Payload::Binary { .. } => None,
        }
    }

    /// The published bytes as lowercase hex, without spaces.
    pub fn hex(&self) -> String {
        to_hex(self.bytes())
    }
}

impl From<String> for Payload {
    fn from(text: String) -> Self {
        Payload::Text(text)
    }
}

impl From<&str> for Payload {
    fn from(text: &str) -> Self {
        Payload::Text(text.to_string())
    }
}

/// A text payload equals its text; a binary one equals no text.
impl PartialEq<str> for Payload {
    fn eq(&self, other: &str) -> bool {
        self.as_text() == Some(other)
    }
}

impl PartialEq<&str> for Payload {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}

/// The MQTT 5 properties a message is published with. Empty for messages that set none, and
/// always empty on MQTT 3.1.1 (validation rejects properties there).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MessageProperties {
    /// Message expiry interval in seconds.
    pub message_expiry: Option<u32>,
    pub content_type: Option<String>,
    /// Rendered user properties, sorted by key.
    pub user_properties: Vec<(String, String)>,
}

impl MessageProperties {
    pub fn is_empty(&self) -> bool {
        self.message_expiry.is_none()
            && self.content_type.is_none()
            && self.user_properties.is_empty()
    }

    /// Bytes these properties add to a PUBLISH packet, without the length of the property list
    /// itself: an identifier byte per property, plus two length bytes per string.
    pub fn encoded_len(&self) -> usize {
        let expiry = self.message_expiry.map_or(0, |_| 1 + 4);
        let content_type = self.content_type.as_ref().map_or(0, |c| 1 + 2 + c.len());
        let user: usize = self
            .user_properties
            .iter()
            .map(|(name, value)| 1 + 2 + name.len() + 2 + value.len())
            .sum();
        expiry + content_type + user
    }
}

/// One message's template names in the environment, and the properties that are not templates.
struct CompiledMessage {
    topic: String,
    payload: String,
    message_expiry: Option<u32>,
    content_type: Option<String>,
    /// Each user property's name and the name of its value's template.
    user_properties: Vec<(String, String)>,
}

/// The compiled topic/payload templates of one template, and how each message's payload is
/// encoded.
pub struct InstanceTemplates {
    env: Environment<'static>,
    messages: Vec<CompiledMessage>,
    /// One per message.
    encoders: Vec<Encoder>,
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
    // The same for the filters that turn values into text; `~` cannot be changed this way.
    env.add_filter("string", string_filter);
    env.add_filter("join", join_filter);
    env.add_filter("format", format_filter);
    add_byte_filters(&mut env);
    env
}

/// `value`, or `true`/`false` as text when it is a boolean.
fn lowercase_bool(value: minijinja::Value) -> minijinja::Value {
    if value.kind() == ValueKind::Bool {
        minijinja::Value::from(if value.is_true() { "true" } else { "false" })
    } else {
        value
    }
}

fn string_filter(
    state: &minijinja::State,
    value: &minijinja::Value,
) -> Result<minijinja::Value, minijinja::Error> {
    minijinja::filters::string(state, &lowercase_bool(value.clone()))
}

fn join_filter(
    state: &mut minijinja::State,
    value: &minijinja::Value,
    joiner: Option<StringInput<'_>>,
) -> Result<minijinja::Value, minijinja::Error> {
    let value = match (value.kind(), value.try_iter()) {
        (ValueKind::Seq, Ok(items)) => {
            minijinja::Value::from(items.map(lowercase_bool).collect::<Vec<_>>())
        }
        _ => value.clone(),
    };
    minijinja::filters::join(state, &value, joiner)
}

fn format_filter(
    state: &mut minijinja::State,
    format: &minijinja::Value,
    Rest(args): Rest<ValueOrKwargs>,
) -> Result<minijinja::Value, minijinja::Error> {
    let args = args
        .into_iter()
        .map(|arg| ValueOrKwargs::from(lowercase_bool(arg.into_value())))
        .collect();
    minijinja::filters::format(state, format, Rest(args))
}

/// Adds the byte filters, which write a number as the hex digits of a binary integer or float,
/// for building binary payloads with `encoding = "hex"`.
fn add_byte_filters(env: &mut Environment<'static>) {
    macro_rules! integer {
        ($name:literal, $type:ty, $endian:ident) => {
            env.add_filter($name, |value: minijinja::Value| {
                let n = integer($name, &value, <$type>::MIN.into(), <$type>::MAX.into())?;
                Ok::<_, minijinja::Error>(to_hex(&(n as $type).$endian()))
            });
        };
    }
    integer!("u8", u8, to_be_bytes);
    integer!("i8", i8, to_be_bytes);
    integer!("u16be", u16, to_be_bytes);
    integer!("u16le", u16, to_le_bytes);
    integer!("i16be", i16, to_be_bytes);
    integer!("i16le", i16, to_le_bytes);
    integer!("u32be", u32, to_be_bytes);
    integer!("u32le", u32, to_le_bytes);
    integer!("i32be", i32, to_be_bytes);
    integer!("i32le", i32, to_le_bytes);
    macro_rules! float {
        ($name:literal, $type:ty, $endian:ident) => {
            env.add_filter($name, |value: minijinja::Value| {
                let x = number($name, &value)?;
                let narrowed = x as $type;
                if x.is_finite() && !narrowed.is_finite() {
                    return Err(filter_error(
                        $name,
                        format!(
                            "{:e} is out of range for a {}-bit float",
                            x,
                            8 * std::mem::size_of::<$type>()
                        ),
                    ));
                }
                Ok(to_hex(&narrowed.$endian()))
            });
        };
    }
    float!("f32be", f32, to_be_bytes);
    float!("f32le", f32, to_le_bytes);
    float!("f64be", f64, to_be_bytes);
    float!("f64le", f64, to_le_bytes);
}

fn filter_error(filter: &str, message: String) -> minijinja::Error {
    minijinja::Error::new(ErrorKind::InvalidOperation, format!("{filter}: {message}"))
}

/// `value` as a number, for the byte filter `filter`.
fn number(filter: &str, value: &minijinja::Value) -> Result<f64, minijinja::Error> {
    if value.kind() != ValueKind::Number {
        return Err(filter_error(
            filter,
            format!("expected a number, got {}", value.kind()),
        ));
    }
    f64::try_from(value.clone()).map_err(|e| filter_error(filter, e.to_string()))
}

/// `value` rounded to the nearest integer, which must lie in `min..=max`.
fn integer(
    filter: &str,
    value: &minijinja::Value,
    min: i64,
    max: i64,
) -> Result<i64, minijinja::Error> {
    let rounded = number(filter, value)?.round();
    if rounded.is_nan() || rounded < min as f64 || rounded > max as f64 {
        return Err(filter_error(
            filter,
            format!("{value} is out of range ({min} to {max})"),
        ));
    }
    Ok(rounded as i64)
}

fn topic_name(index: usize) -> String {
    format!("messages[{}].topic", index + 1)
}

fn payload_name(index: usize) -> String {
    format!("messages[{}].payload", index + 1)
}

fn user_property_name(index: usize, key: &str) -> String {
    format!("messages[{}].user_properties.{key}", index + 1)
}

/// The key path of message `index`'s user properties as a whole.
fn user_properties_field(index: usize) -> String {
    format!("messages[{}].user_properties", index + 1)
}

/// Adds the template `name` with `source` to `env`; a syntax error names `field`.
fn add_template(
    env: &mut Environment<'static>,
    name: &str,
    source: &str,
    field: impl FnOnce() -> String,
) -> Result<(), TemplateError> {
    env.add_template_owned(name.to_string(), source.to_string())
        .map_err(|e| TemplateError::new(field(), e.to_string()))
}

impl InstanceTemplates {
    /// Compiles all templates; the error names the first broken one. Each message's payload is
    /// encoded as its `encoding` says, with `.proto` files relative to the working directory:
    /// validation compiles with [`Self::compile_with`] and the project's folder instead.
    pub fn compile(
        messages: &[MessageSpec],
        availability: Option<&AvailabilitySpec>,
    ) -> Result<Self, String> {
        Self::compile_located(messages, availability).map_err(|e| e.message)
    }

    /// [`Self::compile`], with the key path of the broken template.
    pub(crate) fn compile_located(
        messages: &[MessageSpec],
        availability: Option<&AvailabilitySpec>,
    ) -> Result<Self, TemplateError> {
        let encoders = messages
            .iter()
            .enumerate()
            .map(|(i, message)| {
                message_encoder_located(message, i, Path::new("."))
                    .map_err(|mut problems| problems.remove(0))
            })
            .collect::<Result<_, _>>()?;
        Self::compile_with_located(messages, availability, encoders)
    }

    /// Compiles all templates, with one encoder per message.
    pub fn compile_with(
        messages: &[MessageSpec],
        availability: Option<&AvailabilitySpec>,
        encoders: Vec<Encoder>,
    ) -> Result<Self, String> {
        Self::compile_with_located(messages, availability, encoders).map_err(|e| e.message)
    }

    /// [`Self::compile_with`], with the key path of the broken template.
    pub(crate) fn compile_with_located(
        messages: &[MessageSpec],
        availability: Option<&AvailabilitySpec>,
        encoders: Vec<Encoder>,
    ) -> Result<Self, TemplateError> {
        assert_eq!(encoders.len(), messages.len(), "one encoder per message");
        #[cfg(test)]
        compile_count::record(messages);
        let mut env = strict_environment();
        let mut compiled = Vec::with_capacity(messages.len());
        for (i, message) in messages.iter().enumerate() {
            let topic = topic_name(i);
            add_template(&mut env, &topic, &message.topic, || topic.clone())?;
            let payload = payload_name(i);
            add_template(&mut env, &payload, &message.payload, || payload.clone())?;
            let mut user_properties = Vec::with_capacity(message.user_properties.len());
            for (key, value) in &message.user_properties {
                let name = user_property_name(i, key);
                add_template(&mut env, &name, value, || user_properties_field(i))?;
                user_properties.push((key.clone(), name));
            }
            compiled.push(CompiledMessage {
                topic,
                payload,
                message_expiry: message.message_expiry.map(mqtt_seconds),
                content_type: message.content_type.clone(),
                user_properties,
            });
        }
        if let Some(availability) = availability {
            add_template(&mut env, AVAILABILITY_TOPIC, &availability.topic, || {
                AVAILABILITY_TOPIC.to_string()
            })?;
        }
        Ok(Self {
            env,
            messages: compiled,
            encoders,
            has_availability: availability.is_some(),
        })
    }

    /// How each message's payload is encoded, in message order.
    pub fn encoders(&self) -> &[Encoder] {
        &self.encoders
    }

    /// Renders the availability topic, or `None` when the template has no availability.
    pub fn availability_topic(&self, ctx: &Context) -> Result<Option<String>, String> {
        self.availability_topic_located(ctx).map_err(|e| e.message)
    }

    /// [`Self::availability_topic`], with the key path of the template that failed.
    pub(crate) fn availability_topic_located(
        &self,
        ctx: &Context,
    ) -> Result<Option<String>, TemplateError> {
        if !self.has_availability {
            return Ok(None);
        }
        let failed = |message| TemplateError::new(AVAILABILITY_TOPIC, message);
        let topic = render_named(&self.env, AVAILABILITY_TOPIC, ctx).map_err(failed)?;
        check_topic(&topic).map_err(|e| failed(format!("availability topic: {e}")))?;
        Ok(Some(topic))
    }

    /// Renders every message. A failure affects only that message.
    pub fn render_messages(&self, ctx: &Context) -> Vec<Result<RenderedMessage, String>> {
        self.render_messages_located(ctx)
            .map(|rendered| rendered.map_err(|e| e.message))
            .collect()
    }

    /// [`Self::render_messages`], with the key path of the template that failed.
    pub(crate) fn render_messages_located(
        &self,
        ctx: &Context,
    ) -> impl Iterator<Item = Result<RenderedMessage, TemplateError>> {
        self.messages
            .iter()
            .enumerate()
            .map(|(i, message)| self.render_message(i, message, ctx))
    }

    /// Renders the topic of every message, without its payload. A failure affects only that topic.
    pub fn render_topics(&self, ctx: &Context) -> Vec<Result<String, String>> {
        self.messages
            .iter()
            .enumerate()
            .map(|(i, message)| self.render_topic(i, message, ctx).map_err(|e| e.message))
            .collect()
    }

    fn render_topic(
        &self,
        i: usize,
        message: &CompiledMessage,
        ctx: &Context,
    ) -> Result<String, TemplateError> {
        let topic = render_named(&self.env, &message.topic, ctx)
            .map_err(|e| TemplateError::new(&message.topic, e))?;
        check_topic(&topic).map_err(|e| {
            TemplateError::new(&message.topic, format!("message {} topic: {e}", i + 1))
        })?;
        Ok(topic)
    }

    fn render_message(
        &self,
        i: usize,
        message: &CompiledMessage,
        ctx: &Context,
    ) -> Result<RenderedMessage, TemplateError> {
        let topic = self.render_topic(i, message, ctx)?;
        let text = render_named(&self.env, &message.payload, ctx)
            .map_err(|e| TemplateError::new(&message.payload, e))?;
        let payload = match &self.encoders[i] {
            Encoder::Text => Payload::Text(text),
            encoder => {
                let encoding = encoder.encoding();
                let bytes = encoder.encode(&text).map_err(|e| {
                    TemplateError::new(
                        &message.payload,
                        format!("message {} payload as {}: {e}", i + 1, encoding.name()),
                    )
                })?;
                Payload::Binary {
                    encoding,
                    bytes,
                    text,
                }
            }
        };
        let user_properties = message
            .user_properties
            .iter()
            .map(|(key, name)| {
                let value = render_named(&self.env, name, ctx).map_err(|e| {
                    TemplateError::new(user_properties_field(i), format!("{name}: {e}"))
                })?;
                Ok((key.clone(), value))
            })
            .collect::<Result<_, TemplateError>>()?;
        Ok(RenderedMessage {
            topic,
            payload,
            properties: MessageProperties {
                message_expiry: message.message_expiry,
                content_type: message.content_type.clone(),
                user_properties,
            },
        })
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
    /// What errors call the template: `<string>`, or the key path it came from.
    name: String,
}

impl CompiledTemplate {
    /// Parses `source`; a syntax error is reported here rather than on the first render.
    pub fn compile(source: &str) -> Result<Self, String> {
        Self::compile_named(STRING_TEMPLATE, source)
    }

    /// [`Self::compile`] for a template whose errors name it `name`, such as
    /// `subscriptions[1].topic`.
    pub fn compile_named(name: &str, source: &str) -> Result<Self, String> {
        let mut env = strict_environment();
        env.add_template_owned(name.to_string(), source.to_string())
            .map_err(|e| e.to_string())?;
        Ok(Self {
            env,
            name: name.to_string(),
        })
    }

    pub fn render(&self, ctx: &Context) -> Result<String, String> {
        render_named(&self.env, &self.name, ctx)
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

/// Counts [`InstanceTemplates::compile`] calls by the first message's topic, so a test that gives
/// its template a topic of its own sees only its compiles, whatever runs beside it.
#[cfg(test)]
pub(crate) mod compile_count {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};

    use crate::project::file::MessageSpec;

    fn counts() -> &'static Mutex<HashMap<String, usize>> {
        static COUNTS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
        COUNTS.get_or_init(Mutex::default)
    }

    pub(crate) fn record(messages: &[MessageSpec]) {
        if let Some(first) = messages.first() {
            *counts()
                .lock()
                .unwrap()
                .entry(first.topic.clone())
                .or_default() += 1;
        }
    }

    /// How often templates whose first message has `topic` were compiled.
    pub(crate) fn of(topic: &str) -> usize {
        counts().lock().unwrap().get(topic).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::time::Duration;

    fn message(topic: &str, payload: &str) -> MessageSpec {
        MessageSpec::new(topic, payload)
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
                properties: MessageProperties::default(),
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
    fn booleans_render_lowercase_through_filters_and_concatenation() {
        let values = BTreeMap::from([("open".to_string(), Value::Bool(true))]);
        let ctx = tick_ctx(&values);
        for (source, expected) in [
            ("{{ open | string }}", "true"),
            ("{{ 'door ' ~ (open | string) }}", "door true"),
            ("{{ '%s' | format(open) }}", "true"),
            ("{{ open | tojson }}", "true"),
            ("{{ [open, false] | join(',') }}", "true,false"),
        ] {
            let templates = InstanceTemplates::compile(&[message("t", source)], None).unwrap();
            assert_eq!(
                templates.render_messages(&ctx).remove(0).unwrap().payload,
                expected,
                "{source}"
            );
        }
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
                payload: "ok".into(),
                properties: MessageProperties::default(),
            })
        );
    }

    #[test]
    fn availability_topic_uses_base_context() {
        let availability = AvailabilitySpec {
            topic: "b/{{ room }}/{{ device_id }}/status".into(),
            online: "online".into(),
            offline: "offline".into(),
            will_delay: None,
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
            will_delay: None,
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

    #[test]
    fn properties_are_rendered_with_the_message_context() {
        let mut spec = message("t", "p");
        spec.message_expiry = Some(Duration::from_secs(60));
        spec.content_type = Some("application/json".into());
        spec.user_properties = BTreeMap::from([
            ("seq".to_string(), "{{ seq }}".to_string()),
            ("room".to_string(), "{{ room }}-{{ device_id }}".to_string()),
        ]);
        let templates = InstanceTemplates::compile(&[spec], None).unwrap();
        let ctx = tick_context(
            &base_context("office-0", 0, "t", &params()),
            &values(),
            now(),
            2,
        );
        let rendered = templates.render_messages(&ctx).remove(0).unwrap();
        assert_eq!(
            rendered.properties,
            MessageProperties {
                message_expiry: Some(60),
                content_type: Some("application/json".into()),
                user_properties: vec![
                    ("room".into(), "office-office-0".into()),
                    ("seq".into(), "2".into()),
                ],
            }
        );
    }

    #[test]
    fn encoded_len_counts_identifiers_and_string_lengths() {
        let properties = MessageProperties {
            message_expiry: Some(60),
            content_type: Some("text".into()),
            user_properties: vec![("ab".into(), "cde".into())],
        };
        assert_eq!(properties.encoded_len(), 5 + (3 + 4) + (5 + 2 + 3));
        assert_eq!(MessageProperties::default().encoded_len(), 0);
    }

    #[test]
    fn a_message_without_properties_renders_none() {
        let templates = InstanceTemplates::compile(&[message("t", "p")], None).unwrap();
        let rendered = templates
            .render_messages(&tick_ctx(&BTreeMap::new()))
            .remove(0);
        assert!(rendered.unwrap().properties.is_empty());
    }

    #[test]
    fn a_user_property_that_does_not_render_names_the_property() {
        let mut spec = message("t", "p");
        spec.user_properties = BTreeMap::from([("room".to_string(), "{{ rom }}".to_string())]);
        let templates = InstanceTemplates::compile(&[spec], None).unwrap();
        let err = templates
            .render_messages(&tick_ctx(&BTreeMap::new()))
            .remove(0)
            .unwrap_err();
        assert!(
            err.starts_with("messages[1].user_properties.room: "),
            "{err}"
        );
        assert!(err.contains("rom"), "{err}");
    }

    #[test]
    fn a_user_property_syntax_error_fails_compile_and_names_it() {
        let mut spec = message("t", "p");
        spec.user_properties = BTreeMap::from([("room".to_string(), "{{ x ".to_string())]);
        let err = InstanceTemplates::compile(&[spec], None).err().unwrap();
        assert!(err.contains("messages[1].user_properties.room"), "{err}");
    }

    /// What `source` renders to in a payload, with no variables.
    fn render_payload(source: &str) -> Result<String, String> {
        let templates = InstanceTemplates::compile(&[message("t", source)], None)?;
        templates
            .render_messages(&tick_ctx(&BTreeMap::new()))
            .remove(0)
            .map(|m| m.payload.text().to_string())
    }

    #[test]
    fn byte_filters_write_big_and_little_endian_hex() {
        for (source, expected) in [
            ("{{ 215 | i16be }}", "00d7"),
            ("{{ 215 | i16le }}", "d700"),
            ("{{ -2 | i16be }}", "fffe"),
            ("{{ 300 | u16le }}", "2c01"),
            ("{{ 255 | u8 }}", "ff"),
            ("{{ -128 | i8 }}", "80"),
            ("{{ 127 | i8 }}", "7f"),
            ("{{ 4294967295 | u32be }}", "ffffffff"),
            ("{{ 1 | u32le }}", "01000000"),
            ("{{ -2147483648 | i32le }}", "00000080"),
            ("{{ 1 | i32be }}", "00000001"),
            ("{{ 1.5 | f32be }}", "3fc00000"),
            ("{{ 1.5 | f32le }}", "0000c03f"),
            ("{{ 1.5 | f64be }}", "3ff8000000000000"),
            ("{{ 1.5 | f64le }}", "000000000000f83f"),
            ("{{ 3 | f32be }}", "40400000"),
        ] {
            assert_eq!(render_payload(source), Ok(expected.into()), "{source}");
        }
    }

    #[test]
    fn integer_byte_filters_round_to_the_nearest_integer() {
        for (source, expected) in [
            ("{{ 21.4 | u8 }}", "15"),
            ("{{ 21.5 | u8 }}", "16"),
            ("{{ 21.6 | u16be }}", "0016"),
            ("{{ -0.4 | u8 }}", "00"),
            ("{{ -1.5 | i8 }}", "fe"),
            ("{{ 255.4 | u8 }}", "ff"),
        ] {
            assert_eq!(render_payload(source), Ok(expected.into()), "{source}");
        }
    }

    #[test]
    fn each_byte_filter_writes_its_width() {
        for (filter, digits) in [
            ("u8", 2),
            ("i8", 2),
            ("u16be", 4),
            ("u16le", 4),
            ("i16be", 4),
            ("i16le", 4),
            ("u32be", 8),
            ("u32le", 8),
            ("i32be", 8),
            ("i32le", 8),
            ("f32be", 8),
            ("f32le", 8),
            ("f64be", 16),
            ("f64le", 16),
        ] {
            let rendered = render_payload(&format!("{{{{ 0 | {filter} }}}}")).unwrap();
            assert_eq!(rendered, "0".repeat(digits), "{filter}");
        }
    }

    #[test]
    fn a_value_out_of_range_for_a_byte_filter_is_an_error() {
        for (source, expected) in [
            ("{{ 256 | u8 }}", "u8: 256 is out of range (0 to 255)"),
            ("{{ 255.5 | u8 }}", "u8: 255.5 is out of range (0 to 255)"),
            ("{{ -1 | u16be }}", "u16be: -1 is out of range (0 to 65535)"),
            ("{{ 128 | i8 }}", "i8: 128 is out of range (-128 to 127)"),
            (
                "{{ 2147483648 | i32le }}",
                "i32le: 2147483648 is out of range (-2147483648 to 2147483647)",
            ),
            (
                "{{ 1e39 | f32be }}",
                "f32be: 1e39 is out of range for a 32-bit float",
            ),
            ("{{ 'a' | u8 }}", "u8: expected a number, got string"),
            ("{{ true | i16be }}", "i16be: expected a number, got bool"),
        ] {
            let error = render_payload(source).unwrap_err();
            assert!(error.contains(expected), "{source}: {error}");
        }
    }

    fn encoded(source: &str, encoding: Encoding) -> MessageSpec {
        MessageSpec {
            encoding,
            ..message("t", source)
        }
    }

    #[test]
    fn a_binary_message_keeps_its_text_and_publishes_the_bytes() {
        let templates = InstanceTemplates::compile(
            &[encoded("{{ 215 | i16be }} {{ 1 | u8 }}", Encoding::Hex)],
            None,
        )
        .unwrap();
        let rendered = templates
            .render_messages(&tick_ctx(&BTreeMap::new()))
            .remove(0)
            .unwrap();
        assert_eq!(
            rendered.payload,
            Payload::Binary {
                encoding: Encoding::Hex,
                bytes: vec![0x00, 0xd7, 0x01],
                text: "00d7 01".into(),
            }
        );
        assert_eq!(rendered.payload.bytes(), [0x00, 0xd7, 0x01]);
        assert_eq!(rendered.payload.text(), "00d7 01");
        assert_eq!(rendered.payload.encoding(), Encoding::Hex);
    }

    #[test]
    fn a_text_message_publishes_its_text() {
        let payload = Payload::from("21.5");
        assert_eq!(payload, Payload::Text("21.5".into()));
        assert_eq!(payload.bytes(), b"21.5");
        assert_eq!(payload.text(), "21.5");
        assert_eq!(payload.encoding(), Encoding::Text);
    }

    #[test]
    fn a_payload_that_does_not_encode_is_an_error_at_the_payload() {
        let templates = InstanceTemplates::compile(
            &[
                encoded("{{ 2 | u8 }}z", Encoding::Hex),
                encoded("{co2: 1}", Encoding::Cbor),
            ],
            None,
        )
        .unwrap();
        let ctx = tick_ctx(&BTreeMap::new());
        let rendered: Vec<_> = templates.render_messages_located(&ctx).collect();
        assert_eq!(
            rendered[0],
            Err(TemplateError::new(
                "messages[1].payload",
                "message 1 payload as hex: `z` is not a hex digit"
            ))
        );
        let error = rendered[1].clone().unwrap_err();
        assert_eq!(error.field, "messages[2].payload");
        assert!(
            error
                .message
                .starts_with("message 2 payload as CBOR: not JSON: "),
            "{}",
            error.message
        );
    }

    #[test]
    fn templates_with_the_same_messages_differ_by_their_encoders() {
        let messages = [message("t", "{{ 1 | u8 }}")];
        let text = InstanceTemplates::compile(&messages, None).unwrap();
        let hex = InstanceTemplates::compile_with(&messages, None, vec![Encoder::Hex]).unwrap();
        assert_eq!(text.encoders(), [Encoder::Text]);
        assert_eq!(hex.encoders(), [Encoder::Hex]);
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
