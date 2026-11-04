//! What the server reads without asking the host: a copy of the document and the engine's observer.
//! Reads keep working while the host is busy (or, in the GUI, while no frames are drawn).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
pub struct View(Arc<Mutex<Option<Snapshot>>>);

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
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(snapshot);
    }

    pub(crate) fn get(&self) -> Result<Snapshot, String> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or_else(|| "Senzor is still starting; try again".to_string())
    }

    pub(crate) fn observer(&self) -> Result<EngineObserver, String> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|s| s.observer.clone())
            .ok_or_else(|| "Senzor is still starting; try again".to_string())
    }
}
