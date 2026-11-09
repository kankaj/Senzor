//! Rhai scripts: the `script` generator of a field and the script of a subscription.
//!
//! A [`Script`] is compiled once, the first time validation checks it, and every clone shares the
//! result. A [`Runtime`] runs the scripts of one instance: it holds the instance's `state`, which
//! all its scripts share, and the functions they can call.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::RngExt;
use rand::rngs::ChaCha8Rng;
use rhai::packages::{Package, StandardPackage};
use rhai::{AST, Dynamic, Engine, EvalAltResult, FLOAT, INT, ImmutableString, Map, Module, Scope};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::value::Value;

/// How many operations one run of a script may take before it is stopped.
pub const MAX_OPERATIONS: u64 = 100_000;
/// The longest string a script may build, in bytes: the size of the largest message.
pub const MAX_STRING_BYTES: usize = 1024 * 1024;
/// The most items an array or object map a script builds may hold.
pub const MAX_ITEMS: usize = 10_000;
/// How deep function calls may nest.
pub const MAX_CALL_DEPTH: usize = 32;

/// A script as written in the project file. Equal scripts have the same text.
#[derive(Clone)]
pub struct Script {
    source: String,
    /// Shared by every clone, so a script compiles once however often a project is copied into
    /// instances.
    compiled: Arc<OnceLock<Result<Arc<AST>, String>>>,
}

impl Script {
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            compiled: Arc::default(),
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// The compiled script, or what is wrong with it: `is empty` or `does not compile: <reason>`.
    pub fn compiled(&self) -> Result<Arc<AST>, String> {
        self.compiled
            .get_or_init(|| {
                if self.source.trim().is_empty() {
                    return Err("is empty".into());
                }
                compiler()
                    .compile(&self.source)
                    .map(Arc::new)
                    .map_err(|e| format!("does not compile: {e}"))
            })
            .clone()
    }

    /// Whether both share one compile, not just the text.
    #[cfg(test)]
    pub(crate) fn is_shared_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.compiled, &other.compiled)
    }
}

impl PartialEq for Script {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

impl fmt::Debug for Script {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.source, f)
    }
}

impl Serialize for Script {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.source)
    }
}

impl<'de> Deserialize<'de> for Script {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Script::new)
    }
}

#[cfg(feature = "schema")]
impl schemars::JsonSchema for Script {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Script".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "A Rhai script; see https://senzor.kankaj.cz/docs/concepts/scripts"
        })
    }
}

/// Rhai's standard functions, built once for every engine.
fn standard_library() -> rhai::Shared<Module> {
    static LIBRARY: OnceLock<rhai::Shared<Module>> = OnceLock::new();
    LIBRARY
        .get_or_init(|| StandardPackage::new().as_shared_module())
        .clone()
}

/// An engine with the standard functions and the limits, and nothing that reaches outside:
/// no `import` of files and no `print` output.
fn limited_engine() -> Engine {
    let mut engine = Engine::new_raw();
    engine.register_global_module(standard_library());
    engine
        .set_max_operations(MAX_OPERATIONS)
        .set_max_string_size(MAX_STRING_BYTES)
        .set_max_array_size(MAX_ITEMS)
        .set_max_map_size(MAX_ITEMS)
        .set_max_call_levels(MAX_CALL_DEPTH);
    engine
}

/// The engine that compiles scripts; running them takes an instance's [`Runtime`].
fn compiler() -> &'static Engine {
    static COMPILER: OnceLock<Engine> = OnceLock::new();
    COMPILER.get_or_init(limited_engine)
}

/// What a subscription script asked for, in the order it asked.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Publish { topic: String, payload: String },
    Pin(String, Value),
    Unpin(String),
}

/// What the functions registered on an instance's engine work with during one run.
struct Host {
    /// The instance's RNG while a script runs; otherwise a stand-in that nothing draws from.
    rng: ChaCha8Rng,
    /// Whether a subscription script is running: only those may publish, pin and unpin.
    in_subscription: bool,
    actions: Vec<Action>,
}

