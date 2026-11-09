//! Changing one value of the project file by its key path, e.g. `devices.office.count`: the
//! `set` and `unset` tools.

use senzor_core::project::ProjectFile;
use senzor_core::project::builtin::is_builtin;
use toml::{Table, Value as Toml};

/// What a path looks like, for errors that reject one.
pub const PATH_HELP: &str = "a path is the keys of the project file joined by dots, with devices \
and templates named by id and messages numbered from 1, e.g. `broker.port`, `simulation.seed`, \
`devices.office.count`, `devices.office.params.room`, `templates.office-air.interval`, \
`templates.office-air.fields.co2.max`, `templates.office-air.messages.1.payload` (or \
`messages[1]`); quote a key that holds a dot: `devices.office.params.\"a.b\"`";

/// The lists whose entries a path names by id rather than by number.
const BY_ID: [(&str, &str); 2] = [("devices", "device"), ("templates", "template")];

/// `file` with the value at `path` set to `value` (JSON: a string, number, bool, or an object or
/// array where the key holds a table or list). Fails, naming the path, when the result would not
/// be a valid project file, e.g. because the value has the wrong type for the key.
pub fn set(
    file: &ProjectFile,
    path: &str,
    value: &serde_json::Value,
) -> Result<ProjectFile, String> {
    let keys = parse(path)?;
    let value =
        toml_from_json(value).map_err(|e| format!("cannot set `{}`: {e}", render(&keys)))?;
    change(file, &keys, Some(value))
}

/// `file` without the key at `path`, so its default applies; a path to a message removes that
/// message. Fails, naming the path, for a key that is not set or that the project file requires.
pub fn unset(file: &ProjectFile, path: &str) -> Result<ProjectFile, String> {
    let keys = parse(path)?;
    change(file, &keys, None)
}

fn change(file: &ProjectFile, keys: &[String], new: Option<Toml>) -> Result<ProjectFile, String> {
    let full = render(keys);
    let verb = if new.is_some() { "set" } else { "unset" };
    if let [list, _, key] = keys
        && key == "id"
        && let Some(kind) = by_id(list)
    {
        return Err(format!(
            "`{full}` is the {kind}'s id; change it with rename_{kind}"
        ));
    }
    let mut root = Toml::try_from(file).map_err(|e| format!("internal error: {e}"))?;
    let (last, parents) = keys.split_last().expect("a parsed path has a key");
    let mut node = &mut root;
    for (depth, key) in parents.iter().enumerate() {
        node = child(node, &keys[..depth], key, new.is_some(), &full)?;
    }
    let here = &keys[..parents.len()];
    match node {
        Toml::Table(table) => match new {
            Some(value) => {
                table.insert(last.clone(), value);
            }
            None => {
                if table.remove(last).is_none() {
                    return Err(not_set(&full));
                }
            }
        },
        Toml::Array(items) => {
            let index = entry(items, here, last, new.is_some())?;
            let kind = match here {
                [list] => by_id(list),
                _ => None,
            };
            match (new, kind) {
                (None, Some(kind)) => {
                    return Err(format!(
                        "unset removes a key; remove_{kind} removes the {kind} `{last}`"
                    ));
                }
                (None, None) => {
                    items.remove(index);
                }
                (Some(mut value), kind) => {
                    if let Some(kind) = kind {
                        keep_id(&mut value, kind, last, &full)?;
                    }
                    if index == items.len() {
                        items.push(value);
                    } else {
                        items[index] = value;
                    }
                }
            }
        }
        other => return Err(no_keys(here, other, last)),
    }
    root.try_into::<ProjectFile>()
        .map_err(|e| format!("cannot {verb} `{full}`: {}", e.message()))
}

/// The value under `key` of `node`, whose path is `here`. With `create`, a missing key becomes
/// an empty table, for setting a key inside it.
fn child<'a>(
    node: &'a mut Toml,
    here: &[String],
    key: &str,
    create: bool,
    full: &str,
) -> Result<&'a mut Toml, String> {
    match node {
        Toml::Table(table) => {
            if !table.contains_key(key) {
                let empty = if here.is_empty() && by_id(key).is_some() {
                    Toml::Array(Vec::new())
                } else if create {
                    Toml::Table(Table::new())
                } else {
                    return Err(not_set(full));
                };
                table.insert(key.to_string(), empty);
            }
            Ok(table.get_mut(key).expect("inserted above"))
        }
        Toml::Array(items) => {
            let index = entry(items, here, key, false)?;
            Ok(&mut items[index])
        }
        other => Err(no_keys(here, other, key)),
    }
}

