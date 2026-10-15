use std::sync::OnceLock;

use serde::Deserialize;

use super::file::TemplateSpec;

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
        let file: BuiltinFile =
            toml::from_str(include_str!("builtin.toml")).expect("builtin.toml is valid");
        file.templates
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
    fn templates_are_parsed_once_and_shared() {
        let a = builtin_templates();
        let b = builtin_templates();
        assert!(std::ptr::eq(a.as_ptr(), b.as_ptr()));
    }
}