/// What a field script sees besides `state`.
pub struct FieldInput<'a> {
    /// The field's previous value (its `start` on the first tick).
    pub value: &'a Value,
    /// Time since the instance's run started.
    pub elapsed: Duration,
    pub seq: u64,
    pub now: SystemTime,
    /// Every field's value from the previous tick.
    pub fields: &'a BTreeMap<String, Value>,
}

/// A received message, for a subscription script.
pub struct Message<'a> {
    pub topic: &'a str,
    pub payload: &'a [u8],
    /// Every field's current value.
    pub fields: &'a BTreeMap<String, Value>,
}

/// Runs the scripts of one instance. Its `state` lasts as long as the runtime: one run of the
/// instance, across configuration updates.
pub struct Runtime {
    engine: Engine,
    host: Arc<Mutex<Host>>,
    state: Map,
    device_id: ImmutableString,
    params: Map,
}

impl Runtime {
    pub fn new(device_id: &str, params: &BTreeMap<String, Value>) -> Self {
        let host = Arc::new(Mutex::new(Host {
            rng: rand::make_rng(),
            in_subscription: false,
            actions: Vec::new(),
        }));
        let mut engine = limited_engine();
        register_functions(&mut engine, &host);
        Self {
            engine,
            host,
            state: Map::new(),
            device_id: device_id.into(),
            params: values_map(params),
        }
    }

    /// Takes the instance ID and params of an updated configuration; `state` stays.
    pub fn set_device(&mut self, device_id: &str, params: &BTreeMap<String, Value>) {
        self.device_id = device_id.into();
        self.params = values_map(params);
    }

    /// Runs a field script and returns the field's new value.
    pub fn field_value(
        &mut self,
        script: &AST,
        input: FieldInput<'_>,
        rng: &mut ChaCha8Rng,
    ) -> Result<Value, String> {
        let mut scope = Scope::new();
        scope.push_dynamic("value", to_dynamic(input.value));
        scope.push("t", input.elapsed.as_secs_f64() as FLOAT);
        scope.push("seq", input.seq as INT);
        scope.push("now_ms", now_ms(input.now));
        scope.push("device_id", self.device_id.clone());
        scope.push("params", self.params.clone());
        scope.push("fields", values_map(input.fields));
        let (result, _) = self.run(script, scope, rng, false);
        from_dynamic(result?).map_err(|e| format!("the script returned {e}"))
    }

    /// Runs a subscription script for `message`. Returns what it asked for, also when it failed
    /// part of the way, and whether it failed.
    pub fn on_message(
        &mut self,
        script: &AST,
        message: Message<'_>,
        rng: &mut ChaCha8Rng,
    ) -> (Vec<Action>, Result<(), String>) {
        let payload = String::from_utf8_lossy(message.payload).into_owned();
        let json = serde_json::from_slice(message.payload).map_or(Dynamic::UNIT, json_to_dynamic);
        let mut scope = Scope::new();
        scope.push("topic", message.topic.to_string());
        scope.push("payload", payload);
        scope.push_dynamic("json", json);
        scope.push("device_id", self.device_id.clone());
        scope.push("params", self.params.clone());
        scope.push("fields", values_map(message.fields));
        let (result, actions) = self.run(script, scope, rng, true);
        (actions, result.map(|_| ()))
    }

    /// Runs `script` with `scope` plus `state`, using `rng` for `random` and `random_range`.
    fn run(
        &mut self,
        script: &AST,
        mut scope: Scope<'_>,
        rng: &mut ChaCha8Rng,
        in_subscription: bool,
    ) -> (Result<Dynamic, String>, Vec<Action>) {
        {
            let mut host = self.host();
            std::mem::swap(&mut host.rng, rng);
            host.in_subscription = in_subscription;
            host.actions.clear();
        }
        scope.push("state", std::mem::take(&mut self.state));
        let ours = scope.len();
        let result = self
            .engine
            .eval_ast_with_scope::<Dynamic>(&mut scope, script)
            .map_err(|e| e.to_string());
        // Drop the script's own top-level variables, so a `let state` cannot hide ours.
        scope.rewind(ours);
        let state = scope.remove::<Map>("state");
        let mut host = self.host();
        std::mem::swap(&mut host.rng, rng);
        let actions = std::mem::take(&mut host.actions);
        drop(host);
        let result = match state {
            Some(state) => {
                self.state = state;
                result
            }
            None => result.and(Err(
                "state must stay an object map; it starts again empty".into()
            )),
        };
        (result, actions)
    }

