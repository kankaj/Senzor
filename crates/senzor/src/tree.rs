//! The device tree shown in the left panel, built from the file, its validation and engine state.

use std::collections::{HashMap, HashSet};

use senzor_core::engine::{EngineState, InstanceStatus};
use senzor_core::project::{DeviceConfig, ProjectFile, Validated};

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
