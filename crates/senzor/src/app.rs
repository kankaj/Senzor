use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use eframe::egui;
use senzor_core::engine::{Command, Engine, EngineHandle, EngineState, Target};
use senzor_core::project::ProjectFile;
use tokio::task::JoinHandle;

use crate::dialogs::{FileDialogs, project_file_path};
use crate::document::Document;
use crate::edits;
use crate::selection::{ProblemTarget, Selection, keep_valid};
use crate::ui::modals::{self, Outcome, UnsavedChoice};
use crate::ui::{self, Action, Modal, Pending, UiState};

/// The Senzor window: one document, one engine, and the GUI state around them.
pub struct SenzorApp {
    runtime: tokio::runtime::Runtime,
    engine: Option<EngineHandle>,
    /// Shutdowns of engines replaced by another document. They run in the background so the
    /// window stays responsive while their instances stop (up to 2 s).
    retired: Vec<JoinHandle<()>>,
    /// The document's autostart devices are waiting for `retired` to finish: their instances
    /// reuse the old ones' client IDs, so starting them early makes the broker see the new
    /// `online` before the old `offline` (and drop the old connection with its Last Will).
    autostart_pending: bool,
    /// The user's engine commands while `retired` is still stopping, sent in order (after the
    /// autostart) once it has stopped, for the same reason.
    queued: Vec<Command>,
    doc: Document,
    state: EngineState,
    ui: UiState,
    dialogs: Box<dyn FileDialogs>,
    ctx: egui::Context,
    allow_close: bool,
    window_title: String,
}