    fn host(&self) -> MutexGuard<'_, Host> {
        self.host.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[cfg(test)]
    pub(crate) fn state(&self) -> &Map {
        &self.state
    }
}

type FnResult<T> = Result<T, Box<EvalAltResult>>;

/// Registers the functions scripts can call besides Rhai's own.
fn register_functions(engine: &mut Engine, host: &Arc<Mutex<Host>>) {
    let h = host.clone();
    engine.register_fn("random", move || -> FLOAT {
        with_host(&h, |host| host.rng.random::<f64>())
    });
    let h = host.clone();
    engine.register_fn(
        "random_range",
        move |lo: Dynamic, hi: Dynamic| -> FnResult<Dynamic> {
            match (lo.as_int(), hi.as_int()) {
                (Ok(lo), Ok(hi)) if lo < hi => {
                    Ok(with_host(&h, |host| host.rng.random_range(lo..hi)).into())
                }
                (Ok(_), Ok(_)) => Err("random_range: lo must be less than hi".into()),
                _ => {
                    let (lo, hi) = (number(&lo, "random_range")?, number(&hi, "random_range")?);
                    if lo < hi {
                        Ok(with_host(&h, |host| host.rng.random_range(lo..hi)).into())
                    } else {
                        Err("random_range: lo must be less than hi".into())
                    }
                }
            }
        },
    );
    engine.register_fn(
        "clamp",
        |x: Dynamic, lo: Dynamic, hi: Dynamic| -> FnResult<Dynamic> {
            if let (Ok(x), Ok(lo), Ok(hi)) = (x.as_int(), lo.as_int(), hi.as_int()) {
                return if lo <= hi {
                    Ok(x.clamp(lo, hi).into())
                } else {
                    Err("clamp: lo must not be above hi".into())
                };
            }
            let (x, lo, hi) = (
                number(&x, "clamp")?,
                number(&lo, "clamp")?,
                number(&hi, "clamp")?,
            );
            if lo <= hi {
                Ok(x.clamp(lo, hi).into())
            } else {
                Err("clamp: lo must not be above hi".into())
            }
        },
    );
    let h = host.clone();
    engine.register_fn(
        "publish",
        move |topic: ImmutableString, payload: Dynamic| -> FnResult<()> {
            if topic.is_empty() {
                return Err("publish: the topic is empty".into());
            }
            if topic.contains(['+', '#']) {
                return Err(
                    format!("publish: topic `{topic}` contains a wildcard (+ or #)").into(),
                );
            }
            // Text as it is; anything else as Rhai writes it, e.g. `21.5` or `true`.
            let payload = payload.to_string();
            act(
                &h,
                "publish",
                Action::Publish {
                    topic: topic.to_string(),
                    payload,
                },
            )
        },
    );
    let h = host.clone();
    engine.register_fn(
        "pin",
        move |field: ImmutableString, value: Dynamic| -> FnResult<()> {
            let value = from_dynamic(value).map_err(|e| format!("pin: the value is {e}"))?;
            act(&h, "pin", Action::Pin(field.to_string(), value))
        },
    );
    let h = host.clone();
    engine.register_fn("unpin", move |field: ImmutableString| -> FnResult<()> {
        act(&h, "unpin", Action::Unpin(field.to_string()))
    });
}

fn with_host<R>(host: &Mutex<Host>, f: impl FnOnce(&mut Host) -> R) -> R {
    f(&mut host.lock().unwrap_or_else(PoisonError::into_inner))
}

/// Records `action`, which only a subscription script may ask for.
fn act(host: &Mutex<Host>, name: &str, action: Action) -> FnResult<()> {
    with_host(host, |host| {
        if host.in_subscription {
            host.actions.push(action);
            Ok(())
        } else {
            Err(format!("{name} works only in a subscription script").into())
        }
    })
}

