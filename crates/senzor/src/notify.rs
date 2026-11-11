//! Desktop notifications when instances fail while the window is in the background.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use senzor_core::engine::{EngineObserver, InstanceState, InstanceStatus};

use crate::i18n::{Language, t};

#[cfg(unix)]
use crate::icon::APP_ID;

/// Desktop notifications, behind a trait so tests can record them instead.
pub trait Notifier {
    /// Shows a notification. Never fails: a notification that cannot be shown is only logged.
    fn notify(&mut self, title: &str, body: &str);
}

/// The desktop's notifications: freedesktop notifications over D-Bus on Linux, toasts on
/// Windows, the notification centre on macOS.
pub struct DesktopNotifier;

impl Notifier for DesktopNotifier {
    fn notify(&mut self, title: &str, body: &str) {
        #[cfg(target_os = "macos")]
        use_own_bundle();
        let mut notification = notify_rust::Notification::new();
        notification.summary(title).body(body).appname("Senzor");
        // The desktop finds the app's name and icon through its `.desktop` file.
        #[cfg(all(unix, not(target_os = "macos")))]
        notification
            .icon(APP_ID)
            .hint(notify_rust::Hint::DesktopEntry(APP_ID.to_string()));
        // Sending waits for D-Bus or the notification centre to answer, so not on the UI thread.
        let sent = std::thread::Builder::new()
            .name("notification".into())
            .spawn(move || {
                if let Err(error) = notification.show() {
                    eprintln!("Could not show a desktop notification: {error}");
                }
            });
        if let Err(error) = sent {
            eprintln!("Could not show a desktop notification: {error}");
        }
    }
}

/// Inside `Senzor.app`, notifications come from Senzor, with its name and icon; otherwise
/// notify-rust sends them as another app (Finder). Only when macOS knows an app of that name with
/// Senzor's bundle identifier, so a bare `senzor` binary keeps notifying as before.
#[cfg(target_os = "macos")]
fn use_own_bundle() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if notify_rust::get_bundle_identifier_or_default("Senzor") == APP_ID {
            // An error only means another app stays the sender.
            let _ = notify_rust::set_application(APP_ID);
        }
    });
}

/// An instance that failed, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub instance: String,
    pub reason: String,
}

