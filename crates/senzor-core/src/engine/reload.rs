//! What to restart, start and stop when a running project is replaced by an edited one.

use std::collections::HashMap;

use crate::project::{InstanceConfig, Project};

use super::{Command, Target};

/// The instance changes that bring a run of one project in line with an edited one. Instances
/// whose configuration is the same in both keep running untouched.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Reload {
    /// The `[broker]` section changed: every instance that keeps running restarts.
    pub broker_changed: bool,
    /// Running instances to stop and start again with their new configuration.
    pub restart: Vec<String>,
    /// Instances that did not run and now should.
    pub start: Vec<String>,
    /// Running instances that should no longer run.
    pub stop: Vec<String>,
}

/// A [`Reload`] by device, for a person to read. Each entry is a device ID when the change takes
/// in the whole device, or an instance ID when some of the device's instances keep running.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReloadSummary {
    pub restarted: Vec<String>,
    pub started: Vec<String>,
    pub stopped: Vec<String>,
}

impl ReloadSummary {
    pub fn is_empty(&self) -> bool {
        self.restarted.is_empty() && self.started.is_empty() && self.stopped.is_empty()
    }
}

impl Reload {
    /// Compares the instances of `old_devices` in `old`, which run, with those of `new_devices`
    /// in `new`, which should. Device IDs that a project does not have are ignored.
    pub fn plan(
        old: &Project,
        old_devices: &[String],
        new: &Project,
        new_devices: &[String],
    ) -> Self {
        let broker_changed = old.broker != new.broker;
        let running: HashMap<&str, &InstanceConfig> = selected(old, old_devices)
            .map(|i| (i.id.as_str(), i))
            .collect();
        let mut plan = Self {
            broker_changed,
            ..Self::default()
        };
        for config in selected(new, new_devices) {
            match running.get(config.id.as_str()) {
                None => plan.start.push(config.id.clone()),
                Some(old) if broker_changed || *old != config => {
                    plan.restart.push(config.id.clone())
                }
                Some(_) => {}
            }
        }
        let wanted: Vec<&str> = selected(new, new_devices).map(|i| i.id.as_str()).collect();
        plan.stop = selected(old, old_devices)
            .filter(|i| !wanted.contains(&i.id.as_str()))
            .map(|i| i.id.clone())
            .collect();
        plan
    }

    pub fn is_empty(&self) -> bool {
        self.restart.is_empty() && self.start.is_empty() && self.stop.is_empty()
    }

    /// The engine commands that carry out this reload, beginning with swapping in `new`.
    pub fn commands(&self, new: Project) -> Vec<Command> {
        let instance = |id: &String| Target::Instance(id.clone());
        let mut commands = vec![Command::ReplaceProject(new)];
        commands.extend(self.stop.iter().map(|id| Command::Stop(instance(id))));
        commands.extend(self.restart.iter().map(|id| Command::Restart(instance(id))));
        commands.extend(self.start.iter().map(|id| Command::Start(instance(id))));
        commands
    }

    /// This reload by device; `old` and `new` are the projects given to [`Reload::plan`].
    /// A device whose instances all start, restart or stop is named as a whole; otherwise the
    /// instances that change are named and the rest keep running.
    pub fn summary(&self, old: &Project, new: &Project, new_devices: &[String]) -> ReloadSummary {
        let has = |list: &[String], id: &str| list.iter().any(|x| x == id);
        let stopped_of = |device: &str| -> Vec<String> {
            old.instances()
                .filter(|i| i.device == device && has(&self.stop, &i.id))
                .map(|i| i.id.clone())
                .collect()
        };
        let mut devices: Vec<&str> = new.devices.iter().map(|d| d.id.as_str()).collect();
        for device in &old.devices {
            if !devices.contains(&device.id.as_str()) {
                devices.push(&device.id);
            }
        }
        let mut summary = ReloadSummary::default();
        for device in devices {
            let running: Vec<&InstanceConfig> = new
                .device(device)
                .filter(|_| has(new_devices, device))
                .map(|d| d.instances.iter().collect())
                .unwrap_or_default();
            let restarted: Vec<String> = ids_in(&running, &self.restart);
            let started: Vec<String> = ids_in(&running, &self.start);
            let stopped = stopped_of(device);
            let kept = running.len() - restarted.len() - started.len();
            if restarted.is_empty() && started.is_empty() && stopped.is_empty() {
                continue;
            }
            let whole = device.to_string();
            match (kept, running.is_empty()) {
                (_, true) => summary.stopped.push(whole),
                (0, false) if restarted.is_empty() && stopped.is_empty() => {
                    summary.started.push(whole)
                }
                (0, false) => summary.restarted.push(whole),
                _ => {
                    summary.restarted.extend(restarted);
                    summary.started.extend(started);
                    summary.stopped.extend(stopped);
                }
            }
        }
        summary
    }
}

