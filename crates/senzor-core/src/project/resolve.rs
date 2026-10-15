use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;
use std::time::SystemTime;

use super::broker::{EnvLookup, broker_config, resolve_broker};
use super::builtin::{builtin_templates, is_builtin};
use super::error::ProjectError;
use super::file::{DeviceSpec, ProjectFile, TemplateSpec, default_instance_id_pattern};
use super::model::{DeviceConfig, InstanceConfig, Project, Validated};
use crate::generator::GeneratorSpec;
use crate::render::{
    CompiledTemplate, InstanceTemplates, base_context, instance_id_context, is_reserved,
    tick_context,
};
use crate::value::Value;

/// Whether devices can use a template.
enum TemplateStatus<'a> {
    Usable(&'a TemplateSpec),
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
        let templates = self.template_lookup(&mut errors);
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
        errors: &mut Vec<ProjectError>,
    ) -> HashMap<String, TemplateStatus<'a>> {
        let mut templates: HashMap<String, TemplateStatus<'a>> = builtin_templates()
            .iter()
            .map(|t| (t.id.clone(), TemplateStatus::Usable(t)))
            .collect();
        for template in &self.templates {
            if is_builtin(&template.id) {
                errors.push(ProjectError::template(
                    &template.id,
                    format!(
                        "`{}` is a built-in template id; choose another id",
                        template.id
                    ),
                ));
                continue;
            }
            if templates.contains_key(&template.id) {
                errors.push(ProjectError::template(
                    &template.id,
                    "duplicate template id",
                ));
                templates.insert(template.id.clone(), TemplateStatus::Broken);
                continue;
            }
            let messages = check_template(template);
            let status = if messages.is_empty() {
                TemplateStatus::Usable(template)
            } else {
                TemplateStatus::Broken
            };
            errors.extend(
                messages
                    .into_iter()
                    .map(|m| ProjectError::template(&template.id, m)),
            );
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
                Err(vec!["duplicate device id".to_string()])
            };
            match resolved {
                Ok(device) => {
                    for instance in &device.instances {
                        instance_owners.insert(instance.id.clone(), spec.id.clone());
                    }
                    devices.push(device);
                }
                Err(messages) => {
                    errors.extend(
                        messages
                            .into_iter()
                            .map(|m| ProjectError::device(&spec.id, m)),
                    );
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
) -> Result<DeviceConfig, Vec<String>> {
    let template = match templates.get(spec.template.as_str()) {
        Some(TemplateStatus::Usable(template)) => *template,
        Some(TemplateStatus::Broken) => return Err(Vec::new()),
        None => return Err(vec![format!("unknown template `{}`", spec.template)]),
    };
    let device = resolve_device(spec, template, seed)?;
    match instance_clash(&device, instance_owners) {
        Some(message) => Err(vec![message]),
        None => Ok(device),
    }
}

/// The error for the first instance of `device` whose ID another device already uses.
fn instance_clash(device: &DeviceConfig, owners: &HashMap<String, String>) -> Option<String> {
    device.instances.iter().find_map(|instance| {
        owners.get(&instance.id).map(|owner| {
            format!(
                "instance id `{}` is already used by device `{owner}`",
                instance.id
            )
        })
    })
}

fn check_template(template: &TemplateSpec) -> Vec<String> {
    let mut messages = Vec::new();
    if template.messages.is_empty() {
        messages.push("at least one message is required".to_string());
    }
    if template.fields.is_empty() {
        messages.push("at least one field is required".to_string());
    }
    if template.qos > 2 {
        messages.push(format!("qos must be 0, 1 or 2, got {}", template.qos));
    }
    if template.interval.is_zero() {
        messages.push("interval must be greater than 0".to_string());
    }
    for (name, generator) in &template.fields {
        if is_reserved(name) {
            messages.push(format!("field `{name}` uses a reserved name"));
        }
        if let Err(e) = generator.validate() {
            messages.push(format!("field `{name}`: {e}"));
        }
    }
    if let Err(e) = InstanceTemplates::compile(&template.messages, template.availability.as_ref()) {
        messages.push(e);
    }
    messages
}

fn resolve_device(
    spec: &DeviceSpec,
    template: &TemplateSpec,
    seed: Option<u64>,
) -> Result<DeviceConfig, Vec<String>> {
    let mut messages = check_device(spec, template);
    let fields = merge_fields(template, &spec.fields).unwrap_or_else(|mut e| {
        messages.append(&mut e);
        BTreeMap::new()
    });
    if !messages.is_empty() {
        return Err(messages);
    }

    let ids = instance_ids(spec).map_err(|e| vec![e])?;
    let instances: Vec<InstanceConfig> = (0..spec.count)
        .zip(ids)
        .map(|(index, id)| InstanceConfig {
            id,
            device: spec.id.clone(),
            index,
            template_id: template.id.clone(),
            messages: template.messages.clone(),
            availability: template.availability.clone(),
            qos: template.qos,
            retain: template.retain,
            interval: spec.interval.unwrap_or(template.interval),
            fields: fields.clone(),
            params: spec.params.clone(),
            seed,
        })
        .collect();
    sample_render(&instances[0]).map_err(|e| vec![e])?;
    Ok(DeviceConfig {
        id: spec.id.clone(),
        autostart: spec.autostart,
        instances,
    })
}

/// Checks the device's count, interval and param names.
fn check_device(spec: &DeviceSpec, template: &TemplateSpec) -> Vec<String> {
    let mut messages = Vec::new();
    if spec.count == 0 {
        messages.push("count must be at least 1".to_string());
    }
    if spec.interval.is_some_and(|i| i.is_zero()) {
        messages.push("interval must be greater than 0".to_string());
    }
    for name in spec.params.keys() {
        if is_reserved(name) {
            messages.push(format!("param `{name}` uses a reserved name"));
        } else if template.fields.contains_key(name) {
            messages.push(format!("param `{name}` has the same name as a field"));
        }
    }
    messages
}

/// Renders `instance_id` (or the default pattern) for every index. Fails on the first ID that is
/// empty or repeats.
fn instance_ids(spec: &DeviceSpec) -> Result<Vec<String>, String> {
    let pattern = spec
        .instance_id
        .as_deref()
        .unwrap_or(default_instance_id_pattern(spec.count));
    // Compiled once and rendered per index: validation runs on every edit in the GUI and a
    // device may have thousands of instances.
    let id_error = |e: String| format!("instance_id: {e}");
    let pattern = CompiledTemplate::compile(pattern).map_err(id_error)?;
    let mut ctx = instance_id_context(&spec.id, &spec.params);
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    for index in 0..spec.count {
        ctx.insert("index".into(), index.into());
        let id = pattern.render(&ctx).map_err(id_error)?;
        if id.is_empty() {
            return Err("instance_id renders to an empty string".to_string());
        }
        if !seen.insert(id.clone()) {
            return Err(format!(
                "instance id `{id}` repeats; include {{{{ index }}}} in instance_id"
            ));
        }
        ids.push(id);
    }
    Ok(ids)
}

/// Applies device overrides: with `generator` the override replaces the field, otherwise keys merge.
fn merge_fields(
    template: &TemplateSpec,
    overrides: &BTreeMap<String, toml::Table>,
) -> Result<BTreeMap<String, GeneratorSpec>, Vec<String>> {
    let mut fields = template.fields.clone();
    let mut messages = Vec::new();
    for (name, table) in overrides {
        let Some(base) = fields.get(name) else {
            messages.push(format!("override of unknown field `{name}`"));
            continue;
        };
        let merged = if table.contains_key("generator") {
            table.clone()
        } else {
            let mut merged =
                toml::Table::try_from(base).expect("generator spec serializes to a table");
            merged.extend(table.clone());
            merged
        };
        let parsed = toml::Value::Table(merged)
            .try_into::<GeneratorSpec>()
            .map_err(|e| e.message().to_string())
            .and_then(|spec| spec.validate().map(|()| spec));
        match parsed {
            Ok(spec) => {
                fields.insert(name.clone(), spec);
            }
            Err(e) => messages.push(format!("field `{name}` override: {e}")),
        }
    }
    if messages.is_empty() {
        Ok(fields)
    } else {
        Err(messages)
    }
}

/// Renders every template once for index 0 with sample values, so errors show up at load time.
fn sample_render(instance: &InstanceConfig) -> Result<(), String> {
    let templates = InstanceTemplates::compile(&instance.messages, instance.availability.as_ref())?;
    let base = base_context(
        &instance.id,
        instance.index,
        &instance.template_id,
        &instance.params,
    );
    templates.availability_topic(&base)?;
    let values: BTreeMap<String, Value> = instance
        .fields
        .iter()
        .map(|(name, g)| (name.clone(), g.initial_value()))
        .collect();
    let ctx = tick_context(&base, &values, SystemTime::now(), 0);
    for rendered in templates.render_messages(&ctx) {
        rendered?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
            *field,
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
            *field,
            GeneratorSpec::Constant {
                value: Value::Number(2500.0),
                noise: None
            }
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
}
