//! Runs instances on a tokio runtime and exposes their state.

mod instance;
mod mqtt;
mod state;
#[cfg(test)]
mod test_support;

use std::collections::{BTreeSet, HashMap, HashSet};

use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::{Id, JoinError, JoinHandle, JoinSet};

use crate::project::{BrokerConfig, InstanceConfig, Project};
use crate::value::Value;
use instance::{InstanceCmd, STOP_TIMEOUT, run_instance};
use state::Shared;

pub use state::{
    EngineState, InstanceState, InstanceStatus, LOG_CAPACITY, LogEntry, NOTIFY_INTERVAL, Notify,
};

/// Which instances a command applies to.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    All,
    /// All instances of a `[[devices]]` entry.
    Device(String),
    Instance(String),
}

impl Target {
    fn matches(&self, instance_id: &str, device: &str) -> bool {
        match self {
            Target::All => true,
            Target::Device(id) => id == device,
            Target::Instance(id) => id == instance_id,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    /// Starts stopped or failed instances of the current project; running ones are left alone.
    Start(Target),
    Stop(Target),
    Kill(Target),
    /// Stops running instances gracefully and, once each has stopped, starts it again with the
    /// current project's configuration (spec §8.6, `needs_restart`). Orphans are only stopped.
    /// Instances of the current project that are not running are started, as with `Start`.
    Restart(Target),
    Pin {
        instance: String,
        field: String,
        value: Value,
    },
    Unpin {
        instance: String,
        field: String,
    },
    /// Swaps the project. Running instances keep their configuration until restarted.
    ReplaceProject(Project),
    #[cfg(test)]
    PanicInstance(String),
}

/// Handle to a running engine. Dropping it stops every instance in the background without
/// waiting; `shutdown` waits until they have stopped.
pub struct EngineHandle {
    tx: mpsc::UnboundedSender<Command>,
    shared: Shared,
    task: JoinHandle<()>,
}

impl EngineHandle {
    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }

    pub fn snapshot(&self) -> EngineState {
        self.shared.snapshot()
    }

    /// Stops every instance gracefully; whatever still runs after 2 s is killed.
    pub async fn shutdown(self) {
        let Self { tx, task, .. } = self;
        // The closed channel tells the supervisor to shut down once it has handled the queue.
        drop(tx);
        let _ = task.await;
    }
}

pub struct Engine;

impl Engine {
    /// Starts the supervisor on `runtime`. All instances begin `Stopped`.
    pub fn spawn(project: Project, runtime: &Handle, notify: Notify) -> EngineHandle {
        let shared = Shared::default();
        shared.update(|state| sync_instances(state, &project, &HashMap::new()));
        let notifier = runtime.spawn(shared.clone().run_notifier(notify));
        let supervisor = Supervisor {
            project,
            shared: shared.clone(),
            running: HashMap::new(),
            restarting: HashSet::new(),
            tasks: JoinSet::new(),
            task_ids: HashMap::new(),
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let task = runtime.spawn(supervisor.run(rx, notifier));
        EngineHandle { tx, shared, task }
    }
}

struct Running {
    commands: mpsc::UnboundedSender<InstanceCmd>,
    config: InstanceConfig,
    broker: BrokerConfig,
}

struct Supervisor {
    project: Project,
    shared: Shared,
    running: HashMap<String, Running>,
    /// Running instances to start again when their task ends. An instance cannot start while its
    /// old task still runs, so `Restart` has to wait for the stop to finish.
    restarting: HashSet<String>,
    tasks: JoinSet<()>,
    task_ids: HashMap<Id, String>,
}

impl Supervisor {
    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Command>, notifier: JoinHandle<()>) {
        loop {
            tokio::select! {
                command = rx.recv() => match command {
                    Some(command) => self.handle(command),
                    None => {
                        self.shutdown().await;
                        notifier.abort();
                        return;
                    }
                },
                Some(result) = self.tasks.join_next_with_id(), if !self.tasks.is_empty() => {
                    self.finished(result);
                }
            }
        }
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::Start(target) => {
                for id in self.project_targets(&target) {
                    self.start(&id);
                }
            }
            Command::Stop(target) => {
                self.cancel_restarts(&target);
                self.send_running(&target, || InstanceCmd::Stop);
            }
            Command::Kill(target) => {
                self.cancel_restarts(&target);
                self.send_running(&target, || InstanceCmd::Kill);
            }
            Command::Restart(target) => {
                for id in self.running_targets(&target) {
                    self.send_to(&id, InstanceCmd::Stop);
                    self.restarting.insert(id);
                }
                for id in self.project_targets(&target) {
                    self.start(&id);
                }
            }
            Command::Pin {
                instance,
                field,
                value,
            } => self.send_to(&instance, InstanceCmd::Pin(field, value)),
            Command::Unpin { instance, field } => {
                self.send_to(&instance, InstanceCmd::Unpin(field))
            }
            Command::ReplaceProject(project) => {
                self.project = project;
                let (project, running) = (&self.project, &self.running);
                self.shared
                    .update(|state| sync_instances(state, project, running));
            }
            #[cfg(test)]
            Command::PanicInstance(id) => self.send_to(&id, InstanceCmd::Panic),
        }
    }

