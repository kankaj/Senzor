//! A headless host: one document and one engine, owned by a task that applies requests in order.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use senzor_core::engine::{Command, Engine, EngineHandle, EngineObserver, Target};
use senzor_core::project::{Document, OpenError, ProjectFile};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::View;
use crate::host::{Envelope, Host, apply};

/// The project and engine of a headless MCP session (`senzor-cli mcp`).
pub struct Session {
    runtime: Handle,
    doc: Document,
    engine: EngineHandle,
    /// The shutdown of an engine replaced by another project. The new project's devices reuse
    /// the old client IDs, so they start only once it has finished (see [`Session::settle`]).
    retiring: Option<JoinHandle<()>>,
    autostart_pending: bool,
    open_error: Option<String>,
    view: View,
}

impl Session {
    /// Opens `path`, or starts with an empty project. A file that cannot be opened leaves an
    /// empty project and keeps the error for `get_status`. Call [`Session::settle`] to start the
    /// autostart devices.
    pub fn new(runtime: &Handle, path: Option<&Path>, autostart: bool) -> Self {
        let (doc, open_error) = match path.map(Document::open) {
            None => (Document::new_empty(), None),
            Some(Ok(doc)) => (doc, None),
            Some(Err(error)) => {
                let path = path.expect("an error comes from opening a path");
                (Document::new_empty(), Some(open_error_text(path, &error)))
            }
        };
        let engine = spawn_engine(runtime, &doc);
        let session = Self {
            runtime: runtime.clone(),
            doc,
            engine,
            retiring: None,
            autostart_pending: autostart,
            open_error,
            view: View::default(),
        };
        session.publish();
        session
    }

    /// The read model the server answers reads from.
    pub fn view(&self) -> View {
        self.view.clone()
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// Why opening the project at launch failed, if it did.
    pub fn open_error(&self) -> Option<&str> {
        self.open_error.as_deref()
    }

    /// Starts device `id`, as `senzor-cli run --only` picks devices.
    pub fn start_device(&self, id: &str) {
        self.engine
            .send(Command::Start(Target::Device(id.to_string())));
    }

    /// The current engine's state, for reading it after the session ends.
    pub fn observer(&self) -> EngineObserver {
        self.engine.observer()
    }

    /// Waits for a replaced engine to stop, then starts the autostart devices that are waiting.
    pub async fn settle(&mut self) {
        if let Some(retiring) = self.retiring.take() {
            let _ = retiring.await;
        }
        if std::mem::take(&mut self.autostart_pending) {
            for device in &self.doc.validated().project.devices {
                if device.autostart {
                    self.engine
                        .send(Command::Start(Target::Device(device.id.clone())));
                }
            }
        }
    }

    fn publish(&self) {
        self.view
            .publish(&self.doc, self.engine.observer(), self.open_error.clone());
    }

    /// Applies requests until every sender is gone, then returns the session.
    pub async fn run(mut self, mut requests: mpsc::UnboundedReceiver<Envelope>) -> Self {
        self.settle().await;
        self.publish();
        while let Some(envelope) = requests.recv().await {
            if envelope.is_cancelled() {
                continue;
            }
            let result = apply(&mut self, envelope.request);
            self.settle().await;
            self.publish();
            let _ = envelope.reply.send(result);
        }
        self
    }

    /// Stops every instance gracefully (killing what still runs after 2 s).
    pub async fn shutdown(mut self) {
        if let Some(retiring) = self.retiring.take() {
            let _ = retiring.await;
        }
        self.engine.shutdown().await;
    }
}

/// Why `path` could not be opened, naming the file once.
pub fn open_error_text(path: &Path, error: &OpenError) -> String {
    match error {
        OpenError::Read(message) => message.clone(),
        OpenError::Parse(_) => format!("{}: {error}", path.display()),
    }
}

impl Session {
    /// Hands the project to the engine when `changed`; returns `changed`.
    fn changed(&mut self, changed: bool) -> bool {
        if changed {
            self.engine.send(Command::ReplaceProject(
                self.doc.validated().project.clone(),
            ));
        }
        changed
    }
}

fn spawn_engine(runtime: &Handle, doc: &Document) -> EngineHandle {
    Engine::spawn(doc.validated().project.clone(), runtime, Arc::new(|| {}))
}

impl Host for Session {
    fn document(&self) -> &Document {
        &self.doc
    }

    fn edit(&mut self, f: &mut dyn FnMut(&mut ProjectFile)) -> bool {
        let changed = self.doc.edit(f);
        self.changed(changed)
    }

    fn undo(&mut self) -> bool {
        let changed = self.doc.undo();
        self.changed(changed)
    }

    fn redo(&mut self) -> bool {
        let changed = self.doc.redo();
        self.changed(changed)
    }

    fn send(&mut self, command: Command) {
        self.engine.send(command);
    }

    fn replace(&mut self, doc: Document, autostart: bool) {
        self.doc = doc;
        self.open_error = None;
        let old = std::mem::replace(&mut self.engine, spawn_engine(&self.runtime, &self.doc));
        // `settle` waits for this before anything starts, so at most one is ever pending.
        let previous = self.retiring.take();
        self.retiring = Some(self.runtime.spawn(async move {
            if let Some(previous) = previous {
                let _ = previous.await;
            }
            old.shutdown().await;
        }));
        self.autostart_pending = autostart;
    }

    fn save(&mut self, path: Option<PathBuf>) -> Result<(), String> {
        match path {
            None => self.doc.save()?,
            Some(path) => self.doc.save_as(path)?,
        }
        // Saving elsewhere can change how relative TLS paths resolve.
        self.engine.send(Command::ReplaceProject(
            self.doc.validated().project.clone(),
        ));
        Ok(())
    }

    fn observer(&self) -> EngineObserver {
        self.engine.observer()
    }

    fn commands_sent(&self) -> u64 {
        self.engine.commands_sent()
    }
}
