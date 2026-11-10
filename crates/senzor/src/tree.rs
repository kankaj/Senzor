//! The device tree shown in the left panel, built from the file, its validation and engine state.

use std::collections::{HashMap, HashSet};

use senzor_core::engine::{EngineState, InstanceStatus};
use senzor_core::project::{DeviceConfig, ProjectFile, Validated};

use crate::i18n::t;

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceNode {
    /// Index in the project file; `None` for a device that was removed while its instances still run.
    pub index: Option<usize>,
    pub id: String,
    pub count: u32,
    /// Has validation errors, so its instances cannot start.
    pub blocked: bool,
    pub instances: Vec<InstanceNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstanceNode {
    pub id: String,
    pub status: InstanceStatus,
    pub needs_restart: bool,
    pub orphaned: bool,
}

/// Devices in file order. Each lists its instances in index order, followed by running
/// instances that left the project (orphans). Orphans of removed devices come last.
///
/// The GUI rebuilds the tree every frame and a project may hold thousands of instances,
/// so every lookup is hashed and each device only visits its own engine instances.
pub fn device_tree(
    file: &ProjectFile,
    validated: &Validated,
    state: &EngineState,
) -> Vec<DeviceNode> {
    // The first device with an ID wins, as in `Project::device`.
    let mut configs: HashMap<&str, &DeviceConfig> = HashMap::new();
    for device in &validated.project.devices {
        configs.entry(device.id.as_str()).or_insert(device);
    }
    // Grouped in the engine's (ID) order, which is the order orphans are listed in.
    let mut by_device: HashMap<&str, Vec<&str>> = HashMap::new();
    for (id, instance) in &state.instances {
        by_device
            .entry(instance.device.as_str())
            .or_default()
            .push(id.as_str());
    }

    let mut placed: HashSet<&str> = HashSet::new();
    let mut nodes: Vec<DeviceNode> = file
        .devices
        .iter()
        .enumerate()
        .map(|(index, spec)| {
            let mut ids: Vec<&str> = configs
                .get(spec.id.as_str())
                .map(|d| d.instances.iter().map(|i| i.id.as_str()).collect())
                .unwrap_or_default();
            let listed: HashSet<&str> = ids.iter().copied().collect();
            let orphans = by_device
                .get(spec.id.as_str())
                .into_iter()
                .flatten()
                .copied()
                .filter(|id| !listed.contains(id));
            ids.extend(orphans);
            placed.extend(ids.iter().copied());
            DeviceNode {
                index: Some(index),
                id: spec.id.clone(),
                count: spec.count,
                blocked: validated.blocked.contains(&spec.id),
                instances: ids.iter().map(|id| instance_node(id, state)).collect(),
            }
        })
        .collect();

    let mut removed: HashMap<&str, usize> = HashMap::new();
    for (id, instance) in &state.instances {
        if placed.contains(id.as_str()) {
            continue;
        }
        let at = *removed.entry(instance.device.as_str()).or_insert_with(|| {
            nodes.push(DeviceNode {
                index: None,
                id: instance.device.clone(),
                count: 0,
                blocked: false,
                instances: Vec::new(),
            });
            nodes.len() - 1
        });
        nodes[at].instances.push(instance_node(id, state));
    }
    nodes
}

fn instance_node(id: &str, state: &EngineState) -> InstanceNode {
    match state.instances.get(id) {
        Some(s) => InstanceNode {
            id: id.to_string(),
            status: s.status.clone(),
            needs_restart: s.needs_restart,
            orphaned: s.orphaned,
        },
        None => InstanceNode {
            id: id.to_string(),
            status: InstanceStatus::Stopped,
            needs_restart: false,
            orphaned: false,
        },
    }
}

impl DeviceNode {
    /// How many of the instances shown under the device are online.
    pub fn online(&self) -> usize {
        self.instances
            .iter()
            .filter(|i| i.status == InstanceStatus::Online)
            .count()
    }
}

/// Which instances the device tree shows, by status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusFilter {
    #[default]
    All,
    Online,
    /// Reconnecting or failed.
    Problems,
    Stopped,
}

impl StatusFilter {
    pub const ALL: [StatusFilter; 4] = [
        StatusFilter::All,
        StatusFilter::Online,
        StatusFilter::Problems,
        StatusFilter::Stopped,
    ];

