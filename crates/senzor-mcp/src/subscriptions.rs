//! Resource subscriptions. A task per subscribing connection checks the subscribed resources'
//! versions every [`CHECK_INTERVAL`] and sends `notifications/resources/updated` for each one that
//! changed, at most once per [`MIN_INTERVAL`]: 500 instances publishing make one update of
//! `senzor://messages` every 250 ms, not 500 a tick. A change inside that interval is sent when it
//! ends, so the last change of a burst is never lost.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use rmcp::RoleServer;
use rmcp::model::ResourceUpdatedNotificationParam;
use rmcp::service::Peer;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{Instant, MissedTickBehavior};

use crate::live::Live;
use crate::view::View;

/// The shortest time between two updates of one resource.
pub(crate) const MIN_INTERVAL: Duration = Duration::from_millis(250);
/// How often the subscribed resources are checked for changes.
const CHECK_INTERVAL: Duration = Duration::from_millis(50);

/// The subscribed resources, each with its version when it was subscribed to.
pub(crate) type Watched = BTreeMap<Live, Option<u64>>;

/// Calls `notify` with the URI of every watched resource whose `version` changed, at most once
/// per [`MIN_INTERVAL`] each. Returns when `notify` returns false or `watched`'s sender is gone.
pub(crate) async fn watch<N, F>(
    mut watched: watch::Receiver<Watched>,
    version: impl Fn(Live) -> Option<u64>,
    mut notify: N,
) where
    N: FnMut(&'static str) -> F,
    F: Future<Output = bool>,
{
    struct Seen {
        version: Option<u64>,
        sent: Option<Instant>,
    }
    let mut seen: BTreeMap<Live, Seen> = BTreeMap::new();
    let mut ticker = tokio::time::interval(CHECK_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        if watched.has_changed().is_err() {
            return;
        }
        {
            let current = watched.borrow_and_update();
            seen.retain(|live, _| current.contains_key(live));
            for (live, version) in current.iter() {
                seen.entry(*live).or_insert(Seen {
                    version: *version,
                    sent: None,
                });
            }
        }
        let now = Instant::now();
        for (live, seen) in &mut seen {
            let version = version(*live);
            let throttled = seen.sent.is_some_and(|sent| now - sent < MIN_INTERVAL);
            if version == seen.version || throttled {
                continue;
            }
            if !notify(live.uri()).await {
                return;
            }
            seen.version = version;
            seen.sent = Some(now);
        }
    }
}

/// What one connection subscribed to with `resources/subscribe`. A clone starts with no
/// subscriptions, as every connection serves its own clone of the server; dropping it (the
/// connection ends) stops the updates.
#[derive(Default)]
pub(crate) struct Subscriptions(Mutex<Option<Active>>);

struct Active {
    watched: watch::Sender<Watched>,
    task: JoinHandle<()>,
}

impl Drop for Active {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Clone for Subscriptions {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl Subscriptions {
    /// Sends `peer` an update whenever `live` changes from now on.
    pub fn subscribe(&self, live: Live, view: &View, peer: &Peer<RoleServer>) {
        let since = live.version(view);
        let mut active = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if active.as_ref().is_none_or(|a| a.task.is_finished()) {
            let (sender, receiver) = watch::channel(Watched::new());
            let (view, peer) = (view.clone(), peer.clone());
            let task = tokio::spawn(watch(
                receiver,
                move |live| live.version(&view),
                move |uri| {
                    let peer = peer.clone();
                    async move {
                        peer.notify_resource_updated(ResourceUpdatedNotificationParam::new(uri))
                            .await
                            .is_ok()
                    }
                },
            ));
            *active = Some(Active {
                watched: sender,
                task,
            });
        }
        if let Some(active) = active.as_ref() {
            active.watched.send_modify(|watched| {
                watched.insert(live, since);
            });
        }
    }

    pub fn unsubscribe(&self, live: Live) {
        let active = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(active) = active.as_ref() {
            active.watched.send_modify(|watched| {
                watched.remove(&live);
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;

    /// A watcher of `senzor://project` whose version is `version`; returns the times of its
    /// updates since the start.
    fn watcher(version: Arc<AtomicU64>) -> (watch::Sender<Watched>, Arc<Mutex<Vec<Duration>>>) {
        let start = Instant::now();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let (sender, receiver) = watch::channel(Watched::from([(Live::Project, Some(0))]));
        let record = sent.clone();
        tokio::spawn(watch(
            receiver,
            move |_| Some(version.load(Ordering::SeqCst)),
            move |_| {
                record.lock().unwrap().push(start.elapsed());
                std::future::ready(true)
            },
        ));
        (sender, sent)
    }

    async fn sleep_ms(ms: u64) {
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_is_sent_without_a_change() {
        let (_sender, sent) = watcher(Arc::new(AtomicU64::new(0)));
        sleep_ms(1000).await;
        assert!(sent.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_gives_one_update_at_once_and_one_for_its_end() {
        let version = Arc::new(AtomicU64::new(0));
        let (_sender, sent) = watcher(version.clone());
        sleep_ms(10).await;
        // A change every 10 ms for 200 ms.
        for _ in 0..20 {
            version.fetch_add(1, Ordering::SeqCst);
            sleep_ms(10).await;
        }
        sleep_ms(1000).await;
        let sent = sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 2, "{sent:?}");
        assert!(sent[0] <= Duration::from_millis(60), "{sent:?}");
        assert!(sent[1] - sent[0] >= MIN_INTERVAL, "{sent:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn steady_changes_are_sent_once_per_interval() {
        let version = Arc::new(AtomicU64::new(0));
        let (_sender, sent) = watcher(version.clone());
        for _ in 0..200 {
            version.fetch_add(1, Ordering::SeqCst);
            sleep_ms(10).await;
        }
        let sent = sent.lock().unwrap().clone();
        assert!((7..=9).contains(&sent.len()), "{sent:?}");
        assert!(sent.windows(2).all(|w| w[1] - w[0] >= MIN_INTERVAL));
    }

    #[tokio::test(start_paused = true)]
    async fn an_unwatched_resource_is_not_sent_and_dropping_the_sender_ends_the_task() {
        let version = Arc::new(AtomicU64::new(0));
        let (sender, sent) = watcher(version.clone());
        sender.send_modify(|w| {
            w.remove(&Live::Project);
        });
        sleep_ms(100).await;
        version.fetch_add(1, Ordering::SeqCst);
        sleep_ms(500).await;
        assert!(sent.lock().unwrap().is_empty());

        drop(sender);
        sleep_ms(100).await;
        assert_eq!(Arc::strong_count(&version), 1, "the watcher has ended");
    }
}