/// `value` as a float, for a function that takes numbers.
fn number(value: &Dynamic, function: &str) -> FnResult<FLOAT> {
    value
        .as_float()
        .or_else(|_| value.as_int().map(|i| i as FLOAT))
        .map_err(|_| format!("{function} takes numbers, not {}", value.type_name()).into())
}

fn now_ms(now: SystemTime) -> INT {
    now.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as INT
}

/// A value for a script. Numbers are floats, so `value / 2` does not round.
fn to_dynamic(value: &Value) -> Dynamic {
    match value {
        Value::Bool(b) => (*b).into(),
        Value::Number(n) => (*n as FLOAT).into(),
        Value::String(s) => s.clone().into(),
    }
}

fn values_map(values: &BTreeMap<String, Value>) -> Map {
    values
        .iter()
        .map(|(name, value)| (name.as_str().into(), to_dynamic(value)))
        .collect()
}

/// A script's result as a field value. The error names what it is instead.
fn from_dynamic(value: Dynamic) -> Result<Value, String> {
    if let Ok(n) = value.as_float() {
        return if n.is_finite() {
            Ok(Value::Number(n))
        } else {
            Err(format!("{n}, which is not a finite number"))
        };
    }
    if let Ok(i) = value.as_int() {
        return Ok(Value::Number(i as f64));
    }
    if let Ok(b) = value.as_bool() {
        return Ok(Value::Bool(b));
    }
    let type_name = value.type_name();
    value
        .into_immutable_string()
        .map(|s| Value::String(s.to_string()))
        .map_err(|_| format!("{type_name}; it must return a number, a bool or a string"))
}