impl SenzorApp {
    /// Opens `path` if given (a file that fails to parse shows its errors and leaves an empty project).
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        path: Option<PathBuf>,
        dialogs: Box<dyn FileDialogs>,
    ) -> Self {
        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime starts");
        let mut app = Self {
            runtime,
            engine: None,
            retired: Vec::new(),
            autostart_pending: false,
            queued: Vec::new(),
            doc: Document::new_empty(),
            state: EngineState::default(),
            ui: UiState::default(),
            dialogs,
            ctx: cc.egui_ctx.clone(),
            allow_close: false,
            window_title: String::new(),
        };
        match path {
            Some(path) => app.open_path(&path),
            None => app.replace_document(Document::new_empty()),
        }
        app
    }

    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// The engine's current state (what the panels show).
    pub fn engine_state(&self) -> EngineState {
        self.engine().snapshot()
    }

    fn engine(&self) -> &EngineHandle {
        self.engine
            .as_ref()
            .expect("engine runs while the app is open")
    }

    fn send(&self, command: Command) {
        self.engine().send(command);
    }

    /// Sends a command the user gave, or queues it while a replaced engine is still stopping.
    fn send_when_ready(&mut self, command: Command) {
        self.autostart_when_ready();
        if self.retired.is_empty() {
            self.send(command);
        } else {
            self.queued.push(command);
        }
    }

    /// Swaps in a document and a fresh engine, then starts its autostart devices once the old
    /// engine has stopped. Commands queued for the old document are dropped.
    fn replace_document(&mut self, doc: Document) {
        self.doc = doc;
        self.ui.reset();
        let ctx = self.ctx.clone();
        let engine = Engine::spawn(
            self.doc.validated().project.clone(),
            self.runtime.handle(),
            Arc::new(move || ctx.request_repaint()),
        );
        if let Some(old) = self.engine.replace(engine) {
            self.retired.push(self.runtime.spawn(old.shutdown()));
        }
        self.autostart_pending = true;
        self.queued.clear();
        self.autostart_when_ready();
        self.state = self.engine().snapshot();
    }

    /// Once no replaced engine is still stopping, starts the document's autostart devices,
    /// then sends the queued commands.
    fn autostart_when_ready(&mut self) {
        self.retired.retain(|shutdown| !shutdown.is_finished());
        if !self.retired.is_empty() {
            if self.autostart_pending || !self.queued.is_empty() {
                // Nothing repaints when a shutdown finishes, so look again shortly.
                self.ctx.request_repaint_after(Duration::from_millis(50));
            }
            return;
        }
        if std::mem::take(&mut self.autostart_pending) {
            for device in &self.doc.validated().project.devices {
                if device.autostart {
                    self.send(Command::Start(Target::Device(device.id.clone())));
                }
            }
        }
        for command in std::mem::take(&mut self.queued) {
            self.send(command);
        }
    }

    /// Autostart devices or queued commands are waiting for a replaced engine to stop.
    fn starting_soon(&self) -> bool {
        !self.queued.is_empty()
            || (self.autostart_pending
                && self
                    .doc
                    .validated()
                    .project
                    .devices
                    .iter()
                    .any(|d| d.autostart))
    }

    fn open_path(&mut self, path: &Path) {
        match Document::open(path) {
            Ok(doc) => self.replace_document(doc),
            Err(error) => {
                if self.engine.is_none() {
                    self.replace_document(Document::new_empty());
                }
                self.ui.modal = Some(Modal::OpenFailed {
                    path: path.display().to_string(),
                    message: error.to_string(),
                });
            }
        }
    }

    /// Saves to the current file (or asks where, when there is none yet), then continues with
    /// `then` once the project is saved.
    fn save(&mut self, then: Option<Pending>) {
        if self.doc.path().is_some() {
            let result = self.doc.save();
            self.finish_save(result, then);
        } else {
            self.save_as(then);
        }
    }

    /// Asks where to save, then continues with `then` once the project is saved. Replacing a
    /// file the save dialog did not ask about waits for the user to confirm.
    fn save_as(&mut self, then: Option<Pending>) {
        let suggested = self
            .doc
            .file_name()
            .unwrap_or_else(|| "project.toml".to_string());
        let Some(chosen) = self.dialogs.save_project(&suggested) else {
            return;
        };
        let path = project_file_path(chosen.clone());
        // The dialog only confirmed replacing `chosen`, not the name with `.toml` added.
        if path != chosen && std::fs::symlink_metadata(&path).is_ok() {
            self.ui.modal = Some(Modal::ConfirmReplace { path, then });
            return;
        }
        let result = self.doc.save_as(path);
        self.finish_save(result, then);
    }

    /// Reports a failed save, or tells the engine about the saved project and continues with `then`.
    fn finish_save(&mut self, result: Result<(), String>, then: Option<Pending>) {
        match result {
            Ok(()) => {
                // Saving elsewhere can change how relative TLS paths resolve.
                self.send(Command::ReplaceProject(
                    self.doc.validated().project.clone(),
                ));
                if let Some(pending) = then {
                    self.proceed(pending);
                }
            }
            Err(message) => self.ui.modal = Some(Modal::Error(message)),
        }
    }

    /// Applies `f` to the document, tells the engine if the project changed, and returns what
    /// `f` returned.
    fn edit<R>(&mut self, f: impl FnOnce(&mut ProjectFile) -> R) -> R {
        let mut result = None;
        if self.doc.edit(|file| result = Some(f(file))) {
            self.document_changed();
        }
        result.expect("Document::edit runs the edit")
    }

    /// Tells the engine about the edited project and drops a selection whose item is gone.
    fn document_changed(&mut self) {
        self.send(Command::ReplaceProject(
            self.doc.validated().project.clone(),
        ));
        self.ui.selection = keep_valid(self.ui.selection.take(), self.doc.file());
    }

    fn select(&mut self, selection: Option<Selection>) {
        if selection != self.ui.selection {
            self.ui.buffers.clear();
            self.ui.selection = selection;
        }
    }

    /// Selects `selection` and switches the left panel to the tab that lists it.
    fn reveal(&mut self, selection: Selection) {
        self.ui.left_tab = match selection {
            Selection::Template(_) | Selection::Builtin(_) => ui::LeftTab::Templates,
            Selection::Device(_) | Selection::Instance(_) => ui::LeftTab::Devices,
        };
        self.select(Some(selection));
    }

    /// Continues a New / Open / Close once unsaved changes are dealt with.
    fn proceed(&mut self, pending: Pending) {
        match pending {
            Pending::New => self.replace_document(Document::new_empty()),
            Pending::Open => {
                if let Some(path) = self.dialogs.open_project() {
                    self.open_path(&path);
                }
            }
            Pending::Close => {
                self.allow_close = true;
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// Runs `pending` now, or asks about unsaved changes first.
    fn guard(&mut self, pending: Pending) {
        if self.doc.is_dirty() {
            self.ui.modal = Some(Modal::Unsaved(pending));
        } else {
            self.proceed(pending);
        }
    }

    fn apply(&mut self, action: Action) {
        let action = match ui::apply_to_document(&mut self.doc, action) {
            Ok(changed) => {
                if changed {
                    self.document_changed();
                }
                return;
            }
            Err(action) => *action,
        };
        match action {
            Action::New => self.guard(Pending::New),
            Action::Open => self.guard(Pending::Open),
            Action::Save => self.save(None),
            Action::SaveAs => self.save_as(None),
            Action::StartAll => self.send_when_ready(Command::Start(Target::All)),
            Action::StopAll => {
                // Nothing waiting for the old engine starts once it has stopped.
                self.autostart_pending = false;
                self.queued.clear();
                self.send(Command::Stop(Target::All));
            }
            Action::Engine(command) => self.send_when_ready(command),
            Action::Select(selection) => self.select(selection),
            // Applied to the document above.
            Action::Edit(_) | Action::Rename(_) | Action::Remove(_) => {}
            Action::AddDevice => {
                let index = self.edit(edits::add_device);
                self.reveal(Selection::Device(index));
            }
            Action::AddTemplate => {
                let index = self.edit(edits::add_template);
                self.reveal(Selection::Template(index));
            }
            Action::AskCopyBuiltin(builtin) => {
                let new_id = edits::copy_id_suggestion(self.doc.file(), &builtin);
                self.ui.modal = Some(Modal::CopyBuiltin {
                    builtin,
                    new_id,
                    error: None,
                });
            }
            Action::OpenBroker => self.ui.broker_open = true,
            Action::GoTo(ProblemTarget::Broker) => self.ui.broker_open = true,
            Action::GoTo(ProblemTarget::Select(selection)) => self.reveal(selection),
        }
    }

    fn apply_modal(&mut self, outcome: Outcome) {
        self.ui.modal = None;
        match outcome {
            Outcome::Dismissed | Outcome::Unsaved(_, UnsavedChoice::Cancel) => {}
            Outcome::Unsaved(pending, UnsavedChoice::Discard) => self.proceed(pending),
            Outcome::Unsaved(pending, UnsavedChoice::Save) => self.save(Some(pending)),
            Outcome::Replace { path, then } => {
                let result = self.doc.save_as(path);
                self.finish_save(result, then);
            }
            Outcome::CopyBuiltin { builtin, new_id } => {
                match self.edit(|f| edits::copy_builtin(f, &builtin, &new_id)) {
                    Ok(index) => self.reveal(Selection::Template(index)),
                    Err(error) => {
                        self.ui.modal = Some(Modal::CopyBuiltin {
                            builtin,
                            new_id,
                            error: Some(error),
                        });
                    }
                }
            }
        }
    }

    /// Commits a rename still being typed (closing the window is not a click that would commit
    /// it), then asks about unsaved changes instead of closing.
    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.viewport().close_requested()) || self.allow_close {
            return;
        }
        if let Some(rename) = self.ui.pending_rename.take() {
            self.apply(Action::Rename(rename));
        }
        if self.doc.is_dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.ui.modal = Some(Modal::Unsaved(Pending::Close));
        }
    }
}

