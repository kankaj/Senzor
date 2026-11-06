//! Edits to the project file model that every editor (the GUI, the MCP server) applies the same way.

use std::path::PathBuf;

use super::builtin::{builtin_template, is_builtin};
use super::resolve::instance_ids;
use super::{ProjectFile, TemplateSpec};

/// The extension of project files.
pub const PROJECT_EXTENSION: &str = "toml";

/// `base` if `taken` says it is free, otherwise `base-2`, `base-3`, …
pub fn unique_id(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !taken(candidate))
        .expect("an unused id exists")
}

/// Whether `id` already names a project template or a built-in.
pub fn template_id_taken(file: &ProjectFile, id: &str) -> bool {
    file.templates.iter().any(|t| t.id == id) || is_builtin(id)
}

/// Why `id` cannot name a new or renamed template: it is empty, a built-in's, or taken.
pub fn check_new_template_id(file: &ProjectFile, id: &str) -> Result<(), String> {
    if id.trim().is_empty() {
        return Err("the name must not be empty".into());
    }
    if is_builtin(id) {
        return Err(format!("`{id}` is a built-in template id"));
    }
    if file.templates.iter().any(|t| t.id == id) {
        return Err(format!("a template `{id}` already exists"));
    }
    Ok(())
}

/// The id offered for a copy of built-in `builtin`: `<builtin>-copy`, numbered when taken.
pub fn copy_id_suggestion(file: &ProjectFile, builtin: &str) -> String {
    unique_id(&format!("{builtin}-copy"), |id| template_id_taken(file, id))
}

/// Copies device `id` right after it, under a free ID: `<id>-copy`, then `<id>-copy-2`, …
/// Returns the copy's index, or `None` when there is no such device. The copy can run next to
/// the original: when its `instance_id` pattern would give it the original's instance IDs, the
/// copy uses the default pattern instead.
pub fn duplicate_device(file: &mut ProjectFile, id: &str) -> Option<usize> {
    let index = file.devices.iter().position(|d| d.id == id)?;
    let original = &file.devices[index];
    let mut copy = original.clone();
    copy.id = unique_id(&format!("{id}-copy"), |id| {
        file.devices.iter().any(|d| d.id == id)
    });
    if let (Ok(theirs), Ok(ours)) = (instance_ids(original), instance_ids(&copy))
        && ours.iter().any(|id| theirs.contains(id))
    {
        copy.instance_id = None;
    }
    file.devices.insert(index + 1, copy);
    Some(index + 1)
}

/// Copies built-in template `builtin` into the project under `new_id`; returns the new index.
pub fn copy_builtin(file: &mut ProjectFile, builtin: &str, new_id: &str) -> Result<usize, String> {
    let new_id = new_id.trim();
    check_new_template_id(file, new_id)?;
    let mut template = builtin_template(builtin)
        .cloned()
        .ok_or_else(|| format!("unknown built-in template `{builtin}`"))?;
    template.id = new_id.to_string();
    file.templates.push(template);
    Ok(file.templates.len() - 1)
}

/// The template a device naming `id` runs on. Built-ins come first, as in validation: a project
/// template cannot replace one.
pub fn find_template<'a>(file: &'a ProjectFile, id: &str) -> Option<&'a TemplateSpec> {
    builtin_template(id).or_else(|| file.templates.iter().find(|t| t.id == id))
}

/// Where to save a project under the name `chosen`. A name that does not end in `.toml` (in any
/// case) gets `.toml` appended, so the file shows up in an Open dialog's `*.toml` filter:
/// `notes.txt` is saved as `notes.txt.toml`.
pub fn project_file_path(chosen: PathBuf) -> PathBuf {
    if chosen
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(PROJECT_EXTENSION))
    {
        return chosen;
    }
    let mut name = chosen.into_os_string();
    name.push(".");
    name.push(PROJECT_EXTENSION);
    PathBuf::from(name)
}

