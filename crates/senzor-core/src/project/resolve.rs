use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::time::SystemTime;

use super::broker::{EnvLookup, broker_config, needs_mqtt5, resolve_broker, whole_seconds};
use super::builtin::{builtin_compiled, builtin_templates, is_builtin};
use super::error::ProjectError;
use super::field::FieldSpec;
use super::file::{
    BrokerSpec, DeviceSpec, MqttVersion, ProjectFile, TemplateSpec, default_instance_id_pattern,
};
use super::model::{
    DeviceConfig, InstanceConfig, Project, SharedTemplates, Subscription, Validated,
};
use crate::encoding::{Encoder, message_encoder_located};
use crate::i18n::{Text, text};
use crate::render::{
    CompiledTemplate, TemplateError, base_context, instance_id_context, is_reserved, tick_context,
};
use crate::topic::{FilterProblem, check_filter};
use crate::value::Value;

/// A template's messages and subscription topics, compiled once for all the devices that use it.
struct Compiled {
    templates: SharedTemplates,
    /// One per subscription, in order.
    subscription_topics: Vec<CompiledTemplate>,
}

/// Whether devices can use a template.
enum TemplateStatus<'a> {
    Usable(&'a TemplateSpec, Compiled),
    /// The template has errors; its devices are blocked without an error of their own.
    Broken,
}

impl ProjectFile {
    /// Semantic validation with the real environment. `base_dir` is the project file's directory.
    pub fn validate(&self, base_dir: &Path) -> Validated {
        self.validate_with(base_dir, &|name| std::env::var(name).ok())
    }

    pub fn validate_with(&self, base_dir: &Path, env: EnvLookup<'_>) -> Validated {
        let mut errors = Vec::new();

        let broker = match resolve_broker(&self.broker, base_dir, env) {
            Ok(broker) => Some(broker),
            Err(mut e) => {
                errors.append(&mut e);
                None
            }
        };
        let templates = self.template_lookup(base_dir, &mut errors);
        let (mut devices, mut blocked) = self.resolve_devices(&templates, &mut errors);

        let broker = match broker {
            Some(broker) => broker,
            // A `Project` always has a broker. With every device blocked nothing connects to it,
            // so the settings without credentials and TLS are enough.
            None => {
                blocked.extend(self.devices.iter().map(|d| d.id.clone()));
                devices.clear();
                broker_config(&self.broker, None, None)
            }
        };

        Validated {
            project: Project { broker, devices },
            errors,
            blocked,
        }
    }

    /// Checks the project's templates and returns every known template ID with whether devices
    /// can use it.
    fn template_lookup<'a>(
        &'a self,
        base_dir: &Path,
        errors: &mut Vec<ProjectError>,
    ) -> HashMap<String, TemplateStatus<'a>> {
        let mut templates: HashMap<String, TemplateStatus<'a>> = builtin_templates()
            .iter()
            .zip(builtin_compiled())
            .map(|(t, templates)| {
                let compiled = Compiled {
                    templates: templates.clone(),
                    subscription_topics: Vec::new(),
                };
                (t.id.clone(), TemplateStatus::Usable(t, compiled))
            })
            .collect();
        for template in &self.templates {
            if is_builtin(&template.id) {
                let text = text!("template-builtin-id", id = &template.id);
                errors.push(ProjectError::template(&template.id, text).at("id"));
                continue;
            }
            if templates.contains_key(&template.id) {
                let text = text!("template-duplicate-id");
                errors.push(ProjectError::template(&template.id, text).at("id"));
                templates.insert(template.id.clone(), TemplateStatus::Broken);
                continue;
            }
            let status = match check_template(template, &self.broker, base_dir) {
                Ok(compiled) => TemplateStatus::Usable(template, compiled),
                Err(mut problems) => {
                    errors.append(&mut problems);
                    TemplateStatus::Broken
                }
            };
            templates.insert(template.id.clone(), status);
        }
        templates
    }

    /// Resolves the devices in file order. Returns the devices that can run and the IDs of those
    /// that cannot; the reasons go to `errors`.
    fn resolve_devices(
        &self,
        templates: &HashMap<String, TemplateStatus<'_>>,
        errors: &mut Vec<ProjectError>,
    ) -> (Vec<DeviceConfig>, BTreeSet<String>) {
        let mut id_counts: HashMap<&str, usize> = HashMap::new();
        for spec in &self.devices {
            *id_counts.entry(spec.id.as_str()).or_default() += 1;
        }

        let mut devices = Vec::new();
        let mut blocked = BTreeSet::new();
        let mut instance_owners: HashMap<String, String> = HashMap::new();
        for spec in &self.devices {
            let resolved = if id_counts[spec.id.as_str()] == 1 {
                resolve_entry(spec, templates, &instance_owners, self.simulation.seed)
            } else if blocked.contains(&spec.id) {
                // Reported at the first entry with this ID.
                Err(Vec::new())
            } else {
                Err(vec![
                    ProjectError::device(&spec.id, text!("device-duplicate-id")).at("id"),
                ])
            };
            match resolved {
                Ok(device) => {
                    for instance in &device.instances {
                        instance_owners.insert(instance.id.clone(), spec.id.clone());
                    }
                    devices.push(device);
                }
                Err(mut problems) => {
                    errors.append(&mut problems);
                    blocked.insert(spec.id.clone());
                }
            }
        }
        (devices, blocked)
    }
}

/// Resolves one `[[devices]]` entry whose ID is unique. `instance_owners` maps the instance IDs
/// of the devices accepted so far to their device. `Err` lists the entry's problems; an empty
/// list means its template is broken, which that template's own errors already explain.
fn resolve_entry(
    spec: &DeviceSpec,
    templates: &HashMap<String, TemplateStatus<'_>>,
    instance_owners: &HashMap<String, String>,
    seed: Option<u64>,
) -> Result<DeviceConfig, Vec<ProjectError>> {
    let (template, compiled) = match templates.get(spec.template.as_str()) {
        Some(TemplateStatus::Usable(template, compiled)) => (*template, compiled),
        Some(TemplateStatus::Broken) => return Err(Vec::new()),
        None => {
            let text = text!("device-unknown-template", template = &spec.template);
            return Err(vec![ProjectError::device(&spec.id, text).at("template")]);
        }
    };
    let device = resolve_device(spec, template, compiled, seed)?;
    match instance_clash(&device, instance_owners) {
        Some(message) => Err(vec![
            ProjectError::device(&spec.id, message).at("instance_id"),
        ]),
        None => Ok(device),
    }
}

/// The error for the first instance of `device` whose ID another device already uses.
fn instance_clash(device: &DeviceConfig, owners: &HashMap<String, String>) -> Option<Text> {
    device.instances.iter().find_map(|instance| {
        owners
            .get(&instance.id)
            .map(|owner| text!("instance-id-taken", id = &instance.id, device = owner))
    })
}