/// A JSON value for a script: objects become object maps and `null` becomes `()`. Numbers are
/// floats, as everywhere else in scripts.
fn json_to_dynamic(json: serde_json::Value) -> Dynamic {
    match json {
        serde_json::Value::Null => Dynamic::UNIT,
        serde_json::Value::Bool(b) => b.into(),
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(f64::NAN).into(),
        serde_json::Value::String(s) => s.into(),
        serde_json::Value::Array(items) => {
            Dynamic::from_array(items.into_iter().map(json_to_dynamic).collect())
        }
        serde_json::Value::Object(map) => Dynamic::from_map(
            map.into_iter()
                .map(|(key, value)| (key.into(), json_to_dynamic(value)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::instance_rng;

    fn ast(source: &str) -> Arc<AST> {
        Script::new(source).compiled().unwrap()
    }

    fn params() -> BTreeMap<String, Value> {
        BTreeMap::from([("room".to_string(), Value::String("office".into()))])
    }

    fn runtime() -> Runtime {
        Runtime::new("d-0", &params())
    }

    /// Runs the field script `source` once with `value` as the previous value.
    fn field(runtime: &mut Runtime, source: &str, value: Value) -> Result<Value, String> {
        let fields = BTreeMap::from([("co2".to_string(), Value::Number(600.0))]);
        let input = FieldInput {
            value: &value,
            elapsed: Duration::from_millis(2500),
            seq: 7,
            now: UNIX_EPOCH + Duration::from_millis(1_234),
            fields: &fields,
        };
        runtime.field_value(&ast(source), input, &mut instance_rng(Some(1), "d-0"))
    }

    fn message(
        runtime: &mut Runtime,
        source: &str,
        payload: &str,
    ) -> (Vec<Action>, Result<(), String>) {
        let fields = BTreeMap::from([("co2".to_string(), Value::Number(600.0))]);
        let message = Message {
            topic: "cmd/d-0/heating",
            payload: payload.as_bytes(),
            fields: &fields,
        };
        runtime.on_message(&ast(source), message, &mut instance_rng(Some(1), "d-0"))
    }

    #[test]
    fn a_script_compiles_once_for_all_its_copies() {
        let script = Script::new("value + 1");
        let copy = script.clone();
        assert!(script.compiled().is_ok());
        assert!(copy.is_shared_with(&script));
        assert_eq!(copy, Script::new("value + 1"));
        assert!(!Script::new("value + 1").is_shared_with(&script));
    }

    #[test]
    fn an_empty_or_broken_script_says_what_is_wrong() {
        assert_eq!(Script::new(" \n").compiled().err(), Some("is empty".into()));
        let err = Script::new("value +").compiled().err().unwrap();
        assert!(err.starts_with("does not compile: "), "{err}");
        assert!(err.contains("line 1"), "{err}");
    }

    #[test]
    fn a_script_reads_and_writes_as_its_text() {
        #[derive(Serialize, Deserialize)]
        struct Wrapper {
            script: Script,
        }
        let read: Wrapper = toml::from_str("script = '''\nvalue + 1\n'''").unwrap();
        assert_eq!(read.script.source(), "value + 1\n");
        let written = toml::to_string(&read).unwrap();
        let again: Wrapper = toml::from_str(&written).unwrap();
        assert_eq!(again.script, read.script);
    }

    #[test]
    fn a_field_script_returns_the_new_value() {
        let mut rt = runtime();
        assert_eq!(
            field(&mut rt, "value + 0.5", Value::Number(20.0)),
            Ok(Value::Number(20.5))
        );
        assert_eq!(
            field(&mut rt, "value / 2", Value::Number(5.0)),
            Ok(Value::Number(2.5)),
            "numbers are floats"
        );
        assert_eq!(
            field(&mut rt, "seq * 2", Value::Number(0.0)),
            Ok(Value::Number(14.0))
        );
        assert_eq!(
            field(&mut rt, "!value", Value::Bool(true)),
            Ok(Value::Bool(false))
        );
        assert_eq!(
            field(&mut rt, r#"value + "!""#, Value::String("on".into())),
            Ok(Value::String("on!".into()))
        );
    }

    #[test]
    fn a_field_script_sees_time_params_fields_and_its_instance() {
        let mut rt = runtime();
        let got = field(
            &mut rt,
            r#"`${t} ${now_ms} ${params.room} ${fields.co2} ${device_id}`"#,
            Value::Number(0.0),
        );
        assert_eq!(got, Ok(Value::String("2.5 1234 office 600.0 d-0".into())));
    }

    #[test]
    fn rhai_math_is_there() {
        let mut rt = runtime();
        let got = field(
            &mut rt,
            "round(sqrt(16.0) + abs(-1.0) + floor(2.7) + max(1, 2) + min(1.5, 3.0) + sin(0.0) + cos(0.0))",
            Value::Number(0.0),
        );
        assert_eq!(got, Ok(Value::Number(12.0)));
    }

    #[test]
    fn state_is_shared_by_the_scripts_of_a_runtime_and_unset_keys_read_as_unit() {
        let mut rt = runtime();
        assert_eq!(
            field(&mut rt, "state.heating == ()", Value::Number(0.0)),
            Ok(Value::Bool(true))
        );
        let (_, result) = message(&mut rt, r#"state.heating = payload == "ON";"#, "ON");
        assert_eq!(result, Ok(()));
        assert_eq!(
            field(
                &mut rt,
                "if state.heating == true { value + 1 } else { value }",
                Value::Number(20.0)
            ),
            Ok(Value::Number(21.0))
        );
        assert_eq!(
            field(&mut rt, r#"state.setpoint ?? 21.5"#, Value::Number(0.0)),
            Ok(Value::Number(21.5))
        );
        assert_eq!(
            field(
                &mut rt,
                r#"("heating" in state) && !("setpoint" in state)"#,
                Value::Number(0.0)
            ),
            Ok(Value::Bool(true))
        );
        let err = field(
            &mut rt,
            "if state.missing { 1 } else { 2 }",
            Value::Number(0.0),
        )
        .unwrap_err();
        assert!(err.contains("expecting bool"), "`if` needs a bool: {err}");
        assert_eq!(
            field(&mut rt, "let state = 1; state", Value::Number(0.0)),
            Ok(Value::Number(1.0))
        );
        assert_eq!(
            rt.state()["heating"].as_bool(),
            Ok(true),
            "a `let` hides it only"
        );
        assert!(runtime().state().is_empty(), "each runtime starts empty");
    }

    #[test]
    fn replacing_state_with_something_else_is_an_error_and_empties_it() {
        let mut rt = runtime();
        let _ = message(&mut rt, "state.a = 1;", "");
        let (_, result) = message(&mut rt, "state = 5;", "");
        assert_eq!(
            result,
            Err("state must stay an object map; it starts again empty".into())
        );
        assert!(rt.state().is_empty());
    }

    #[test]
    fn random_draws_from_the_instance_rng_so_a_seed_repeats_it() {
        let draws = |seed| {
            let mut rt = runtime();
            let mut rng = instance_rng(Some(seed), "d-0");
            let script = ast("random() + random_range(10, 20) + random_range(0.5, 1.5)");
            (0..5)
                .map(|seq| {
                    let input = FieldInput {
                        value: &Value::Number(0.0),
                        elapsed: Duration::ZERO,
                        seq,
                        now: UNIX_EPOCH,
                        fields: &BTreeMap::new(),
                    };
                    rt.field_value(&script, input, &mut rng).unwrap()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(draws(1), draws(1));
        assert_ne!(draws(1), draws(2));
        for value in draws(3) {
            let n = value.as_number().unwrap();
            assert!((10.5..21.5).contains(&n), "{n}");
        }
    }

    #[test]
    fn a_run_advances_the_instance_rng_by_what_the_script_drew() {
        let mut rt = runtime();
        let mut rng = instance_rng(Some(7), "d-0");
        let mut expected = instance_rng(Some(7), "d-0");
        let input = FieldInput {
            value: &Value::Number(0.0),
            elapsed: Duration::ZERO,
            seq: 0,
            now: UNIX_EPOCH,
            fields: &BTreeMap::new(),
        };
        let drawn = rt.field_value(&ast("random()"), input, &mut rng).unwrap();
        assert_eq!(drawn, Value::Number(expected.random::<f64>()));
        assert_eq!(rng.random::<u64>(), expected.random::<u64>());
    }

    #[test]
    fn random_range_and_clamp_keep_whole_numbers_whole() {
        let mut rt = runtime();
        let got = field(
            &mut rt,
            "let r = random_range(1, 3); (r == 1 || r == 2) && type_of(r) == \"i64\"",
            Value::Number(0.0),
        );
        assert_eq!(got, Ok(Value::Bool(true)));
        assert_eq!(
            field(&mut rt, "clamp(15, 0, 10)", Value::Number(0.0)),
            Ok(Value::Number(10.0))
        );
        assert_eq!(
            field(&mut rt, "clamp(value, 0, 10.5)", Value::Number(-3.0)),
            Ok(Value::Number(0.0))
        );
        let err = field(&mut rt, "random_range(2, 2)", Value::Number(0.0)).unwrap_err();
        assert!(err.contains("lo must be less than hi"), "{err}");
        let err = field(&mut rt, r#"clamp("a", 0, 1)"#, Value::Number(0.0)).unwrap_err();
        assert!(err.contains("clamp takes numbers"), "{err}");
    }

    #[test]
    fn a_result_that_is_not_a_value_is_an_error() {
        let mut rt = runtime();
        let err = field(&mut rt, "[1, 2]", Value::Number(0.0)).unwrap_err();
        assert_eq!(
            err,
            "the script returned array; it must return a number, a bool or a string"
        );
        let err = field(&mut rt, "let x = 1;", Value::Number(0.0)).unwrap_err();
        assert!(err.starts_with("the script returned ()"), "{err}");
        let err = field(&mut rt, "0.0 / 0.0", Value::Number(0.0)).unwrap_err();
        assert!(err.contains("not a finite number"), "{err}");
    }

    #[test]
    fn a_script_that_runs_too_long_or_grows_too_big_is_stopped() {
        let mut rt = runtime();
        let err = field(&mut rt, "loop {}", Value::Number(0.0)).unwrap_err();
        assert!(err.contains("Too many operations"), "{err}");
        let err = field(
            &mut rt,
            r#"let s = "x"; loop { s += s; }"#,
            Value::Number(0.0),
        )
        .unwrap_err();
        assert!(err.contains("too large"), "{err}");
    }

    #[test]
    fn a_runtime_error_names_the_problem() {
        let mut rt = runtime();
        let err = field(&mut rt, "nope(1)", Value::Number(0.0)).unwrap_err();
        assert!(err.contains("nope"), "{err}");
    }

    #[test]
    fn a_subscription_script_publishes_pins_and_unpins() {
        let mut rt = runtime();
        let (actions, result) = message(
            &mut rt,
            r#"publish("state/" + device_id + "/heating", payload);
               publish("state/" + device_id + "/n", 21.5);
               pin("co2", 2500);
               unpin("co2");"#,
            "ON",
        );
        assert_eq!(result, Ok(()));
        assert_eq!(
            actions,
            [
                Action::Publish {
                    topic: "state/d-0/heating".into(),
                    payload: "ON".into()
                },
                Action::Publish {
                    topic: "state/d-0/n".into(),
                    payload: "21.5".into()
                },
                Action::Pin("co2".into(), Value::Number(2500.0)),
                Action::Unpin("co2".into()),
            ]
        );
    }

    #[test]
    fn a_subscription_script_sees_the_message_and_its_json() {
        let mut rt = runtime();
        let (_, result) = message(
            &mut rt,
            r#"state.got = `${topic} ${json.setpoint} ${json.on} ${fields.co2} ${params.room}`;"#,
            r#"{"setpoint": 22, "on": true}"#,
        );
        assert_eq!(result, Ok(()));
        assert_eq!(
            rt.state()["got"].to_string(),
            "cmd/d-0/heating 22.0 true 600.0 office"
        );
        let _ = message(&mut rt, "state.json = json;", "ON");
        assert!(rt.state()["json"].is_unit(), "not JSON");
    }

    #[test]
    fn a_script_can_write_json() {
        let mut rt = runtime();
        let (_, result) = message(&mut rt, "state.j = #{on: true, t: 21.5}.to_json();", "");
        assert_eq!(result, Ok(()));
        assert_eq!(rt.state()["j"].to_string(), r#"{"on":true,"t":21.5}"#);
    }

    #[test]
    fn what_a_failing_script_did_before_the_error_stands() {
        let mut rt = runtime();
        let (actions, result) = message(&mut rt, r#"publish("a", "1"); state.x = 1; nope();"#, "");
        assert_eq!(actions.len(), 1);
        assert!(result.is_err());
        assert_eq!(rt.state()["x"].as_int(), Ok(1));
    }

    #[test]
    fn publish_pin_and_unpin_work_only_in_subscription_scripts() {
        let mut rt = runtime();
        let err = field(&mut rt, r#"publish("a", "b"); 1"#, Value::Number(0.0)).unwrap_err();
        assert!(
            err.contains("publish works only in a subscription script"),
            "{err}"
        );
        let err = field(&mut rt, r#"pin("co2", 1); 1"#, Value::Number(0.0)).unwrap_err();
        assert!(err.contains("pin works only"), "{err}");
    }

    #[test]
    fn publish_and_pin_refuse_what_they_cannot_send() {
        let mut rt = runtime();
        let (actions, result) = message(&mut rt, r#"publish("a/+", "b");"#, "");
        assert!(actions.is_empty());
        let err = result.unwrap_err();
        assert!(err.contains("topic `a/+` contains a wildcard"), "{err}");
        let (_, result) = message(&mut rt, r#"publish("", "b");"#, "");
        let err = result.unwrap_err();
        assert!(err.contains("the topic is empty"), "{err}");
        let (_, result) = message(&mut rt, r#"pin("co2", [1]);"#, "");
        let err = result.unwrap_err();
        assert!(err.contains("pin: the value is array"), "{err}");
    }

    #[test]
    fn scripts_cannot_import_files_or_print() {
        let mut rt = runtime();
        let err = field(&mut rt, r#"import "x" as x; 1"#, Value::Number(0.0)).unwrap_err();
        assert!(err.contains("x"), "{err}");
        assert_eq!(
            field(&mut rt, r#"print("x"); debug("y"); 1"#, Value::Number(0.0)),
            Ok(Value::Number(1.0)),
            "print and debug do nothing"
        );
    }
}