    pub fn label(self) -> String {
        match self {
            StatusFilter::All => t!("tree-status-all"),
            StatusFilter::Online => t!("tree-status-online"),
            StatusFilter::Problems => t!("tree-status-problems"),
            StatusFilter::Stopped => t!("tree-status-stopped"),
        }
    }

    fn keeps(self, status: &InstanceStatus) -> bool {
        match self {
            StatusFilter::All => true,
            StatusFilter::Online => *status == InstanceStatus::Online,
            StatusFilter::Problems => matches!(
                status,
                InstanceStatus::Reconnecting { .. } | InstanceStatus::Failed { .. }
            ),
            StatusFilter::Stopped => *status == InstanceStatus::Stopped,
        }
    }
}

/// What the device tree shows: devices and instances whose ID contains `text` (in any case),
/// limited to instances in `status`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeFilter {
    pub text: String,
    pub status: StatusFilter,
}

impl TreeFilter {
    pub fn is_active(&self) -> bool {
        !self.text.trim().is_empty() || self.status != StatusFilter::All
    }
}

/// The devices and instances `filter` lets through. A device whose ID matches keeps every
/// instance the status allows; otherwise only its instances whose ID matches stay. A device left
/// with no instance is left out, unless only its name was asked for.
pub fn filter_tree(tree: Vec<DeviceNode>, filter: &TreeFilter) -> Vec<DeviceNode> {
    if !filter.is_active() {
        return tree;
    }
    let text = filter.text.trim().to_lowercase();
    let matches = |id: &str| id.to_lowercase().contains(&text);
    tree.into_iter()
        .filter_map(|mut node| {
            let device_matches = matches(&node.id);
            node.instances
                .retain(|i| filter.status.keeps(&i.status) && (device_matches || matches(&i.id)));
            let keep = !node.instances.is_empty()
                || (device_matches && filter.status == StatusFilter::All);
            keep.then_some(node)
        })
        .collect()
}

/// Why a running instance is an orphan (no longer part of the engine's project).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrphanCause {
    DeviceHasErrors,
    DeviceRemoved,
}

/// Why the orphans of `device` are orphans. A device with errors leaves the engine's project,
/// so its running instances are orphans although the device is still in the file.
pub fn orphan_cause(validated: &Validated, device: &str) -> OrphanCause {
    if validated.blocked.contains(device) {
        OrphanCause::DeviceHasErrors
    } else {
        OrphanCause::DeviceRemoved
    }
}

/// Whether Restart stops the instance and starts it again with the project's current settings:
/// only a running instance of the project. On Restart the engine starts a stopped instance and
/// stops an orphan for good (it has no settings to start with).
pub fn is_restartable(status: &InstanceStatus, orphaned: bool) -> bool {
    status.is_running() && !orphaned
}

