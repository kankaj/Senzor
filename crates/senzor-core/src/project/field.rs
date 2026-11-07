use std::fmt;

use serde::de::value::{MapAccessDeserializer, StringDeserializer};
use serde::de::{self, DeserializeSeed, IntoDeserializer, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::generator::GeneratorSpec;

/// One template field as written in the project file: its generator and, optionally, the unit
/// and readable name people see (`unit = "ppm"`, `name = "CO2"`), in one flat table.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldSpec {
    pub generator: GeneratorSpec,
    /// Free text shown after the value, e.g. `ppm`, `°C` or `%`.
    pub unit: Option<String>,
    /// A readable name, e.g. `CO2` or `Temperature`; without one, people see the field's key.
    pub name: Option<String>,
}

impl FieldSpec {
    /// The keys of a field's table that are not the generator's.
    pub(crate) const LABEL_KEYS: [&str; 2] = ["unit", "name"];

    /// A field with this generator and no unit or name.
    pub fn new(generator: GeneratorSpec) -> Self {
        Self {
            generator,
            unit: None,
            name: None,
        }
    }

    /// Every problem with the field; each message names the offending key.
    pub fn problems(&self) -> Vec<String> {
        let mut problems: Vec<String> = self.generator.validate().err().into_iter().collect();
        for (key, label) in [("unit", &self.unit), ("name", &self.name)] {
            if label.as_deref() == Some("") {
                problems.push(format!("{key} must not be empty"));
            }
        }
        problems
    }

    /// What people call the field whose key is `key`: its `name`, or else the key.
    pub fn display_name<'a>(&'a self, key: &'a str) -> &'a str {
        self.name.as_deref().unwrap_or(key)
    }
}

impl From<GeneratorSpec> for FieldSpec {
    fn from(generator: GeneratorSpec) -> Self {
        Self::new(generator)
    }
}

/// Writes one flat table: the generator's keys, then `unit` and `name` when set.
impl Serialize for FieldSpec {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Flat<'a> {
            #[serde(flatten)]
            generator: &'a GeneratorSpec,
            #[serde(skip_serializing_if = "Option::is_none")]
            unit: Option<&'a str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            name: Option<&'a str>,
        }
        Flat {
            generator: &self.generator,
            unit: self.unit.as_deref(),
            name: self.name.as_deref(),
        }
        .serialize(serializer)
    }
}

/// Takes `unit` and `name` out of the table as it is read and hands every other key to
/// [`GeneratorSpec`], so unknown keys stay errors and errors keep their position in the file.
impl<'de> Deserialize<'de> for FieldSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(FieldVisitor)
    }
}

struct FieldVisitor;

impl<'de> Visitor<'de> for FieldVisitor {
    type Value = FieldSpec;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a field table with a `generator` key")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<FieldSpec, A::Error> {
        let mut labels = Labels::default();
        let generator = GeneratorSpec::deserialize(MapAccessDeserializer::new(WithoutLabels {
            map,
            labels: &mut labels,
        }))?;
        Ok(FieldSpec {
            generator,
            unit: labels.unit,
            name: labels.name,
        })
    }
}

#[derive(Default)]
struct Labels {
    unit: Option<String>,
    name: Option<String>,
}

/// A field table without its `unit` and `name`, which it reads into `labels` on the way.
struct WithoutLabels<'a, A> {
    map: A,
    labels: &'a mut Labels,
}

