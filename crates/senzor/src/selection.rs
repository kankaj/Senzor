use senzor_core::project::builtin::is_builtin;
use senzor_core::project::{Location, ProjectFile};

/// What the detail panel shows. Devices and templates are addressed by their index in the file,
/// so renaming them keeps the selection.
#[derive(Debug, Clone, PartialEq)]
pub enum Selection {
    Device(usize),
    Instance(String),
    /// Several instances, for acting on all of them at once; at least two, in tree order.
    Instances(Vec<String>),
    Template(usize),
    Builtin(String),
}

/// How an instance in the tree was clicked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceClick {
    /// Selects it alone.
    Plain,
    /// Ctrl (Cmd on macOS): adds it to the selected instances, or takes it out.
    Toggle,
    /// Shift: selects every instance from the one clicked before (`anchor`) to this one.
    Range,
}

/// The selection after clicking instance `id`. `order` lists the instances in tree order, for
/// a range.
pub fn click_instance(
    current: Option<&Selection>,
    id: &str,
    click: InstanceClick,
    anchor: Option<&str>,
    order: &[String],
) -> Option<Selection> {
    let selected: Vec<String> = match current {
        Some(Selection::Instance(one)) => vec![one.clone()],
        Some(Selection::Instances(many)) => many.clone(),
        _ => Vec::new(),
    };
    let chosen: Vec<String> = match (click, anchor) {
        (InstanceClick::Toggle, _) if !selected.is_empty() => {
            let mut chosen = selected;
            match chosen.iter().position(|s| s == id) {
                Some(at) => {
                    chosen.remove(at);
                }
                None => chosen.push(id.to_string()),
            }
            chosen
        }
        (InstanceClick::Range, Some(anchor)) => {
            let at = |name: &str| order.iter().position(|o| o == name);
            match (at(anchor), at(id)) {
                (Some(from), Some(to)) => order[from.min(to)..=from.max(to)].to_vec(),
                _ => vec![id.to_string()],
            }
        }
        _ => vec![id.to_string()],
    };
    // Kept in tree order, whatever order they were clicked in.
    let mut chosen: Vec<String> = order
        .iter()
        .filter(|o| chosen.contains(o))
        .cloned()
        .chain(chosen.iter().filter(|c| !order.contains(c)).cloned())
        .collect();
    match chosen.len() {
        0 => None,
        1 => chosen.pop().map(Selection::Instance),
        _ => Some(Selection::Instances(chosen)),
    }
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

    fn order() -> Vec<String> {
        ["a-0", "a-1", "a-2", "b"].map(String::from).to_vec()
    }

    fn many(ids: &[&str]) -> Option<Selection> {
        Some(Selection::Instances(
            ids.iter().map(|s| s.to_string()).collect(),
        ))
    }

    #[test]
    fn a_plain_click_selects_one_instance() {
        let current = many(&["a-0", "a-1"]);
        assert_eq!(
            click_instance(current.as_ref(), "b", InstanceClick::Plain, None, &order()),
            Some(Selection::Instance("b".into()))
        );
    }

    #[test]
    fn a_toggle_click_adds_and_removes_instances() {
        let one = Some(Selection::Instance("a-1".into()));
        let two = click_instance(one.as_ref(), "a-0", InstanceClick::Toggle, None, &order());
        assert_eq!(two, many(&["a-0", "a-1"]), "kept in tree order");
        let three = click_instance(two.as_ref(), "b", InstanceClick::Toggle, None, &order());
        assert_eq!(three, many(&["a-0", "a-1", "b"]));
        let back = click_instance(three.as_ref(), "a-1", InstanceClick::Toggle, None, &order());
        assert_eq!(back, many(&["a-0", "b"]));
        let single = click_instance(back.as_ref(), "b", InstanceClick::Toggle, None, &order());
        assert_eq!(
            single,
            Some(Selection::Instance("a-0".into())),
            "one left is a single selection"
        );
        let device = Some(Selection::Device(0));
        assert_eq!(
            click_instance(device.as_ref(), "b", InstanceClick::Toggle, None, &order()),
            Some(Selection::Instance("b".into())),
            "a device is not kept with instances"
        );
    }

    #[test]
    fn a_range_click_selects_from_the_last_clicked_instance() {
        let current = Some(Selection::Instance("a-2".into()));
        assert_eq!(
            click_instance(
                current.as_ref(),
                "a-0",
                InstanceClick::Range,
                Some("a-2"),
                &order()
            ),
            many(&["a-0", "a-1", "a-2"])
        );
        assert_eq!(
            click_instance(current.as_ref(), "b", InstanceClick::Range, None, &order()),
            Some(Selection::Instance("b".into())),
            "without an anchor it is a plain click"
        );
    }

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