/// Checks `template` and compiles its messages and subscription topics; `Err` lists its problems.
/// `.proto` files are read relative to `base_dir`.
fn check_template(
    template: &TemplateSpec,
    broker: &BrokerSpec,
    base_dir: &Path,
) -> Result<Compiled, Vec<ProjectError>> {
    let at = |field: &str, text: Text| ProjectError::template(&template.id, text).at(field);
    let mut problems = check_properties(template, broker);
    if template.messages.is_empty() {
        problems.push(at("messages", text!("template-no-messages")));
    }
    if template.fields.is_empty() {
        problems.push(at("fields", text!("template-no-fields")));
    }
    if template.qos > 2 {
        problems.push(at("qos", text!("qos-range", qos = template.qos)));
    }
    if template.interval.is_zero() {
        problems.push(at("interval", text!("greater-than-zero", key = "interval")));
    }
    for (name, spec) in &template.fields {
        let field = format!("fields.{name}");
        if is_reserved(name) {
            problems.push(at(&field, text!("field-reserved-name", name = name)));
        }
        let field_problem = |problem: Text| text!("field-problem", name = name, problem = problem);
        for e in spec.problems() {
            problems.push(at(&field, field_problem(e)));
        }
        if let Some(e) = spec.script_problem() {
            problems.push(at(&format!("{field}.script"), field_problem(e)));
        }
    }
    let subscription_topics = check_subscriptions(template, &mut problems);
    let encoders = template
        .messages
        .iter()
        .enumerate()
        .map(|(i, message)| {
            message_encoder_located(message, i, base_dir).unwrap_or_else(|located| {
                problems.extend(located.into_iter().map(|e| at(&e.field, e.message)));
                Encoder::Text
            })
        })
        .collect();
    match SharedTemplates::compile_with_located(
        &template.messages,
        template.availability.as_ref(),
        encoders,
    ) {
        Ok(templates) if problems.is_empty() => Ok(Compiled {
            templates,
            subscription_topics,
        }),
        Ok(_) => Err(problems),
        Err(e) => {
            problems.push(at(&e.field, e.message));
            Err(problems)
        }
    }
}

/// Checks the qos and script of each of `template`'s subscriptions and compiles its topic.
/// Returns the compiled topics; the problems go to `problems`.
fn check_subscriptions(
    template: &TemplateSpec,
    problems: &mut Vec<ProjectError>,
) -> Vec<CompiledTemplate> {
    let mut topics = Vec::with_capacity(template.subscriptions.len());
    for (i, subscription) in template.subscriptions.iter().enumerate() {
        let n = i + 1;
        let mut at = |key: &str, text: Text| {
            problems.push(
                ProjectError::template(&template.id, text).at(format!("subscriptions[{n}].{key}")),
            );
        };
        let subscription_problem =
            |problem: Text| text!("subscription-problem", n = n, problem = problem);
        match CompiledTemplate::compile_named(&topic_key(i), &subscription.topic) {
            Ok(topic) => topics.push(topic),
            // MiniJinja's syntax error, which names the topic's key path.
            Err(e) => at("topic", text!("verbatim", text = e)),
        }
        if subscription.qos > 2 {
            let qos = text!("qos-range", qos = subscription.qos);
            at("qos", subscription_problem(qos));
        }
        if let Err(e) = subscription.script.compiled() {
            at("script", subscription_problem(e));
        }
    }
    topics
}

/// The key path of subscription `index`'s topic, counted from 1 as problems count.
fn topic_key(index: usize) -> String {
    format!("subscriptions[{}].topic", index + 1)
}

fn resolve_device(
    spec: &DeviceSpec,
    template: &TemplateSpec,
    compiled: &Compiled,
    seed: Option<u64>,
) -> Result<DeviceConfig, Vec<ProjectError>> {
    let mut problems = check_device(spec, template);
    let fields = merge_fields(spec, template).unwrap_or_else(|mut e| {
        problems.append(&mut e);
        BTreeMap::new()
    });
    if !problems.is_empty() {
        return Err(problems);
    }

    let ids = instance_ids(spec)
        .map_err(|e| vec![ProjectError::device(&spec.id, e).at("instance_id")])?;
    let instances: Vec<InstanceConfig> = (0..spec.count)
        .zip(ids)
        .map(|(index, id)| InstanceConfig {
            id,
            device: spec.id.clone(),
            index,
            template_id: template.id.clone(),
            messages: template.messages.clone(),
            availability: template.availability.clone(),
            templates: compiled.templates.clone(),
            qos: template.qos,
            retain: template.retain,
            home_assistant: template.home_assistant,
            interval: spec.interval.unwrap_or(template.interval),
            fields: fields.clone(),
            params: spec.params.clone(),
            seed,
            subscriptions: Vec::new(),
        })
        .collect();
    // The field is the template's key path: the device's values make that template fail.
    let failed = |e: TemplateError| vec![ProjectError::device(&spec.id, e.message).at(e.field)];
    sample_render(&instances[0]).map_err(failed)?;
    let mut instances = instances;
    for instance in &mut instances {
        instance.subscriptions = subscriptions(instance, template, compiled).map_err(failed)?;
    }
    Ok(DeviceConfig {
        id: spec.id.clone(),
        autostart: spec.autostart,
        instances,
    })
}

/// Checks the device's count, interval and param names.
fn check_device(spec: &DeviceSpec, template: &TemplateSpec) -> Vec<ProjectError> {
    let at = |field: &str, text: Text| ProjectError::device(&spec.id, text).at(field);
    let mut problems = Vec::new();
    if spec.count == 0 {
        problems.push(at("count", text!("count-zero")));
    }
    if spec.interval.is_some_and(|i| i.is_zero()) {
        problems.push(at("interval", text!("greater-than-zero", key = "interval")));
    }
    for name in spec.params.keys() {
        let field = format!("params.{name}");
        if is_reserved(name) {
            problems.push(at(&field, text!("param-reserved-name", name = name)));
        } else if template.fields.contains_key(name) {
            problems.push(at(&field, text!("param-field-name", name = name)));
        }
    }
    problems
}

/// Renders `instance_id` (or the default pattern) for every index. Fails on the first ID that is
/// empty or repeats.
pub(crate) fn instance_ids(spec: &DeviceSpec) -> Result<Vec<String>, Text> {
    let pattern = spec
        .instance_id
        .as_deref()
        .unwrap_or(default_instance_id_pattern(spec.count));
    // Compiled once and rendered per index: validation runs on every edit in the GUI and a
    // device may have thousands of instances.
    let id_error = |e: String| text!("instance-id-template", error = e);
    let pattern = CompiledTemplate::compile(pattern).map_err(id_error)?;
    let mut ctx = instance_id_context(&spec.id, &spec.params);
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    for index in 0..spec.count {
        ctx.insert("index".into(), index.into());
        let id = pattern.render(&ctx).map_err(id_error)?;
        if id.is_empty() {
            return Err(text!("instance-id-empty"));
        }
        if !seen.insert(id.clone()) {
            return Err(text!("instance-id-repeats", id = id));
        }
        ids.push(id);
    }
    Ok(ids)
}

/// Checks the MQTT 5 properties of `template`'s messages and availability against `broker`.
fn check_properties(template: &TemplateSpec, broker: &BrokerSpec) -> Vec<ProjectError> {
    let at = |field: &str, text: Text| ProjectError::template(&template.id, text).at(field);
    let mut problems = Vec::new();
    let mqtt5 = broker.mqtt_version == MqttVersion::V5;
    for (i, message) in template.messages.iter().enumerate() {
        let key = |name: &str| format!("messages[{}].{name}", i + 1);
        if let Some(expiry) = message.message_expiry {
            let field = key("message_expiry");
            match whole_seconds(&field, expiry) {
                Err(e) => problems.push(at(&field, e)),
                Ok(0) => problems.push(at(&field, text!("at-least-1s", key = &field))),
                Ok(_) => {}
            }
        }
        if message.content_type.as_deref() == Some("") {
            let field = key("content_type");
            problems.push(at(&field, text!("must-not-be-empty", key = &field)));
        }
        if !mqtt5 {
            let mut needs = |field: String| problems.push(at(&field, needs_mqtt5(&field)));
            if message.message_expiry.is_some() {
                needs(key("message_expiry"));
            }
            if message.content_type.is_some() {
                needs(key("content_type"));
            }
            if !message.user_properties.is_empty() {
                needs(key("user_properties"));
            }
        }
    }
    let Some(delay) = template.availability.as_ref().and_then(|a| a.will_delay) else {
        return problems;
    };
    const WILL_DELAY: &str = "availability.will_delay";
    if let Err(e) = whole_seconds(WILL_DELAY, delay) {
        problems.push(at(WILL_DELAY, e));
    } else if !mqtt5 {
        problems.push(at(WILL_DELAY, needs_mqtt5(WILL_DELAY)));
    } else if delay > broker.session_expiry {
        let show = |d| humantime::format_duration(d).to_string();
        let text = text!(
            "will-delay-after-session-expiry",
            delay = show(delay),
            expiry = show(broker.session_expiry)
        );
        problems.push(at(WILL_DELAY, text));
    }
    problems
}

