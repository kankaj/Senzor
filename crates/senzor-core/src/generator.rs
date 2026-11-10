use std::collections::BTreeMap;
use std::f64::consts::TAU;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use rand::RngExt;
use rand::rngs::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::i18n::{Text, text};
use crate::script::{FieldInput, Runtime, Script};
use crate::value::Value;

/// One field's generator as written in the project file (`generator = "..."` plus parameters).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "generator", rename_all = "snake_case", deny_unknown_fields)]
pub enum GeneratorSpec {
    Constant {
        value: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        noise: Option<f64>,
    },
    RandomWalk {
        start: f64,
        min: f64,
        max: f64,
        step: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        noise: Option<f64>,
    },
    Sine {
        center: f64,
        amplitude: f64,
        #[cfg_attr(feature = "schema", schemars(with = "String"))]
        #[serde(with = "humantime_serde")]
        period: Duration,
        #[cfg_attr(feature = "schema", schemars(with = "String", extend("default" = "0s")))]
        #[serde(
            default,
            with = "humantime_serde",
            skip_serializing_if = "Duration::is_zero"
        )]
        phase: Duration,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        noise: Option<f64>,
    },
    Uniform {
        min: f64,
        max: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        noise: Option<f64>,
    },
    Sequence {
        values: Vec<Value>,
        #[serde(default = "default_loop", rename = "loop")]
        looping: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        noise: Option<f64>,
    },
    /// A Rhai script whose result is the field's value; `start` is its `value` on the first tick.
    Script {
        #[cfg_attr(feature = "schema", schemars(extend("default" = 0)))]
        #[serde(default = "default_start", skip_serializing_if = "is_default_start")]
        start: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        noise: Option<f64>,
        script: Script,
    },
}

fn default_start() -> Value {
    Value::Number(0.0)
}

fn is_default_start(start: &Value) -> bool {
    *start == default_start()
}

fn default_loop() -> bool {
    true
}

/// Everything a value source may use to produce the next value.
pub struct TickContext<'a> {
    pub now: SystemTime,
    /// Time since the instance's run started.
    pub elapsed: Duration,
    /// Tick number within the run, starting at 0.
    pub seq: u64,
    pub rng: &'a mut ChaCha8Rng,
    /// Runs the instance's field scripts.
    pub scripts: &'a mut Runtime,
    /// Every field's value from the previous tick, for field scripts.
    pub fields: &'a BTreeMap<String, Value>,
}

/// Produces a field's value on each tick. Only a script can fail; the error says why.
pub trait ValueSource: Send {
    fn next(&mut self, ctx: &mut TickContext<'_>) -> Result<Value, String>;
}

impl GeneratorSpec {
    /// Checks parameter constraints. The text names the offending parameter.
    pub fn validate(&self) -> Result<(), Text> {
        if self.numbers().iter().any(|n| !n.is_finite()) {
            return Err(text!("numbers-not-finite"));
        }
        if let Some(noise) = self.noise() {
            if noise <= 0.0 {
                return Err(text!("greater-than-zero", key = "noise"));
            }
            // A script's noise applies only when its result is a number.
            let numeric = match self {
                GeneratorSpec::Constant { value, .. } => value.is_number(),
                GeneratorSpec::Sequence { values, .. } => values.iter().all(Value::is_number),
                _ => true,
            };
            if !numeric {
                return Err(text!("noise-needs-numbers"));
            }
        }
        match self {
            GeneratorSpec::RandomWalk {
                start,
                min,
                max,
                step,
                ..
            } => {
                if min >= max {
                    Err(text!("min-not-below-max"))
                } else if !(*min..=*max).contains(start) {
                    Err(text!("start-outside-bounds"))
                } else if *step <= 0.0 {
                    Err(text!("greater-than-zero", key = "step"))
                } else {
                    Ok(())
                }
            }
            GeneratorSpec::Sine { period, .. } if period.is_zero() => {
                Err(text!("greater-than-zero", key = "period"))
            }
            GeneratorSpec::Uniform { min, max, .. } if min >= max => {
                Err(text!("min-not-below-max"))
            }
            GeneratorSpec::Sequence { values, .. } if values.is_empty() => {
                Err(text!("must-not-be-empty", key = "values"))
            }
            _ => Ok(()),
        }
    }