/// The instances that are `Failed` in `instances` but were not in `failed_before`.
pub fn newly_failed(
    failed_before: &BTreeSet<String>,
    instances: &BTreeMap<String, InstanceState>,
) -> Vec<Failure> {
    instances
        .iter()
        .filter(|(id, _)| !failed_before.contains(*id))
        .filter_map(|(id, instance)| match &instance.status {
            InstanceStatus::Failed { reason } => Some(Failure {
                instance: id.clone(),
                reason: reason.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// The IDs of the instances that are `Failed`.
fn failed_ids(instances: &BTreeMap<String, InstanceState>) -> BTreeSet<String> {
    instances
        .iter()
        .filter(|(_, i)| matches!(i.status, InstanceStatus::Failed { .. }))
        .map(|(id, _)| id.clone())
        .collect()
}

/// The lines a notification's body lists, at most.
const BODY_LINES: usize = 3;

/// The title and body of one notification about `failures`, in the language of this thread
/// (the window's: [`FailureWatch::look`] runs where the window is drawn).
pub fn message(failures: &[Failure]) -> (String, String) {
    let title = t!("notify-title", count = failures.len());
    let mut lines: Vec<String> = failures
        .iter()
        .take(BODY_LINES)
        .map(|f| format!("{}: {}", f.instance, f.reason))
        .collect();
    if failures.len() > BODY_LINES {
        lines.push(t!("notify-more", count = failures.len() - BODY_LINES));
    }
    (title, lines.join("\n"))
}

/// How long failures are gathered into one notification, from the first one on.
pub const GATHER: Duration = Duration::from_secs(1);

/// What [`FailureWatch::look`] found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Look {
    /// The notification to show now: its title and body.
    pub notify: Option<(String, String)>,
    /// Look again after this long, to show failures still being gathered.
    pub again_in: Option<Duration>,
}

/// Notifies about failures from a thread of its own. While the window is in the background it
/// often draws no frames at all (minimized, or hidden by the compositor, as KDE on Wayland and
/// Windows do), so the engine's change callback wakes this watch ([`Self::waker`]), not the
/// window's frames. The window only says, each frame, whether to notify now and in which
/// language ([`Self::tell`]).
pub struct FailureNotifications {
    wake: mpsc::Sender<Wake>,
    /// What the window last told the watch.
    told: Telling,
    notifier: Arc<Mutex<Box<dyn Notifier + Send>>>,
}

/// What the window last said: whether failures are to be told now (the setting is on and the
/// window is in the background), and the language of its texts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Telling {
    now: bool,
    language: Language,
}

enum Wake {
    /// The engine's state changed.
    Changed,
    /// The window's [`Telling`] changed. What failed until now happened under the old one.
    Tell(Telling),
    /// Another engine runs now (another project opened).
    Engine {
        generation: u64,
        observer: EngineObserver,
    },
    /// The window closes.
    Stop,
}

impl FailureNotifications {
    /// Starts the watch, showing notifications with `notifier`.
    pub fn start(notifier: Box<dyn Notifier + Send>) -> Self {
        let (wake, wakes) = mpsc::channel();
        let told = Telling {
            now: false,
            language: crate::i18n::language(),
        };
        let notifier = Arc::new(Mutex::new(notifier));
        let thread_notifier = notifier.clone();
        let started = std::thread::Builder::new()
            .name("failure-watch".into())
            .spawn(move || watch(&wakes, told, &thread_notifier));
        if let Err(error) = started {
            eprintln!("Could not watch for failures: {error}");
        }
        Self {
            wake,
            told,
            notifier,
        }
    }

    /// Shows notifications with `notifier` from now on.
    pub fn set_notifier(&self, notifier: Box<dyn Notifier + Send>) {
        *self.notifier.lock().unwrap_or_else(PoisonError::into_inner) = notifier;
    }

    /// A callback for the engine to call after its state changes: it wakes the watch.
    pub fn waker(&self) -> impl Fn() + Send + Sync + 'static {
        let wake = self.wake.clone();
        move || {
            let _ = wake.send(Wake::Changed);
        }
    }

    /// Watches the engine `observer` reads from, the `generation`th of this window.
    pub fn watch(&self, generation: u64, observer: EngineObserver) {
        let _ = self.wake.send(Wake::Engine {
            generation,
            observer,
        });
    }

    /// Whether failures are to be told now, and in which language. Only a change reaches the
    /// watch, in order with the engine's changes: a failure from before the window went to the
    /// background was seen and is not told.
    pub fn tell(&mut self, now: bool, language: Language) {
        let telling = Telling { now, language };
        if telling != self.told {
            self.told = telling;
            let _ = self.wake.send(Wake::Tell(telling));
        }
    }
}

impl Drop for FailureNotifications {
    fn drop(&mut self) {
        let _ = self.wake.send(Wake::Stop);
    }
}

/// The watch's thread: looks at the instances whenever the engine changes, and again when
/// failures being gathered are due.
fn watch(
    wakes: &mpsc::Receiver<Wake>,
    mut telling: Telling,
    notifier: &Mutex<Box<dyn Notifier + Send>>,
) {
    let mut failures = FailureWatch::default();
    let mut engine: Option<(u64, EngineObserver)> = None;
    let mut again_in: Option<Duration> = None;
    // Looks at the engine's instances as `telling` says, notifies when due, and says when to
    // look again.
    let mut look = |engine: &Option<(u64, EngineObserver)>, telling: Telling| {
        let (generation, observer) = engine.as_ref()?;
        crate::i18n::set_language(telling.language);
        let look = observer.read(|state| {
            failures.look(*generation, &state.instances, Instant::now(), telling.now)
        });
        if let Some((title, body)) = look.notify {
            notifier
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .notify(&title, &body);
        }
        look.again_in
    };
    loop {
        let wake = match again_in {
            Some(after) => wakes.recv_timeout(after),
            None => wakes.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match wake {
            Ok(Wake::Changed) | Err(RecvTimeoutError::Timeout) => {}
            Ok(Wake::Tell(told)) => {
                look(&engine, telling);
                telling = told;
            }
            Ok(Wake::Engine {
                generation,
                observer,
            }) => engine = Some((generation, observer)),
            Ok(Wake::Stop) | Err(RecvTimeoutError::Disconnected) => return,
        }
        again_in = look(&engine, telling);
    }
}

/// Follows the instances from look to look and tells when to notify about failures.
#[derive(Debug, Default)]
pub struct FailureWatch {
    /// The engine the instances belong to (it changes when another project opens).
    engine: u64,
    /// The instances that were `Failed` at the last look.
    failed: BTreeSet<String>,
    /// Failures gathered for the next notification.
    gathered: Vec<Failure>,
    /// When the first of `gathered` was seen.
    since: Option<Instant>,
}

impl FailureWatch {
    /// Looks at the instances of engine `engine` at `now`. `notify` says whether failures are to
    /// be told about now: the setting is on and the window is in the background.
    pub fn look(
        &mut self,
        engine: u64,
        instances: &BTreeMap<String, InstanceState>,
        now: Instant,
        notify: bool,
    ) -> Look {
        let failed = failed_ids(instances);
        if engine != self.engine {
            *self = Self {
                engine,
                failed,
                ..Self::default()
            };
            return Look::default();
        }
        let new = newly_failed(&self.failed, instances);
        self.failed = failed;
        if !notify {
            self.gathered.clear();
            self.since = None;
            return Look::default();
        }
        if !new.is_empty() {
            self.since.get_or_insert(now);
            self.gathered.extend(new);
        }
        let Some(since) = self.since else {
            return Look::default();
        };
        let waited = now.saturating_duration_since(since);
        if waited < GATHER {
            return Look {
                notify: None,
                again_in: Some(GATHER - waited),
            };
        }
        self.since = None;
        Look {
            notify: Some(message(&std::mem::take(&mut self.gathered))),
            again_in: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instances(statuses: &[(&str, InstanceStatus)]) -> BTreeMap<String, InstanceState> {
        statuses
            .iter()
            .map(|(id, status)| {
                let mut state = InstanceState::new("d");
                state.status = status.clone();
                (id.to_string(), state)
            })
            .collect()
    }

    fn failed(reason: &str) -> InstanceStatus {
        InstanceStatus::Failed {
            reason: reason.into(),
        }
    }

    fn failure(instance: &str, reason: &str) -> Failure {
        Failure {
            instance: instance.into(),
            reason: reason.into(),
        }
    }

    #[test]
    fn only_instances_that_were_not_failed_before_are_newly_failed() {
        let now = instances(&[
            ("a", failed("refused")),
            ("b", failed("timed out")),
            ("c", InstanceStatus::Online),
            (
                "d",
                InstanceStatus::Reconnecting {
                    attempt: 2,
                    last_error: "gone".into(),
                },
            ),
        ]);
        let before = BTreeSet::from(["b".to_string()]);
        assert_eq!(newly_failed(&before, &now), [failure("a", "refused")]);
    }

    #[test]
    fn one_failure_is_named_in_the_title_and_body() {
        let (title, body) = message(&[failure(
            "office-0",
            "broker refused the connection: BadUserNamePassword",
        )]);
        assert_eq!(title, "Senzor: an instance failed");
        assert_eq!(
            body,
            "office-0: broker refused the connection: BadUserNamePassword"
        );
    }

    #[test]
    fn several_failures_make_one_message_with_the_first_three() {
        let failures: Vec<_> = (0..5)
            .map(|n| failure(&format!("office-{n}"), "refused"))
            .collect();
        let (title, body) = message(&failures);
        assert_eq!(title, "Senzor: 5 instances failed");
        assert_eq!(
            body,
            "office-0: refused\noffice-1: refused\noffice-2: refused\nand 2 more"
        );
        let (title, body) = message(&failures[..3]);
        assert_eq!(title, "Senzor: 3 instances failed");
        assert_eq!(
            body,
            "office-0: refused\noffice-1: refused\noffice-2: refused"
        );
    }

    #[test]
    fn the_message_counts_in_czech() {
        crate::i18n::set_language(crate::i18n::Language::Czech);
        let failures = |n: usize| -> Vec<_> {
            (0..n)
                .map(|i| failure(&format!("o-{i}"), "refused"))
                .collect()
        };
        assert_eq!(message(&failures(1)).0, "Senzor: instance selhala");
        assert_eq!(message(&failures(3)).0, "Senzor: 3 instance selhaly");
        let (title, body) = message(&failures(8));
        assert_eq!(title, "Senzor: 8 instancí selhalo");
        assert!(body.ends_with("\na 5 dalších"), "{body}");
        assert!(message(&failures(4)).1.ends_with("\na 1 další"));
    }

    /// A watch that has seen `a` and `b` online in engine 1.
    fn watching(start: Instant) -> FailureWatch {
        let mut watch = FailureWatch::default();
        let online = instances(&[("a", InstanceStatus::Online), ("b", InstanceStatus::Online)]);
        assert_eq!(watch.look(1, &online, start, true), Look::default());
        watch
    }

    #[test]
    fn failures_seen_within_a_moment_make_one_notification() {
        let start = Instant::now();
        let mut watch = watching(start);
        let a_failed = instances(&[("a", failed("refused")), ("b", InstanceStatus::Online)]);
        let look = watch.look(1, &a_failed, start, true);
        assert_eq!(look.notify, None, "gathered first");
        assert_eq!(look.again_in, Some(GATHER));

        let both = instances(&[("a", failed("refused")), ("b", failed("refused"))]);
        let half = start + GATHER / 2;
        assert_eq!(watch.look(1, &both, half, true).again_in, Some(GATHER / 2));

        let look = watch.look(1, &both, start + GATHER, true);
        assert_eq!(
            look.notify,
            Some((
                "Senzor: 2 instances failed".to_string(),
                "a: refused\nb: refused".to_string()
            ))
        );
        assert_eq!(look.again_in, None);
        assert_eq!(
            watch.look(1, &both, start + GATHER * 3, true),
            Look::default(),
            "told once"
        );
    }

    #[test]
    fn nothing_is_told_while_notifying_is_off() {
        let start = Instant::now();
        let mut watch = watching(start);
        let a_failed = instances(&[("a", failed("refused")), ("b", InstanceStatus::Online)]);
        assert_eq!(watch.look(1, &a_failed, start, false), Look::default());
        assert_eq!(
            watch.look(1, &a_failed, start + GATHER, true),
            Look::default(),
            "a failed while the window was focused"
        );
    }

    #[test]
    fn failures_gathered_are_dropped_when_the_window_comes_to_the_front() {
        let start = Instant::now();
        let mut watch = watching(start);
        let a_failed = instances(&[("a", failed("refused")), ("b", InstanceStatus::Online)]);
        watch.look(1, &a_failed, start, true);
        assert_eq!(
            watch.look(1, &a_failed, start + GATHER, false),
            Look::default()
        );
        assert_eq!(
            watch.look(1, &a_failed, start + GATHER * 2, true),
            Look::default()
        );
    }

    #[test]
    fn failures_already_there_when_another_engine_starts_are_not_told() {
        let start = Instant::now();
        let mut watch = watching(start);
        let a_failed = instances(&[("a", failed("refused"))]);
        assert_eq!(watch.look(2, &a_failed, start, true), Look::default());
        assert_eq!(
            watch.look(2, &a_failed, start + GATHER, true),
            Look::default()
        );
    }

    #[test]
    fn an_instance_that_fails_again_after_a_restart_is_told_again() {
        let start = Instant::now();
        let mut watch = watching(start);
        let a_failed = instances(&[("a", failed("refused"))]);
        watch.look(1, &a_failed, start, true);
        assert!(
            watch
                .look(1, &a_failed, start + GATHER, true)
                .notify
                .is_some()
        );
        let restarted = instances(&[("a", InstanceStatus::Connecting)]);
        watch.look(1, &restarted, start + GATHER * 2, true);
        watch.look(1, &a_failed, start + GATHER * 3, true);
        let look = watch.look(1, &a_failed, start + GATHER * 4, true);
        assert_eq!(
            look.notify,
            Some((
                "Senzor: an instance failed".to_string(),
                "a: refused".to_string()
            ))
        );
    }
}