/// `instance`'s subscriptions, with their topics rendered for it; fails on the first topic that
/// does not render to a valid topic filter.
fn subscriptions(
    instance: &InstanceConfig,
    template: &TemplateSpec,
    compiled: &Compiled,
) -> Result<Vec<Subscription>, TemplateError> {
    if template.subscriptions.is_empty() {
        return Ok(Vec::new());
    }
    let base = base_context(
        &instance.id,
        instance.index,
        &instance.template_id,
        &instance.params,
    );
    template
        .subscriptions
        .iter()
        .zip(&compiled.subscription_topics)
        .enumerate()
        .map(|(i, (spec, topic))| {
            let failed = |message| TemplateError {
                field: topic_key(i),
                message,
            };
            let n = i + 1;
            // MiniJinja's render error.
            let filter = topic
                .render(&base)
                .map_err(|e| failed(text!("verbatim", text = e)))?;
            check_filter(&filter).map_err(|e| {
                let invalid = |problem: Text| {
                    text!(
                        "subscription-topic-invalid",
                        n = n,
                        filter = &filter,
                        problem = problem
                    )
                };
                failed(match e {
                    FilterProblem::Empty => text!("subscription-topic-empty", n = n),
                    FilterProblem::HashNotLast => invalid(text!("filter-hash-not-last")),
                    FilterProblem::PlusNotWhole => invalid(text!("filter-plus-not-whole")),
                })
            })?;
            Ok(Subscription {
                filter,
                qos: spec.qos,
                script: spec.script.clone(),
            })
        })
        .collect()
}

/// Applies the device's field overrides to `template`'s fields. Keys merge, except that an
/// override with `generator` replaces the generator and all its keys; the field's `unit` and
/// `name` stay unless the override sets them too.
fn merge_fields(
    spec: &DeviceSpec,
    template: &TemplateSpec,
) -> Result<BTreeMap<String, FieldSpec>, Vec<ProjectError>> {
    let at =
        |name: &str, text: Text| ProjectError::device(&spec.id, text).at(format!("fields.{name}"));
    let mut fields = template.fields.clone();
    let mut problems = Vec::new();
    for (name, table) in &spec.fields {
        let Some(base) = fields.get(name) else {
            problems.push(at(name, text!("field-override-unknown", name = name)));
            continue;
        };
        let override_problem =
            |problem: Text| text!("field-override-problem", name = name, problem = problem);
        let mut merged = toml::Table::try_from(base).expect("a field serializes to a table");
        if table.contains_key("generator") {
            merged.retain(|key, _| FieldSpec::LABEL_KEYS.contains(&key));
        }
        merged.extend(table.clone());
        match toml::Value::Table(merged).try_into::<FieldSpec>() {
            Ok(field) => {
                let field_problems = field.problems();
                let script_problem = field.script_problem();
                let valid = field_problems.is_empty() && script_problem.is_none();
                for e in field_problems {
                    problems.push(at(name, override_problem(e)));
                }
                if let Some(e) = script_problem {
                    problems.push(
                        ProjectError::device(&spec.id, override_problem(e))
                            .at(format!("fields.{name}.script")),
                    );
                }
                if valid {
                    fields.insert(name.clone(), field);
                }
            }
            // The TOML deserializer's own text, such as ``missing field `max` ``.
            Err(e) => problems.push(at(
                name,
                override_problem(text!("verbatim", text = e.message())),
            )),
        }
    }
    if problems.is_empty() {
        Ok(fields)
    } else {
        Err(problems)
    }
}