    /// Every numeric parameter, for the finiteness check (TOML allows `nan` and `inf`).
    fn numbers(&self) -> Vec<f64> {
        let mut numbers = match self {
            GeneratorSpec::Constant { value, .. } => value.as_number().into_iter().collect(),
            GeneratorSpec::RandomWalk {
                start,
                min,
                max,
                step,
                ..
            } => vec![*start, *min, *max, *step],
            GeneratorSpec::Sine {
                center, amplitude, ..
            } => vec![*center, *amplitude],
            GeneratorSpec::Uniform { min, max, .. } => vec![*min, *max],
            GeneratorSpec::Sequence { values, .. } => {
                values.iter().filter_map(Value::as_number).collect()
            }
            GeneratorSpec::Script { start, .. } => start.as_number().into_iter().collect(),
        };
        numbers.extend(self.noise());
        numbers
    }

    /// The value used for sample renders at load time.
    pub fn initial_value(&self) -> Value {
        match self {
            GeneratorSpec::Constant { value, .. } => value.clone(),
            GeneratorSpec::RandomWalk { start, .. } => Value::Number(*start),
            GeneratorSpec::Sine { center, .. } => Value::Number(*center),
            GeneratorSpec::Uniform { min, .. } => Value::Number(*min),
            GeneratorSpec::Sequence { values, .. } => {
                values.first().cloned().unwrap_or(Value::Number(0.0))
            }
            GeneratorSpec::Script { start, .. } => start.clone(),
        }
    }

    /// Builds a fresh value source. Call only on a spec that passed [`GeneratorSpec::validate`].
    pub fn build(&self) -> Box<dyn ValueSource> {
        let source: Box<dyn ValueSource> = match self {
            GeneratorSpec::Constant { value, .. } => Box::new(Constant(value.clone())),
            GeneratorSpec::RandomWalk {
                start,
                min,
                max,
                step,
                ..
            } => Box::new(RandomWalk {
                state: *start,
                min: *min,
                max: *max,
                step: *step,
                started: false,
            }),
            GeneratorSpec::Sine {
                center,
                amplitude,
                period,
                phase,
                ..
            } => Box::new(Sine {
                center: *center,
                amplitude: *amplitude,
                period: *period,
                phase: *phase,
            }),
            GeneratorSpec::Uniform { min, max, .. } => Box::new(Uniform {
                min: *min,
                max: *max,
            }),
            GeneratorSpec::Sequence {
                values, looping, ..
            } => Box::new(Sequence {
                values: values.clone(),
                looping: *looping,
                next_index: 0,
            }),
            GeneratorSpec::Script { start, script, .. } => match script.compiled() {
                Ok(script) => Box::new(ScriptSource {
                    script,
                    value: start.clone(),
                }),
                Err(problem) => Box::new(BrokenScript(problem.to_string())),
            },
        };
        match self.noise() {
            Some(noise) => Box::new(Noisy {
                inner: source,
                noise,
            }),
            None => source,
        }
    }

