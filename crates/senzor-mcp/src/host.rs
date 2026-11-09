//! The protocol between the MCP tools and the program that owns the project (the host).
//!
//! Tools never touch the document or the engine. Changes travel as a [`Request`] to the host,
//! which applies them one at a time with [`apply`] (so the CLI and the GUI behave the same) and
//! answers with a [`Reply`]. Reads go through the [`View`](crate::View) instead.

use std::path::{Path, PathBuf};

use senzor_core::engine::{Command, EngineObserver, EngineState, InstanceStatus, Target};
use senzor_core::project::builtin::is_builtin;
use senzor_core::project::edit::{Removal, Rename, copy_builtin, project_file_path};
use senzor_core::project::{
    BrokerSpec, DeviceSpec, Document, ProjectError, ProjectFile, TemplateSpec, Validated,
};
use senzor_core::value::Value;
use tokio::sync::oneshot;

use crate::key_path;
use crate::output::StepChanges;
use crate::redact::restore_password;

/// What the server needs from the program that owns the project and the engine.
pub trait Host {
    fn document(&self) -> &Document;
    /// Applies `f` to the project file; when it changed, revalidates it and hands the project to
    /// the engine, as an edit made by the user does. Returns whether the file changed.
    fn edit(&mut self, f: &mut dyn FnMut(&mut ProjectFile)) -> bool;
    /// Sends `command` to the engine, as a command given by the user is.
    fn send(&mut self, command: Command);
    /// Replaces the document and its engine, as opening a project does. With `autostart`, the
    /// devices with `autostart = true` start once the old engine has stopped.
    fn replace(&mut self, doc: Document, autostart: bool);
    /// Saves to the document's file, or to `path` (which then becomes its file).
    fn save(&mut self, path: Option<PathBuf>) -> Result<(), String>;
    fn observer(&self) -> EngineObserver;
    /// [`EngineHandle::commands_sent`](senzor_core::engine::EngineHandle::commands_sent) of the
    /// current engine.
    fn commands_sent(&self) -> u64;
    /// Takes back the latest edit as the user's Undo does, and hands the project to the engine.
    /// Returns false when there is nothing to undo.
    fn undo(&mut self) -> bool;
    /// Applies the latest undone edit again as the user's Redo does; false when there is none.
    fn redo(&mut self) -> bool;
    /// Why the host cannot save or replace the project right now (the GUI shows a dialog).
    fn busy(&self) -> Option<String> {
        None
    }
}

/// A request from a tool and the channel its reply goes back on.
pub struct Envelope {
    pub request: Request,
    pub reply: oneshot::Sender<Result<Reply, String>>,
}

