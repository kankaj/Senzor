//! JSON Schemas for types that have no `JsonSchema` implementation of their own.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};

use crate::generator::GeneratorSpec;
use crate::project::FieldSpec;

/// A device's field override: a partial field table (`{ start = 1200 }`).
pub(crate) struct TomlTable;

impl JsonSchema for TomlTable {
    fn schema_name() -> Cow<'static, str> {
        "FieldOverride".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "object",
            "description": "Generator parameters, unit or name that replace the template field's, e.g. {\"start\": 1200}"
        })
    }
}

/// The generator's schema, one object per generator, with `unit` and `name` added to each.
impl JsonSchema for FieldSpec {
    fn schema_name() -> Cow<'static, str> {
        "FieldSpec".into()
    }

    fn schema_id() -> Cow<'static, str> {
        concat!(module_path!(), "::FieldSpec").into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let mut schema = GeneratorSpec::json_schema(generator);
        schema.insert(
            "description".into(),
            "One field: its generator (`generator` plus parameters), and optionally a unit and \
             a readable name."
                .into(),
        );
        let labels = [
            (
                "unit",
                json_schema!({
                    "type": "string",
                    "description": "Free text shown after the value, e.g. \"ppm\", \"°C\" or \"%\"."
                }),
            ),
            (
                "name",
                json_schema!({
                    "type": "string",
                    "description": "A readable name, e.g. \"CO2\"; without one, the field's key is shown."
                }),
            ),
        ];
        let variants = schema
            .get_mut("oneOf")
            .and_then(|v| v.as_array_mut())
            .expect("an internally tagged enum's schema is a oneOf");
        for variant in variants {
            let properties = variant
                .get_mut("properties")
                .and_then(|p| p.as_object_mut())
                .expect("every generator is an object with properties");
            for (key, label) in &labels {
                properties.insert((*key).into(), label.clone().to_value());
            }
        }
        schema
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::schema_for;

    #[test]
    fn every_generator_takes_a_unit_and_a_name() {
        let schema = schema_for!(FieldSpec).to_value();
        let variants = schema["oneOf"].as_array().unwrap();
        assert_eq!(variants.len(), 5);
        for variant in variants {
            assert_eq!(variant["additionalProperties"], false, "{variant}");
            for key in ["unit", "name"] {
                assert_eq!(variant["properties"][key]["type"], "string", "{variant}");
            }
        }
    }
}
