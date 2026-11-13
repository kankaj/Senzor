use std::sync::OnceLock;

use serde::Deserialize;

use super::file::TemplateSpec;
use super::model::SharedTemplates;

/// The built-in templates as TOML, as they ship: what `senzor-mcp` offers agents to read.
pub const BUILTIN_TOML: &str = include_str!("builtin.toml");

#[derive(Deserialize)]
struct BuiltinFile {
    templates: Vec<TemplateSpec>,
}

/// Templates that ship with the app, available by ID without being declared.
///
/// Parsed on first use and shared afterwards, because validation and the GUI ask for them
/// repeatedly (the GUI several times per frame).
pub fn builtin_templates() -> &'static [TemplateSpec] {
    static TEMPLATES: OnceLock<Vec<TemplateSpec>> = OnceLock::new();
    TEMPLATES.get_or_init(|| {
        let file: BuiltinFile = toml::from_str(BUILTIN_TOML).expect("builtin.toml is valid");
        file.templates
    })
}

/// The built-in templates' messages compiled, in the order of [`builtin_templates`]. Compiled on
/// first use and shared by every validation and instance afterwards.
pub(crate) fn builtin_compiled() -> &'static [SharedTemplates] {
    static COMPILED: OnceLock<Vec<SharedTemplates>> = OnceLock::new();
    COMPILED.get_or_init(|| {
        builtin_templates()
            .iter()
            .map(|t| {
                SharedTemplates::compile(&t.messages, t.availability.as_ref())
                    .expect("built-in templates compile")
            })
            .collect()
    })
}

/// The built-in template with this ID.
pub fn builtin_template(id: &str) -> Option<&'static TemplateSpec> {
    builtin_templates().iter().find(|t| t.id == id)
}

pub fn is_builtin(id: &str) -> bool {
    builtin_template(id).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ships_co2_temperature_and_humidity() {
        let ids: Vec<String> = builtin_templates().iter().map(|t| t.id.clone()).collect();
        assert_eq!(ids, ["co2", "temperature", "humidity"]);
        assert!(is_builtin("co2"));
        assert!(!is_builtin("office-air"));
    }

    #[test]
    fn built_in_fields_have_a_unit_and_a_name() {
        let labels: Vec<(&str, Option<&str>, Option<&str>)> = builtin_templates()
            .iter()
            .flat_map(|t| &t.fields)
            .map(|(key, field)| (key.as_str(), field.unit.as_deref(), field.name.as_deref()))
            .collect();
        assert_eq!(
            labels,
            [
                ("co2", Some("ppm"), Some("CO2")),
                ("temperature", Some("°C"), Some("Temperature")),
                ("humidity", Some("%"), Some("Humidity")),
            ]
        );
    }

    #[test]
    fn every_built_in_template_compiles() {
        assert_eq!(builtin_compiled().len(), builtin_templates().len());
    }

    #[test]
    fn templates_are_parsed_once_and_shared() {
        let a = builtin_templates();
        let b = builtin_templates();
        assert!(std::ptr::eq(a.as_ptr(), b.as_ptr()));
    }
}