impl<'de, A: MapAccess<'de>> MapAccess<'de> for WithoutLabels<'_, A> {
    type Error = A::Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, A::Error> {
        while let Some(key) = self.map.next_key::<String>()? {
            let (slot, label) = match key.as_str() {
                "unit" => (&mut self.labels.unit, "unit"),
                "name" => (&mut self.labels.name, "name"),
                _ => {
                    let key: StringDeserializer<A::Error> = key.into_deserializer();
                    return seed.deserialize(key).map(Some);
                }
            };
            if slot.is_some() {
                return Err(de::Error::duplicate_field(label));
            }
            *slot = Some(self.map.next_value()?);
        }
        Ok(None)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, A::Error> {
        self.map.next_value_seed(seed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(src: &str) -> FieldSpec {
        toml::from_str(src).unwrap()
    }

    #[test]
    fn unit_and_name_sit_next_to_the_generator_keys() {
        let spec =
            field("generator = \"uniform\"\nmin = 1\nmax = 2\nunit = \"ppm\"\nname = \"CO2\"");
        assert_eq!(spec.unit.as_deref(), Some("ppm"));
        assert_eq!(spec.name.as_deref(), Some("CO2"));
        assert_eq!(
            spec.generator,
            GeneratorSpec::Uniform {
                min: 1.0,
                max: 2.0,
                noise: None
            }
        );
    }

    #[test]
    fn unit_and_name_are_optional() {
        let spec = field("generator = \"uniform\"\nmin = 1\nmax = 2");
        assert_eq!((spec.unit, spec.name), (None, None));
    }

    #[test]
    fn the_display_name_falls_back_to_the_key() {
        let mut spec = field("generator = \"constant\"\nvalue = 1");
        assert_eq!(spec.display_name("co2"), "co2");
        spec.name = Some("CO2".into());
        assert_eq!(spec.display_name("co2"), "CO2");
    }

    #[test]
    fn problems_name_the_empty_key_after_the_generator_problem() {
        let mut spec = field("generator = \"uniform\"\nmin = 2\nmax = 1");
        spec.unit = Some(String::new());
        spec.name = Some(String::new());
        assert_eq!(
            spec.problems(),
            [
                "min must be less than max",
                "unit must not be empty",
                "name must not be empty"
            ]
        );
        spec.unit = Some(" ".into());
        spec.name = Some("x".into());
        assert_eq!(spec.problems(), ["min must be less than max"]);
    }

    #[test]
    fn unknown_keys_are_still_rejected() {
        let err =
            toml::from_str::<FieldSpec>("generator = \"uniform\"\nmin = 1\nmax = 2\nunits = \"x\"")
                .unwrap_err();
        assert!(
            err.message().contains("unknown field `units`"),
            "{}",
            err.message()
        );
    }

    #[test]
    fn a_unit_must_be_text() {
        let err =
            toml::from_str::<FieldSpec>("generator = \"uniform\"\nmin = 1\nmax = 2\nunit = 5")
                .unwrap_err();
        assert!(
            err.message()
                .contains("invalid type: integer `5`, expected a string"),
            "{}",
            err.message()
        );
    }

    #[test]
    fn a_unit_alone_still_needs_a_generator() {
        let err = toml::from_str::<FieldSpec>("unit = \"ppm\"").unwrap_err();
        assert_eq!(err.message(), "missing field `generator`");
    }

    #[test]
    fn unit_and_name_are_written_after_the_generator_keys_and_only_when_set() {
        let mut spec = field("generator = \"uniform\"\nmin = 1\nmax = 2");
        let plain = toml::to_string(&spec).unwrap();
        assert_eq!(plain, "generator = \"uniform\"\nmin = 1.0\nmax = 2.0\n");
        spec.unit = Some("%".into());
        spec.name = Some("Humidity".into());
        let written = toml::to_string(&spec).unwrap();
        assert_eq!(
            written,
            "generator = \"uniform\"\nmin = 1.0\nmax = 2.0\nunit = \"%\"\nname = \"Humidity\"\n"
        );
        assert_eq!(field(&written), spec);
    }

    #[test]
    fn json_reads_and_writes_the_same_keys() {
        let json = serde_json::json!({
            "generator": "constant", "value": 21.5, "unit": "°C", "name": "Temperature"
        });
        let spec: FieldSpec = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(spec.unit.as_deref(), Some("°C"));
        assert_eq!(serde_json::to_value(&spec).unwrap(), json);
    }

    #[test]
    fn json_with_a_repeated_unit_is_rejected() {
        let err = serde_json::from_str::<FieldSpec>(
            r#"{"generator": "constant", "value": 1, "unit": "a", "unit": "b"}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("duplicate field `unit`"), "{err}");
    }
}