/// A rename typed into a name box. It names what it renames (`from`), so a rename that arrives
/// after the item changed or moved is refused instead of hitting another item.
#[derive(Debug, Clone, PartialEq)]
pub enum Rename {
    Device {
        index: usize,
        from: String,
        to: String,
    },
    Template {
        index: usize,
        from: String,
        to: String,
    },
    Param {
        device: usize,
        from: String,
        to: String,
    },
    Field {
        template: usize,
        from: String,
        to: String,
    },
}

fn stale_rename() -> String {
    "it was changed or removed in the meantime".to_string()
}

impl Rename {
    fn names(&self) -> (&str, &str) {
        match self {
            Rename::Device { from, to, .. }
            | Rename::Template { from, to, .. }
            | Rename::Param { from, to, .. }
            | Rename::Field { from, to, .. } => (from, to),
        }
    }

    /// Why the rename cannot be applied; shown under the name box.
    pub fn check(&self, file: &ProjectFile) -> Result<(), String> {
        let (from, to) = self.names();
        if to.trim().is_empty() {
            return Err("the name must not be empty".into());
        }
        match self {
            Rename::Device { index, .. } => {
                file.devices
                    .get(*index)
                    .filter(|d| d.id == from)
                    .ok_or_else(stale_rename)?;
                if to != from && file.devices.iter().any(|d| d.id == to) {
                    return Err(format!("a device `{to}` already exists"));
                }
            }
            Rename::Template { index, .. } => {
                file.templates
                    .get(*index)
                    .filter(|t| t.id == from)
                    .ok_or_else(stale_rename)?;
                if to != from {
                    check_new_template_id(file, to)?;
                }
            }
            Rename::Param { device, .. } => {
                let device = file
                    .devices
                    .get(*device)
                    .filter(|d| d.params.contains_key(from))
                    .ok_or_else(stale_rename)?;
                if to != from && device.params.contains_key(to) {
                    return Err(format!("the device already has a param `{to}`"));
                }
            }
            Rename::Field { template, .. } => {
                let template = file
                    .templates
                    .get(*template)
                    .filter(|t| t.fields.contains_key(from))
                    .ok_or_else(stale_rename)?;
                if to != from && template.fields.contains_key(to) {
                    return Err(format!("the template already has a field `{to}`"));
                }
            }
        }
        Ok(())
    }

    /// Applies the rename and carries it to what refers to the old name: devices follow a
    /// renamed template, device overrides follow a renamed field.
    pub fn apply(&self, file: &mut ProjectFile) -> Result<(), String> {
        self.check(file)?;
        let (from, to) = self.names();
        let (from, to) = (from.to_string(), to.to_string());
        match self {
            Rename::Device { index, .. } => file.devices[*index].id = to,
            Rename::Template { index, .. } => {
                file.templates[*index].id = to.clone();
                for device in file.devices.iter_mut().filter(|d| d.template == from) {
                    device.template = to.clone();
                }
            }
            Rename::Param { device, .. } => {
                let params = &mut file.devices[*device].params;
                if let Some(value) = params.remove(&from) {
                    params.insert(to, value);
                }
            }
            Rename::Field { template, .. } => {
                let template = &mut file.templates[*template];
                if let Some(spec) = template.fields.remove(&from) {
                    template.fields.insert(to.clone(), spec);
                }
                let template_id = template.id.clone();
                for device in file
                    .devices
                    .iter_mut()
                    .filter(|d| d.template == template_id)
                {
                    if let Some(table) = device.fields.remove(&from) {
                        device.fields.insert(to.clone(), table);
                    }
                }
            }
        }
        Ok(())
    }
}

/// Deleting a device or template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    Device(usize),
    Template(usize),
}

