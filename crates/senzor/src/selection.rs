use senzor_core::project::builtin::is_builtin;
use senzor_core::project::{Location, ProjectFile};

/// What the detail panel shows. Devices and templates are addressed by their index in the file,
/// so renaming them keeps the selection.
#[derive(Debug, Clone, PartialEq)]
pub enum Selection {
    Device(usize),
    Instance(String),
    Template(usize),
    Builtin(String),
}

/// Where clicking a problem takes the user.
#[derive(Debug, Clone, PartialEq)]
pub enum ProblemTarget {
    Select(Selection),
    Broker,
}

/// Where clicking a problem at `location` goes. Of several templates sharing an id it picks the
/// last, since validation reports "duplicate template id" on the later copies, not the first.
pub fn problem_target(location: &Location, file: &ProjectFile) -> Option<ProblemTarget> {
    match location {
        Location::Broker => Some(ProblemTarget::Broker),
        Location::Device { id } => file
            .devices
            .iter()
            .position(|d| &d.id == id)
            .map(|i| ProblemTarget::Select(Selection::Device(i))),
        Location::Template { id } => match file.templates.iter().rposition(|t| &t.id == id) {
            Some(i) => Some(ProblemTarget::Select(Selection::Template(i))),
            None if is_builtin(id) => Some(ProblemTarget::Select(Selection::Builtin(id.clone()))),
            None => None,
        },
        Location::Toml { .. } | Location::File => None,
    }
}

/// Drops a selection whose device or template no longer exists after an edit.
pub fn keep_valid(selection: Option<Selection>, file: &ProjectFile) -> Option<Selection> {
    match selection {
        Some(Selection::Device(i)) if i >= file.devices.len() => None,
        Some(Selection::Template(i)) if i >= file.templates.len() => None,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file() -> ProjectFile {
        ProjectFile::parse(
            r#"
[[templates]]
id = "t"
messages = [{ topic = "a", payload = "b" }]
fields.v = { generator = "constant", value = 1 }

[[devices]]
id = "first"
template = "t"

[[devices]]
id = "second"
template = "t"
"#,
        )
        .unwrap()
    }

    #[test]
    fn problems_map_to_the_item_they_are_about() {
        let file = file();
        let target = |location: Location| problem_target(&location, &file);
        assert_eq!(
            target(Location::Device {
                id: "second".into()
            }),
            Some(ProblemTarget::Select(Selection::Device(1)))
        );
        assert_eq!(
            target(Location::Template { id: "t".into() }),
            Some(ProblemTarget::Select(Selection::Template(0)))
        );
        assert_eq!(
            target(Location::Template { id: "co2".into() }),
            Some(ProblemTarget::Select(Selection::Builtin("co2".into())))
        );
        assert_eq!(target(Location::Broker), Some(ProblemTarget::Broker));
        assert_eq!(target(Location::Device { id: "gone".into() }), None);
        assert_eq!(target(Location::File), None);
    }

    #[test]
    fn a_duplicate_template_id_problem_selects_the_later_duplicate() {
        let mut file = file();
        file.templates.push(file.templates[0].clone());
        let location = Location::Template { id: "t".into() };
        assert!(
            file.validate(std::path::Path::new("."))
                .errors
                .iter()
                .any(|e| e.location == location && e.message == "duplicate template id")
        );
        assert_eq!(
            problem_target(&location, &file),
            Some(ProblemTarget::Select(Selection::Template(1)))
        );
    }

    #[test]
    fn selections_of_removed_items_are_dropped() {
        let file = file();
        assert_eq!(
            keep_valid(Some(Selection::Device(1)), &file),
            Some(Selection::Device(1))
        );
        assert_eq!(keep_valid(Some(Selection::Device(2)), &file), None);
        assert_eq!(keep_valid(Some(Selection::Template(1)), &file), None);
        assert_eq!(
            keep_valid(Some(Selection::Instance("x".into())), &file),
            Some(Selection::Instance("x".into()))
        );
    }
}
