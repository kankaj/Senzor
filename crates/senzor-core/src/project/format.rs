//! Saving a project without losing how its file was written: comments, the order of keys and
//! tables, inline or section tables, and how unchanged values are spelled (`600` rather than
//! `600.0`, `"60s"` rather than `"1m"`).

use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, TableLike, Value};

use super::ProjectFile;

/// The text to save for `current`, written over `text`, the file as it was read, which holds
/// `base`. Only what differs between `base` and `current` changes in the text. When the result
/// would not hold `current` exactly, it is the plain form [`ProjectFile::to_toml_string`] writes.
pub fn update_text(text: &str, base: &ProjectFile, current: &ProjectFile) -> String {
    let plain = || current.to_toml_string();
    let parsed = (
        text.parse::<DocumentMut>(),
        base.to_toml_string().parse::<DocumentMut>(),
        plain().parse::<DocumentMut>(),
    );
    let (Ok(mut doc), Ok(old), Ok(new)) = parsed else {
        return plain();
    };
    sync_table(doc.as_table_mut(), old.as_table(), new.as_table());
    let updated = doc.to_string();
    match ProjectFile::parse(&updated) {
        Ok(file) if file == *current => updated,
        _ => plain(),
    }
}

/// Brings `text` (a table of the file) from `old` to `new`, both written in the plain form.
fn sync_table(text: &mut dyn TableLike, old: &dyn TableLike, new: &dyn TableLike) {
    let removed: Vec<String> = old
        .iter()
        .map(|(key, _)| key.to_string())
        .filter(|key| new.get(key).is_none())
        .collect();
    for key in removed {
        text.remove(&key);
    }
    for (key, new_item) in new.iter() {
        match (text.get_mut(key), old.get(key)) {
            (Some(item), Some(old_item)) => sync_item(item, old_item, new_item),
            // A value the plain form leaves out (a default) that is now set.
            (Some(item), None) => replace(item, new_item),
            // The file leaves out a value it did not change, such as a default: keep it out.
            (None, Some(old_item)) if same(old_item, new_item) => {}
            (None, _) => {
                text.insert(key, fresh(new_item));
            }
        }
    }
}

fn sync_item(text: &mut Item, old: &Item, new: &Item) {
    if same(old, new) {
        return;
    }
    if let (Some(text), Some(old), Some(new)) = (
        text.as_table_like_mut(),
        old.as_table_like(),
        new.as_table_like(),
    ) {
        sync_table(text, old, new);
        return;
    }
    if let (Some(mut list), Some(old_list), Some(new_list)) =
        (Tables::of(text), Tables::read(old), Tables::read(new))
    {
        sync_tables(&mut list, &old_list, &new_list);
        return;
    }
    replace(text, new);
}

/// Replaces a value, keeping the spaces and comments around it.
fn replace(text: &mut Item, new: &Item) {
    match (text.as_value_mut(), new.as_value()) {
        (Some(value), Some(new_value)) => {
            let decor = value.decor().clone();
            *value = new_value.clone();
            *value.decor_mut() = decor;
        }
        _ => *text = fresh(new),
    }
}

/// `item` taken from the plain form, to be placed in the file: tables after what is there.
fn fresh(item: &Item) -> Item {
    let mut item = item.clone();
    match &mut item {
        Item::Table(table) => unplace(table),
        Item::ArrayOfTables(tables) => tables.iter_mut().for_each(unplace),
        _ => {}
    }
    item
}

fn unplace(table: &mut Table) {
    table.set_position(None);
    for (_, item) in table.iter_mut() {
        if let Item::Table(inner) = item {
            unplace(inner);
        }
    }
}

/// A list of tables, written as `[[sections]]` or as an inline array of `{ ... }`.
enum Tables<'a> {
    Sections(&'a mut ArrayOfTables),
    Inline(&'a mut Array),
}

/// A list of tables to read: the plain form's.
struct TablesRead<'a>(Vec<&'a dyn TableLike>);

impl<'a> Tables<'a> {
    fn of(item: &'a mut Item) -> Option<Self> {
        match item {
            Item::ArrayOfTables(tables) => Some(Tables::Sections(tables)),
            Item::Value(Value::Array(array))
                if array.iter().all(|value| value.as_inline_table().is_some()) =>
            {
                Some(Tables::Inline(array))
            }
            _ => None,
        }
    }

    fn read(item: &Item) -> Option<TablesRead<'_>> {
        match item {
            Item::ArrayOfTables(tables) => Some(TablesRead(
                tables.iter().map(|t| t as &dyn TableLike).collect(),
            )),
            Item::Value(Value::Array(array)) => array
                .iter()
                .map(|value| value.as_inline_table().map(|t| t as &dyn TableLike))
                .collect::<Option<Vec<_>>>()
                .map(TablesRead),
            _ => None,
        }
    }
}

