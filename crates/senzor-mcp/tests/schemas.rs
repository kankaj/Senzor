//! The JSON Schemas derived for the project-file types must accept what the project file accepts,
//! or the MCP tools would advertise inputs that do not match what they parse.

use std::path::{Path, PathBuf};

use schemars::{JsonSchema, schema_for};
use senzor_core::project::builtin::builtin_templates;
use senzor_core::project::{ProjectFile, TemplateSpec};
use serde_json::Value;

fn assert_valid<T: JsonSchema>(instance: &Value, what: &str) {
    let schema = serde_json::to_value(schema_for!(T)).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errors: Vec<String> = validator
        .iter_errors(instance)
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{what}: {errors:#?}\n{instance:#}");
}

fn examples() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    paths.sort();
    assert!(!paths.is_empty());
    paths
}

#[test]
fn every_example_as_written_matches_the_project_schema() {
    for path in examples() {
        let text = std::fs::read_to_string(&path).unwrap();
        let raw: toml::Table = toml::from_str(&text).unwrap();
        assert_valid::<ProjectFile>(
            &serde_json::to_value(raw).unwrap(),
            &path.display().to_string(),
        );
    }
}

#[test]
fn every_example_as_saved_matches_the_project_schema() {
    for path in examples() {
        let file = ProjectFile::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_valid::<ProjectFile>(
            &serde_json::to_value(&file).unwrap(),
            &path.display().to_string(),
        );
    }
}

#[test]
fn every_builtin_template_matches_the_template_schema() {
    for template in builtin_templates() {
        assert_valid::<TemplateSpec>(&serde_json::to_value(template).unwrap(), &template.id);
    }
}

#[test]
fn json_that_matches_the_schema_parses_like_toml() {
    for path in examples() {
        let text = std::fs::read_to_string(&path).unwrap();
        let from_toml = ProjectFile::parse(&text).unwrap();
        let json = serde_json::to_value(&from_toml).unwrap();
        let from_json: ProjectFile = serde_json::from_value(json).unwrap();
        assert_eq!(from_json, from_toml, "{}", path.display());
    }
}

#[test]
fn the_schema_rejects_what_the_file_rejects() {
    let schema = serde_json::to_value(schema_for!(ProjectFile)).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for bad in [
        serde_json::json!({"devices": [{"id": "a", "template": "co2", "cnt": 3}]}),
        serde_json::json!({"templates": [{"id": "t", "messages": [], "fields": {"v": {"generator": "nope"}}}]}),
        serde_json::json!({"broker": {"keep_alive": 30}}),
    ] {
        assert!(!validator.is_valid(&bad), "accepted {bad}");
        assert!(serde_json::from_value::<ProjectFile>(bad).is_err());
    }
}
