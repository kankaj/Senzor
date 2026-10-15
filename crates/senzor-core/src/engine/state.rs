use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use crate::value::Value;

/// How many published messages the shared log keeps.
pub const LOG_CAPACITY: usize = 1000;

/// The engine calls this after state changes, at most once per [`NOTIFY_INTERVAL`].
pub type Notify = Arc<dyn Fn() + Send + Sync>;

pub const NOTIFY_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq)]
pub enum InstanceStatus {
    Stopped,
    Connecting,
    Online,
    Reconnecting { attempt: u32, last_error: String },
    Failed { reason: String },
}

impl InstanceStatus {
    pub fn is_running(&self) -> bool {
        matches!(
            self,
            InstanceStatus::Connecting
                | InstanceStatus::Online
                | InstanceStatus::Reconnecting { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstanceState {
    pub device: String,
    pub status: InstanceStatus,
    /// Field values of the last tick (or a freshly pinned value).
    pub values: BTreeMap<String, Value>,
    pub pinned: BTreeSet<String>,
    pub published: u64,
    pub errors: u64,
    pub last_publish: Option<SystemTime>,
    /// Running with a configuration that differs from the current project.
    pub needs_restart: bool,
    /// Running, but no longer part of the current project.
    pub orphaned: bool,
}

impl InstanceState {
    pub fn new(device: &str) -> Self {
        Self {
            device: device.to_string(),
            status: InstanceStatus::Stopped,
            values: BTreeMap::new(),
            pinned: BTreeSet::new(),
            published: 0,
            errors: 0,
            last_publish: None,
            needs_restart: false,
            orphaned: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    pub time: SystemTime,
    pub instance: String,
    pub topic: String,
    pub payload: String,
    pub qos: u8,
    pub retain: bool,
}

/// Everything the GUI and CLI can observe about a running engine.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EngineState {
    pub instances: BTreeMap<String, InstanceState>,
    /// The last [`LOG_CAPACITY`] published messages, oldest first.
    pub log: VecDeque<LogEntry>,
}

impl EngineState {
    pub fn push_log(&mut self, entry: LogEntry) {
        if self.log.len() == LOG_CAPACITY {
            self.log.pop_front();
        }
        self.log.push_back(entry);
    }
}

/// State shared between the supervisor, instance tasks and readers.
#[derive(Clone, Default)]
pub(crate) struct Shared {
    state: Arc<Mutex<EngineState>>,
    dirty: Arc<AtomicBool>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, EngineState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn update<R>(&self, f: impl FnOnce(&mut EngineState) -> R) -> R {
        let result = f(&mut self.lock());
        self.dirty.store(true, Ordering::Release);
        result
    }

    /// Runs `f` on one instance's state, if it exists.
    pub fn update_instance(&self, id: &str, f: impl FnOnce(&mut InstanceState)) {
        self.update(|state| {
            if let Some(instance) = state.instances.get_mut(id) {
                f(instance);
            }
        });
    }

    pub fn set_status(&self, id: &str, status: InstanceStatus) {
        match &status {
            InstanceStatus::Failed { reason } => {
                tracing::error!(instance = %id, "failed: {reason}")
            }
            InstanceStatus::Reconnecting {
                attempt,
                last_error,
            } => {
                tracing::info!(instance = %id, attempt, "reconnecting: {last_error}")
            }
            other => tracing::info!(instance = %id, "{other:?}"),
        }
        self.update_instance(id, |instance| instance.status = status);
    }

    /// Counts a published message and adds it to the log.
    pub fn record_publish(&self, entry: LogEntry) {
        self.update(|state| {
            if let Some(instance) = state.instances.get_mut(&entry.instance) {
                instance.published += 1;
                instance.last_publish = Some(entry.time);
            }
            state.push_log(entry);
        });
    }

    /// Counts a failed publish or render and logs it.
    pub fn record_error(&self, id: &str, message: &str) {
        tracing::warn!(instance = %id, "{message}");
        self.update_instance(id, |instance| instance.errors += 1);
    }

    pub fn snapshot(&self) -> EngineState {
        self.lock().clone()
    }

    /// Calls `notify` every [`NOTIFY_INTERVAL`] if anything changed since the last call.
    pub async fn run_notifier(self, notify: Notify) {
        let mut ticker = tokio::time::interval(NOTIFY_INTERVAL);
        loop {
            ticker.tick().await;
            if self.dirty.swap(false, Ordering::AcqRel) {
                notify();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn entry(n: usize) -> LogEntry {
        LogEntry {
            time: SystemTime::UNIX_EPOCH,
            instance: "i".into(),
            topic: format!("t/{n}"),
            payload: String::new(),
            qos: 0,
            retain: false,
        }
    }

    #[test]
    fn log_keeps_only_the_newest_entries() {
        let mut state = EngineState::default();
        for n in 0..LOG_CAPACITY + 5 {
            state.push_log(entry(n));
        }
        assert_eq!(state.log.len(), LOG_CAPACITY);
        assert_eq!(state.log.front().unwrap().topic, "t/5");
        assert_eq!(
            state.log.back().unwrap().topic,
            format!("t/{}", LOG_CAPACITY + 4)
        );
    }

    #[test]
    fn running_statuses() {
        assert!(InstanceStatus::Online.is_running());
        assert!(InstanceStatus::Connecting.is_running());
        assert!(
            InstanceStatus::Reconnecting {
                attempt: 1,
                last_error: String::new()
            }
            .is_running()
        );
        assert!(!InstanceStatus::Stopped.is_running());
        assert!(
            !InstanceStatus::Failed {
                reason: String::new()
            }
            .is_running()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn notifier_coalesces_changes_into_one_call_per_interval() {
        let shared = Shared::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        tokio::spawn(shared.clone().run_notifier(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        })));
        tokio::task::yield_now().await;

        for _ in 0..10 {
            shared.update(|_| ());
        }
        tokio::time::sleep(NOTIFY_INTERVAL + Duration::from_millis(1)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        tokio::time::sleep(NOTIFY_INTERVAL * 3).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1, "no changes, no calls");

        shared.update(|_| ());
        tokio::time::sleep(NOTIFY_INTERVAL + Duration::from_millis(1)).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