impl eframe::App for SenzorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.autostart_when_ready();
        self.state = self.engine().snapshot();
        self.handle_close_request(&ctx);
        // The panels record a rename still being typed again while they draw.
        self.ui.pending_rename = None;

        let title = format!("Senzor — {}", self.doc.title());
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }

        let mut actions = Vec::new();
        let starting = self.starting_soon();
        egui::Panel::top("top-bar").show(ui, |ui| {
            ui::top_bar::show(ui, &self.doc, starting, &mut actions);
        });
        egui::Panel::bottom("bottom")
            .resizable(true)
            .default_size(180.0)
            .show(ui, |ui| {
                ui::bottom::show(ui, &self.doc, &self.state, &mut self.ui, &mut actions);
            });
        egui::Panel::left("left")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| {
                ui::left_panel::show(ui, &self.doc, &self.state, &mut self.ui, &mut actions);
            });
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| match self.ui.selection.clone() {
                    Some(Selection::Instance(id)) => ui::instance::show(
                        ui,
                        &id,
                        &self.doc,
                        &self.state,
                        &mut self.ui,
                        &mut actions,
                    ),
                    Some(Selection::Device(index)) => {
                        ui::device::show(ui, index, &self.doc, &mut self.ui, &mut actions)
                    }
                    Some(Selection::Template(index)) => {
                        ui::template::show_user(ui, index, &self.doc, &mut self.ui, &mut actions)
                    }
                    Some(Selection::Builtin(id)) => {
                        ui::template::show_builtin(ui, &id, &self.doc, &mut self.ui, &mut actions)
                    }
                    None => {
                        ui.heading("Senzor");
                        ui.label("Select a device, an instance or a template on the left.");
                    }
                });
        });
        if self.ui.broker_open {
            ui::broker::show(&ctx, &self.doc, &mut self.ui, &mut actions);
        }

        // Edits, then renames, then removals, then the rest (see `ui::phase`).
        actions.sort_by_key(ui::phase);
        for action in actions {
            self.apply(action);
        }
        if let Some(mut modal) = self.ui.modal.take() {
            let outcome = modals::show(&ctx, &mut modal);
            self.ui.modal = Some(modal);
            if let Some(outcome) = outcome {
                self.apply_modal(outcome);
            }
        }
    }

    /// Waits for the current engine and for replaced ones still stopping: dropping the runtime
    /// would cancel their shutdown halfway, and their devices would disappear through the Last
    /// Will instead of a clean `offline`.
    fn on_exit(&mut self) {
        let engine = self.engine.take();
        let retired = &mut self.retired;
        self.runtime.block_on(async move {
            if let Some(engine) = engine {
                engine.shutdown().await;
            }
            for shutdown in retired.iter_mut() {
                let _ = shutdown.await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use senzor_core::engine::InstanceStatus;
    use senzor_testkit::slow_broker;

    use super::*;
    use crate::testing::{
        App, FRAME_PAUSE, FakeDialogs, TIMEOUT, click, status, wait_online, wait_until,
        write_project,
    };

    /// Device `d`, which starts when its project opens.
    const AUTOSTART_D: &str = "[[devices]]\nid = \"d\"\ntemplate = \"co2\"\nautostart = true\n";

    /// A project with `devices` on the broker at `port`.
    fn project(port: u16, devices: &str) -> String {
        format!("[broker]\nhost = \"127.0.0.1\"\nport = {port}\n\n{devices}")
    }

    /// Opens a project on the fake broker and reopens it once its autostart device `d` is
    /// online, so the replaced engine takes its full 2 s to stop `d`. `e` does not autostart.
    /// The runtime runs the broker and must outlive the test.
    fn reopened_while_the_old_engine_stops(name: &str) -> (tokio::runtime::Runtime, App) {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let port = runtime.block_on(slow_broker(Duration::ZERO));
        let devices = format!(
            "{AUTOSTART_D}\n[[devices]]\nid = \"e\"\ntemplate = \"co2\"\nautostart = false\n"
        );
        let path = write_project(name, &project(port, &devices));
        let opened = path.clone();
        let mut harness = Harness::builder().build_eframe(move |cc| {
            SenzorApp::new(cc, Some(opened), Box::new(FakeDialogs::default()))
        });
        wait_online(&mut harness, "d");
        harness
            .state_mut()
            .replace_document(Document::open(&path).unwrap());
        harness.step();
        assert!(!old_engine_stopped(&harness), "the old engine is stopping");
        (runtime, harness)
    }

    fn old_engine_stopped(harness: &App) -> bool {
        harness.state().retired.iter().all(JoinHandle::is_finished)
    }

    /// Steps frames for `duration` (long enough for the engine to act on what was sent).
    fn keep_stepping(harness: &mut App, duration: Duration) {
        let until = Instant::now() + duration;
        while Instant::now() < until {
            harness.step();
            std::thread::sleep(FRAME_PAUSE);
        }
    }

    /// Dropping the runtime after `on_exit` would cancel a replaced engine's shutdown halfway,
    /// and its devices would disappear through the Last Will instead of a clean `offline`.
    /// The fake broker never closes a connection, so that shutdown takes its full 2 s.
    #[test]
    fn exiting_waits_for_replaced_engines_to_finish_stopping() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let port = runtime.block_on(slow_broker(Duration::ZERO));
        let path = write_project("exit", &project(port, AUTOSTART_D));
        let mut harness = Harness::builder().build_eframe(move |cc| {
            SenzorApp::new(cc, Some(path), Box::new(FakeDialogs::default()))
        });
        wait_online(&mut harness, "d");
        harness.state_mut().replace_document(Document::new_empty());
        assert_eq!(
            harness.state().retired.len(),
            1,
            "the old engine is stopping"
        );

        eframe::App::on_exit(harness.state_mut());

        assert!(old_engine_stopped(&harness));
    }

    #[test]
    fn stop_all_while_waiting_for_the_old_engine_cancels_the_autostart() {
        let (_runtime, mut h) = reopened_while_the_old_engine_stops("stop-all");
        click(&mut h, "Stop all");
        wait_until(&mut h, "the old engine stopped", old_engine_stopped);
        keep_stepping(&mut h, Duration::from_millis(300));
        assert_eq!(status(&h, "d"), InstanceStatus::Stopped);
    }

    /// Clicks Start all and checks nothing runs before the old engine has stopped, and both
    /// devices start afterwards.
    fn start_all_waits_for_the_old_engine(h: &mut App) {
        click(h, "Start all");
        // A frame may send the queued commands as soon as the old engine has stopped, so look
        // before stepping.
        let deadline = Instant::now() + TIMEOUT;
        while !old_engine_stopped(h) {
            assert!(
                Instant::now() < deadline,
                "timed out waiting for the old engine"
            );
            for id in ["d", "e"] {
                assert_eq!(
                    status(h, id),
                    InstanceStatus::Stopped,
                    "{id} started while the old engine was still stopping"
                );
            }
            h.step();
            std::thread::sleep(FRAME_PAUSE);
        }
        wait_until(h, "d and e running", |h| {
            ["d", "e"].iter().all(|id| status(h, id).is_running())
        });
    }

    #[test]
    fn start_all_while_waiting_for_the_old_engine_runs_once_it_has_stopped() {
        let (_runtime, mut h) = reopened_while_the_old_engine_stops("start-all");
        start_all_waits_for_the_old_engine(&mut h);
    }

    #[test]
    fn start_all_after_stop_all_still_waits_for_the_old_engine() {
        let (_runtime, mut h) = reopened_while_the_old_engine_stops("stop-start");
        click(&mut h, "Stop all");
        start_all_waits_for_the_old_engine(&mut h);
    }

    #[test]
    fn commands_queued_for_a_replaced_document_are_dropped() {
        let (_runtime, mut h) = reopened_while_the_old_engine_stops("queued-reopen");
        click(&mut h, "Start all");
        let path = h.state().doc.path().unwrap().to_path_buf();
        h.state_mut()
            .replace_document(Document::open(&path).unwrap());
        wait_until(&mut h, "the old engines stopped", old_engine_stopped);
        wait_until(&mut h, "d running", |h| status(h, "d").is_running());
        keep_stepping(&mut h, Duration::from_millis(300));
        assert_eq!(status(&h, "e"), InstanceStatus::Stopped);
    }

    const STARTING: &str = "starting devices…";

    #[test]
    fn the_top_bar_says_devices_are_starting_while_they_wait_for_the_old_engine() {
        let (_runtime, mut h) = reopened_while_the_old_engine_stops("hint");
        assert!(h.query_by_label(STARTING).is_some());
        h.get_by_label(STARTING).hover();
        // A step is 0.25 s; tooltips wait 0.5 s for the pointer to rest.
        h.run_steps(4);
        assert!(
            h.query_by_label("Waiting for the previous project's devices to go offline")
                .is_some()
        );
        wait_until(&mut h, "the old engine stopped", old_engine_stopped);
        h.run_steps(2);
        assert!(h.query_by_label(STARTING).is_none());
        assert!(status(&h, "d").is_running());
    }
}