/// IDs of the instances currently shown under a device, for device-level Stop and Kill.
/// (Targeting instances rather than the device also covers instances started under an old name.)
pub fn instance_ids(node: &DeviceNode) -> Vec<String> {
    node.instances.iter().map(|i| i.id.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use senzor_core::engine::InstanceState;
    use std::path::Path;

    fn file(src: &str) -> (ProjectFile, Validated) {
        let file = ProjectFile::parse(src).unwrap();
        let validated = file.validate(Path::new("."));
        (file, validated)
    }

    fn state(entries: &[(&str, &str, InstanceStatus, bool)]) -> EngineState {
        let mut state = EngineState::default();
        for (id, device, status, orphaned) in entries {
            let mut instance = InstanceState::new(device);
            instance.status = status.clone();
            instance.orphaned = *orphaned;
            state.instances.insert(id.to_string(), instance);
        }
        state
    }

    #[test]
    fn instances_follow_index_order_not_string_order() {
        let (file, validated) = file("[[devices]]\nid = \"d\"\ntemplate = \"co2\"\ncount = 11\n");
        let tree = device_tree(&file, &validated, &EngineState::default());
        let ids: Vec<&str> = tree[0].instances.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids[1], "d-1");
        assert_eq!(ids[2], "d-2");
        assert_eq!(ids[10], "d-10");
        assert_eq!(tree[0].count, 11);
        assert!(!tree[0].blocked);
    }

    #[test]
    fn status_comes_from_engine_state() {
        let (file, validated) = file("[[devices]]\nid = \"d\"\ntemplate = \"co2\"\n");
        let state = state(&[("d", "d", InstanceStatus::Online, false)]);
        let tree = device_tree(&file, &validated, &state);
        assert_eq!(tree[0].instances[0].status, InstanceStatus::Online);
    }

    #[test]
    fn blocked_device_keeps_its_running_orphans_visible() {
        let (file, validated) = file("[[devices]]\nid = \"d\"\ntemplate = \"nope\"\n");
        let state = state(&[("d", "d", InstanceStatus::Online, true)]);
        let tree = device_tree(&file, &validated, &state);
        assert_eq!(tree.len(), 1);
        assert!(tree[0].blocked);
        assert_eq!(tree[0].instances[0].id, "d");
        assert!(tree[0].instances[0].orphaned);
    }

    fn ids(tree: &[DeviceNode]) -> Vec<(String, Vec<String>)> {
        tree.iter()
            .map(|d| (d.id.clone(), instance_ids(d)))
            .collect()
    }

    fn two_devices() -> Vec<DeviceNode> {
        let (file, validated) = file(
            "[[devices]]\nid = \"office\"\ntemplate = \"co2\"\ncount = 3\n\n\
             [[devices]]\nid = \"lab\"\ntemplate = \"co2\"\ncount = 2\n",
        );
        let state = state(&[
            ("office-0", "office", InstanceStatus::Online, false),
            (
                "office-1",
                "office",
                InstanceStatus::Failed {
                    reason: "refused".into(),
                },
                false,
            ),
            (
                "lab-1",
                "lab",
                InstanceStatus::Reconnecting {
                    attempt: 1,
                    last_error: "gone".into(),
                },
                false,
            ),
        ]);
        device_tree(&file, &validated, &state)
    }

    #[test]
    fn no_filter_keeps_everything() {
        let tree = two_devices();
        assert_eq!(filter_tree(tree.clone(), &TreeFilter::default()), tree);
        assert!(!TreeFilter::default().is_active());
    }

    #[test]
    fn a_device_whose_name_matches_keeps_all_its_instances() {
        let filter = TreeFilter {
            text: "OFF".into(),
            status: StatusFilter::All,
        };
        assert_eq!(
            ids(&filter_tree(two_devices(), &filter)),
            [(
                "office".to_string(),
                vec!["office-0".to_string(), "office-1".into(), "office-2".into()]
            )]
        );
    }

    #[test]
    fn an_instance_whose_name_matches_is_kept_without_its_siblings() {
        let filter = TreeFilter {
            text: "-1".into(),
            status: StatusFilter::All,
        };
        assert_eq!(
            ids(&filter_tree(two_devices(), &filter)),
            [
                ("office".to_string(), vec!["office-1".to_string()]),
                ("lab".to_string(), vec!["lab-1".to_string()]),
            ]
        );
    }

    #[test]
    fn the_status_filter_keeps_matching_instances() {
        let only = |status| {
            ids(&filter_tree(
                two_devices(),
                &TreeFilter {
                    text: String::new(),
                    status,
                },
            ))
        };
        assert_eq!(
            only(StatusFilter::Online),
            [("office".to_string(), vec!["office-0".to_string()])]
        );
        assert_eq!(
            only(StatusFilter::Problems),
            [
                ("office".to_string(), vec!["office-1".to_string()]),
                ("lab".to_string(), vec!["lab-1".to_string()]),
            ]
        );
        assert_eq!(
            only(StatusFilter::Stopped),
            [
                ("office".to_string(), vec!["office-2".to_string()]),
                ("lab".to_string(), vec!["lab-0".to_string()]),
            ]
        );
    }

    #[test]
    fn text_and_status_filters_combine() {
        let filter = TreeFilter {
            text: "lab".into(),
            status: StatusFilter::Online,
        };
        assert!(filter_tree(two_devices(), &filter).is_empty());
    }

    #[test]
    fn online_counts_the_instances_that_are_online() {
        let tree = two_devices();
        assert_eq!(tree[0].online(), 1);
        assert_eq!(tree[1].online(), 0);
    }

    #[test]
    fn orphans_of_removed_devices_are_listed_last() {
        let (file, validated) = file("[[devices]]\nid = \"d\"\ntemplate = \"co2\"\n");
        let state = state(&[
            ("d", "d", InstanceStatus::Stopped, false),
            ("old-0", "old", InstanceStatus::Online, true),
            ("old-1", "old", InstanceStatus::Online, true),
        ]);
        let tree = device_tree(&file, &validated, &state);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[1].index, None);
        assert_eq!(tree[1].id, "old");
        assert_eq!(instance_ids(&tree[1]), ["old-0", "old-1"]);
    }
}