impl Envelope {
    /// The tool stopped waiting (it timed out): the request must not be applied any more.
    pub fn is_cancelled(&self) -> bool {
        self.reply.is_closed()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunAction {
    Start,
    Stop,
    Kill,
    Restart,
}

/// Which instances a run action applies to.
#[derive(Debug, Clone, PartialEq)]
pub enum Targets {
    All,
    Devices(Vec<String>),
    Instances(Vec<String>),
}

/// A change a tool asks the host to make.
#[derive(Debug, Clone)]
pub enum Request {
    Run(RunAction, Targets),
    Pin {
        instance: String,
        field: String,
        value: Value,
    },
    Unpin {
        instance: String,
        field: Option<String>,
    },
    SetProject {
        toml: String,
        dry_run: bool,
    },
    PutDevice(DeviceSpec),
    RenameDevice {
        from: String,
        to: String,
    },
    RemoveDevice(String),
    PutTemplate(TemplateSpec),
    RenameTemplate {
        from: String,
        to: String,
    },
    RemoveTemplate(String),
    CopyBuiltin {
        builtin: String,
        new_id: String,
    },
    SetBroker(BrokerSpec),
    /// Sets the value at a key path (see the `set` tool).
    Set {
        path: String,
        value: serde_json::Value,
    },
    /// Removes the key at a key path, so its default applies.
    Unset {
        path: String,
    },
    Undo,
    Redo,
    NewProject {
        discard_unsaved: bool,
    },
    OpenProject {
        path: PathBuf,
        discard_unsaved: bool,
        autostart: bool,
    },
    SaveProject {
        path: Option<PathBuf>,
        overwrite: bool,
    },
}

/// An instance a run action waits for, with what it looked like before the action.
#[derive(Debug, Clone, PartialEq)]
pub struct Watched {
    pub id: String,
    pub runs_before: u64,
    pub was_running: bool,
}

/// What the host did. `sync` is the engine's [`Host::commands_sent`] right after the change:
/// once the engine has handled that many commands, its state reflects the change.
#[derive(Debug, Clone)]
pub enum Reply {
    Ran {
        action: RunAction,
        watched: Vec<Watched>,
        sync: u64,
    },
    Pinned {
        sync: u64,
    },
    Edited {
        changed: bool,
        sync: u64,
    },
    /// An undo or redo: what it changed, and what can be undone and redone now.
    Stepped {
        changes: StepChanges,
        can_undo: bool,
        can_redo: bool,
        sync: u64,
    },
    /// A dry run: the problems the project would have.
    Checked(Box<Validated>),
    Opened {
        sync: u64,
    },
    Saved {
        path: PathBuf,
    },
}

/// Applies `request` to `host`. The single place that decides what each request means.
pub fn apply(host: &mut impl Host, request: Request) -> Result<Reply, String> {
    match request {
        Request::Run(action, targets) => run(host, action, &targets),
        Request::Pin {
            instance,
            field,
            value,
        } => {
            check_field(host, &instance, &field)?;
            host.send(Command::Pin {
                instance,
                field,
                value,
            });
            Ok(Reply::Pinned {
                sync: host.commands_sent(),
            })
        }
        Request::Unpin { instance, field } => {
            let fields = match field {
                Some(field) => {
                    check_field(host, &instance, &field)?;
                    vec![field]
                }
                None => {
                    let state = running_instance(host, &instance)?;
                    state.pinned.into_iter().collect()
                }
            };
            for field in fields {
                host.send(Command::Unpin {
                    instance: instance.clone(),
                    field,
                });
            }
            Ok(Reply::Pinned {
                sync: host.commands_sent(),
            })
        }
        Request::SetProject { toml, dry_run } => {
            let mut file = ProjectFile::parse(&toml).map_err(|errors| problem_list(&errors))?;
            restore_password(&mut file.broker, &host.document().file().broker);
            if dry_run {
                let folder = project_folder(host.document().path());
                return Ok(Reply::Checked(Box::new(file.validate(&folder))));
            }
            edit(host, |f| *f = file.clone())
        }
        Request::PutDevice(device) => edit(host, |f| {
            match f.devices.iter_mut().find(|d| d.id == device.id) {
                Some(existing) => *existing = device.clone(),
                None => f.devices.push(device.clone()),
            }
        }),
        Request::RenameDevice { from, to } => {
            let index = device_index(host.document().file(), &from)?;
            try_edit(host, |f| {
                Rename::Device {
                    index,
                    from: from.clone(),
                    to: to.clone(),
                }
                .apply(f)
            })
        }
        Request::RemoveDevice(id) => {
            let index = device_index(host.document().file(), &id)?;
            edit(host, |f| Removal::Device(index).apply(f))
        }
        Request::PutTemplate(template) => {
            if is_builtin(&template.id) {
                return Err(format!(
                    "`{}` is a built-in template id; use another id, or copy_builtin_template",
                    template.id
                ));
            }
            edit(host, |f| {
                match f.templates.iter_mut().find(|t| t.id == template.id) {
                    Some(existing) => *existing = template.clone(),
                    None => f.templates.push(template.clone()),
                }
            })
        }
        Request::RenameTemplate { from, to } => {
            let index = template_index(host.document().file(), &from)?;
            try_edit(host, |f| {
                Rename::Template {
                    index,
                    from: from.clone(),
                    to: to.clone(),
                }
                .apply(f)
            })
        }
        Request::RemoveTemplate(id) => {
            let index = template_index(host.document().file(), &id)?;
            edit(host, |f| Removal::Template(index).apply(f))
        }
        Request::CopyBuiltin { builtin, new_id } => {
            try_edit(host, |f| copy_builtin(f, &builtin, &new_id).map(|_| ()))
        }
        Request::SetBroker(mut broker) => {
            restore_password(&mut broker, &host.document().file().broker);
            edit(host, |f| f.broker = broker.clone())
        }
        Request::Set { path, value } => try_edit(host, |f| {
            let mut changed = key_path::set(f, &path, &value)?;
            restore_password(&mut changed.broker, &f.broker);
            *f = changed;
            Ok(())
        }),
        Request::Unset { path } => try_edit(host, |f| {
            *f = key_path::unset(f, &path)?;
            Ok(())
        }),
        Request::Undo => step(host, false),
        Request::Redo => step(host, true),
        Request::NewProject { discard_unsaved } => {
            check_can_replace(host, discard_unsaved)?;
            host.replace(Document::new_empty(), false);
            Ok(Reply::Opened {
                sync: host.commands_sent(),
            })
        }
        Request::OpenProject {
            path,
            discard_unsaved,
            autostart,
        } => {
            check_can_replace(host, discard_unsaved)?;
            let doc = Document::open(&absolute(&path)?).map_err(|e| e.to_string())?;
            host.replace(doc, autostart);
            Ok(Reply::Opened {
                sync: host.commands_sent(),
            })
        }
        Request::SaveProject { path, overwrite } => save(host, path, overwrite),
    }
}

fn edit(host: &mut impl Host, mut f: impl FnMut(&mut ProjectFile)) -> Result<Reply, String> {
    let changed = host.edit(&mut f);
    Ok(Reply::Edited {
        changed,
        sync: host.commands_sent(),
    })
}

/// Like [`edit`] for a change that can be refused; a refused change leaves the file alone.
fn try_edit(
    host: &mut impl Host,
    mut f: impl FnMut(&mut ProjectFile) -> Result<(), String>,
) -> Result<Reply, String> {
    let mut result = Ok(());
    let changed = host.edit(&mut |file| result = f(file));
    result?;
    Ok(Reply::Edited {
        changed,
        sync: host.commands_sent(),
    })
}

/// Undoes (or with `redo`, redoes) the latest edit and says what that changed.
fn step(host: &mut impl Host, redo: bool) -> Result<Reply, String> {
    let before = host.document().file().clone();
    let done = if redo { host.redo() } else { host.undo() };
    if !done {
        return Err(if redo {
            "there is nothing to redo: redo applies again what undo took back, until the next edit"
        } else {
            "there is nothing to undo: the history starts when the project is opened or created"
        }
        .into());
    }
    let doc = host.document();
    Ok(Reply::Stepped {
        changes: StepChanges::between(&before, doc.file()),
        can_undo: doc.can_undo(),
        can_redo: doc.can_redo(),
        sync: host.commands_sent(),
    })
}

fn device_index(file: &ProjectFile, id: &str) -> Result<usize, String> {
    file.devices
        .iter()
        .position(|d| d.id == id)
        .ok_or_else(|| format!("the project has no device `{id}`"))
}

fn template_index(file: &ProjectFile, id: &str) -> Result<usize, String> {
    if let Some(index) = file.templates.iter().position(|t| t.id == id) {
        return Ok(index);
    }
    if is_builtin(id) {
        return Err(format!(
            "`{id}` is a built-in template; built-ins cannot be changed (copy_builtin_template copies one into the project)"
        ));
    }
    Err(format!("the project has no template `{id}`"))
}

/// One line per problem, as `senzor-cli validate` prints them.
pub(crate) fn problem_list(errors: &[ProjectError]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The folder relative TLS paths resolve against: the project file's, or the working directory.
fn project_folder(path: Option<&Path>) -> PathBuf {
    match path.and_then(Path::parent) {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// `path` resolved against the host's working directory.
fn absolute(path: &Path) -> Result<PathBuf, String> {
    std::path::absolute(path).map_err(|e| format!("cannot resolve {}: {e}", path.display()))
}

fn check_can_replace(host: &impl Host, discard_unsaved: bool) -> Result<(), String> {
    if let Some(reason) = host.busy() {
        return Err(reason);
    }
    if host.document().is_dirty() && !discard_unsaved {
        return Err(
            "the project has unsaved changes; save it first, or pass discard_unsaved = true".into(),
        );
    }
    Ok(())
}

fn save(host: &mut impl Host, path: Option<PathBuf>, overwrite: bool) -> Result<Reply, String> {
    if let Some(reason) = host.busy() {
        return Err(reason);
    }
    let Some(path) = path else {
        let path = host
            .document()
            .path()
            .map(absolute)
            .ok_or("the project has no file yet; pass a path to save it as")??;
        host.save(None)?;
        return Ok(Reply::Saved { path });
    };
    let target = project_file_path(absolute(&path)?);
    let current = host.document().path().and_then(|p| absolute(p).ok());
    if current.as_ref() != Some(&target) && std::fs::symlink_metadata(&target).is_ok() && !overwrite
    {
        return Err(format!(
            "{} already exists; pass overwrite = true to replace it",
            target.display()
        ));
    }
    host.save(Some(target.clone()))?;
    Ok(Reply::Saved { path: target })
}

/// The state of instance `id`, which must be running.
fn running_instance(
    host: &impl Host,
    id: &str,
) -> Result<senzor_core::engine::InstanceState, String> {
    let state = host
        .observer()
        .instance(id)
        .ok_or_else(|| format!("there is no instance `{id}`"))?;
    if !state.status.is_running() {
        return Err(format!("instance `{id}` is not running; start it first"));
    }
    Ok(state)
}

/// Checks that `field` is one of running instance `id`'s fields.
fn check_field(host: &impl Host, id: &str, field: &str) -> Result<(), String> {
    let state = running_instance(host, id)?;
    let known: Vec<String> = match host.document().validated().project.instance(id) {
        Some(config) => config.fields.keys().cloned().collect(),
        None => state.values.keys().cloned().collect(),
    };
    if known.iter().any(|f| f == field) {
        Ok(())
    } else {
        Err(format!(
            "instance `{id}` has no field `{field}`; its fields are: {}",
            known.join(", ")
        ))
    }
}

fn run(host: &mut impl Host, action: RunAction, targets: &Targets) -> Result<Reply, String> {
    let state = host.observer().snapshot();
    let doc = host.document();
    let validated = doc.validated();
    let starts = matches!(action, RunAction::Start | RunAction::Restart);
    let stops = !matches!(action, RunAction::Start);

    let commands: Vec<Target> = match targets {
        Targets::All => vec![Target::All],
        Targets::Devices(ids) => {
            let mut unknown = Vec::new();
            for id in ids {
                let in_file = doc.file().devices.iter().any(|d| &d.id == id);
                let has_orphans = state
                    .instances
                    .values()
                    .any(|i| &i.device == id && i.orphaned);
                if starts && in_file && validated.blocked.contains(id) {
                    return Err(format!(
                        "device `{id}` has validation errors and cannot start; get_project lists them"
                    ));
                }
                if !(in_file || (stops && has_orphans)) {
                    unknown.push(id.as_str());
                }
            }
            check_unknown("device", &unknown)?;
            ids.iter().cloned().map(Target::Device).collect()
        }
        Targets::Instances(ids) => {
            let unknown: Vec<&str> = ids
                .iter()
                .filter(|id| {
                    let in_project = validated.project.instance(id).is_some();
                    let in_state = state.instances.contains_key(id.as_str());
                    !(in_project || (stops && in_state))
                })
                .map(String::as_str)
                .collect();
            check_unknown("instance", &unknown)?;
            ids.iter().cloned().map(Target::Instance).collect()
        }
    };

    let watched = watched_instances(&state, validated, action, &commands);
    for target in commands {
        host.send(match action {
            RunAction::Start => Command::Start(target),
            RunAction::Stop => Command::Stop(target),
            RunAction::Kill => Command::Kill(target),
            RunAction::Restart => Command::Restart(target),
        });
    }
    Ok(Reply::Ran {
        action,
        watched,
        sync: host.commands_sent(),
    })
}

fn check_unknown(kind: &str, unknown: &[&str]) -> Result<(), String> {
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(format!("unknown {kind} id(s): {}", unknown.join(", ")))
    }
}

/// The instances `action` on `targets` affects, as the engine resolves them: current-project
/// instances for starts, instances in the state (orphans included) for stops.
fn watched_instances(
    state: &EngineState,
    validated: &Validated,
    action: RunAction,
    targets: &[Target],
) -> Vec<Watched> {
    let matches = |target: &Target, id: &str, device: &str| match target {
        Target::All => true,
        Target::Device(d) => d == device,
        Target::Instance(i) => i == id,
    };
    let mut ids: Vec<String> = Vec::new();
    for target in targets {
        if action != RunAction::Start {
            for (id, instance) in &state.instances {
                if matches(target, id, &instance.device) && !ids.contains(id) {
                    ids.push(id.clone());
                }
            }
        }
        if matches!(action, RunAction::Start | RunAction::Restart) {
            for config in validated.project.instances() {
                if matches(target, &config.id, &config.device) && !ids.contains(&config.id) {
                    ids.push(config.id.clone());
                }
            }
        }
    }
    ids.sort();
    ids.into_iter()
        .map(|id| {
            let instance = state.instances.get(&id);
            Watched {
                runs_before: instance.map_or(0, |i| i.runs),
                was_running: instance.is_some_and(|i| i.status.is_running()),
                id,
            }
        })
        .collect()
}

/// Whether `watched` has reached the state `action` leads to. An instance that left the state
/// (an orphan that stopped) has settled.
pub(crate) fn settled(action: RunAction, watched: &Watched, state: &EngineState) -> bool {
    let Some(instance) = state.instances.get(&watched.id) else {
        return true;
    };
    let up = matches!(
        instance.status,
        InstanceStatus::Online
            | InstanceStatus::Failed { .. }
            | InstanceStatus::Reconnecting { .. }
    );
    let new_run = instance.runs > watched.runs_before;
    match action {
        RunAction::Start if watched.was_running => up,
        RunAction::Start | RunAction::Restart => new_run && up,
        RunAction::Stop | RunAction::Kill => !instance.status.is_running(),
    }
}
