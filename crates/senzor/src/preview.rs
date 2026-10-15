//! Live preview of a template's topics and payloads with sample values (spec §7.2).

use std::collections::BTreeMap;
use std::time::SystemTime;

use senzor_core::project::{Project, TemplateSpec};
use senzor_core::render::{InstanceTemplates, RenderedMessage, base_context, tick_context};
use senzor_core::value::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    /// The instance whose params and overrides were used, if any device uses the template.
    pub instance: Option<String>,
    /// Set when the templates do not compile; nothing else is rendered then.
    pub compile_error: Option<String>,
    pub availability_topic: Option<Result<String, String>>,
    pub messages: Vec<Result<RenderedMessage, String>>,
}

/// Renders `template` as the first instance that uses it would (index 0 params and overrides),
/// or with no params when no valid device uses it.
pub fn preview(template: &TemplateSpec, project: &Project, now: SystemTime) -> Preview {
    let instance = project.instances().find(|i| i.template_id == template.id);
    let templates =
        match InstanceTemplates::compile(&template.messages, template.availability.as_ref()) {
            Ok(templates) => templates,
            Err(e) => {
                return Preview {
                    instance: instance.map(|i| i.id.clone()),
                    compile_error: Some(e),
                    availability_topic: None,
                    messages: Vec::new(),
                };
            }
        };
    let empty = BTreeMap::new();
    let (device_id, index, params) = match instance {
        Some(i) => (i.id.as_str(), i.index, &i.params),
        None => ("preview", 0, &empty),
    };
    let base = base_context(device_id, index, &template.id, params);
    let values: BTreeMap<String, Value> = template
        .fields
        .iter()
        .map(|(name, spec)| {
            let spec = instance.and_then(|i| i.fields.get(name)).unwrap_or(spec);
            (name.clone(), spec.initial_value())
        })
        .collect();
    let ctx = tick_context(&base, &values, now, 0);
    Preview {
        instance: instance.map(|i| i.id.clone()),
        compile_error: None,
        availability_topic: templates.availability_topic(&base).transpose(),
        messages: templates.render_messages(&ctx),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use senzor_core::project::ProjectFile;
    use std::path::Path;
    use std::time::{Duration, UNIX_EPOCH};

    const SRC: &str = r#"
[[templates]]
id = "air"
availability = { topic = "b/{{ room }}/{{ device_id }}/status", online = "on", offline = "off" }
messages = [{ topic = "b/{{ room }}/{{ device_id }}", payload = '{"co2": {{ co2 }}, "ts": {{ now_ms }}}' }]
fields.co2 = { generator = "constant", value = 600 }

[[devices]]
id = "office"
template = "air"
count = 2
params = { room = "kitchen" }
fields.co2 = { value = 1200 }
"#;

    fn now() -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(5)
    }

    #[test]
    fn renders_as_the_first_instance_using_the_template() {
        let file = ProjectFile::parse(SRC).unwrap();
        let project = file.validate(Path::new(".")).project;
        let p = preview(&file.templates[0], &project, now());
        assert_eq!(p.instance.as_deref(), Some("office-0"));
        assert_eq!(
            p.availability_topic,
            Some(Ok("b/kitchen/office-0/status".into()))
        );
        assert_eq!(
            p.messages,
            vec![Ok(RenderedMessage {
                topic: "b/kitchen/office-0".into(),
                payload: r#"{"co2": 1200, "ts": 5}"#.into(),
            })]
        );
    }

    #[test]
    fn unused_template_renders_without_params_and_reports_what_is_missing() {
        let file = ProjectFile::parse(SRC).unwrap();
        let project = file.validate(Path::new(".")).project;
        let mut template = file.templates[0].clone();
        template.id = "unused".into();
        let p = preview(&template, &project, now());
        assert_eq!(p.instance, None);
        let error = p.messages[0].clone().unwrap_err();
        assert!(error.contains("room"), "{error}");
    }

    #[test]
    fn edited_template_is_previewed_before_it_is_saved() {
        let file = ProjectFile::parse(SRC).unwrap();
        let project = file.validate(Path::new(".")).project;
        let mut template = file.templates[0].clone();
        template.messages[0].payload = "{{ co2 * 2 }}".into();
        let p = preview(&template, &project, now());
        assert_eq!(p.messages[0].as_ref().unwrap().payload, "2400");
    }

    #[test]
    fn syntax_errors_are_reported_without_rendering() {
        let file = ProjectFile::parse(SRC).unwrap();
        let project = file.validate(Path::new(".")).project;
        let mut template = file.templates[0].clone();
        template.messages[0].payload = "{{ co2 ".into();
        let p = preview(&template, &project, now());
        assert!(p.compile_error.unwrap().contains("messages[1].payload"));
        assert!(p.messages.is_empty());
    }
}
