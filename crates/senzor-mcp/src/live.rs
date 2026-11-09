//! Resources whose content changes: the project, the instances and the message log. A client
//! can subscribe to them (see `subscriptions`) instead of polling the tools.

use std::hash::{DefaultHasher, Hash, Hasher};

use senzor_core::engine::{EngineState, InstanceStatus};
use serde::Serialize;

use crate::output::{Message, ProjectOut, Status, problems};
use crate::params::DEFAULT_MESSAGE_LIMIT;
use crate::redact::redacted;
use crate::view::{Snapshot, View};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Live {
    Project,
    Instances,
    Messages,
}

impl Live {
    pub const ALL: [Live; 3] = [Live::Project, Live::Instances, Live::Messages];

    pub fn find(uri: &str) -> Option<Live> {
        Self::ALL.into_iter().find(|live| live.uri() == uri)
    }

    pub fn uri(self) -> &'static str {
        match self {
            Live::Project => "senzor://project",
            Live::Instances => "senzor://instances",
            Live::Messages => "senzor://messages",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Live::Project => "project",
            Live::Instances => "instances",
            Live::Messages => "messages",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Live::Project => {
                "The project as get_project returns it: TOML, file, unsaved-changes flag, problems and blocked devices"
            }
            Live::Instances => {
                "Every instance with its status, pinned fields and restart/orphan flags (counters and values are in get_status and get_instance)"
            }
            Live::Messages => {
                "The newest 100 published messages and last_seq, as get_messages returns them without arguments"
            }
        }
    }

    /// The content as JSON.
    pub fn read(self, view: &View) -> Result<String, String> {
        let json = match self {
            Live::Project => serde_json::to_string_pretty(&project(&view.get()?)),
            Live::Instances => serde_json::to_string_pretty(&view.observer()?.read(instances)),
            Live::Messages => serde_json::to_string_pretty(&view.observer()?.read(messages)),
        };
        json.map_err(|e| format!("internal error: {e}"))
    }

    /// A number that changes when the content may have, cheap enough to ask for often. None
    /// before the host first published.
    pub fn version(self, view: &View) -> Option<u64> {
        match self {
            Live::Project => view.project_version(),
            Live::Instances => Some(view.observer().ok()?.read(instances_hash)),
            Live::Messages => Some(
                view.observer()
                    .ok()?
                    .read(|s| s.log.back().map_or(0, |entry| entry.seq)),
            ),
        }
    }
}

/// What `get_project` and `senzor://project` show.
pub(crate) fn project(snapshot: &Snapshot) -> ProjectOut {
    ProjectOut {
        toml: redacted(&snapshot.file).to_toml_string(),
        path: snapshot.path.as_deref().map(|p| p.display().to_string()),
        dirty: snapshot.dirty,
        problems: problems(&snapshot.validated),
        blocked: snapshot.validated.blocked.iter().cloned().collect(),
    }
}

#[derive(Serialize)]
struct InstancesOut {
    instances: Vec<InstanceOut>,
}

#[derive(Serialize)]
struct InstanceOut {
    id: String,
    device: String,
    status: Status,
    pinned: Vec<String>,
    needs_restart: bool,
    orphaned: bool,
}

fn instances(state: &EngineState) -> InstancesOut {
    InstancesOut {
        instances: state
            .instances
            .iter()
            .map(|(id, i)| InstanceOut {
                id: id.clone(),
                device: i.device.clone(),
                status: (&i.status).into(),
                pinned: i.pinned.iter().cloned().collect(),
                needs_restart: i.needs_restart,
                orphaned: i.orphaned,
            })
            .collect(),
    }
}

/// A hash of everything [`instances`] shows, without building it.
fn instances_hash(state: &EngineState) -> u64 {
    let mut hasher = DefaultHasher::new();
    for (id, i) in &state.instances {
        (id, &i.device, &i.pinned, i.needs_restart, i.orphaned).hash(&mut hasher);
        match &i.status {
            InstanceStatus::Stopped => 0.hash(&mut hasher),
            InstanceStatus::Connecting => 1.hash(&mut hasher),
            InstanceStatus::Online => 2.hash(&mut hasher),
            InstanceStatus::Reconnecting {
                attempt,
                last_error,
            } => (3, attempt, last_error).hash(&mut hasher),
            InstanceStatus::Failed { reason } => (4, reason).hash(&mut hasher),
        }
    }
    hasher.finish()
}

#[derive(Serialize)]
struct MessagesOut {
    messages: Vec<Message>,
    last_seq: u64,
}

fn messages(state: &EngineState) -> MessagesOut {
    let page = state.log_after(0, DEFAULT_MESSAGE_LIMIT, |_| true);
    MessagesOut {
        messages: page.entries.iter().map(Message::from).collect(),
        last_seq: page.last_seq,
    }
}
