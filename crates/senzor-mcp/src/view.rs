//! What the server reads without asking the host: a copy of the document and the engine's observer.
//! Reads keep working while the host is busy (or, in the GUI, while no frames are drawn).

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use senzor_core::engine::EngineObserver;
use senzor_core::project::{Document, ProjectFile, Validated};

/// The project as the host last published it.
#[derive(Clone)]
pub(crate) struct Snapshot {
    pub file: ProjectFile,
    pub validated: Validated,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    /// Why the last attempt to open a project file failed, until another project is opened.
    pub open_error: Option<String>,
    pub observer: EngineObserver,
}

/// The read model a host publishes for the server. Cheap to clone and shared across threads.
#[derive(Clone, Default)]
pub struct View(Arc<Mutex<Published>>);

#[derive(Default)]
struct Published {
    snapshot: Option<Snapshot>,
    /// Grows whenever the project, its file, its unsaved-changes flag or the open error changes.
    project_version: u64,
}

impl View {
    /// Replaces what readers see. Hosts call this after every change to the document or engine.
    pub fn publish(&self, doc: &Document, observer: EngineObserver, open_error: Option<String>) {
        let snapshot = Snapshot {
            file: doc.file().clone(),
            validated: doc.validated().clone(),
            path: doc.path().map(PathBuf::from),
            dirty: doc.is_dirty(),
            open_error,
            observer,
        };
        let mut published = self.lock();
        let project_changed = published.snapshot.as_ref().is_none_or(|old| {
            old.file != snapshot.file
                || old.path != snapshot.path
                || old.dirty != snapshot.dirty
                || old.open_error != snapshot.open_error
        });
        if project_changed {
            published.project_version += 1;
        }
        published.snapshot = Some(snapshot);
    }

    fn lock(&self) -> MutexGuard<'_, Published> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn get(&self) -> Result<Snapshot, String> {
        self.lock().snapshot.clone().ok_or_else(starting)
    }

    pub(crate) fn observer(&self) -> Result<EngineObserver, String> {
        self.lock()
            .snapshot
            .as_ref()
            .map(|s| s.observer.clone())
            .ok_or_else(starting)
    }

    /// A number that changes whenever what `get_project` shows may have; none before the host
    /// first published.
    pub(crate) fn project_version(&self) -> Option<u64> {
        let published = self.lock();
        published
            .snapshot
            .as_ref()
            .map(|_| published.project_version)
    }
}

fn starting() -> String {
    "Senzor is still starting; try again".to_string()
}