/// The instances of `devices` in `project`, in project order.
fn selected<'a>(
    project: &'a Project,
    devices: &'a [String],
) -> impl Iterator<Item = &'a InstanceConfig> {
    project
        .devices
        .iter()
        .filter(|d| devices.contains(&d.id))
        .flat_map(|d| d.instances.iter())
}

/// The IDs of `instances` that are in `list`.
fn ids_in(instances: &[&InstanceConfig], list: &[String]) -> Vec<String> {
    instances
        .iter()
        .filter(|i| list.contains(&i.id))
        .map(|i| i.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::test_support::valid_project;
    use super::*;

    fn project(broker: &str, devices: &str) -> Project {
        valid_project(&format!(
            r#"
[broker]
host = "127.0.0.1"
port = 1883
{broker}

[[templates]]
id = "air"
interval = "100ms"
messages = [{{ topic = "rl/{{{{ device_id }}}}", payload = "{{{{ v }}}}" }}]
fields.v = {{ generator = "constant", value = 1 }}

[[templates]]
id = "water"
interval = "100ms"
messages = [{{ topic = "rl/{{{{ device_id }}}}", payload = "{{{{ v }}}}" }}]
fields.v = {{ generator = "constant", value = 2 }}

{devices}
"#
        ))
    }

    fn device(id: &str, extra: &str) -> String {
        format!("[[devices]]\nid = \"{id}\"\ntemplate = \"air\"\n{extra}\n")
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn strings(list: &[String]) -> Vec<&str> {
        list.iter().map(String::as_str).collect()
    }

    /// Plans from `old` to `new`, both running every device, and returns the plan and summary.
    fn reload(old: &Project, new: &Project) -> (Reload, ReloadSummary) {
        let all = |p: &Project| p.devices.iter().map(|d| d.id.clone()).collect::<Vec<_>>();
        let plan = Reload::plan(old, &all(old), new, &all(new));
        let summary = plan.summary(old, new, &all(new));
        (plan, summary)
    }

    #[test]
    fn an_unchanged_project_changes_nothing() {
        let p = project("", &(device("a", "count = 2") + &device("b", "")));
        let (plan, summary) = reload(&p, &p.clone());
        assert_eq!(plan, Reload::default());
        assert!(plan.is_empty() && summary.is_empty());
    }

    #[test]
    fn a_changed_device_restarts_and_the_others_keep_running() {
        let old = project("", &(device("a", "count = 2") + &device("b", "")));
        let new = project(
            "",
            &(device("a", "count = 2\nfields.v = { value = 5 }") + &device("b", "")),
        );
        let (plan, summary) = reload(&old, &new);
        assert_eq!(strings(&plan.restart), ["a-0", "a-1"]);
        assert!(plan.start.is_empty() && plan.stop.is_empty());
        assert!(!plan.broker_changed);
        assert_eq!(strings(&summary.restarted), ["a"]);
        assert!(summary.started.is_empty() && summary.stopped.is_empty());
    }

    #[test]
    fn a_new_template_restarts_the_device() {
        let old = project("", &device("a", ""));
        let new = project("", &device("a", "").replace("\"air\"", "\"water\""));
        let (plan, summary) = reload(&old, &new);
        assert_eq!(strings(&plan.restart), ["a"]);
        assert_eq!(strings(&summary.restarted), ["a"]);
    }

    #[test]
    fn an_added_device_starts_and_a_removed_one_stops() {
        let old = project("", &(device("a", "") + &device("gone", "count = 2")));
        let new = project("", &(device("a", "") + &device("lab", "count = 2")));
        let (plan, summary) = reload(&old, &new);
        assert!(plan.restart.is_empty());
        assert_eq!(strings(&plan.start), ["lab-0", "lab-1"]);
        assert_eq!(strings(&plan.stop), ["gone-0", "gone-1"]);
        assert_eq!(strings(&summary.started), ["lab"]);
        assert_eq!(strings(&summary.stopped), ["gone"]);
        assert!(summary.restarted.is_empty());
    }

    #[test]
    fn a_higher_count_starts_only_the_new_instances() {
        let old = project("", &device("a", "count = 2"));
        let new = project("", &device("a", "count = 3"));
        let (plan, summary) = reload(&old, &new);
        assert!(plan.restart.is_empty() && plan.stop.is_empty());
        assert_eq!(strings(&plan.start), ["a-2"]);
        assert_eq!(
            strings(&summary.started),
            ["a-2"],
            "a-0 and a-1 keep running"
        );
    }

    #[test]
    fn a_lower_count_stops_only_the_instances_that_are_gone() {
        let old = project("", &device("a", "count = 3"));
        let new = project("", &device("a", "count = 2"));
        let (plan, summary) = reload(&old, &new);
        assert!(plan.restart.is_empty() && plan.start.is_empty());
        assert_eq!(strings(&plan.stop), ["a-2"]);
        assert_eq!(strings(&summary.stopped), ["a-2"]);
    }

    /// From one instance to two, the default instance IDs change (`a` to `a-0`, `a-1`): every
    /// instance of the device is new, so the device restarts.
    #[test]
    fn a_count_that_renames_every_instance_restarts_the_device() {
        let old = project("", &device("a", ""));
        let new = project("", &device("a", "count = 2"));
        let (plan, summary) = reload(&old, &new);
        assert_eq!(strings(&plan.stop), ["a"]);
        assert_eq!(strings(&plan.start), ["a-0", "a-1"]);
        assert_eq!(strings(&summary.restarted), ["a"]);
        assert!(summary.started.is_empty() && summary.stopped.is_empty());
    }

    #[test]
    fn a_changed_broker_restarts_every_running_instance() {
        let old = project("", &(device("a", "count = 2") + &device("b", "")));
        let new = project(
            "keep_alive = \"10s\"",
            &(device("a", "count = 2") + &device("b", "") + &device("c", "")),
        );
        let (plan, summary) = reload(&old, &new);
        assert!(plan.broker_changed);
        assert_eq!(strings(&plan.restart), ["a-0", "a-1", "b"]);
        assert_eq!(strings(&plan.start), ["c"]);
        assert_eq!(strings(&summary.restarted), ["a", "b"]);
        assert_eq!(strings(&summary.started), ["c"]);
    }

    #[test]
    fn only_the_selected_devices_run() {
        let old = project("", &(device("a", "") + &device("b", "") + &device("c", "")));
        let new = project(
            "",
            &(device("a", "fields.v = { value = 3 }")
                + &device("b", "fields.v = { value = 3 }")
                + &device("c", "")),
        );
        // a runs and changed; b did not run and now should; c ran and now should not.
        let plan = Reload::plan(&old, &ids(&["a", "c"]), &new, &ids(&["a", "b"]));
        assert_eq!(strings(&plan.restart), ["a"]);
        assert_eq!(strings(&plan.start), ["b"]);
        assert_eq!(strings(&plan.stop), ["c"]);
        let summary = plan.summary(&old, &new, &ids(&["a", "b"]));
        assert_eq!(
            (
                strings(&summary.restarted),
                strings(&summary.started),
                strings(&summary.stopped)
            ),
            (vec!["a"], vec!["b"], vec!["c"])
        );
    }

    #[test]
    fn selected_ids_the_project_lacks_are_ignored() {
        let old = project("", &device("a", ""));
        let new = project("", &device("b", ""));
        let plan = Reload::plan(&old, &ids(&["a", "ghost"]), &new, &ids(&["a", "ghost"]));
        assert_eq!(strings(&plan.stop), ["a"]);
        assert!(plan.start.is_empty() && plan.restart.is_empty());
    }

    /// A rename that keeps the instance IDs moves the running instance to the renamed device; it
    /// restarts once, under the new name.
    #[test]
    fn a_rename_that_keeps_the_instance_ids_restarts_under_the_new_name() {
        let old = project("", &device("a", "instance_id = \"room\""));
        let new = project("", &device("z", "instance_id = \"room\""));
        let (plan, summary) = reload(&old, &new);
        assert_eq!(strings(&plan.restart), ["room"]);
        assert!(plan.start.is_empty() && plan.stop.is_empty());
        assert_eq!(strings(&summary.restarted), ["z"]);
        assert!(summary.started.is_empty() && summary.stopped.is_empty());
    }

    #[test]
    fn commands_swap_the_project_first_then_act_on_instances() {
        let new = project("", &device("a", ""));
        let plan = Reload {
            broker_changed: false,
            restart: ids(&["r"]),
            start: ids(&["s"]),
            stop: ids(&["x"]),
        };
        let commands = plan.commands(new.clone());
        assert!(matches!(&commands[0], Command::ReplaceProject(p) if *p == new));
        let rest: Vec<String> = commands[1..].iter().map(|c| format!("{c:?}")).collect();
        assert_eq!(
            rest,
            [
                r#"Stop(Instance("x"))"#,
                r#"Restart(Instance("r"))"#,
                r#"Start(Instance("s"))"#
            ]
        );
    }
}
