//! Live preview of a template's topics and payloads with sample values (spec §7.2).

use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

use senzor_core::encoding::{Encoder, Encoding, message_encoder};
use senzor_core::project::{Project, TemplateSpec};
use senzor_core::render::{InstanceTemplates, RenderedMessage, base_context, tick_context};
use senzor_core::value::Value;

use crate::i18n::t;

#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    /// The instance whose params and overrides were used, if any device uses the template.
    pub instance: Option<String>,
    /// Set when the templates do not compile; nothing else is rendered then.
    pub compile_error: Option<String>,
    pub availability_topic: Option<Result<String, String>>,
    pub messages: Vec<Result<PreviewMessage, String>>,
}

/// One rendered message of a preview.
#[derive(Debug, Clone, PartialEq)]
pub struct PreviewMessage {
    /// The message with its payload as rendered text.
    pub rendered: RenderedMessage,
    /// For a message with a binary encoding: the bytes its text encodes to, or why it does not.
    pub encoded: Option<Result<Vec<u8>, String>>,
}

/// Renders `template` as the first instance that uses it would (index 0 params and overrides),
/// or with no params when no valid device uses it. `.proto` files are read relative to
/// `folder`, the project's folder.
pub fn preview(
    template: &TemplateSpec,
    project: &Project,
    folder: &Path,
    now: SystemTime,
) -> Preview {
    let instance = project.instances().find(|i| i.template_id == template.id);
    // Rendered as text: each binary message is encoded below, so its text shows even when it
    // does not encode.
    let text = vec![Encoder::Text; template.messages.len()];
    let templates = match InstanceTemplates::compile_with(
        &template.messages,
        template.availability.as_ref(),
        text,
    ) {
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
            (name.clone(), spec.generator.initial_value())
        })
        .collect();
    let ctx = tick_context(&base, &values, now, 0);
    let messages = templates
        .render_messages(&ctx)
        .into_iter()
        .zip(template.messages.iter().enumerate())
        .map(|(rendered, (i, spec))| {
            rendered.map(|rendered| {
                let encoded = (!spec.encoding.is_text()).then(|| {
                    message_encoder(spec, i, folder)
                        .and_then(|encoder| encoder.encode(rendered.payload.text()))
                });
                PreviewMessage { rendered, encoded }
            })
        })
        .collect();
    Preview {
        instance: instance.map(|i| i.id.clone()),
        compile_error: None,
        availability_topic: templates.availability_topic(&base).transpose(),
        messages,
    }
}

