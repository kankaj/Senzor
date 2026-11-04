//! JSON Schemas for types that have no `JsonSchema` implementation of their own.

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};

/// A device's field override: a partial generator table (`{ start = 1200 }`).
pub(crate) struct TomlTable;

impl JsonSchema for TomlTable {
    fn schema_name() -> Cow<'static, str> {
        "FieldOverride".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "object",
            "description": "Generator parameters that replace the template field's, e.g. {\"start\": 1200}"
        })
    }
}
