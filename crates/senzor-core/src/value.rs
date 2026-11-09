use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

/// Integral numbers below this magnitude are treated as integers: a round bound just under 2^53,
/// up to which `f64` represents every integer exactly (and far inside the `i64` range).
const MAX_EXACT_INTEGER: f64 = 9.0e15;

/// A field or param value as written in the project file and produced by generators.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum Value {
    Bool(bool),
    Number(f64),
    String(String),
}

/// By hand rather than `untagged`, so a list or a table is refused with what is allowed instead
/// of "did not match any variant".
impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ValueVisitor;

        impl Visitor<'_> for ValueVisitor {
            type Value = Value;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a number, a boolean or a string")
            }

            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
                Ok(Value::Bool(v))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
                Ok(Value::Number(v as f64))
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
                Ok(Value::Number(v as f64))
            }

            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
                Ok(Value::Number(v))
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
                Ok(Value::String(v.to_string()))
            }

            fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
                Ok(Value::String(v))
            }
        }

        deserializer.deserialize_any(ValueVisitor)
    }
}

impl Value {
    pub fn is_number(&self) -> bool {
        matches!(self, Value::Number(_))
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Parses text a person typed: a finite number, then `true`/`false`, otherwise the text itself.
    pub fn parse_input(input: &str) -> Value {
        let trimmed = input.trim();
        if let Ok(n) = trimmed.parse::<f64>()
            && n.is_finite()
        {
            return Value::Number(n);
        }
        match trimmed {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => Value::String(input.to_string()),
        }
    }

    /// The number as an `i64` when it is integral and within the range where `f64` represents
    /// every integer exactly.
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Number(n) if n.fract() == 0.0 && n.abs() < MAX_EXACT_INTEGER => Some(*n as i64),
            _ => None,
        }
    }

    /// The value as text to edit, integral numbers without `.0`. [`Value::parse_input`] turns it
    /// back into this value, except text that itself reads as a number or bool (`"12"` comes back
    /// as the number 12).
    pub fn to_input(&self) -> String {
        match self {
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => match self.as_integer() {
                Some(i) => i.to_string(),
                None => n.to_string(),
            },
            Value::String(s) => s.clone(),
        }
    }

    /// Converts to a template value. Integral numbers become integers so that
    /// `params = { floor = 3 }` renders as `3`, not `3.0`.
    pub fn to_jinja(&self) -> minijinja::Value {
        match self {
            Value::Bool(b) => minijinja::Value::from(*b),
            Value::Number(n) => match self.as_integer() {
                Some(i) => minijinja::Value::from(i),
                None => minijinja::Value::from(*n),
            },
            Value::String(s) => minijinja::Value::from(s.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integral_numbers_render_without_decimal_point() {
        assert_eq!(Value::Number(3.0).to_jinja().to_string(), "3");
        assert_eq!(Value::Number(-1200.0).to_jinja().to_string(), "-1200");
    }

    #[test]
    fn fractional_numbers_keep_their_fraction() {
        assert_eq!(Value::Number(22.5).to_jinja().to_string(), "22.5");
    }

    #[test]
    fn bools_and_strings_convert() {
        assert_eq!(Value::Bool(true).to_jinja(), minijinja::Value::from(true));
        assert_eq!(Value::String("open".into()).to_jinja().to_string(), "open");
    }

    #[test]
    fn parse_input_prefers_numbers_then_bools_then_text() {
        assert_eq!(Value::parse_input("2500"), Value::Number(2500.0));
        assert_eq!(Value::parse_input(" 22.5 "), Value::Number(22.5));
        assert_eq!(Value::parse_input("true"), Value::Bool(true));
        assert_eq!(Value::parse_input("false"), Value::Bool(false));
        assert_eq!(Value::parse_input("open"), Value::String("open".into()));
        assert_eq!(Value::parse_input(""), Value::String(String::new()));
    }

    #[test]
    fn parse_input_keeps_non_finite_words_as_text() {
        assert_eq!(Value::parse_input("nan"), Value::String("nan".into()));
        assert_eq!(Value::parse_input("inf"), Value::String("inf".into()));
    }

    #[test]
    fn to_input_round_trips() {
        for value in [
            Value::Number(3.0),
            Value::Number(-0.25),
            Value::Bool(false),
            Value::String("office".into()),
        ] {
            assert_eq!(Value::parse_input(&value.to_input()), value);
        }
        assert_eq!(Value::Number(3.0).to_input(), "3");
    }

    #[test]
    fn deserializes_mixed_toml_values() {
        #[derive(Deserialize)]
        struct Wrapper {
            values: Vec<Value>,
        }
        let parsed: Wrapper = toml::from_str(r#"values = [3, 2.5, true, "open"]"#).unwrap();
        assert_eq!(
            parsed.values,
            vec![
                Value::Number(3.0),
                Value::Number(2.5),
                Value::Bool(true),
                Value::String("open".into()),
            ]
        );
    }
}