/// Why `payload` should be JSON but is not: it starts with `{` or `[`, the message's
/// `content_type` names JSON, or its `encoding` (CBOR, protobuf) reads JSON. `None` when it
/// parses or nothing says it should be JSON.
pub fn json_problem(
    payload: &str,
    content_type: Option<&str>,
    encoding: Encoding,
) -> Option<String> {
    let looks_like_json = matches!(payload.trim_start().chars().next(), Some('{' | '['));
    let declared = content_type.is_some_and(|t| t.to_ascii_lowercase().contains("json"));
    let needed = matches!(encoding, Encoding::Cbor | Encoding::Protobuf);
    if !looks_like_json && !declared && !needed {
        return None;
    }
    let error = serde_json::from_str::<serde_json::Value>(payload)
        .err()?
        .to_string();
    Some(if needed {
        t!(
            "preview-json-needed",
            encoding = encoding.name(),
            error = error
        )
    } else if looks_like_json {
        t!("preview-json-looks", error = error)
    } else {
        t!("preview-json-content-type", error = error)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_payload_that_starts_like_json_must_parse() {
        assert_eq!(json_problem(r#"{"co2": 600}"#, None, Encoding::Text), None);
        assert_eq!(json_problem(" [1, 2] ", None, Encoding::Text), None);
        let problem = json_problem(r#"{"co2": 600,}"#, None, Encoding::Text).unwrap();
        assert!(
            problem.starts_with("looks like JSON but is not valid JSON: "),
            "{problem}"
        );
        assert!(json_problem("[1, 2", None, Encoding::Text).is_some());
    }

    #[test]
    fn the_json_problem_is_told_in_czech() {
        crate::i18n::set_language(crate::i18n::Language::Czech);
        let problem = json_problem("co2=600", None, Encoding::Cbor).unwrap();
        assert!(
            problem.starts_with("kódování CBOR potřebuje JSON, ale payload není platný JSON: "),
            "{problem}"
        );
    }

    #[test]
    fn a_payload_that_does_not_look_like_json_is_left_alone() {
        assert_eq!(json_problem("600", None, Encoding::Text), None);
        assert_eq!(json_problem("on", None, Encoding::Text), None);
    }

    #[test]
    fn a_json_content_type_asks_for_json() {
        let problem = json_problem("600 ppm", Some("application/json"), Encoding::Text).unwrap();
        assert!(
            problem.starts_with("the content type is JSON, but the payload is not valid JSON: "),
            "{problem}"
        );
        assert_eq!(
            json_problem("600", Some("application/json"), Encoding::Text),
            None,
            "a number is JSON"
        );
        assert_eq!(
            json_problem("600 ppm", Some("text/plain"), Encoding::Text),
            None
        );
    }
    use senzor_core::project::ProjectFile;
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
        let p = preview(&file.templates[0], &project, Path::new("."), now());
        assert_eq!(p.instance.as_deref(), Some("office-0"));
        assert_eq!(
            p.availability_topic,
            Some(Ok("b/kitchen/office-0/status".into()))
        );
        assert_eq!(
            p.messages,
            vec![Ok(PreviewMessage {
                rendered: RenderedMessage {
                    topic: "b/kitchen/office-0".into(),
                    payload: r#"{"co2": 1200, "ts": 5}"#.into(),
                    properties: Default::default(),
                },
                encoded: None,
            })]
        );
    }

    #[test]
    fn unused_template_renders_without_params_and_reports_what_is_missing() {
        let file = ProjectFile::parse(SRC).unwrap();
        let project = file.validate(Path::new(".")).project;
        let mut template = file.templates[0].clone();
        template.id = "unused".into();
        let p = preview(&template, &project, Path::new("."), now());
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
        let p = preview(&template, &project, Path::new("."), now());
        assert_eq!(p.messages[0].as_ref().unwrap().rendered.payload, "2400");
    }

    #[test]
    fn syntax_errors_are_reported_without_rendering() {
        let file = ProjectFile::parse(SRC).unwrap();
        let project = file.validate(Path::new(".")).project;
        let mut template = file.templates[0].clone();
        template.messages[0].payload = "{{ co2 ".into();
        let p = preview(&template, &project, Path::new("."), now());
        assert!(p.compile_error.unwrap().contains("messages[1].payload"));
        assert!(p.messages.is_empty());
    }

    #[test]
    fn cbor_and_protobuf_payloads_must_be_json() {
        let problem = json_problem("co2=600", None, Encoding::Cbor).unwrap();
        assert!(
            problem.starts_with("encoding CBOR needs JSON, but the payload is not valid JSON: "),
            "{problem}"
        );
        assert!(json_problem("600 ppm", None, Encoding::Protobuf).is_some());
        assert_eq!(
            json_problem(r#"{"co2": 600}"#, None, Encoding::Protobuf),
            None
        );
        assert_eq!(json_problem("00d7", None, Encoding::Hex), None);
    }

    /// The preview of the template of `SRC` with its message changed by `edit`, with `.proto`
    /// files read from `folder`.
    fn preview_edited(
        edit: impl FnOnce(&mut senzor_core::project::MessageSpec),
        folder: &Path,
    ) -> Preview {
        let file = ProjectFile::parse(SRC).unwrap();
        let project = file.validate(Path::new(".")).project;
        let mut template = file.templates[0].clone();
        edit(&mut template.messages[0]);
        preview(&template, &project, folder, now())
    }

    #[test]
    fn a_binary_message_shows_its_text_and_the_bytes_it_encodes_to() {
        let p = preview_edited(
            |m| {
                m.payload = "{{ co2 | u16be }}".into();
                m.encoding = Encoding::Hex;
            },
            Path::new("."),
        );
        let message = p.messages[0].as_ref().unwrap();
        assert_eq!(message.rendered.payload, "04b0", "rendered as text");
        assert_eq!(message.encoded, Some(Ok(vec![0x04, 0xb0])));
    }

    #[test]
    fn a_payload_that_does_not_encode_shows_why() {
        let p = preview_edited(|m| m.encoding = Encoding::Base64, Path::new("."));
        let message = p.messages[0].as_ref().unwrap();
        assert!(
            matches!(&message.encoded, Some(Err(e)) if e.contains("base64")),
            "{message:?}"
        );
    }

    #[test]
    fn a_protobuf_message_reads_its_proto_file_from_the_project_folder() {
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../senzor-core/testdata/proto");
        let p = preview_edited(
            |m| {
                m.payload = r#"{"humidity": 40}"#.into();
                m.encoding = Encoding::Protobuf;
                m.proto = Some("sensor.proto".into());
                m.message = Some("sensors.v1.Reading".into());
            },
            &folder,
        );
        assert_eq!(
            p.messages[0].as_ref().unwrap().encoded,
            Some(Ok(vec![0x18, 40]))
        );
        let elsewhere = preview_edited(
            |m| {
                m.encoding = Encoding::Protobuf;
                m.proto = Some("sensor.proto".into());
                m.message = Some("sensors.v1.Reading".into());
            },
            Path::new("."),
        );
        let encoded = elsewhere.messages[0].as_ref().unwrap().encoded.clone();
        assert!(
            matches!(&encoded, Some(Err(e)) if e.contains("cannot read proto `sensor.proto`")),
            "{encoded:?}"
        );
    }
}