/// The index of the entry `key` names in the list at `here`: a device or template by id, any
/// other entry by its number from 1. With `append`, the number after the last adds an entry.
fn entry(items: &[Toml], here: &[String], key: &str, append: bool) -> Result<usize, String> {
    if let [list] = here
        && let Some(kind) = by_id(list)
    {
        return items
            .iter()
            .position(|item| item.get("id").and_then(Toml::as_str) == Some(key))
            .ok_or_else(|| {
                if kind == "template" && is_builtin(key) {
                    format!(
                        "`{key}` is a built-in template; built-ins cannot be changed (copy_builtin_template copies one into the project)"
                    )
                } else {
                    format!("the project has no {kind} `{key}` (put_{kind} adds one)")
                }
            });
    }
    let count = items.len();
    let last = if append { count + 1 } else { count };
    match key.parse::<usize>() {
        Ok(number) if (1..=last).contains(&number) => Ok(number - 1),
        _ => {
            let path = render(here);
            let add = if append {
                format!(", or {} to add one", count + 1)
            } else {
                String::new()
            };
            Err(if count == 0 {
                format!("`{path}` is an empty list{add}")
            } else {
                format!("`{path}` is a list: number its entries from 1 to {count}{add}")
            })
        }
    }
}

/// `value`, which replaces the device or template `id`, keeps that id (filled in when missing).
fn keep_id(value: &mut Toml, kind: &str, id: &str, full: &str) -> Result<(), String> {
    let Toml::Table(table) = value else {
        return Ok(());
    };
    match table.get("id").and_then(Toml::as_str) {
        None => {
            table.insert("id".into(), Toml::String(id.to_string()));
            Ok(())
        }
        Some(same) if same == id => Ok(()),
        Some(_) => Err(format!(
            "`{full}` takes the {kind} with its id `{id}`; change the id with rename_{kind}"
        )),
    }
}

fn by_id(list: &str) -> Option<&'static str> {
    BY_ID
        .iter()
        .find(|(name, _)| *name == list)
        .map(|(_, kind)| *kind)
}

fn not_set(full: &str) -> String {
    format!("`{full}` is not set, so there is nothing to unset (get_project shows the project)")
}

fn no_keys(here: &[String], node: &Toml, key: &str) -> String {
    let kind = node.type_str();
    let article = if kind.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    };
    format!(
        "`{}` holds {article} {kind}, which has no key `{key}`",
        render(here)
    )
}

/// The keys of `path`: `a.b`, `"a.b"` for a key with a dot, `messages[1]` as `messages.1`.
fn parse(path: &str) -> Result<Vec<String>, String> {
    let bad = |why: &str| format!("`{path}` is not a path: {why}; {PATH_HELP}");
    let mut keys = Vec::new();
    let mut chars = path.trim().chars().peekable();
    if chars.peek().is_none() {
        return Err(bad("it is empty"));
    }
    loop {
        keys.push(parse_key(&mut chars).map_err(bad)?);
        parse_indices(&mut chars, &mut keys).map_err(bad)?;
        match chars.next() {
            None => return Ok(keys),
            Some('.') => {}
            Some(c) => return Err(bad(&format!("`{c}` follows a quoted key"))),
        }
    }
}

type Chars<'a> = std::iter::Peekable<std::str::Chars<'a>>;