/// The `id` of each table, when every table has one: then tables are matched by it, so a table
/// removed from the middle takes its own comments along.
fn ids(tables: &TablesRead) -> Option<Vec<String>> {
    tables
        .0
        .iter()
        .map(|table| table.get("id")?.as_str().map(str::to_string))
        .collect()
}

/// Brings a list of tables from `old` to `new`: kept tables are updated where they are, others
/// removed, new ones added at the end.
fn sync_tables(text: &mut Tables, old: &TablesRead, new: &TablesRead) {
    // For each table of `new`, the index of the table in `old` (and in the text) it continues.
    let matched: Vec<Option<usize>> = match (ids(old), ids(new)) {
        (Some(old_ids), Some(new_ids)) => new_ids
            .iter()
            .map(|id| old_ids.iter().position(|old_id| old_id == id))
            .collect(),
        _ => (0..new.0.len())
            .map(|i| (i < old.0.len()).then_some(i))
            .collect(),
    };
    match text {
        Tables::Sections(tables) => {
            let kept: Vec<Table> = tables.iter().cloned().collect();
            tables.clear();
            for (index, new_table) in new.0.iter().enumerate() {
                let table = match matched[index] {
                    Some(at) if at < kept.len() => {
                        let mut table = kept[at].clone();
                        sync_table(&mut table, old.0[at], *new_table);
                        table
                    }
                    _ => {
                        let mut table = Table::new();
                        sync_table(&mut table, &Table::new(), *new_table);
                        table
                    }
                };
                tables.push(table);
            }
        }
        Tables::Inline(array) => {
            let kept: Vec<Value> = array.iter().cloned().collect();
            array.clear();
            for (index, new_table) in new.0.iter().enumerate() {
                let value = match matched[index] {
                    Some(at) if at < kept.len() => {
                        let mut value = kept[at].clone();
                        if let Some(table) = value.as_inline_table_mut() {
                            sync_table(table, old.0[at], *new_table);
                        }
                        value
                    }
                    _ => {
                        let mut table = toml_edit::InlineTable::new();
                        sync_table(&mut table, &Table::new(), *new_table);
                        Value::InlineTable(table)
                    }
                };
                array.push_formatted(value);
            }
        }
    }
}

/// Whether two items of the plain form hold the same thing.
fn same(a: &Item, b: &Item) -> bool {
    match (a.as_table_like(), b.as_table_like()) {
        (Some(a), Some(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, item)| b.get(key).is_some_and(|other| same(item, other)))
        }
        (None, None) => match (a, b) {
            (Item::ArrayOfTables(a), Item::ArrayOfTables(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b.iter())
                        .all(|(a, b)| same(&Item::Table(a.clone()), &Item::Table(b.clone())))
            }
            (Item::Value(a), Item::Value(b)) => same_value(a, b),
            (Item::None, Item::None) => true,
            _ => false,
        },
        _ => false,
    }
}

fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| same_value(a, b))
        }
        (Value::InlineTable(a), Value::InlineTable(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, value)| b.get(key).is_some_and(|other| same_value(value, other)))
        }
        (Value::String(a), Value::String(b)) => a.value() == b.value(),
        (Value::Integer(a), Value::Integer(b)) => a.value() == b.value(),
        (Value::Float(a), Value::Float(b)) => a.value() == b.value(),
        (Value::Boolean(a), Value::Boolean(b)) => a.value() == b.value(),
        (Value::Datetime(a), Value::Datetime(b)) => a.value() == b.value(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-written project: comments, integers, a duration in seconds, inline tables, and a
    /// `#:schema` line.
    const HAND_WRITTEN: &str = r#"#:schema https://senzor.kankaj.cz/schema/project.json
# The office sensors.

[broker]
host = "localhost"  # the broker in Docker
port = 1883

# Air quality in the offices.
[[templates]]
id = "office-air"
interval = "60s"
messages = [
    { topic = "office/{{ device_id }}/state", payload = '{"co2": {{ co2 | round | int }}}' },
]
fields.co2 = { generator = "random_walk", start = 600, min = 400, max = 2000, step = 15 }

[[devices]]
id = "office"      # five desks
template = "office-air"
count = 5

# The lab.
[[devices]]
id = "lab"
template = "office-air"
"#;

    fn parse(text: &str) -> ProjectFile {
        ProjectFile::parse(text).unwrap()
    }

    /// `HAND_WRITTEN` saved after `edit`, checked to hold the edited project.
    fn saved_after(edit: impl FnOnce(&mut ProjectFile)) -> String {
        let base = parse(HAND_WRITTEN);
        let mut current = base.clone();
        edit(&mut current);
        let text = update_text(HAND_WRITTEN, &base, &current);
        assert_eq!(
            parse(&text),
            current,
            "the saved text holds the project:\n{text}"
        );
        text
    }

    #[test]
    fn an_unchanged_project_is_saved_as_it_was_written() {
        assert_eq!(saved_after(|_| {}), HAND_WRITTEN);
    }

    #[test]
    fn a_changed_value_keeps_its_comment_and_the_rest_of_the_file() {
        let text = saved_after(|file| file.devices[0].count = 8);
        assert_eq!(
            text,
            HAND_WRITTEN.replace("count = 5", "count = 8"),
            "only the count changes"
        );
    }

    #[test]
    fn a_value_in_an_inline_table_changes_in_place() {
        let text = saved_after(|file| {
            let co2 = file.templates[0].fields.get_mut("co2").unwrap();
            if let crate::generator::GeneratorSpec::RandomWalk { max, .. } = &mut co2.generator {
                *max = 2500.0;
            }
        });
        assert!(
            text.contains("start = 600, min = 400, max = 2500.0, step = 15 }"),
            "{text}"
        );
        assert!(text.contains("# Air quality in the offices."), "{text}");
    }

    #[test]
    fn a_removed_device_takes_its_comment_with_it_and_the_others_keep_theirs() {
        let text = saved_after(|file| {
            file.devices.remove(0);
        });
        assert!(!text.contains("five desks"), "{text}");
        assert!(
            text.contains("# The lab.\n[[devices]]\nid = \"lab\""),
            "{text}"
        );
    }

    #[test]
    fn an_added_device_and_key_are_written_after_what_is_there() {
        let text = saved_after(|file| {
            file.broker.username = Some("me".into());
            file.devices
                .push(crate::project::DeviceSpec::new("hall", "office-air"));
        });
        assert!(text.contains("port = 1883\nusername = \"me\"\n"), "{text}");
        // A new table is written in the plain form, with its defaults.
        assert!(
            text.ends_with(
                "id = \"hall\"\ntemplate = \"office-air\"\ncount = 1\nautostart = true\n"
            ),
            "{text}"
        );
        assert!(text.starts_with("#:schema "), "{text}");
    }

    #[test]
    fn a_removed_key_goes_and_a_renamed_template_follows_in_its_devices() {
        let text = saved_after(|file| {
            file.templates[0].id = "air".into();
            for device in &mut file.devices {
                device.template = "air".into();
            }
        });
        assert_eq!(text.matches("\"air\"").count(), 3, "{text}");
        assert!(!text.contains("office-air"), "{text}");
        assert!(text.contains("id = \"office\"      # five desks"), "{text}");
    }

    /// Typical edits to every example keep its own text instead of falling back to the plain
    /// form, which would drop its comments.
    #[test]
    fn edits_to_the_examples_keep_their_comments() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        type Edit = (&'static str, fn(&mut ProjectFile));
        let edits: [Edit; 4] = [
            ("count", |file| {
                if let Some(device) = file.devices.first_mut() {
                    device.count += 1;
                }
            }),
            ("remove a device", |file| {
                file.devices.pop();
            }),
            ("add a field", |file| {
                if let Some(template) = file.templates.first_mut() {
                    template.fields.insert(
                        "extra".into(),
                        crate::generator::GeneratorSpec::Constant {
                            value: crate::value::Value::Number(1.0),
                            noise: None,
                        }
                        .into(),
                    );
                }
            }),
            ("broker", |file| file.broker.port = 18831),
        ];
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|ext| ext != "toml") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let base = parse(&text);
            assert_eq!(update_text(&text, &base, &base), text, "{}", path.display());
            for (name, edit) in edits {
                let mut current = base.clone();
                edit(&mut current);
                let saved = update_text(&text, &base, &current);
                assert_eq!(parse(&saved), current, "{name} in {}", path.display());
                assert_ne!(
                    saved,
                    current.to_toml_string(),
                    "{name} in {} fell back to the plain form",
                    path.display()
                );
                assert!(
                    saved.starts_with(text.lines().next().unwrap()),
                    "{name} in {}:\n{saved}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn text_that_cannot_be_followed_falls_back_to_the_plain_form() {
        let base = parse(HAND_WRITTEN);
        let mut current = base.clone();
        current.devices[0].count = 8;
        // Not the text `base` was read from: the result would not hold `current`.
        let text = update_text("[broker]\nport = 1\n", &base, &current);
        assert_eq!(parse(&text), current);
    }
}