impl Removal {
    pub fn apply(&self, file: &mut ProjectFile) {
        match *self {
            Removal::Device(index) if index < file.devices.len() => {
                file.devices.remove(index);
            }
            Removal::Template(index) if index < file.templates.len() => {
                file.templates.remove(index);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::builtin::builtin_templates;
    use crate::value::Value;
    use std::path::Path;

    #[test]
    fn unique_id_appends_a_counter() {
        assert_eq!(unique_id("device", |_| false), "device");
        assert_eq!(
            unique_id("device", |id| id == "device" || id == "device-2"),
            "device-3"
        );
    }

    #[test]
    fn copy_builtin_copies_under_a_new_id() {
        let mut file = ProjectFile::default();
        let i = copy_builtin(&mut file, "co2", "my-co2").unwrap();
        assert_eq!(file.templates[i].id, "my-co2");
        assert_eq!(
            file.templates[i].messages,
            builtin_templates()[0].messages,
            "content is the built-in's"
        );
        assert!(file.validate(Path::new(".")).errors.is_empty());
    }

    #[test]
    fn the_suggested_copy_id_is_not_taken() {
        let mut file = ProjectFile::default();
        assert_eq!(copy_id_suggestion(&file, "co2"), "co2-copy");
        copy_builtin(&mut file, "co2", "co2-copy").unwrap();
        assert_eq!(copy_id_suggestion(&file, "co2"), "co2-copy-2");
    }

    #[test]
    fn copy_builtin_rejects_bad_ids() {
        let mut file = ProjectFile::default();
        copy_builtin(&mut file, "co2", "mine").unwrap();
        assert_eq!(
            copy_builtin(&mut file, "co2", " "),
            Err("the name must not be empty".into())
        );
        assert_eq!(
            copy_builtin(&mut file, "co2", "humidity"),
            Err("`humidity` is a built-in template id".into())
        );
        assert_eq!(
            copy_builtin(&mut file, "co2", "mine"),
            Err("a template `mine` already exists".into())
        );
        assert_eq!(
            copy_builtin(&mut file, "nope", "x"),
            Err("unknown built-in template `nope`".into())
        );
    }

    #[test]
    fn a_name_without_an_extension_is_saved_as_toml() {
        assert_eq!(
            project_file_path(PathBuf::from("/home/me/myproject")),
            PathBuf::from("/home/me/myproject.toml")
        );
    }

    #[test]
    fn a_toml_name_is_kept_in_any_case() {
        for name in ["/home/me/a.toml", "/home/me/a.TOML", "/home/me/a.b.Toml"] {
            assert_eq!(project_file_path(PathBuf::from(name)), PathBuf::from(name));
        }
    }

    #[test]
    fn any_other_extension_gets_toml_appended() {
        for (name, saved) in [
            ("/home/me/a.txt", "/home/me/a.txt.toml"),
            ("/home/me/a.b.c", "/home/me/a.b.c.toml"),
        ] {
            assert_eq!(project_file_path(PathBuf::from(name)), PathBuf::from(saved));
        }
    }

    fn project() -> ProjectFile {
        ProjectFile::parse(
            r#"
[[templates]]
id = "air"
messages = [{ topic = "a/{{ device_id }}", payload = "{{ co2 }}" }]
fields.co2 = { generator = "constant", value = 1 }

[[devices]]
id = "office"
template = "air"
params = { room = "kitchen" }
fields.co2 = { value = 2 }

[[devices]]
id = "lab"
template = "air"
"#,
        )
        .unwrap()
    }

    fn rename_template(to: &str) -> Rename {
        Rename::Template {
            index: 0,
            from: "air".into(),
            to: to.into(),
        }
    }

    #[test]
    fn renaming_a_template_updates_the_devices_that_use_it() {
        let mut file = project();
        rename_template("air2").apply(&mut file).unwrap();
        assert_eq!(file.templates[0].id, "air2");
        assert!(file.devices.iter().all(|d| d.template == "air2"));
        assert!(file.validate(Path::new(".")).errors.is_empty());
    }

    #[test]
    fn renaming_a_field_carries_the_device_overrides() {
        let mut file = project();
        Rename::Field {
            template: 0,
            from: "co2".into(),
            to: "carbon".into(),
        }
        .apply(&mut file)
        .unwrap();
        assert!(file.templates[0].fields.contains_key("carbon"));
        assert!(file.devices[0].fields.contains_key("carbon"));
        assert!(!file.devices[0].fields.contains_key("co2"));
    }

    #[test]
    fn renaming_a_param_and_a_device() {
        let mut file = project();
        Rename::Param {
            device: 0,
            from: "room".into(),
            to: "floor".into(),
        }
        .apply(&mut file)
        .unwrap();
        Rename::Device {
            index: 1,
            from: "lab".into(),
            to: "lab-2".into(),
        }
        .apply(&mut file)
        .unwrap();
        assert_eq!(
            file.devices[0].params.get("floor"),
            Some(&Value::String("kitchen".into()))
        );
        assert_eq!(file.devices[1].id, "lab-2");
    }

    #[test]
    fn renames_are_checked() {
        let file = project();
        let check = |rename: Rename| rename.check(&file).unwrap_err();
        assert_eq!(check(rename_template(" ")), "the name must not be empty");
        assert_eq!(
            check(rename_template("co2")),
            "`co2` is a built-in template id"
        );
        assert_eq!(
            check(Rename::Device {
                index: 1,
                from: "lab".into(),
                to: "office".into()
            }),
            "a device `office` already exists"
        );
        assert_eq!(
            check(Rename::Param {
                device: 0,
                from: "room".into(),
                to: "".into()
            }),
            "the name must not be empty"
        );
        assert_eq!(
            check(Rename::Device {
                index: 1,
                from: "gone".into(),
                to: "x".into()
            }),
            "it was changed or removed in the meantime"
        );
        assert_eq!(rename_template("air").check(&file), Ok(()));
    }

    #[test]
    fn a_rename_that_fails_its_check_leaves_the_file_alone() {
        let mut file = project();
        let before = file.clone();
        assert!(rename_template("co2").apply(&mut file).is_err());
        assert_eq!(file, before);
    }

    #[test]
    fn removals_ignore_indices_out_of_range() {
        let mut file = project();
        Removal::Device(5).apply(&mut file);
        Removal::Template(1).apply(&mut file);
        assert_eq!(file, project());
        Removal::Device(0).apply(&mut file);
        assert_eq!(file.devices[0].id, "lab");
    }

    fn devices(src: &str) -> ProjectFile {
        ProjectFile::parse(src).unwrap()
    }

    #[test]
    fn a_duplicate_goes_right_after_its_device_under_a_free_id() {
        let mut file = devices(
            "[[devices]]\nid = \"a\"\ntemplate = \"co2\"\ncount = 3\n\n\
             [[devices]]\nid = \"b\"\ntemplate = \"co2\"\n",
        );
        assert_eq!(duplicate_device(&mut file, "a"), Some(1));
        assert_eq!(duplicate_device(&mut file, "a"), Some(1));
        let ids: Vec<&str> = file.devices.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["a", "a-copy-2", "a-copy", "b"]);
        assert_eq!(file.devices[1].count, 3, "everything else is copied");
        assert!(file.validate(Path::new(".")).errors.is_empty());
    }

    #[test]
    fn a_duplicate_whose_instance_ids_would_repeat_the_originals_uses_the_default_pattern() {
        let mut file = devices(
            "[[devices]]\nid = \"a\"\ntemplate = \"co2\"\ncount = 2\n\
             instance_id = \"room-{{ index }}\"\n\n\
             [[devices]]\nid = \"b\"\ntemplate = \"co2\"\ncount = 2\n\
             instance_id = \"{{ id }}-n{{ index }}\"\n",
        );
        duplicate_device(&mut file, "a");
        assert_eq!(file.devices[1].instance_id, None);
        duplicate_device(&mut file, "b");
        assert_eq!(
            file.devices[3].instance_id.as_deref(),
            Some("{{ id }}-n{{ index }}"),
            "a pattern with the device ID stays"
        );
        assert!(file.validate(Path::new(".")).errors.is_empty());
    }

    #[test]
    fn duplicating_an_unknown_device_changes_nothing() {
        let mut file = devices("");
        assert_eq!(duplicate_device(&mut file, "nope"), None);
        assert!(file.devices.is_empty());
    }
}