    /// Instance IDs of the current project matching `target`.
    fn project_targets(&self, target: &Target) -> Vec<String> {
        self.project
            .instances()
            .filter(|i| target.matches(&i.id, &i.device))
            .map(|i| i.id.clone())
            .collect()
    }

    /// IDs of running instances matching `target`, orphans included.
    fn running_targets(&self, target: &Target) -> Vec<String> {
        self.running
            .iter()
            .filter(|(id, running)| target.matches(id, &running.config.device))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// The latest command wins: a `Stop` or `Kill` after a `Restart` leaves the instance stopped.
    fn cancel_restarts(&mut self, target: &Target) {
        for id in self.running_targets(target) {
            self.restarting.remove(&id);
        }
    }

    fn send_running(&self, target: &Target, command: impl Fn() -> InstanceCmd) {
        for id in self.running_targets(target) {
            self.send_to(&id, command());
        }
    }

    /// Sends `command` to instance `id` if it is running.
    fn send_to(&self, id: &str, command: InstanceCmd) {
        if let Some(running) = self.running.get(id) {
            let _ = running.commands.send(command);
        }
    }

    fn start(&mut self, id: &str) {
        if self.running.contains_key(id) {
            return;
        }
        let Some(config) = self.project.instance(id).cloned() else {
            return;
        };
        let broker = self.project.broker.clone();
        let (commands, rx) = mpsc::unbounded_channel();
        let handle = self.tasks.spawn(run_instance(
            config.clone(),
            broker.clone(),
            rx,
            self.shared.clone(),
        ));
        self.task_ids.insert(handle.id(), id.to_string());
        self.running.insert(
            id.to_string(),
            Running {
                commands,
                config,
                broker,
            },
        );
        self.shared
            .update_instance(id, |instance| instance.needs_restart = false);
    }

    fn finished(&mut self, result: Result<(Id, ()), JoinError>) {
        let (task_id, failure) = match result {
            Ok((task_id, ())) => (task_id, None),
            Err(error) if error.is_panic() => {
                let task_id = error.id();
                (task_id, Some(panic_message(error)))
            }
            Err(error) => (error.id(), None),
        };
        let Some(id) = self.task_ids.remove(&task_id) else {
            return;
        };
        self.running.remove(&id);
        if let Some(reason) = failure {
            self.shared
                .set_status(&id, InstanceStatus::Failed { reason });
        }
        self.shared.update(|state| mark_ended(state, &id));
        if self.restarting.remove(&id) {
            self.start(&id);
        }
    }

    async fn shutdown(&mut self) {
        // `finished` would otherwise start them again, and nothing would stop the new tasks.
        self.restarting.clear();
        for running in self.running.values() {
            let _ = running.commands.send(InstanceCmd::Stop);
        }
        if tokio::time::timeout(STOP_TIMEOUT, self.drain())
            .await
            .is_err()
        {
            self.tasks.abort_all();
            self.drain().await;
        }
    }

    /// Waits until every instance task has ended.
    async fn drain(&mut self) {
        while let Some(result) = self.tasks.join_next_with_id().await {
            self.finished(result);
        }
    }
}

fn panic_message(error: JoinError) -> String {
    let payload = error.into_panic();
    let text = payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown".to_string());
    format!("panic: {text}")
}

/// Aligns the instance list with `project`: adds new instances as `Stopped`, flags running ones
/// whose configuration changed (`needs_restart`) or that left the project (`orphaned`), and drops
/// stopped instances that left the project.
fn sync_instances(state: &mut EngineState, project: &Project, running: &HashMap<String, Running>) {
    let mut current = BTreeSet::new();
    for config in project.instances() {
        current.insert(config.id.clone());
        let instance = state
            .instances
            .entry(config.id.clone())
            .or_insert_with(|| InstanceState::new(&config.device));
        instance.device = config.device.clone();
        instance.orphaned = false;
        instance.needs_restart = running
            .get(&config.id)
            .is_some_and(|r| r.config != *config || r.broker != project.broker);
    }
    state.instances.retain(|id, instance| {
        if current.contains(id) {
            true
        } else if running.contains_key(id) {
            instance.orphaned = true;
            instance.needs_restart = false;
            true
        } else {
            false
        }
    });
}

/// Updates an instance whose task has ended: an orphan leaves the list, any other instance is
/// stopped and current.
fn mark_ended(state: &mut EngineState, id: &str) {
    let Some(instance) = state.instances.get_mut(id) else {
        return;
    };
    if instance.orphaned {
        state.instances.remove(id);
        return;
    }
    instance.needs_restart = false;
    // An aborted task never reaches its final `set_status`.
    if instance.status.is_running() {
        instance.status = InstanceStatus::Stopped;
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{WAIT, eventually, valid_project};
    use super::*;
    use senzor_testkit::{Subscriber, start_broker};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::time::Instant;

    fn project(port: u16, devices: &str) -> Project {
        let src = format!(
            r#"
[broker]
host = "127.0.0.1"
port = {port}

[[templates]]
id = "air"
interval = "100ms"
availability = {{ topic = "sv/{{{{ device_id }}}}/status", online = "online", offline = "offline" }}
messages = [{{ topic = "sv/{{{{ device_id }}}}/state", payload = "{{{{ v }}}}" }}]
fields.v = {{ generator = "constant", value = 1 }}

{devices}
"#
        );
        valid_project(&src)
    }

    const TWO_DEVICES: &str = r#"
[[devices]]
id = "a"
template = "air"
count = 2

[[devices]]
id = "b"
template = "air"
"#;

    fn spawn(project: Project) -> EngineHandle {
        Engine::spawn(project, &Handle::current(), Arc::new(|| {}))
    }

    async fn wait_until(engine: &EngineHandle, what: &str, pred: impl Fn(&EngineState) -> bool) {
        eventually(
            || {
                format!(
                    "timed out waiting for {what}: {:#?}",
                    engine.snapshot().instances
                )
            },
            || pred(&engine.snapshot()),
        )
        .await;
    }

    async fn wait_for_status(engine: &EngineHandle, id: &str, expected: InstanceStatus) {
        wait_until(engine, &format!("{id} {expected:?}"), |s| {
            status(s, id) == expected
        })
        .await;
    }

    async fn start_online(engine: &EngineHandle, id: &str) {
        engine.send(Command::Start(Target::Instance(id.into())));
        wait_for_status(engine, id, InstanceStatus::Online).await;
    }

    fn status(state: &EngineState, id: &str) -> InstanceStatus {
        state.instances[id].status.clone()
    }

    fn all_in(state: &EngineState, expected: &InstanceStatus) -> bool {
        state.instances.values().all(|i| i.status == *expected)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn instances_start_stopped_and_targets_select_them() {
        let port = start_broker();
        let engine = spawn(project(port, TWO_DEVICES));
        let ids: Vec<String> = engine.snapshot().instances.keys().cloned().collect();
        assert_eq!(ids, ["a-0", "a-1", "b"]);
        assert!(all_in(&engine.snapshot(), &InstanceStatus::Stopped));

        engine.send(Command::Start(Target::Device("a".into())));
        wait_until(&engine, "a-* online", |s| {
            status(s, "a-0") == InstanceStatus::Online && status(s, "a-1") == InstanceStatus::Online
        })
        .await;
        assert_eq!(status(&engine.snapshot(), "b"), InstanceStatus::Stopped);

        start_online(&engine, "b").await;

        engine.send(Command::Stop(Target::All));
        wait_until(&engine, "all stopped", |s| {
            all_in(s, &InstanceStatus::Stopped)
        })
        .await;
        engine.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn commands_for_instances_that_are_not_running_are_ignored() {
        let port = start_broker();
        let engine = spawn(project(port, TWO_DEVICES));
        engine.send(Command::Stop(Target::Instance("a-0".into())));
        engine.send(Command::Kill(Target::Device("missing".into())));
        engine.send(Command::Pin {
            instance: "b".into(),
            field: "v".into(),
            value: Value::Number(2.0),
        });
        engine.send(Command::Start(Target::Instance("missing".into())));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(all_in(&engine.snapshot(), &InstanceStatus::Stopped));
        engine.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn pin_routes_to_the_instance() {
        let port = start_broker();
        let mut sub = Subscriber::connect(port, "sv/b/state").await;
        let engine = spawn(project(port, TWO_DEVICES));
        engine.send(Command::Start(Target::Instance("b".into())));
        sub.wait_for(WAIT, |m| m.payload == "1").await.unwrap();
        engine.send(Command::Pin {
            instance: "b".into(),
            field: "v".into(),
            value: Value::Number(7.0),
        });
        sub.wait_for(WAIT, |m| m.payload == "7").await.unwrap();
        engine.send(Command::Unpin {
            instance: "b".into(),
            field: "v".into(),
        });
        sub.wait_for(WAIT, |m| m.payload == "1").await.unwrap();
        engine.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn replace_project_flags_restart_and_orphans() {
        let port = start_broker();
        let engine = spawn(project(port, TWO_DEVICES));
        engine.send(Command::Start(Target::All));
        wait_until(&engine, "all online", |s| {
            all_in(s, &InstanceStatus::Online)
        })
        .await;

        // a: count 2 -> 1 (a-1 orphaned) and a new param (a-0 needs restart); b unchanged; c new.
        let changed = project(
            port,
            r#"
[[devices]]
id = "a"
template = "air"
count = 1
instance_id = "a-{{ index }}"
params = { site = "x" }

[[devices]]
id = "b"
template = "air"

[[devices]]
id = "c"
template = "air"
"#,
        );
        engine.send(Command::ReplaceProject(changed));
        wait_until(&engine, "flags", |s| {
            s.instances.get("a-1").is_some_and(|i| i.orphaned)
        })
        .await;
        let state = engine.snapshot();
        assert!(state.instances["a-0"].needs_restart);
        assert!(!state.instances["b"].needs_restart);
        assert_eq!(state.instances["c"].status, InstanceStatus::Stopped);

        engine.send(Command::Stop(Target::Instance("a-1".into())));
        wait_until(&engine, "orphan removed", |s| {
            !s.instances.contains_key("a-1")
        })
        .await;

        engine.send(Command::Stop(Target::Instance("a-0".into())));
        wait_for_status(&engine, "a-0", InstanceStatus::Stopped).await;
        assert!(!engine.snapshot().instances["a-0"].needs_restart);
        engine.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn restart_runs_the_instance_with_the_current_configuration() {
        let port = start_broker();
        let mut sub = Subscriber::connect(port, "sv/b/state").await;
        let engine = spawn(project(port, TWO_DEVICES));
        engine.send(Command::Start(Target::Instance("b".into())));
        sub.wait_for(WAIT, |m| m.payload == "1").await.unwrap();

        let changed = project(
            port,
            r#"
[[devices]]
id = "a"
template = "air"
count = 2

[[devices]]
id = "b"
template = "air"
fields.v = { value = 2 }
"#,
        );
        engine.send(Command::ReplaceProject(changed));
        wait_until(&engine, "b needs restart", |s| {
            s.instances["b"].needs_restart
        })
        .await;

        engine.send(Command::Restart(Target::Instance("b".into())));
        sub.wait_for(WAIT, |m| m.payload == "2").await.unwrap();
        wait_until(&engine, "b online and current", |s| {
            status(s, "b") == InstanceStatus::Online && !s.instances["b"].needs_restart
        })
        .await;
        assert_eq!(
            status(&engine.snapshot(), "a-0"),
            InstanceStatus::Stopped,
            "instances outside the target are left alone"
        );
        engine.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn restart_starts_a_stopped_instance() {
        let port = start_broker();
        let engine = spawn(project(port, TWO_DEVICES));
        engine.send(Command::Restart(Target::Instance("b".into())));
        wait_for_status(&engine, "b", InstanceStatus::Online).await;
        engine.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn restart_stops_an_orphan_without_starting_it_again() {
        let port = start_broker();
        let engine = spawn(project(port, TWO_DEVICES));
        start_online(&engine, "b").await;

        let without_b = project(
            port,
            r#"
[[devices]]
id = "a"
template = "air"
count = 2
"#,
        );
        engine.send(Command::ReplaceProject(without_b));
        wait_until(&engine, "b orphaned", |s| s.instances["b"].orphaned).await;

        engine.send(Command::Restart(Target::Instance("b".into())));
        wait_until(&engine, "orphan removed", |s| {
            !s.instances.contains_key("b")
        })
        .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(!engine.snapshot().instances.contains_key("b"));
        engine.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_after_restart_cancels_the_restart() {
        let port = start_broker();
        let engine = spawn(project(port, TWO_DEVICES));
        start_online(&engine, "b").await;

        engine.send(Command::Restart(Target::Instance("b".into())));
        engine.send(Command::Stop(Target::Instance("b".into())));
        wait_for_status(&engine, "b", InstanceStatus::Stopped).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(status(&engine.snapshot(), "b"), InstanceStatus::Stopped);
        engine.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_does_not_start_instances_waiting_for_a_restart() {
        let port = start_broker();
        let engine = spawn(project(port, TWO_DEVICES));
        start_online(&engine, "b").await;

        engine.send(Command::Restart(Target::Instance("b".into())));
        let started = Instant::now();
        engine.shutdown().await;
        assert!(
            started.elapsed() < STOP_TIMEOUT,
            "a restarted instance would outlive the graceful stop and be killed at the deadline"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn panicking_instance_fails_alone() {
        let port = start_broker();
        let engine = spawn(project(port, TWO_DEVICES));
        engine.send(Command::Start(Target::All));
        wait_until(&engine, "all online", |s| {
            all_in(s, &InstanceStatus::Online)
        })
        .await;

        engine.send(Command::PanicInstance("b".into()));
        wait_until(&engine, "b failed", |s| {
            matches!(status(s, "b"), InstanceStatus::Failed { .. })
        })
        .await;
        assert_eq!(
            status(&engine.snapshot(), "b"),
            InstanceStatus::Failed {
                reason: "panic: test panic".into()
            }
        );
        assert_eq!(status(&engine.snapshot(), "a-0"), InstanceStatus::Online);

        start_online(&engine, "b").await;
        engine.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_stops_everything_gracefully() {
        let port = start_broker();
        let mut sub = Subscriber::connect(port, "sv/+/status").await;
        let engine = spawn(project(port, TWO_DEVICES));
        engine.send(Command::Start(Target::All));
        wait_until(&engine, "all online", |s| {
            all_in(s, &InstanceStatus::Online)
        })
        .await;
        sub.collect(Duration::from_millis(200)).await;

        let started = Instant::now();
        engine.shutdown().await;
        assert!(started.elapsed() < STOP_TIMEOUT + Duration::from_millis(500));

        let offline = sub.collect(Duration::from_millis(500)).await;
        let mut topics: Vec<String> = offline
            .iter()
            .filter(|m| m.payload == "offline")
            .map(|m| m.topic.clone())
            .collect();
        topics.sort();
        assert_eq!(
            topics,
            ["sv/a-0/status", "sv/a-1/status", "sv/b/status"],
            "one offline each, no Last Will"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn notify_is_called_after_changes() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        // No instance is started, so nothing connects to port 1.
        let engine = Engine::spawn(
            project(1, TWO_DEVICES),
            &Handle::current(),
            Arc::new(move || {
                counter.fetch_add(1, Ordering::SeqCst);
            }),
        );
        tokio::time::sleep(NOTIFY_INTERVAL * 2).await;
        assert!(calls.load(Ordering::SeqCst) >= 1);
        engine.shutdown().await;
    }
}