/// Renders every template once for index 0 with sample values, so errors show up at load time.
fn sample_render(instance: &InstanceConfig) -> Result<(), TemplateError> {
    let templates = &instance.templates;
    let base = base_context(
        &instance.id,
        instance.index,
        &instance.template_id,
        &instance.params,
    );
    templates.availability_topic_located(&base)?;
    let values: BTreeMap<String, Value> = instance
        .fields
        .iter()
        .map(|(name, field)| (name.clone(), field.generator.initial_value()))
        .collect();
    let ctx = tick_context(&base, &values, SystemTime::now(), 0);
    for rendered in templates.render_messages_located(&ctx) {
        rendered?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoding::Encoding;
    use crate::generator::GeneratorSpec;
    use crate::i18n::Language;
    use crate::project::error::Location;
    use std::time::Duration;

    fn validate(src: &str) -> Validated {
        let env = |name: &str| (name == "MQTT_PASSWORD").then(|| "secret".to_string());
        ProjectFile::parse(src)
            .unwrap()
            .validate_with(Path::new("."), &env)
    }

    fn error_strings(validated: &Validated) -> Vec<String> {
        validated.errors.iter().map(ToString::to_string).collect()
    }

    const TEMPLATE: &str = r#"
[[templates]]
id = "air"
interval = "10s"
availability = { topic = "b/{{ room }}/{{ device_id }}/status", online = "online", offline = "offline" }
messages = [{ topic = "b/{{ room }}/{{ device_id }}/state", payload = '{"co2": {{ co2 | round | int }}}' }]
fields.co2 = { generator = "random_walk", start = 600, min = 400, max = 2000, step = 15 }
"#;

    /// Validates device `d` (template `air`, `room = "a"`) with the extra `lines` appended.
    fn validate_device(lines: &str) -> Validated {
        validate(&format!(
            "{TEMPLATE}\n[[devices]]\nid = \"d\"\ntemplate = \"air\"\nparams = {{ room = \"a\" }}\n{lines}"
        ))
    }

    /// Params differ per instance but are not part of the compiled messages, so every instance
    /// of a template shares one compile.
    #[test]
    fn instances_of_a_template_share_its_compiled_messages() {
        let src = format!(
            "{TEMPLATE}
[[devices]]
id = \"d\"
template = \"air\"
count = 2
params = {{ room = \"a\" }}

[[devices]]
id = \"e\"
template = \"air\"
params = {{ room = \"b\" }}

[[devices]]
id = \"c\"
template = \"co2\"
count = 2
"
        );
        let validated = validate(&src);
        assert!(
            validated.errors.is_empty(),
            "{:?}",
            error_strings(&validated)
        );
        let templates = |device: &str| -> Vec<SharedTemplates> {
            validated
                .project
                .device(device)
                .unwrap()
                .instances
                .iter()
                .map(|i| i.templates.clone())
                .collect()
        };
        let air: Vec<_> = templates("d").into_iter().chain(templates("e")).collect();
        let co2 = templates("c");
        assert!(air.iter().all(|t| t.is_shared_with(&air[0])));
        assert!(co2[1].is_shared_with(&co2[0]));
        assert!(!co2[0].is_shared_with(&air[0]));

        let again = validate(&src);
        let co2_again = &again.project.device("c").unwrap().instances[0].templates;
        assert!(
            co2_again.is_shared_with(&co2[0]),
            "built-in templates compile once for all validations"
        );
    }

    #[test]
    fn spec_example_is_valid_and_expands_count() {
        let validated = validate(include_str!("../../tests/fixtures/example.toml"));
        assert!(
            validated.errors.is_empty(),
            "{:?}",
            error_strings(&validated)
        );
        let device = validated.project.device("office").unwrap();
        let ids: Vec<&str> = device.instances.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(
            ids,
            ["office-0", "office-1", "office-2", "office-3", "office-4"]
        );
        assert_eq!(device.instances[3].index, 3);
        assert_eq!(device.instances[0].seed, Some(42));
        assert_eq!(device.instances[0].interval, Duration::from_secs(10));
    }

    #[test]
    fn default_instance_ids_depend_on_count() {
        let src = format!(
            "{TEMPLATE}
[[devices]]
id = \"single\"
template = \"air\"
params = {{ room = \"a\" }}

[[devices]]
id = \"many\"
template = \"air\"
count = 2
params = {{ room = \"a\" }}
"
        );
        let validated = validate(&src);
        assert!(
            validated.errors.is_empty(),
            "{:?}",
            error_strings(&validated)
        );
        let ids: Vec<&str> = validated
            .project
            .instances()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(ids, ["single", "many-0", "many-1"]);
    }

    #[test]
    fn override_without_generator_merges_keys() {
        let validated = validate_device("fields.co2 = { start = 1200 }\n");
        let field = &validated.project.instance("d").unwrap().fields["co2"];
        assert_eq!(
            field.generator,
            GeneratorSpec::RandomWalk {
                start: 1200.0,
                min: 400.0,
                max: 2000.0,
                step: 15.0,
                noise: None
            }
        );
    }

    #[test]
    fn override_with_generator_replaces_the_field() {
        let validated =
            validate_device("fields.co2 = { generator = \"constant\", value = 2500 }\n");
        let field = &validated.project.instance("d").unwrap().fields["co2"];
        assert_eq!(
            field.generator,
            GeneratorSpec::Constant {
                value: Value::Number(2500.0),
                noise: None
            }
        );
    }

    /// Like [`validate_device`], with the template's `co2` field labelled `ppm` and `CO2`.
    fn validate_labelled_device(lines: &str) -> Validated {
        let template =
            TEMPLATE.replace("step = 15 }", "step = 15, unit = \"ppm\", name = \"CO2\" }");
        assert_ne!(template, TEMPLATE);
        validate(&format!(
            "{template}\n[[devices]]\nid = \"d\"\ntemplate = \"air\"\nparams = {{ room = \"a\" }}\n{lines}"
        ))
    }

    /// The unit and name of instance `d`'s field `co2`.
    fn labels(validated: &Validated) -> (Option<&str>, Option<&str>) {
        assert!(
            validated.errors.is_empty(),
            "{:?}",
            error_strings(validated)
        );
        let field = &validated.project.instance("d").unwrap().fields["co2"];
        (field.unit.as_deref(), field.name.as_deref())
    }

    #[test]
    fn an_instance_has_its_fields_unit_and_name() {
        let validated = validate_labelled_device("");
        assert_eq!(labels(&validated), (Some("ppm"), Some("CO2")));
        assert_eq!(labels(&validate_device("")), (None, None));
    }

    #[test]
    fn an_override_can_set_the_unit_and_name() {
        let validated = validate_labelled_device("fields.co2 = { unit = \"mg/m³\" }\n");
        assert_eq!(labels(&validated), (Some("mg/m³"), Some("CO2")));
        let validated = validate_device("fields.co2 = { name = \"Carbon dioxide\" }\n");
        assert_eq!(labels(&validated), (None, Some("Carbon dioxide")));
    }

    #[test]
    fn an_override_with_a_generator_keeps_the_unit_and_name() {
        let validated =
            validate_labelled_device("fields.co2 = { generator = \"constant\", value = 2500 }\n");
        assert_eq!(labels(&validated), (Some("ppm"), Some("CO2")));
        let validated = validate_labelled_device(
            "fields.co2 = { generator = \"constant\", value = 2500, name = \"Fixed CO2\" }\n",
        );
        assert_eq!(labels(&validated), (Some("ppm"), Some("Fixed CO2")));
    }

    #[test]
    fn an_empty_unit_or_name_is_a_problem() {
        let src = "[[templates]]\nid = \"t\"\nmessages = [{ topic = \"a\", payload = \"b\" }]\n\
                   fields.v = { generator = \"constant\", value = 1, unit = \"\", name = \"\" }\n";
        let validated = validate(src);
        assert_eq!(
            error_strings(&validated),
            [
                "template `t`: field `v`: unit must not be empty",
                "template `t`: field `v`: name must not be empty",
            ]
        );
        assert_eq!(fields_of(&validated), [Some("fields.v"), Some("fields.v")]);

        let validated = validate_labelled_device("fields.co2 = { unit = \"\" }\n");
        assert_eq!(
            error_strings(&validated),
            ["device `d`: field `co2` override: unit must not be empty"]
        );
        assert_eq!(fields_of(&validated), [Some("fields.co2")]);
    }

    #[test]
    fn an_override_with_an_unknown_key_names_it() {
        let validated = validate_labelled_device("fields.co2 = { units = \"x\" }\n");
        let errors = error_strings(&validated);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].starts_with("device `d`: field `co2` override: unknown field `units`"),
            "{errors:?}"
        );
    }

    #[test]
    fn builtin_template_can_be_used_by_id() {
        let validated = validate("[[devices]]\nid = \"c\"\ntemplate = \"co2\"\n");
        assert!(
            validated.errors.is_empty(),
            "{:?}",
            error_strings(&validated)
        );
        assert_eq!(validated.project.instance("c").unwrap().template_id, "co2");
    }

    #[test]
    fn every_builtin_template_validates() {
        let validated = validate(
            "[[devices]]\nid = \"a\"\ntemplate = \"co2\"\n\
             [[devices]]\nid = \"b\"\ntemplate = \"temperature\"\n\
             [[devices]]\nid = \"c\"\ntemplate = \"humidity\"\n",
        );
        assert!(
            validated.errors.is_empty(),
            "{:?}",
            error_strings(&validated)
        );
        assert_eq!(validated.project.devices.len(), 3);
    }

    /// A project with one of each template and device error, plus one device that runs.
    fn every_error_class() -> String {
        format!(
            r#"{TEMPLATE}
[[templates]]
id = "co2"
messages = [{{ topic = "x", payload = "y" }}]
fields.v = {{ generator = "constant", value = 1 }}

[[templates]]
id = "broken"
messages = [{{ topic = "t", payload = "{{{{ v " }}]
fields.v = {{ generator = "random_walk", start = 1, min = 0, max = 10, step = 0 }}
fields.seq = {{ generator = "constant", value = 1 }}

[[devices]]
id = "unknown"
template = "nope"

[[devices]]
id = "uses-broken"
template = "broken"

[[devices]]
id = "a"
template = "air"
params = {{ room = "r" }}

[[devices]]
id = "dup"
template = "air"
params = {{ room = "r" }}

[[devices]]
id = "dup"
template = "air"
params = {{ room = "r" }}

[[devices]]
id = "clash"
template = "air"
instance_id = "a"
params = {{ room = "r" }}

[[devices]]
id = "bad-override"
template = "air"
params = {{ room = "r", index = 1 }}
fields.temp = {{ start = 1 }}

[[devices]]
id = "typo"
template = "air"
params = {{ rooom = "r" }}

[[devices]]
id = "zero"
template = "air"
count = 0
params = {{ room = "r" }}
"#
        )
    }

    #[test]
    fn reports_every_error_class_at_once() {
        let validated = validate(&every_error_class());
        let errors = error_strings(&validated);
        let expected = [
            "template `co2`: `co2` is a built-in template id; choose another id",
            "template `broken`: field `seq` uses a reserved name",
            "template `broken`: field `v`: step must be greater than 0",
            "device `unknown`: unknown template `nope`",
            "device `dup`: duplicate device id",
            "device `clash`: instance id `a` is already used by device `a`",
            "device `bad-override`: param `index` uses a reserved name",
            "device `bad-override`: override of unknown field `temp`",
            "device `zero`: count must be at least 1",
        ];
        for message in expected {
            assert!(
                errors.iter().any(|e| e == message),
                "missing `{message}` in {errors:#?}"
            );
        }
        assert!(
            errors
                .iter()
                .any(|e| e.starts_with("template `broken`: syntax error")),
            "{errors:#?}"
        );
        assert!(
            errors
                .iter()
                .any(|e| e.starts_with("device `typo`:") && e.contains("room")),
            "{errors:#?}"
        );
        let blocked: Vec<&str> = validated.blocked.iter().map(String::as_str).collect();
        assert_eq!(
            blocked,
            [
                "bad-override",
                "clash",
                "dup",
                "typo",
                "unknown",
                "uses-broken",
                "zero"
            ]
        );
        let running: Vec<&str> = validated
            .project
            .devices
            .iter()
            .map(|d| d.id.as_str())
            .collect();
        assert_eq!(running, ["a"]);
    }

    #[test]
    fn errors_are_listed_broker_first_then_in_file_order() {
        let validated = validate(&format!("[broker]\nport = 0\n{}", every_error_class()));
        assert_eq!(
            error_strings(&validated),
            [
                "broker: port must not be 0",
                "template `co2`: `co2` is a built-in template id; choose another id",
                "template `broken`: field `seq` uses a reserved name",
                "template `broken`: field `v`: step must be greater than 0",
                "template `broken`: syntax error: unexpected end of input, \
                 expected end of variable block (in messages[1].payload:1)",
                "device `unknown`: unknown template `nope`",
                "device `dup`: duplicate device id",
                "device `clash`: instance id `a` is already used by device `a`",
                "device `bad-override`: param `index` uses a reserved name",
                "device `bad-override`: override of unknown field `temp`",
                "device `typo`: undefined value: `room` is undefined (in availability.topic:1)",
                "device `zero`: count must be at least 1",
            ]
        );
    }

    #[test]
    fn problems_show_in_czech_with_library_texts_left_as_they_are() {
        let validated = validate(&format!("[broker]\nport = 0\n{}", every_error_class()));
        let czech: Vec<String> = validated
            .errors
            .iter()
            .map(|e| e.to_string_in(Language::Czech))
            .collect();
        assert_eq!(
            czech,
            [
                "broker: port nesmí být 0",
                "šablona `co2`: `co2` je ID vestavěné šablony; zvolte jiné ID",
                "šablona `broken`: pole `seq` používá vyhrazený název",
                "šablona `broken`: pole `v`: step musí být větší než 0",
                "šablona `broken`: syntax error: unexpected end of input, \
                 expected end of variable block (in messages[1].payload:1)",
                "zařízení `unknown`: neznámá šablona `nope`",
                "zařízení `dup`: duplicitní ID zařízení",
                "zařízení `clash`: ID instance `a` už používá zařízení `a`",
                "zařízení `bad-override`: parametr `index` používá vyhrazený název",
                "zařízení `bad-override`: přepsání neznámého pole `temp`",
                "zařízení `typo`: undefined value: `room` is undefined (in availability.topic:1)",
                "zařízení `zero`: count musí být alespoň 1",
            ]
        );
        for error in &validated.errors {
            assert_eq!(error.message_in(Language::English), error.message);
        }
    }

    #[test]
    fn a_toml_error_keeps_the_parser_text_after_a_czech_location() {
        let errors = ProjectFile::parse("[broker]\nport = \"x\"\n").unwrap_err();
        let czech = errors[0].to_string_in(Language::Czech);
        assert!(
            czech.starts_with("řádek 2, sloupec 8: invalid type"),
            "{czech}"
        );
    }

    #[test]
    fn param_with_field_name_is_an_error() {
        let src = format!(
            "{TEMPLATE}
[[devices]]
id = \"d\"
template = \"air\"
params = {{ room = \"a\", co2 = 1 }}
"
        );
        assert_eq!(
            error_strings(&validate(&src)),
            ["device `d`: param `co2` has the same name as a field"]
        );
    }

    #[test]
    fn wildcard_in_rendered_topic_is_an_error() {
        let src = format!(
            "{TEMPLATE}
[[devices]]
id = \"d\"
template = \"air\"
params = {{ room = \"+\" }}
"
        );
        let errors = error_strings(&validate(&src));
        assert!(
            errors[0].starts_with(
                "device `d`: availability topic: topic `b/+/d/status` contains a wildcard"
            ),
            "{errors:?}"
        );
    }

    #[test]
    fn a_message_topic_that_renders_empty_or_with_a_wildcard_names_its_message() {
        let template = |topic: &str| {
            format!(
                "[[templates]]\nid = \"t\"\nmessages = [{{ topic = \"a\", payload = \"1\" }}, \
                 {{ topic = \"{topic}\", payload = \"1\" }}]\n\
                 fields.v = {{ generator = \"constant\", value = 1 }}\n\
                 [[devices]]\nid = \"d\"\ntemplate = \"t\"\nparams = {{ p = \"+\" }}\n"
            )
        };
        let errors = |topic: &str| {
            let validated = validate(&template(topic));
            let english = error_strings(&validated);
            let czech: Vec<String> = validated
                .errors
                .iter()
                .map(|e| e.message_in(Language::Czech))
                .collect();
            (english, czech)
        };
        assert_eq!(
            errors("{{ '' }}"),
            (
                vec!["device `d`: message 2 topic: topic is empty".to_string()],
                vec!["topic zprávy 2: topic je prázdný".to_string()]
            )
        );
        assert_eq!(
            errors("x/{{ p }}"),
            (
                vec![
                    "device `d`: message 2 topic: topic `x/+` contains a wildcard (+ or #)"
                        .to_string()
                ],
                vec!["topic zprávy 2: topic `x/+` obsahuje wildcard (+ nebo #)".to_string()]
            )
        );
    }

    #[test]
    fn instance_id_can_use_params_and_index() {
        let validated =
            validate_device("count = 2\ninstance_id = \"{{ room }}-{{ id }}-{{ index }}\"\n");
        let ids: Vec<&str> = validated
            .project
            .instances()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(ids, ["a-d-0", "a-d-1"]);
    }

    #[test]
    fn instance_id_errors_name_the_problem() {
        let cases = [
            (
                "instance_id = \"{{ id \"\n",
                "device `d`: instance_id: syntax error: unexpected end of input, \
                 expected end of variable block (in <string>:1)",
            ),
            (
                "instance_id = \"{{ idx }}\"\n",
                "device `d`: instance_id: undefined value: `idx` is undefined (in <string>:1)",
            ),
            (
                "instance_id = \"\"\n",
                "device `d`: instance_id renders to an empty string",
            ),
            (
                "count = 2\ninstance_id = \"x\"\n",
                "device `d`: instance id `x` repeats; include {{ index }} in instance_id",
            ),
        ];
        for (lines, expected) in cases {
            assert_eq!(
                error_strings(&validate_device(lines)),
                [expected],
                "{lines}"
            );
        }
    }

    #[test]
    fn broker_error_blocks_every_device() {
        let src = format!(
            "[broker]\nport = 0\n{TEMPLATE}
[[devices]]
id = \"d\"
template = \"air\"
params = {{ room = \"a\" }}
"
        );
        let validated = validate(&src);
        assert_eq!(validated.errors[0].location, Location::Broker);
        assert!(validated.project.devices.is_empty());
        assert!(validated.blocked.contains("d"));
    }

    /// A project with template `t` (one message, availability) and device `d` (`room = "a"`).
    /// `broker` lines go into `[broker]`, `message` and `availability` are extra inline-table keys.
    fn properties_project(broker: &str, message: &str, availability: &str) -> Validated {
        validate(&format!(
            r#"
[broker]
{broker}

[[templates]]
id = "t"
availability = {{ topic = "s/{{{{ device_id }}}}", online = "on", offline = "off"{availability} }}
messages = [{{ topic = "t/{{{{ device_id }}}}", payload = "p"{message} }}]
fields.v = {{ generator = "constant", value = 1 }}

[[devices]]
id = "d"
template = "t"
params = {{ room = "a" }}
"#
        ))
    }

    #[test]
    fn mqtt5_properties_are_valid_and_reach_the_instance() {
        let validated = properties_project(
            r#"session_expiry = "60s""#,
            r#", message_expiry = "60s", content_type = "application/json", user_properties = { room = "{{ room }}" }"#,
            r#", will_delay = "5s""#,
        );
        assert!(
            validated.errors.is_empty(),
            "{:?}",
            error_strings(&validated)
        );
        let instance = validated.project.instance("d").unwrap();
        assert_eq!(
            instance.messages[0].message_expiry,
            Some(Duration::from_secs(60))
        );
        assert_eq!(
            instance.availability.as_ref().unwrap().will_delay,
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            validated.project.broker.session_expiry,
            Duration::from_secs(60)
        );
    }

    #[test]
    fn message_expiry_must_be_whole_seconds_and_at_least_one() {
        let half = properties_project("", r#", message_expiry = "1500ms""#, "");
        assert_eq!(
            error_strings(&half),
            ["template `t`: messages[1].message_expiry must be whole seconds, got 1s 500ms"]
        );
        assert!(half.blocked.contains("d"));
        let zero = properties_project("", r#", message_expiry = "0s""#, "");
        assert_eq!(
            error_strings(&zero),
            ["template `t`: messages[1].message_expiry must be at least 1s"]
        );
    }

    #[test]
    fn content_type_must_not_be_empty() {
        let validated = properties_project("", r#", content_type = """#, "");
        assert_eq!(
            error_strings(&validated),
            ["template `t`: messages[1].content_type must not be empty"]
        );
    }

    #[test]
    fn properties_with_mqtt_3_1_1_are_problems_on_the_template() {
        let validated = properties_project(
            r#"mqtt_version = "3.1.1""#,
            r#", message_expiry = "60s", content_type = "text/plain", user_properties = { a = "b" }"#,
            r#", will_delay = "5s""#,
        );
        assert_eq!(
            error_strings(&validated),
            [
                "template `t`: messages[1].message_expiry needs MQTT 5; the broker settings use MQTT 3.1.1",
                "template `t`: messages[1].content_type needs MQTT 5; the broker settings use MQTT 3.1.1",
                "template `t`: messages[1].user_properties needs MQTT 5; the broker settings use MQTT 3.1.1",
                "template `t`: availability.will_delay needs MQTT 5; the broker settings use MQTT 3.1.1",
            ]
        );
        assert!(validated.blocked.contains("d"));
    }

    #[test]
    fn session_expiry_with_mqtt_3_1_1_is_a_broker_problem() {
        let validated =
            properties_project("mqtt_version = \"3.1.1\"\nsession_expiry = \"60s\"", "", "");
        assert_eq!(
            error_strings(&validated),
            ["broker: session_expiry needs MQTT 5; the broker settings use MQTT 3.1.1"]
        );
        assert!(validated.project.devices.is_empty());
    }

    #[test]
    fn will_delay_longer_than_session_expiry_is_a_problem() {
        let validated = properties_project("", "", r#", will_delay = "30s""#);
        assert_eq!(
            error_strings(&validated),
            [
                "template `t`: availability.will_delay 30s is longer than the broker's \
                 session_expiry 0s; set session_expiry to at least 30s"
            ]
        );
    }

    #[test]
    fn will_delay_must_be_whole_seconds() {
        let validated = properties_project(
            r#"session_expiry = "60s""#,
            "",
            r#", will_delay = "2500ms""#,
        );
        assert_eq!(
            error_strings(&validated),
            ["template `t`: availability.will_delay must be whole seconds, got 2s 500ms"]
        );
    }

    #[test]
    fn session_expiry_must_be_whole_seconds() {
        let validated = properties_project(r#"session_expiry = "1500ms""#, "", "");
        assert_eq!(
            error_strings(&validated),
            ["broker: session_expiry must be whole seconds, got 1s 500ms"]
        );
    }

    #[test]
    fn a_user_property_that_does_not_render_is_reported_like_a_payload() {
        let validated = properties_project("", r#", user_properties = { room = "{{ rom }}" }"#, "");
        let errors = error_strings(&validated);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].starts_with("device `d`: messages[1].user_properties.room: undefined value"),
            "{}",
            errors[0]
        );
    }

    /// Each problem's text with the field it names.
    fn error_fields(validated: &Validated) -> Vec<(String, Option<String>)> {
        validated
            .errors
            .iter()
            .map(|e| (e.to_string(), e.field.clone()))
            .collect()
    }

    fn fields_of(validated: &Validated) -> Vec<Option<&str>> {
        validated
            .errors
            .iter()
            .map(|e| e.field.as_deref())
            .collect()
    }

    #[test]
    fn every_error_class_names_its_field() {
        let validated = validate(&format!("[broker]\nport = 0\n{}", every_error_class()));
        assert_eq!(
            fields_of(&validated),
            [
                Some("port"),
                Some("id"),
                Some("fields.seq"),
                Some("fields.v"),
                Some("messages[1].payload"),
                Some("template"),
                Some("id"),
                Some("instance_id"),
                Some("params.index"),
                Some("fields.temp"),
                Some("availability.topic"),
                Some("count"),
            ],
            "{:#?}",
            error_fields(&validated)
        );
    }

    #[test]
    fn template_problems_name_their_field() {
        let validated = validate(
            r#"
[[templates]]
id = "bad"
qos = 3
interval = "0s"
messages = []
fields = {}

[[templates]]
id = "x"
messages = [{ topic = "t", payload = "p" }]
fields.v = { generator = "constant", value = 1 }

[[templates]]
id = "x"
messages = [{ topic = "t", payload = "p" }]
fields.v = { generator = "constant", value = 1 }
"#,
        );
        assert_eq!(
            fields_of(&validated),
            [
                Some("messages"),
                Some("fields"),
                Some("qos"),
                Some("interval"),
                Some("id"),
            ],
            "{:#?}",
            error_fields(&validated)
        );
    }

    /// Template `t` with one message (`topic`, `payload`, plus `extra` keys in its inline table)
    /// and the `availability` line (empty for none), used by device `d` with `room` as its param.
    fn template_project(
        topic: &str,
        payload: &str,
        extra: &str,
        availability: &str,
        room: &str,
    ) -> Validated {
        validate(&format!(
            r#"
[[templates]]
id = "t"
{availability}
messages = [{{ topic = "{topic}", payload = "{payload}"{extra} }}]
fields.v = {{ generator = "constant", value = 1 }}

[[devices]]
id = "d"
template = "t"
params = {{ room = "{room}" }}
"#
        ))
    }

    fn availability(topic: &str) -> String {
        format!(r#"availability = {{ topic = "{topic}", online = "on", offline = "off" }}"#)
    }

    #[test]
    fn template_compile_problems_name_the_broken_template() {
        let cases = [
            (
                template_project("{{ x", "p", "", "", "a"),
                "messages[1].topic",
            ),
            (
                template_project("t", "{{ x", "", "", "a"),
                "messages[1].payload",
            ),
            (
                template_project("t", "p", r#", user_properties = { k = "{{ x" }"#, "", "a"),
                "messages[1].user_properties",
            ),
            (
                template_project("t", "p", "", &availability("{{ x"), "a"),
                "availability.topic",
            ),
        ];
        for (validated, field) in cases {
            assert_eq!(
                fields_of(&validated),
                [Some(field)],
                "{:#?}",
                error_fields(&validated)
            );
        }
    }

    #[test]
    fn render_problems_name_the_template_that_failed() {
        let cases = [
            (
                template_project("t/{{ room }}", "p", "", "", "+"),
                "messages[1].topic",
            ),
            (
                template_project("t", "{{ rom }}", "", "", "a"),
                "messages[1].payload",
            ),
            (
                template_project(
                    "t",
                    "p",
                    r#", user_properties = { k = "{{ rom }}" }"#,
                    "",
                    "a",
                ),
                "messages[1].user_properties",
            ),
            (
                template_project("t", "p", "", &availability("s/{{ room }}"), "+"),
                "availability.topic",
            ),
            (
                template_project("t", "p", "", &availability("s/{{ rom }}"), "a"),
                "availability.topic",
            ),
        ];
        for (validated, field) in cases {
            assert_eq!(
                fields_of(&validated),
                [Some(field)],
                "{:#?}",
                error_fields(&validated)
            );
        }
    }

    #[test]
    fn message_property_problems_name_their_field() {
        let cases = [
            (
                properties_project("", r#", message_expiry = "1500ms""#, ""),
                vec!["messages[1].message_expiry"],
            ),
            (
                properties_project("", r#", message_expiry = "0s""#, ""),
                vec!["messages[1].message_expiry"],
            ),
            (
                properties_project("", r#", content_type = """#, ""),
                vec!["messages[1].content_type"],
            ),
            (
                properties_project(
                    r#"mqtt_version = "3.1.1""#,
                    r#", message_expiry = "60s", content_type = "text/plain", user_properties = { a = "b" }"#,
                    r#", will_delay = "5s""#,
                ),
                vec![
                    "messages[1].message_expiry",
                    "messages[1].content_type",
                    "messages[1].user_properties",
                    "availability.will_delay",
                ],
            ),
            (
                properties_project("", "", r#", will_delay = "30s""#),
                vec!["availability.will_delay"],
            ),
            (
                properties_project(
                    r#"session_expiry = "60s""#,
                    "",
                    r#", will_delay = "2500ms""#,
                ),
                vec!["availability.will_delay"],
            ),
        ];
        for (validated, expected) in cases {
            let expected: Vec<Option<&str>> = expected.into_iter().map(Some).collect();
            assert_eq!(
                fields_of(&validated),
                expected,
                "{:#?}",
                error_fields(&validated)
            );
        }
    }

    #[test]
    fn device_problems_name_their_field() {
        let cases = [
            ("interval = \"0s\"\n", "interval"),
            ("fields.co2 = { min = 5000 }\n", "fields.co2"),
            ("instance_id = \"{{ id \"\n", "instance_id"),
            ("instance_id = \"{{ idx }}\"\n", "instance_id"),
            ("instance_id = \"\"\n", "instance_id"),
            ("count = 2\ninstance_id = \"x\"\n", "instance_id"),
        ];
        for (lines, field) in cases {
            let validated = validate_device(lines);
            assert_eq!(
                fields_of(&validated),
                [Some(field)],
                "{lines}: {:#?}",
                error_fields(&validated)
            );
        }
        let src = format!(
            "{TEMPLATE}
[[devices]]
id = \"d\"
template = \"air\"
params = {{ room = \"a\", co2 = 1 }}
"
        );
        assert_eq!(fields_of(&validate(&src)), [Some("params.co2")]);
    }

    /// Template `s` with a `temp` field script and the given `subscription` table entries (TOML
    /// after `[[templates.subscriptions]]`, empty for none), used by device `d` with `room` as
    /// its param and `extra` device lines.
    fn scripted(script: &str, subscription: &str, room: &str, extra: &str) -> Validated {
        let subscription = if subscription.is_empty() {
            String::new()
        } else {
            format!("[[templates.subscriptions]]\n{subscription}\n")
        };
        validate(&format!(
            r#"
[[templates]]
id = "s"
messages = [{{ topic = "s/{{{{ device_id }}}}", payload = "{{{{ temp }}}}" }}]
fields.temp = {{ generator = "script", start = 20, script = '{script}' }}
{subscription}
[[devices]]
id = "d"
template = "s"
count = 2
params = {{ room = "{room}" }}
{extra}
"#
        ))
    }

    const SUBSCRIPTION: &str = "topic = \"cmd/{{ room }}/{{ device_id }}/+\"\nqos = 1\nscript = 'state.on = payload == \"ON\";'";

    #[test]
    fn instances_get_their_subscriptions_with_rendered_filters_and_shared_scripts() {
        let validated = scripted("value + 1", SUBSCRIPTION, "hall", "");
        assert!(
            validated.errors.is_empty(),
            "{:?}",
            error_strings(&validated)
        );
        let device = validated.project.device("d").unwrap();
        let filters: Vec<(&str, u8)> = device
            .instances
            .iter()
            .flat_map(|i| &i.subscriptions)
            .map(|s| (s.filter.as_str(), s.qos))
            .collect();
        assert_eq!(filters, [("cmd/hall/d-0/+", 1), ("cmd/hall/d-1/+", 1)]);
        let scripts: Vec<_> = device
            .instances
            .iter()
            .map(|i| i.subscriptions[0].script.clone())
            .collect();
        assert!(scripts[1].is_shared_with(&scripts[0]), "compiled once");
        let field = |i: usize| match &device.instances[i].fields["temp"].generator {
            GeneratorSpec::Script { script, .. } => script.clone(),
            other => panic!("{other:?}"),
        };
        assert!(field(1).is_shared_with(&field(0)), "compiled once");
    }

    #[test]
    fn script_and_subscription_problems_name_their_field() {
        let validated = scripted(
            "value +",
            "topic = \"cmd/{{ room\"\nqos = 3\nscript = \"\"",
            "hall",
            "",
        );
        assert_eq!(
            error_fields(&validated)
                .into_iter()
                .map(|(text, field)| (text.split(" (").next().unwrap().to_string(), field))
                .collect::<Vec<_>>(),
            [
                (
                    "template `s`: field `temp`: script does not compile: Script is incomplete"
                        .to_string(),
                    Some("fields.temp.script".to_string())
                ),
                (
                    "template `s`: syntax error: unexpected end of input, expected end of \
                     variable block"
                        .to_string(),
                    Some("subscriptions[1].topic".to_string())
                ),
                (
                    "template `s`: subscription 1: qos must be 0, 1 or 2, got 3".to_string(),
                    Some("subscriptions[1].qos".to_string())
                ),
                (
                    "template `s`: subscription 1: script is empty".to_string(),
                    Some("subscriptions[1].script".to_string())
                ),
            ],
        );
        let errors = error_strings(&validated);
        assert!(
            errors[1].contains("(in subscriptions[1].topic:1)"),
            "{errors:?}"
        );
    }

    #[test]
    fn a_subscription_topic_must_render_to_a_valid_filter() {
        let cases = [
            ("topic = \"\"", "device `d`: subscription 1 topic is empty"),
            (
                "topic = \"cmd/{{ room }}\"",
                "device `d`: subscription 1 topic `cmd/a#`: `#` must be the whole last level, \
                 as in `a/#`",
            ),
            (
                "topic = \"cmd/#/x\"",
                "device `d`: subscription 1 topic `cmd/#/x`: `#` must be the whole last level, \
                 as in `a/#`",
            ),
            (
                "topic = \"cmd/x+\"",
                "device `d`: subscription 1 topic `cmd/x+`: `+` must be a whole level, as in \
                 `a/+/b`",
            ),
            (
                "topic = \"cmd/{{ rom }}\"",
                "device `d`: undefined value: `rom` is undefined",
            ),
        ];
        for (topic, expected) in cases {
            let validated = scripted("value", &format!("{topic}\nscript = \"1\""), "a#", "");
            let errors = error_fields(&validated);
            assert_eq!(errors.len(), 1, "{topic}: {errors:?}");
            assert!(errors[0].0.starts_with(expected), "{topic}: {errors:?}");
            assert_eq!(
                errors[0].1.as_deref(),
                Some("subscriptions[1].topic"),
                "{topic}"
            );
        }
    }

    #[test]
    fn a_device_can_override_the_script_and_start_of_a_field() {
        let validated = scripted(
            "value + 1",
            "",
            "a",
            "fields.temp = { start = 5, script = \"value * 2\" }",
        );
        assert!(
            validated.errors.is_empty(),
            "{:?}",
            error_strings(&validated)
        );
        let field = &validated.project.instance("d-0").unwrap().fields["temp"];
        match &field.generator {
            GeneratorSpec::Script { start, script, .. } => {
                assert_eq!(*start, Value::Number(5.0));
                assert_eq!(script.source(), "value * 2");
            }
            other => panic!("{other:?}"),
        }

        let broken = scripted("value + 1", "", "a", "fields.temp = { script = \"(\" }");
        let errors = error_fields(&broken);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0]
                .0
                .starts_with("device `d`: field `temp` override: script does not compile: "),
            "{errors:?}"
        );
        assert_eq!(errors[0].1.as_deref(), Some("fields.temp.script"));
    }

    /// A project with one template whose one message has `message` (the keys after `topic`),
    /// and a device, validated with the project file in `dir`.
    fn encoded_in(dir: &Path, message: &str) -> Validated {
        let src = format!(
            "[[templates]]\nid = \"t\"\n\
             messages = [{{ topic = \"t/{{{{ device_id }}}}\", {message} }}]\n\
             fields.v = {{ generator = \"constant\", value = 215 }}\n\n\
             [[devices]]\nid = \"d\"\ntemplate = \"t\"\n"
        );
        ProjectFile::parse(&src)
            .unwrap()
            .validate_with(dir, &|_| None)
    }

    fn proto_dir() -> std::path::PathBuf {
        crate::proto::tests::sensor_proto()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    #[test]
    fn a_protobuf_message_validates_with_its_proto_file_next_to_the_project() {
        let validated = encoded_in(
            &proto_dir(),
            r#"payload = '{"humidity": {{ v }}}', encoding = "protobuf", proto = "sensor.proto", message = "sensors.v1.Reading""#,
        );
        assert!(
            validated.errors.is_empty(),
            "{:?}",
            error_strings(&validated)
        );
        let templates = &validated.project.instance("d").unwrap().templates;
        assert_eq!(templates.encoders()[0].encoding(), Encoding::Protobuf);
    }

    #[test]
    fn encoding_problems_name_their_key_and_block_the_template() {
        let here = Path::new(".");
        let cases = [
            (
                encoded_in(here, r#"payload = "{}", encoding = "protobuf""#),
                vec![
                    (
                        "template `t`: message 1: encoding protobuf needs a proto file",
                        "messages[1].proto",
                    ),
                    (
                        "template `t`: message 1: encoding protobuf needs a message type",
                        "messages[1].message",
                    ),
                ],
            ),
            (
                encoded_in(
                    here,
                    r#"payload = "{}", encoding = "cbor", proto = "a.proto""#,
                ),
                vec![(
                    "template `t`: message 1: proto is used only with encoding protobuf",
                    "messages[1].proto",
                )],
            ),
            (
                encoded_in(
                    &proto_dir(),
                    r#"payload = "{}", encoding = "protobuf", proto = "sensor.proto", message = "sensors.v1.Nope""#,
                ),
                vec![(
                    "template `t`: message 1: proto `sensor.proto` has no message type \
                     `sensors.v1.Nope`",
                    "messages[1].message",
                )],
            ),
        ];
        for (validated, expected) in cases {
            let found: Vec<(String, Option<String>)> = error_fields(&validated);
            let expected: Vec<(String, Option<String>)> = expected
                .into_iter()
                .map(|(m, f)| (m.to_string(), Some(f.to_string())))
                .collect();
            assert_eq!(found, expected);
            assert!(validated.blocked.contains("d"));
        }
    }

    #[test]
    fn a_proto_file_that_cannot_be_read_names_the_file() {
        let validated = encoded_in(
            Path::new("."),
            r#"payload = "{}", encoding = "protobuf", proto = "nope/missing.proto", message = "a.A""#,
        );
        let errors = error_fields(&validated);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0]
                .0
                .starts_with("template `t`: message 1: cannot read proto `nope/missing.proto`: "),
            "{errors:?}"
        );
        assert_eq!(errors[0].1.as_deref(), Some("messages[1].proto"));
    }

    #[test]
    fn a_sample_render_that_does_not_encode_is_a_problem_at_the_payload() {
        let validated = encoded_in(
            Path::new("."),
            r#"payload = "{{ v | u8 }}{{ v }}", encoding = "hex""#,
        );
        assert_eq!(
            error_fields(&validated),
            [(
                "device `d`: message 1 payload as hex: odd number of hex digits (5)".to_string(),
                Some("messages[1].payload".to_string())
            )]
        );
        let fine = encoded_in(
            Path::new("."),
            r#"payload = "{{ v | u16be }}", encoding = "hex""#,
        );
        assert!(fine.errors.is_empty(), "{:?}", error_strings(&fine));
    }

    #[test]
    fn validating_again_does_not_compile_an_unchanged_proto_file_and_sees_a_changed_one() {
        let dir = crate::proto::tests::temp_dir("validate-cache");
        let path = dir.join("a.proto");
        std::fs::write(&path, "syntax = \"proto3\";\nmessage A { int32 v = 1; }\n").unwrap();
        let message = r#"payload = '{"v": {{ v }}}', encoding = "protobuf", proto = "a.proto", message = "A""#;
        let first = encoded_in(&dir, message);
        let again = encoded_in(&dir, message);
        assert!(first.errors.is_empty(), "{:?}", error_strings(&first));
        assert_eq!(crate::proto::compile_count::of(&path), 1);
        assert_eq!(
            first.project.instance("d"),
            again.project.instance("d"),
            "an unchanged file compiles to the same configuration"
        );

        std::fs::write(&path, "syntax = \"proto3\";\nmessage A { int64 v = 1; }\n").unwrap();
        let changed = encoded_in(&dir, message);
        assert_eq!(crate::proto::compile_count::of(&path), 2);
        assert_ne!(
            first.project.instance("d"),
            changed.project.instance("d"),
            "a running instance takes the changed file as an edit"
        );
    }
}