    fn noise(&self) -> Option<f64> {
        match self {
            GeneratorSpec::Constant { noise, .. }
            | GeneratorSpec::RandomWalk { noise, .. }
            | GeneratorSpec::Sine { noise, .. }
            | GeneratorSpec::Uniform { noise, .. }
            | GeneratorSpec::Sequence { noise, .. }
            | GeneratorSpec::Script { noise, .. } => *noise,
        }
    }
}

struct Constant(Value);

impl ValueSource for Constant {
    fn next(&mut self, _ctx: &mut TickContext<'_>) -> Result<Value, String> {
        Ok(self.0.clone())
    }
}

struct RandomWalk {
    state: f64,
    min: f64,
    max: f64,
    step: f64,
    started: bool,
}

impl ValueSource for RandomWalk {
    fn next(&mut self, ctx: &mut TickContext<'_>) -> Result<Value, String> {
        if self.started {
            let delta = ctx.rng.random_range(-self.step..=self.step);
            self.state = (self.state + delta).clamp(self.min, self.max);
        }
        self.started = true;
        Ok(Value::Number(self.state))
    }
}

struct Sine {
    center: f64,
    amplitude: f64,
    period: Duration,
    phase: Duration,
}

impl ValueSource for Sine {
    fn next(&mut self, ctx: &mut TickContext<'_>) -> Result<Value, String> {
        let t = (ctx.elapsed + self.phase).as_secs_f64();
        let angle = TAU * t / self.period.as_secs_f64();
        Ok(Value::Number(self.center + self.amplitude * angle.sin()))
    }
}

struct Uniform {
    min: f64,
    max: f64,
}

impl ValueSource for Uniform {
    fn next(&mut self, ctx: &mut TickContext<'_>) -> Result<Value, String> {
        Ok(Value::Number(ctx.rng.random_range(self.min..self.max)))
    }
}

struct Sequence {
    values: Vec<Value>,
    looping: bool,
    next_index: usize,
}

impl ValueSource for Sequence {
    fn next(&mut self, _ctx: &mut TickContext<'_>) -> Result<Value, String> {
        if self.next_index >= self.values.len() {
            if self.looping {
                self.next_index = 0;
            } else {
                return Ok(self.values[self.values.len() - 1].clone());
            }
        }
        let value = self.values[self.next_index].clone();
        self.next_index += 1;
        Ok(value)
    }
}

/// Runs a field script; its last result is the script's next `value`.
struct ScriptSource {
    script: Arc<rhai::AST>,
    value: Value,
}

impl ValueSource for ScriptSource {
    fn next(&mut self, ctx: &mut TickContext<'_>) -> Result<Value, String> {
        let input = FieldInput {
            value: &self.value,
            elapsed: ctx.elapsed,
            seq: ctx.seq,
            now: ctx.now,
            fields: ctx.fields,
        };
        let value = ctx.scripts.field_value(&self.script, input, ctx.rng)?;
        self.value = value.clone();
        Ok(value)
    }
}

/// A script that does not compile, which validation reports; it fails every tick.
struct BrokenScript(String);

impl ValueSource for BrokenScript {
    fn next(&mut self, _ctx: &mut TickContext<'_>) -> Result<Value, String> {
        Err(self.0.clone())
    }
}

/// Adds uniform(-noise, +noise) to the inner source's output without touching its state.
struct Noisy {
    inner: Box<dyn ValueSource>,
    noise: f64,
}

impl ValueSource for Noisy {
    fn next(&mut self, ctx: &mut TickContext<'_>) -> Result<Value, String> {
        Ok(match self.inner.next(ctx)? {
            Value::Number(n) => Value::Number(n + ctx.rng.random_range(-self.noise..=self.noise)),
            other => other,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::instance_rng;

    fn spec(toml_src: &str) -> GeneratorSpec {
        toml::from_str(toml_src).unwrap()
    }

    fn run(spec: &GeneratorSpec, ticks: usize) -> Vec<Value> {
        run_seeded(spec, ticks, 7)
            .into_iter()
            .map(Result::unwrap)
            .collect()
    }

    /// Runs one source of `spec` with the instance RNG of `seed`.
    fn run_seeded(spec: &GeneratorSpec, ticks: usize, seed: u64) -> Vec<Result<Value, String>> {
        const TICK: Duration = Duration::from_secs(1);
        let mut rng = instance_rng(Some(seed), "test");
        let mut scripts = Runtime::new("test", &BTreeMap::new());
        let fields = BTreeMap::new();
        let mut source = spec.build();
        (0..ticks)
            .map(|i| {
                let mut ctx = TickContext {
                    now: SystemTime::UNIX_EPOCH,
                    elapsed: TICK * i as u32,
                    seq: i as u64,
                    rng: &mut rng,
                    scripts: &mut scripts,
                    fields: &fields,
                };
                source.next(&mut ctx)
            })
            .collect()
    }

    fn numbers(values: &[Value]) -> Vec<f64> {
        values
            .iter()
            .map(|v| match v {
                Value::Number(n) => *n,
                other => panic!("expected number, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn constant_repeats_its_value() {
        let s = spec(
            r#"generator = "constant"
value = "1.2.3""#,
        );
        assert_eq!(run(&s, 3), vec![Value::String("1.2.3".into()); 3]);
    }

    #[test]
    fn random_walk_starts_at_start_and_stays_in_bounds() {
        let s = spec(
            r#"generator = "random_walk"
start = 600
min = 590
max = 610
step = 15"#,
        );
        let values = numbers(&run(&s, 500));
        assert_eq!(values[0], 600.0);
        assert!(values.iter().all(|v| (590.0..=610.0).contains(v)));
        assert!(values.iter().any(|v| *v != 600.0));
    }

    #[test]
    fn random_walk_is_deterministic_for_a_seed() {
        let s = spec(
            r#"generator = "random_walk"
start = 600
min = 400
max = 2000
step = 15"#,
        );
        assert_eq!(run(&s, 20), run(&s, 20));
    }

    #[test]
    fn sine_follows_the_formula() {
        let s = spec(
            r#"generator = "sine"
center = 20
amplitude = 2
period = "4s""#,
        );
        let values = numbers(&run(&s, 4));
        let expected = [20.0, 22.0, 20.0, 18.0];
        for (got, want) in values.iter().zip(expected) {
            assert!((got - want).abs() < 1e-9, "got {got}, want {want}");
        }
    }

    #[test]
    fn sine_phase_shifts_the_wave() {
        let s = spec(
            r#"generator = "sine"
center = 20
amplitude = 2
period = "4s"
phase = "1s""#,
        );
        let first = numbers(&run(&s, 1))[0];
        assert!((first - 22.0).abs() < 1e-9);
    }

    #[test]
    fn uniform_stays_in_range() {
        let s = spec(
            r#"generator = "uniform"
min = 1
max = 2"#,
        );
        let values = numbers(&run(&s, 200));
        assert!(values.iter().all(|v| (1.0..2.0).contains(v)));
    }

    #[test]
    fn sequence_loops_by_default() {
        let s = spec(
            r#"generator = "sequence"
values = ["open", "closed"]"#,
        );
        let got = run(&s, 3);
        assert_eq!(
            got,
            vec![
                Value::String("open".into()),
                Value::String("closed".into()),
                Value::String("open".into())
            ]
        );
    }

    #[test]
    fn sequence_without_loop_holds_last_value() {
        let s = spec(
            r#"generator = "sequence"
values = [1, 2]
loop = false"#,
        );
        assert_eq!(numbers(&run(&s, 4)), vec![1.0, 2.0, 2.0, 2.0]);
    }

    #[test]
    fn noise_stays_within_its_range() {
        let s = spec(
            r#"generator = "constant"
value = 10
noise = 0.5"#,
        );
        let values = numbers(&run(&s, 200));
        assert!(values.iter().all(|v| (9.5..=10.5).contains(v)));
        assert!(values.iter().any(|v| *v != 10.0));
    }

    #[test]
    fn validate_reports_each_constraint() {
        let cases = [
            (
                "generator = \"random_walk\"\nstart = 5\nmin = 10\nmax = 1\nstep = 1",
                "min must be less than max",
            ),
            (
                "generator = \"random_walk\"\nstart = 50\nmin = 0\nmax = 10\nstep = 1",
                "start must be between min and max",
            ),
            (
                "generator = \"random_walk\"\nstart = 5\nmin = 0\nmax = 10\nstep = 0",
                "step must be greater than 0",
            ),
            (
                "generator = \"sine\"\ncenter = 1\namplitude = 1\nperiod = \"0s\"",
                "period must be greater than 0",
            ),
            (
                "generator = \"uniform\"\nmin = 2\nmax = 2",
                "min must be less than max",
            ),
            (
                "generator = \"sequence\"\nvalues = []",
                "values must not be empty",
            ),
            (
                "generator = \"constant\"\nvalue = 1\nnoise = 0",
                "noise must be greater than 0",
            ),
            (
                "generator = \"constant\"\nvalue = \"x\"\nnoise = 1",
                "noise requires numeric values",
            ),
            (
                "generator = \"sequence\"\nvalues = [1, \"x\"]\nnoise = 1",
                "noise requires numeric values",
            ),
            (
                "generator = \"uniform\"\nmin = nan\nmax = 2",
                "numbers must be finite",
            ),
            (
                "generator = \"constant\"\nvalue = inf",
                "numbers must be finite",
            ),
        ];
        for (src, message) in cases {
            assert_eq!(
                spec(src).validate().map_err(|e| e.to_string()),
                Err(message.to_string()),
                "for {src}"
            );
        }
    }

    #[test]
    fn initial_values_match_the_spec() {
        let cases = [
            ("generator = \"constant\"\nvalue = true", Value::Bool(true)),
            (
                "generator = \"random_walk\"\nstart = 5\nmin = 0\nmax = 10\nstep = 1",
                Value::Number(5.0),
            ),
            (
                "generator = \"sine\"\ncenter = 21\namplitude = 1\nperiod = \"1h\"",
                Value::Number(21.0),
            ),
            (
                "generator = \"uniform\"\nmin = 3\nmax = 4",
                Value::Number(3.0),
            ),
            (
                "generator = \"sequence\"\nvalues = [\"a\", \"b\"]",
                Value::String("a".into()),
            ),
        ];
        for (src, expected) in cases {
            assert_eq!(spec(src).initial_value(), expected, "for {src}");
        }
    }

    #[test]
    fn a_script_carries_on_from_its_previous_value() {
        let s = spec("generator = \"script\"\nstart = 20\nscript = \"value + 1\"");
        assert_eq!(numbers(&run(&s, 3)), vec![21.0, 22.0, 23.0]);
        assert_eq!(s.initial_value(), Value::Number(20.0));
    }

    #[test]
    fn a_script_starts_at_zero_by_default_and_may_return_any_value_type() {
        let s = spec("generator = \"script\"\nscript = \"value + 2\"");
        assert_eq!(s.initial_value(), Value::Number(0.0));
        assert_eq!(numbers(&run(&s, 2)), vec![2.0, 4.0]);
        let s = spec("generator = \"script\"\nstart = true\nscript = \"!value\"");
        assert_eq!(
            run(&s, 2),
            vec![Value::Bool(false), Value::Bool(true)],
            "a bool flips"
        );
        let s = spec(
            "generator = \"script\"\nscript = 'if seq % 2 == 0 { \"open\" } else { \"closed\" }'",
        );
        assert_eq!(
            run(&s, 2),
            vec![Value::String("open".into()), Value::String("closed".into())]
        );
    }

    #[test]
    fn a_script_sees_seq_and_time() {
        let s = spec("generator = \"script\"\nscript = \"seq * 10 + t\"");
        assert_eq!(numbers(&run(&s, 3)), vec![0.0, 11.0, 22.0]);
    }

    #[test]
    fn noise_applies_to_a_script_and_not_to_its_value() {
        let s = spec("generator = \"script\"\nstart = 10\nnoise = 0.5\nscript = \"value\"");
        let values = numbers(&run(&s, 200));
        assert!(values.iter().all(|v| (9.5..=10.5).contains(v)));
        assert!(values.iter().any(|v| *v != 10.0));
    }

    #[test]
    fn a_random_script_repeats_with_the_same_seed() {
        let s = spec("generator = \"script\"\nscript = \"value + random_range(-1.0, 1.0)\"");
        assert_eq!(run_seeded(&s, 10, 1), run_seeded(&s, 10, 1));
        assert_ne!(run_seeded(&s, 10, 1), run_seeded(&s, 10, 2));
    }

    #[test]
    fn a_failing_script_fails_each_tick_with_the_reason() {
        let s = spec("generator = \"script\"\nscript = \"[value]\"");
        let got = run_seeded(&s, 2, 1);
        assert!(got.iter().all(|r| {
            r.as_ref()
                .is_err_and(|e| e.contains("it must return a number"))
        }));
        let s = spec("generator = \"script\"\nscript = \"value +\"");
        let err = run_seeded(&s, 1, 1).remove(0).unwrap_err();
        assert!(err.starts_with("script does not compile: "), "{err}");
    }

    #[test]
    fn a_script_writes_start_only_when_it_is_not_zero() {
        let s = spec("generator = \"script\"\nscript = \"value\"");
        assert_eq!(
            toml::to_string(&s).unwrap(),
            "generator = \"script\"\nscript = \"value\"\n"
        );
        let s = spec("generator = \"script\"\nstart = 5\nnoise = 1\nscript = \"value\"");
        let written = toml::to_string(&s).unwrap();
        assert_eq!(
            written,
            "generator = \"script\"\nstart = 5.0\nnoise = 1.0\nscript = \"value\"\n"
        );
        assert_eq!(spec(&written), s);
    }

    #[test]
    fn a_script_start_must_be_finite_and_its_noise_positive() {
        let s = spec("generator = \"script\"\nstart = nan\nscript = \"value\"");
        let problem = |s: GeneratorSpec| s.validate().map_err(|e| e.to_string());
        assert_eq!(problem(s), Err("numbers must be finite".into()));
        let s = spec("generator = \"script\"\nnoise = 0\nscript = \"value\"");
        assert_eq!(problem(s), Err("noise must be greater than 0".into()));
        let s = spec("generator = \"script\"\nstart = \"x\"\nnoise = 1\nscript = \"value\"");
        assert_eq!(
            s.validate(),
            Ok(()),
            "noise applies to numeric results only"
        );
    }

    #[test]
    fn unknown_parameter_is_rejected() {
        let err =
            toml::from_str::<GeneratorSpec>("generator = \"uniform\"\nmin = 1\nmax = 2\nstepp = 1")
                .unwrap_err();
        assert!(
            err.message().contains("unknown field `stepp`"),
            "{}",
            err.message()
        );
    }
}
