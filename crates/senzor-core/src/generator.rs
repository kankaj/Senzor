use std::f64::consts::TAU;
use std::time::{Duration, SystemTime};

use rand::RngExt;
use rand::rngs::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::value::Value;

/// One field's generator as written in the project file (`generator = "..."` plus parameters).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
        #[serde(with = "humantime_serde")]
        period: Duration,
        #[serde(default, with = "humantime_serde")]
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
}

/// Produces a field's value on each tick. Generators implement it now; scripts will later.
pub trait ValueSource: Send {
    fn next(&mut self, ctx: &mut TickContext<'_>) -> Value;
}

impl GeneratorSpec {
    /// Checks parameter constraints. The message names the offending parameter.
    pub fn validate(&self) -> Result<(), String> {
        if self.numbers().iter().any(|n| !n.is_finite()) {
            return Err("numbers must be finite".into());
        }
        if let Some(noise) = self.noise() {
            if noise <= 0.0 {
                return Err("noise must be greater than 0".into());
            }
            let numeric = match self {
                GeneratorSpec::Constant { value, .. } => value.is_number(),
                GeneratorSpec::Sequence { values, .. } => values.iter().all(Value::is_number),
                _ => true,
            };
            if !numeric {
                return Err("noise requires numeric values".into());
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
                    Err("min must be less than max".into())
                } else if !(*min..=*max).contains(start) {
                    Err("start must be between min and max".into())
                } else if *step <= 0.0 {
                    Err("step must be greater than 0".into())
                } else {
                    Ok(())
                }
            }
            GeneratorSpec::Sine { period, .. } if period.is_zero() => {
                Err("period must be greater than 0".into())
            }
            GeneratorSpec::Uniform { min, max, .. } if min >= max => {
                Err("min must be less than max".into())
            }
            GeneratorSpec::Sequence { values, .. } if values.is_empty() => {
                Err("values must not be empty".into())
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
            | GeneratorSpec::Sequence { noise, .. } => *noise,
        }
    }
}

struct Constant(Value);

impl ValueSource for Constant {
    fn next(&mut self, _ctx: &mut TickContext<'_>) -> Value {
        self.0.clone()
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
    fn next(&mut self, ctx: &mut TickContext<'_>) -> Value {
        if self.started {
            let delta = ctx.rng.random_range(-self.step..=self.step);
            self.state = (self.state + delta).clamp(self.min, self.max);
        }
        self.started = true;
        Value::Number(self.state)
    }
}

struct Sine {
    center: f64,
    amplitude: f64,
    period: Duration,
    phase: Duration,
}

impl ValueSource for Sine {
    fn next(&mut self, ctx: &mut TickContext<'_>) -> Value {
        let t = (ctx.elapsed + self.phase).as_secs_f64();
        let angle = TAU * t / self.period.as_secs_f64();
        Value::Number(self.center + self.amplitude * angle.sin())
    }
}

struct Uniform {
    min: f64,
    max: f64,
}

impl ValueSource for Uniform {
    fn next(&mut self, ctx: &mut TickContext<'_>) -> Value {
        Value::Number(ctx.rng.random_range(self.min..self.max))
    }
}

struct Sequence {
    values: Vec<Value>,
    looping: bool,
    next_index: usize,
}

impl ValueSource for Sequence {
    fn next(&mut self, _ctx: &mut TickContext<'_>) -> Value {
        if self.next_index >= self.values.len() {
            if self.looping {
                self.next_index = 0;
            } else {
                return self.values[self.values.len() - 1].clone();
            }
        }
        let value = self.values[self.next_index].clone();
        self.next_index += 1;
        value
    }
}

/// Adds uniform(-noise, +noise) to the inner source's output without touching its state.
struct Noisy {
    inner: Box<dyn ValueSource>,
    noise: f64,
}

impl ValueSource for Noisy {
    fn next(&mut self, ctx: &mut TickContext<'_>) -> Value {
        match self.inner.next(ctx) {
            Value::Number(n) => Value::Number(n + ctx.rng.random_range(-self.noise..=self.noise)),
            other => other,
        }
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
        const TICK: Duration = Duration::from_secs(1);
        let mut rng = instance_rng(Some(7), "test");
        let mut source = spec.build();
        (0..ticks)
            .map(|i| {
                let mut ctx = TickContext {
                    now: SystemTime::UNIX_EPOCH,
                    elapsed: TICK * i as u32,
                    seq: i as u64,
                    rng: &mut rng,
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
            assert_eq!(spec(src).validate(), Err(message.to_string()), "for {src}");
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