/// One key, quoted or bare, up to the next `.` or `[`.
fn parse_key(chars: &mut Chars<'_>) -> Result<String, &'static str> {
    let mut key = String::new();
    if chars.next_if_eq(&'"').is_some() {
        loop {
            match chars.next() {
                Some('"') => return Ok(key),
                Some('\\') => key.extend(chars.next()),
                Some(c) => key.push(c),
                None => return Err("a quote is not closed"),
            }
        }
    }
    while let Some(c) = chars.next_if(|c| *c != '.' && *c != '[') {
        key.push(c);
    }
    let key = key.trim();
    if key.is_empty() {
        return Err("a key is empty");
    }
    Ok(key.to_string())
}

/// The `[n]` message numbers after a key, each added to `keys`.
fn parse_indices(chars: &mut Chars<'_>, keys: &mut Vec<String>) -> Result<(), &'static str> {
    while chars.next_if_eq(&'[').is_some() {
        let mut number = String::new();
        while let Some(c) = chars.next_if(|c| *c != ']') {
            number.push(c);
        }
        if chars.next().is_none() {
            return Err("a `[` is not closed");
        }
        if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
            return Err("`[…]` takes a message number");
        }
        keys.push(number);
    }
    Ok(())
}

/// `keys` as a path, quoting a key that is not a bare TOML key.
fn render(keys: &[String]) -> String {
    keys.iter()
        .map(|key| {
            let bare = !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if bare {
                key.clone()
            } else {
                format!("{key:?}")
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// `value` as TOML holds it. TOML has no null; integers beyond 64 bits do not fit.
fn toml_from_json(value: &serde_json::Value) -> Result<Toml, String> {
    use serde_json::Value as Json;
    Ok(match value {
        Json::Null => {
            return Err("null is not a value in the project file; unset removes a key".into());
        }
        Json::Bool(b) => Toml::Boolean(*b),
        Json::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Toml::Integer(i),
            (None, Some(f)) if !n.is_u64() => Toml::Float(f),
            _ => return Err(format!("{n} is too large for the project file")),
        },
        Json::String(s) => Toml::String(s.clone()),
        Json::Array(items) => {
            Toml::Array(items.iter().map(toml_from_json).collect::<Result<_, _>>()?)
        }
        Json::Object(map) => Toml::Table(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), toml_from_json(v)?)))
                .collect::<Result<_, String>>()?,
        ),
    })
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use senzor_core::value::Value;
    use serde_json::json;

    use super::*;

    const PROJECT: &str = r#"
[broker]
host = "localhost"
port = 1883

[[templates]]
id = "office-air"
interval = "10s"

[[templates.messages]]
topic = "b/{{ device_id }}/state"
payload = "{{ co2 }}"

[[templates.messages]]
topic = "b/{{ device_id }}/temp"
payload = "{{ temp }}"

[templates.fields.co2]
generator = "uniform"
min = 400
max = 800

[templates.fields.temp]
generator = "constant"
value = 21

[[devices]]
id = "office"
template = "office-air"
count = 5
params = { room = "office" }

[[devices]]
id = "lab"
template = "co2"
"#;

    fn file() -> ProjectFile {
        ProjectFile::parse(PROJECT).unwrap()
    }

    fn set_ok(path: &str, value: serde_json::Value) -> ProjectFile {
        set(&file(), path, &value).unwrap_or_else(|e| panic!("set {path}: {e}"))
    }

    fn set_err(path: &str, value: serde_json::Value) -> String {
        set(&file(), path, &value).expect_err(path)
    }

    fn unset_ok(path: &str) -> ProjectFile {
        unset(&file(), path).unwrap_or_else(|e| panic!("unset {path}: {e}"))
    }

    fn unset_err(path: &str) -> String {
        unset(&file(), path).expect_err(path)
    }

    fn co2_max(file: &ProjectFile) -> f64 {
        let co2 = serde_json::to_value(&file.templates[0].fields["co2"]).unwrap();
        co2["max"].as_f64().unwrap()
    }

    #[test]
    fn devices_and_templates_are_addressed_by_id() {
        let file = set_ok("devices.lab.count", json!(3));
        assert_eq!(file.devices[1].count, 3);
        assert_eq!(file.devices[0].count, 5, "the other device is left alone");

        let file = set_ok("templates.office-air.interval", json!("30s"));
        assert_eq!(file.templates[0].interval, Duration::from_secs(30));
    }

    #[test]
    fn nested_keys_params_and_the_broker() {
        let file = set_ok("templates.office-air.fields.co2.max", json!(900));
        assert_eq!(co2_max(&file), 900.0);

        let file = set_ok("devices.office.params.room", json!("lab"));
        assert_eq!(file.devices[0].params["room"], Value::String("lab".into()));
        let file = set_ok("devices.lab.params.floor", json!(2));
        assert_eq!(file.devices[1].params["floor"], Value::Number(2.0));

        let file = set_ok("devices.lab.fields.co2.max", json!(1500));
        assert_eq!(
            file.devices[1].fields["co2"]["max"].as_integer(),
            Some(1500)
        );

        assert_eq!(set_ok("broker.port", json!(8883)).broker.port, 8883);
        assert_eq!(set_ok("simulation.seed", json!(7)).simulation.seed, Some(7));
        let file = set_ok("broker.tls.ca_file", json!("ca.pem"));
        assert_eq!(
            file.broker.tls.unwrap().ca_file.unwrap().to_str(),
            Some("ca.pem")
        );
    }

    #[test]
    fn messages_are_numbered_from_one() {
        let file = set_ok(
            "templates.office-air.messages.2.payload",
            json!("t={{ temp }}"),
        );
        assert_eq!(file.templates[0].messages[1].payload, "t={{ temp }}");
        let same = set_ok(
            "templates.office-air.messages[2].payload",
            json!("t={{ temp }}"),
        );
        assert_eq!(same, file, "messages[2] as the problems write it");

        let file = set_ok(
            "templates.office-air.messages.3",
            json!({"topic": "b/x", "payload": "1"}),
        );
        assert_eq!(
            file.templates[0].messages.len(),
            3,
            "one past the end adds a message"
        );
        assert_eq!(file.templates[0].messages[2].topic, "b/x");

        let file = unset_ok("templates.office-air.messages.1");
        assert_eq!(file.templates[0].messages.len(), 1);
        assert_eq!(file.templates[0].messages[0].payload, "{{ temp }}");

        for bad in ["0", "4", "x"] {
            let message = set_err(
                &format!("templates.office-air.messages.{bad}.payload"),
                json!("p"),
            );
            assert!(message.contains("1 to 2"), "{bad}: {message}");
        }
    }

    #[test]
    fn whole_tables_and_lists_can_be_set() {
        let file = set_ok(
            "templates.office-air.fields.co2",
            json!({"generator": "constant", "value": 500, "unit": "ppm"}),
        );
        assert_eq!(file.templates[0].fields["co2"].unit.as_deref(), Some("ppm"));

        let file = set_ok(
            "templates.office-air.messages",
            json!([{"topic": "only", "payload": "{{ co2 }}"}]),
        );
        assert_eq!(file.templates[0].messages.len(), 1);

        let file = set_ok(
            "devices.lab",
            json!({"template": "temperature", "count": 2}),
        );
        assert_eq!(file.devices[1].id, "lab", "the id is kept");
        assert_eq!(file.devices[1].template, "temperature");
        assert_eq!(file.devices[1].count, 2);
    }

    #[test]
    fn a_wrong_type_names_the_path_and_what_the_key_takes() {
        let message = set_err("devices.office.count", json!("many"));
        assert!(message.contains("`devices.office.count`"), "{message}");
        assert!(message.contains("u32"), "{message}");

        let message = set_err("templates.office-air.interval", json!(30));
        assert!(
            message.contains("`templates.office-air.interval`"),
            "{message}"
        );
        assert!(message.contains("duration"), "{message}");

        let message = set_err("broker.port", json!(70000));
        assert!(message.contains("`broker.port`"), "{message}");
    }

    #[test]
    fn unknown_paths_are_refused_with_what_is_accepted() {
        let message = set_err("devices.nope.count", json!(1));
        assert!(message.contains("no device `nope`"), "{message}");
        let message = set_err("templates.nope.interval", json!("1s"));
        assert!(message.contains("no template `nope`"), "{message}");
        let message = set_err("templates.co2.interval", json!("1s"));
        assert!(message.contains("built-in"), "{message}");

        let message = set_err("devices.office.colour", json!("red"));
        assert!(message.contains("colour"), "{message}");
        assert!(message.contains("expected one of"), "{message}");

        let message = set_err("brokr.port", json!(1));
        assert!(message.contains("brokr"), "{message}");

        let message = set_err("devices.office.count.x", json!(1));
        assert!(message.contains("`devices.office.count`"), "{message}");

        for bad in [
            "",
            "devices..count",
            "devices.office.",
            "messages[x]",
            "\"open",
        ] {
            let message = set_err(bad, json!(1));
            assert!(message.contains(PATH_HELP), "{bad:?}: {message}");
        }
    }

    #[test]
    fn ids_change_by_renaming() {
        let message = set_err("devices.office.id", json!("desk"));
        assert!(message.contains("rename_device"), "{message}");
        let message = set_err("templates.office-air.id", json!("air"));
        assert!(message.contains("rename_template"), "{message}");
        let message = set_err("devices.lab", json!({"id": "desk", "template": "co2"}));
        assert!(message.contains("rename_device"), "{message}");
    }

    #[test]
    fn unset_lets_the_default_apply() {
        assert_eq!(unset_ok("devices.office.count").devices[0].count, 1);
        assert_eq!(
            unset_ok("templates.office-air.interval").templates[0].interval,
            Duration::from_secs(10)
        );
        assert_eq!(unset_ok("broker.port").broker.port, 1883);
        assert!(
            unset_ok("devices.office.params.room").devices[0]
                .params
                .is_empty()
        );

        let message = unset_err("devices.office.template");
        assert!(message.contains("`devices.office.template`"), "{message}");
        assert!(message.contains("template"), "{message}");

        let message = unset_err("devices.office.interval");
        assert!(message.contains("not set"), "{message}");
        let message = unset_err("devices.office");
        assert!(message.contains("remove_device"), "{message}");
    }

    #[test]
    fn quoted_keys_may_hold_dots() {
        let file = set_ok("devices.office.params.\"a.b\"", json!(true));
        assert_eq!(file.devices[0].params["a.b"], Value::Bool(true));
    }

    #[test]
    fn json_values_become_toml_values() {
        let file = set_ok("devices.office.params.ratio", json!(0.5));
        assert_eq!(file.devices[0].params["ratio"], Value::Number(0.5));
        let message = set_err("devices.office.params.room", json!(null));
        assert!(message.contains("unset"), "{message}");
    }

    #[test]
    fn an_unchanged_project_round_trips() {
        let original = file();
        assert_eq!(set_ok("devices.office.count", json!(5)), original);
        for example in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        {
            let file = ProjectFile::parse(&std::fs::read_to_string(&example).unwrap()).unwrap();
            let port = file.broker.port;
            let same = set(&file, "broker.port", &json!(port)).unwrap();
            assert_eq!(same, file, "{}", example.display());
        }
    }
}
