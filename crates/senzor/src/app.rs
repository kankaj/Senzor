use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use senzor_core::engine::{
    CLEAR_RETAINED_TIMEOUT, CONNECTION_TEST_TIMEOUT, Command, Engine, EngineHandle, EngineState,
    Target, clear_retained, retained_topics, test_connection,
};
use senzor_core::project::edit::{duplicate_device, duplicate_template};
use senzor_core::project::{BrokerConfig, ProjectFile};
use tokio::task::JoinHandle;

use crate::dialogs::{FileDialogs, project_file_path};
use crate::document::Document;
use crate::edits;
use crate::i18n::t;
use crate::notify::{DesktopNotifier, FailureNotifications, Notifier};
use crate::rate::MessageRate;
use crate::selection::{ProblemTarget, Selection, keep_valid};
use crate::settings::{AppSettings, PanelTabs};
use crate::ui::modals::{self, Outcome, UnsavedChoice};
use crate::ui::{self, Action, Modal, Pending, UiState};
use crate::update::{Release, ReleaseSource};

mod mcp;

pub use mcp::McpStatus;

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
    /// Counts the engines this window has had, so a replaced engine can be noticed.
    engine_generation: u64,
    /// Why the last attempt to open a project file failed, until another project is opened.
    open_error: Option<String>,
    /// The MCP endpoint while it runs.
    mcp: Option<mcp::Mcp>,
    /// The port turning MCP on uses.
    mcp_port: u16,
    /// Why the last attempt to turn MCP on failed.
    mcp_error: Option<String>,
    /// The app's own settings, saved between launches.
    settings: AppSettings,
    /// The latest broker connection test.
    connection_test: Option<ConnectionTestRun>,
    /// The latest clearing of retained messages: how far it is, and where its result comes in.
    clear_retained: Option<ClearRetainedRun>,
    /// When the project's file was last checked for changes made elsewhere.
    disk_checked: Instant,
    /// The version of Senzor that ran before this launch, if any ran with these settings.
    version_seen_before: Option<String>,
    /// How fast the engine publishes, for the status bar.
    rate: MessageRate,
    /// Notifies about instances that fail while the window is in the background.
    failure_notifications: FailureNotifications,
    /// Where the update check asks for the latest release; taken when it starts, so a window
    /// asks once at most. Without one, there is no check.
    release_source: Option<Box<dyn ReleaseSource>>,
    /// Where the answer of the running update check comes in.
    release_check: Option<std::sync::mpsc::Receiver<Result<Option<Release>, String>>>,
}

/// From this width of the detail panel on, a template's preview sits beside its editor.
const PREVIEW_BESIDE_MIN_WIDTH: f32 = 900.0;

/// How often the window looks whether another program changed the project's file.
const DISK_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// Clearing retained messages: how far it is, and where its result (the topics cleared) comes in.
struct ClearRetainedRun {
    state: ui::ClearRetained,
    result: Option<std::sync::mpsc::Receiver<Result<usize, String>>>,
}

/// A connection test: the broker settings it tests, how far it is, and where its result comes in.
struct ConnectionTestRun {
    broker: BrokerConfig,
    state: ui::ConnectionTest,
    result: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
}

impl SenzorApp {
    /// Opens `path` if given (a file that fails to parse shows its errors and leaves an empty
    /// project), with the settings the last window saved.
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        path: Option<PathBuf>,
        dialogs: Box<dyn FileDialogs>,
    ) -> Self {
        let settings = AppSettings::load(cc.storage);
        Self::with_settings(cc, path, dialogs, settings)
    }

    /// Like [`SenzorApp::new`], with `settings` instead of the saved ones. MCP starts when the
    /// settings say it was on.
    pub fn with_settings(
        cc: &eframe::CreationContext<'_>,
        path: Option<PathBuf>,
        dialogs: Box<dyn FileDialogs>,
        settings: AppSettings,
    ) -> Self {
        let mut settings = settings;
        ui::install_icon_font(&cc.egui_ctx);
        settings.apply(&cc.egui_ctx);
        let version_seen_before = settings
            .last_version
            .replace(env!("CARGO_PKG_VERSION").to_string());
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
            engine_generation: 0,
            open_error: None,
            mcp: None,
            mcp_port: settings.mcp_port,
            mcp_error: None,
            settings,
            connection_test: None,
            clear_retained: None,
            disk_checked: Instant::now(),
            rate: MessageRate::default(),
            version_seen_before,
            failure_notifications: FailureNotifications::start(Box::new(DesktopNotifier)),
            release_source: None,
            release_check: None,
        };
        match path {
            Some(path) => app.open_path(&path),
            None => app.replace_document(Document::new_empty()),
        }
        // After an update, what changed since the version that ran before.
        if let Some(previous) = &app.version_seen_before
            && previous != env!("CARGO_PKG_VERSION")
            && app.ui.modal.is_none()
        {
            let releases = crate::whats_new::since_version(previous);
            if !releases.is_empty() {
                app.ui.modal = Some(Modal::WhatsNew(releases));
            }
        }
        // After the document, whose arrival resets the panels.
        app.ui.left_tab = app.settings.tabs.left;
        app.ui.bottom_tab = app.settings.tabs.bottom;
        if app.settings.mcp_on {
            app.start_mcp(app.settings.mcp_port);
        }
        app
    }

    /// Shows desktop notifications with `notifier` instead of the desktop's.
    pub fn with_notifier(self, notifier: Box<dyn Notifier + Send>) -> Self {
        self.failure_notifications.set_notifier(notifier);
        self
    }

    /// Asks `source` whether a newer release is out, instead of nobody. The check runs in the
    /// background once the window shows, when the settings allow it and the last one is a day
    /// old.
    pub fn with_release_source(mut self, source: Box<dyn ReleaseSource>) -> Self {
        self.release_source = Some(source);
        self
    }

    /// The app's own settings, saved between launches.
    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    /// A release newer than this version, while update checks are on.
    pub fn newer_release(&self) -> Option<&Release> {
        self.settings
            .latest_release
            .as_ref()
            .filter(|_| self.settings.check_updates)
            .filter(|r| crate::update::is_newer(&r.version, env!("CARGO_PKG_VERSION")))
    }

    /// The version of Senzor that ran before this launch, or `None` on the first launch.
    pub fn version_seen_before(&self) -> Option<&str> {
        self.version_seen_before.as_deref()
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
        self.open_error = None;
        self.connection_test = None;
        self.clear_retained = None;
        self.rate.clear();
        self.engine_generation += 1;
        self.ui.reset();
        let ctx = self.ctx.clone();
        let wake = self.failure_notifications.waker();
        let engine = Engine::spawn(
            self.doc.validated().project.clone(),
            self.runtime.handle(),
            Arc::new(move || {
                ctx.request_repaint();
                wake();
            }),
        );
        self.failure_notifications
            .watch(self.engine_generation, engine.observer());
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
            Ok(doc) => {
                self.settings.remember(path);
                self.replace_document(doc);
            }
            Err(error) => {
                if self.engine.is_none() {
                    self.replace_document(Document::new_empty());
                }
                self.open_error = Some(senzor_mcp::open_error_text(path, &error));
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
                if let Some(path) = self.doc.path().map(Path::to_path_buf) {
                    self.settings.remember(&path);
                }
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

    /// The instance IDs the device tree shows, in its order (after its filter).
    fn tree_order(&self) -> Vec<String> {
        let tree = crate::tree::device_tree(self.doc.file(), self.doc.validated(), &self.state);
        crate::tree::filter_tree(tree, &self.ui.tree_filter)
            .into_iter()
            .flat_map(|node| node.instances.into_iter().map(|i| i.id))
            .collect()
    }

    /// Selects `selection` and switches the left panel to the tab that lists it.
    fn reveal(&mut self, selection: Selection) {
        self.ui.left_tab = match selection {
            Selection::Template(_) | Selection::Builtin(_) => ui::LeftTab::Templates,
            Selection::Device(_) | Selection::Instance(_) | Selection::Instances(_) => {
                ui::LeftTab::Devices
            }
            Selection::Source => self.ui.left_tab,
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
            Pending::OpenPath(path) => self.open_path(&path),
            Pending::OpenExample(index) => {
                self.replace_document(crate::examples::EXAMPLES[index].document());
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

    /// The text field that has the keyboard, if any: edits typed into it make one undo step.
    fn typing_into(&self) -> Option<u64> {
        self.ctx
            .memory(|m| m.focused())
            .filter(|_| self.ctx.egui_wants_keyboard_input())
            .map(|id| id.value())
    }

    /// Text the forms keep for a value they could not parse yet belongs to the model undone.
    fn after_undo_or_redo(&mut self) {
        self.ui.buffers.clear();
        self.document_changed();
    }

    /// About once a second, while no dialog is open, asks about a project file another program
    /// changed.
    fn check_disk(&mut self) {
        if self.doc.path().is_none() {
            return;
        }
        self.ctx.request_repaint_after(DISK_CHECK_INTERVAL);
        if self.ui.modal.is_some() || self.disk_checked.elapsed() < DISK_CHECK_INTERVAL {
            return;
        }
        self.disk_checked = Instant::now();
        if self.doc.check_disk() {
            self.ui.modal = Some(Modal::FileChanged {
                name: self.doc.file_name().unwrap_or_default(),
                unsaved: self.doc.is_dirty(),
            });
        }
    }

    /// Connects once with the project's broker settings, in the background.
    fn start_connection_test(&mut self) {
        let broker = self.doc.validated().project.broker.clone();
        let (send, result) = std::sync::mpsc::channel();
        let ctx = self.ctx.clone();
        let tested = broker.clone();
        self.runtime.spawn(async move {
            let _ = send.send(test_connection(&tested, CONNECTION_TEST_TIMEOUT).await);
            ctx.request_repaint();
        });
        self.connection_test = Some(ConnectionTestRun {
            broker,
            state: ui::ConnectionTest::Running,
            result: Some(result),
        });
    }

    /// Takes in the update check's answer, and starts the check when the settings allow it and
    /// the last one is a day old. It runs on a thread of its own, so a slow network never holds
    /// the window up; when it fails, the next launch tries again.
    fn check_for_updates(&mut self) {
        if let Some(answer) = self.release_check.as_ref().and_then(|r| r.try_recv().ok()) {
            self.release_check = None;
            match answer {
                Ok(latest) => {
                    self.settings.update_checked = Some(crate::update::now_secs());
                    self.settings.latest_release = latest;
                }
                Err(error) => eprintln!("Could not check for a newer Senzor: {error}"),
            }
        }
        if !self.settings.check_updates
            || !crate::update::due(self.settings.update_checked, crate::update::now_secs())
        {
            return;
        }
        let Some(source) = self.release_source.take() else {
            return;
        };
        let (send, answer) = std::sync::mpsc::channel();
        let ctx = self.ctx.clone();
        let started = std::thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let _ = send.send(source.latest());
                ctx.request_repaint();
            });
        match started {
            Ok(_) => self.release_check = Some(answer),
            Err(error) => eprintln!("Could not check for a newer Senzor: {error}"),
        }
    }

    /// Asks where to save the messages the log shows, then writes them as CSV.
    fn export_log(&mut self) {
        let Some(path) = self.dialogs.save_log("messages.csv") else {
            return;
        };
        let log = self.ui.log_frozen.as_ref().unwrap_or(&self.state.log);
        let shown: Vec<_> = log
            .iter()
            .filter(|e| self.ui.log_filter.matches(e))
            .collect();
        if let Err(error) = std::fs::write(&path, crate::log_view::csv(&shown)) {
            self.ui.modal = Some(Modal::Error(t!(
                "app-could-not-save",
                path = path.display().to_string(),
                error = error.to_string()
            )));
        }
    }

    /// Asks to confirm clearing the retained topics of the project's instances, or says there are
    /// none.
    fn ask_clear_retained(&mut self) {
        let project = &self.doc.validated().project;
        let topics = retained_topics(project, None);
        if topics.is_empty() {
            self.clear_retained = Some(ClearRetainedRun {
                state: ui::ClearRetained::Nothing,
                result: None,
            });
            return;
        }
        let broker = format!("{}:{}", project.broker.host, project.broker.port);
        self.ui.modal = Some(Modal::ClearRetained { topics, broker });
    }

    /// Clears `topics` on the project's broker, in the background.
    fn start_clear_retained(&mut self, topics: Vec<String>) {
        let broker = self.doc.validated().project.broker.clone();
        let (send, result) = std::sync::mpsc::channel();
        let ctx = self.ctx.clone();
        self.runtime.spawn(async move {
            let cleared = clear_retained(&broker, &topics, CLEAR_RETAINED_TIMEOUT)
                .await
                .map(|()| topics.len());
            let _ = send.send(cleared);
            ctx.request_repaint();
        });
        self.clear_retained = Some(ClearRetainedRun {
            state: ui::ClearRetained::Running,
            result: Some(result),
        });
    }

    /// Takes in the result of clearing once it is there.
    fn update_clear_retained(&mut self) {
        if let Some(run) = &mut self.clear_retained
            && let Some(Ok(outcome)) = run.result.as_ref().map(std::sync::mpsc::Receiver::try_recv)
        {
            run.state = match outcome {
                Ok(count) => ui::ClearRetained::Cleared(count),
                Err(reason) => {
                    ui::ClearRetained::Failed(t!("app-could-not-clear", reason = reason))
                }
            };
            run.result = None;
        }
        self.ui.clear_retained = self.clear_retained.as_ref().map(|run| run.state.clone());
    }

    /// Takes in a finished test's result, and shows it only while the broker settings are the
    /// ones it tested.
    fn update_connection_test(&mut self) {
        let current = &self.doc.validated().project.broker;
        self.ui.connection_test = self.connection_test.as_mut().and_then(|run| {
            if let Some(result) = &run.result
                && let Ok(outcome) = result.try_recv()
            {
                run.state = match outcome {
                    Ok(()) => ui::ConnectionTest::Passed,
                    Err(reason) => ui::ConnectionTest::Failed(reason),
                };
                run.result = None;
            }
            (run.broker == *current).then(|| run.state.clone())
        });
    }

    fn apply(&mut self, action: Action) {
        let typing = self.typing_into();
        let action = match ui::apply_to_document(&mut self.doc, action, typing) {
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
            Action::ClickInstance { id, click } => self.click_instance(id, click),
            // Applied to the document above.
            Action::Edit(_) | Action::Rename(_) | Action::Remove(_) | Action::ApplyText(_) => {}
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
            Action::OpenSettings => self.ui.settings_open = true,
            Action::TestConnection => self.start_connection_test(),
            Action::AskClearRetained => self.ask_clear_retained(),
            Action::DuplicateDevice(id) => {
                if let Some(index) = self.edit(|f| duplicate_device(f, &id)) {
                    self.reveal(Selection::Device(index));
                }
            }
            Action::DuplicateTemplate(id) => {
                if let Some(index) = self.edit(|f| duplicate_template(f, &id)) {
                    self.reveal(Selection::Template(index));
                }
            }
            Action::DeleteSelected => self.delete_selected(),
            Action::ExportLog => self.export_log(),
            Action::ClearLog => self.ui.clear_log(&self.state.log),
            Action::OpenPalette => self.ui.palette = Some(Default::default()),
            Action::CopySnippet(snippet) => self.copy_snippet(snippet),
            Action::FindInLog => {
                self.ui.bottom_tab = ui::BottomTab::Log;
                self.ui.focus_log_filter = true;
            }
            Action::Undo => {
                if self.doc.undo() {
                    self.after_undo_or_redo();
                }
            }
            Action::Redo => {
                if self.doc.redo() {
                    self.after_undo_or_redo();
                }
            }
            Action::OpenRecent(path) => self.open_recent(path),
            Action::OpenExample(index) => self.guard(Pending::OpenExample(index)),
            Action::OpenDropped(path) => self.guard(Pending::OpenPath(path)),
            Action::ClearRecent => self.settings.recent.clear(),
            Action::SetLanguage(language) => {
                self.settings.language = language;
                crate::i18n::set_language(language);
            }
            Action::SetTheme(theme) => {
                self.settings.theme = theme;
                self.ctx.set_theme(theme.preference());
            }
            Action::ShowWhatsNew => {
                self.ui.modal = Some(Modal::WhatsNew(crate::whats_new::this_version()));
            }
            Action::ToggleMcp => self.toggle_mcp(),
            Action::SetNotifyFailures(on) => self.settings.notify_failures = on,
            Action::SetCheckUpdates(on) => self.settings.check_updates = on,
            Action::OpenNewRelease => {
                if let Some(release) = self.newer_release() {
                    self.ctx
                        .open_url(egui::OpenUrl::new_tab(release.url.clone()));
                }
            }
            Action::SetMcpPort(port) => {
                self.settings.mcp_port = port;
                self.mcp_port = port;
                self.mcp_error = None;
            }
            Action::SetMcpRemote(remote) => self.set_mcp_remote(remote),
            Action::NewMcpToken => self.new_mcp_token(),
            Action::CopyMcpToken => {
                if let Some(token) = &self.settings.mcp_token {
                    self.ctx.copy_text(token.clone());
                }
            }
            Action::GoTo(ProblemTarget::Broker) => self.ui.broker_open = true,
            Action::GoTo(ProblemTarget::Select(selection)) => self.reveal(selection),
        }
    }

    fn click_instance(&mut self, id: String, click: crate::selection::InstanceClick) {
        let order = self.tree_order();
        let next = crate::selection::click_instance(
            self.ui.selection.as_ref(),
            &id,
            click,
            self.ui.instance_anchor.as_deref(),
            &order,
        );
        // A range keeps its start, so the next Shift+click moves its other end.
        if click != crate::selection::InstanceClick::Range {
            self.ui.instance_anchor = Some(id);
        }
        self.select(next);
    }

    fn delete_selected(&mut self) {
        let removal = match self.ui.selection {
            Some(Selection::Device(index)) => edits::Removal::Device(index),
            Some(Selection::Template(index)) => edits::Removal::Template(index),
            _ => return,
        };
        self.apply(Action::Remove(removal));
        self.select(None);
    }

    fn copy_snippet(&mut self, snippet: crate::snippets::Snippet) {
        match self.doc.file_name() {
            Some(name) => {
                let text = crate::snippets::text(snippet, &name, self.doc.file());
                self.ctx.copy_text(text);
                self.ui.snippet_copied = Some(snippet);
            }
            None => {
                self.ui.modal = Some(Modal::Error(t!("app-snippet-needs-save")));
            }
        }
    }

    fn open_recent(&mut self, path: std::path::PathBuf) {
        if path.exists() {
            self.guard(Pending::OpenPath(path));
        } else {
            self.settings.forget(&path);
            self.ui.modal = Some(Modal::Error(t!(
                "app-recent-gone",
                path = path.display().to_string()
            )));
        }
    }

    fn toggle_mcp(&mut self) {
        if self.mcp.is_some() {
            self.stop_mcp();
        } else {
            self.start_mcp(self.mcp_port);
        }
        self.settings.mcp_on = self.mcp.is_some();
    }

    fn set_mcp_remote(&mut self, remote: bool) {
        self.settings.mcp_remote = remote;
        if remote && self.settings.mcp_token.is_none() {
            self.settings.mcp_token = Some(senzor_mcp::new_token());
        }
        self.restart_mcp();
    }

    fn new_mcp_token(&mut self) {
        self.settings.mcp_token = Some(senzor_mcp::new_token());
        if self.settings.mcp_remote {
            self.restart_mcp();
        }
    }

    fn apply_modal(&mut self, outcome: Outcome) {
        self.ui.modal = None;
        match outcome {
            Outcome::Dismissed | Outcome::Unsaved(_, UnsavedChoice::Cancel) => {}
            Outcome::Unsaved(pending, UnsavedChoice::Discard) => self.proceed(pending),
            Outcome::Unsaved(pending, UnsavedChoice::Save) => self.save(Some(pending)),
            Outcome::Reload => {
                self.doc.reload();
                self.after_undo_or_redo();
            }
            Outcome::KeepMine => self.doc.keep_mine(),
            Outcome::ClearRetained(topics) => self.start_clear_retained(topics),
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

    /// Tells the failure watch whether to notify now: the setting is on and the window is in the
    /// background (unfocused or minimized). Losing the focus and minimizing both bring a frame,
    /// so this stays right even when no frame follows.
    fn watch_failures(&mut self, ctx: &egui::Context) {
        let in_background = ctx.input(|i| {
            let viewport = i.viewport();
            viewport.focused == Some(false) || viewport.minimized == Some(true)
        });
        self.failure_notifications.tell(
            self.settings.notify_failures && in_background,
            crate::i18n::language(),
        );
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
    /// Runs before every frame, also when `ui` does not (a minimized window), but not at all
    /// while the window gets no frames: failures are watched on a thread of their own.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.watch_failures(ctx);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.settings.tabs = PanelTabs {
            left: self.ui.left_tab,
            bottom: self.ui.bottom_tab,
        };
        self.settings.save(storage);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // The zoom changes through egui as well, with Ctrl + +, Ctrl + − and Ctrl + 0.
        self.settings.zoom = ctx.zoom_factor();
        self.autostart_when_ready();
        self.handle_mcp_requests();
        self.state = self.engine().snapshot();
        let now = Instant::now();
        self.rate
            .record(now, self.state.log.back().map(|entry| entry.seq));
        let rate = self.rate.per_second(now);
        if rate > 0.0 {
            // Nothing repaints once publishing stops; the rate still has to fall to 0.
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        self.update_connection_test();
        self.update_clear_retained();
        self.check_for_updates();
        self.ui.devices_running = self.state.instances.values().any(|i| i.status.is_running());
        self.check_disk();
        self.handle_close_request(&ctx);
        // The panels record a rename still being typed again while they draw.
        self.ui.pending_rename = None;

        let title = format!("Senzor — {}", ui::project_title(&self.doc));
        if title != self.window_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.window_title = title;
        }

        let mut actions = Vec::new();
        // While a dialog asks something, it gets the keys, and dropped files are ignored.
        if self.ui.modal.is_none() {
            actions.extend(ui::shortcuts::pressed(&ctx));
            actions.extend(ui::dropped_file(&ctx).map(Action::OpenDropped));
        }
        let starting = self.starting_soon();
        let newer = self.newer_release().cloned();
        egui::Panel::top("top-bar").show(ui, |ui| {
            let mcp = match self.mcp_status() {
                McpStatus::Listening(address) => Some(address),
                McpStatus::Off { .. } => None,
            };
            ui::top_bar::show(
                ui,
                &self.doc,
                &self.settings.recent,
                starting,
                mcp,
                &mut actions,
            );
        });
        egui::Panel::bottom("status-bar").show(ui, |ui| {
            let mcp = match self.mcp_status() {
                McpStatus::Listening(address) => Some(address),
                McpStatus::Off { .. } => None,
            };
            ui::status_bar::show(
                ui,
                &self.state,
                &self.doc.file().broker,
                rate,
                mcp,
                newer.as_ref(),
                &mut actions,
            );
        });
        egui::Panel::bottom("bottom")
            .resizable(true)
            .default_size(180.0)
            .show(ui, |ui| {
                // A panel takes the height of what it shows, so a short tab ("No problems.")
                // would shrink it for good; filling it keeps the height it has.
                ui.take_available_height();
                ui::bottom::show(ui, &self.doc, &self.state, &mut self.ui, &mut actions);
            });
        egui::Panel::left("left")
            .resizable(true)
            .default_size(260.0)
            .show(ui, |ui| {
                ui::left_panel::show(ui, &self.doc, &self.state, &mut self.ui, &mut actions);
            });
        egui::CentralPanel::default().show(ui, |ui| {
            // A wide window shows a template's preview beside its editor, where it stays in view.
            let preview_template = match &self.ui.selection {
                Some(Selection::Template(index)) => self.doc.file().templates.get(*index).cloned(),
                Some(Selection::Builtin(id)) => {
                    senzor_core::project::builtin::builtin_template(id).cloned()
                }
                _ => None,
            };
            let preview_beside =
                preview_template.is_some() && ui.available_width() >= PREVIEW_BESIDE_MIN_WIDTH;
            if preview_beside && let Some(template) = &preview_template {
                egui::Panel::right("template-preview")
                    .resizable(true)
                    .default_size(ui.available_width() * 0.4)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("template-preview")
                            .auto_shrink([false; 2])
                            .show(ui, |ui| {
                                ui::template::preview_beside(ui, template, &self.doc)
                            });
                    });
            }
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .show(ui, |ui| match self.ui.selection.clone() {
                    Some(Selection::Instances(ids)) => {
                        ui::bulk::show(ui, &ids, &self.doc, &self.state, &mut self.ui, &mut actions)
                    }
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
                    Some(Selection::Template(index)) => ui::template::show_user(
                        ui,
                        index,
                        &self.doc,
                        preview_beside,
                        &mut self.ui,
                        &mut actions,
                    ),
                    Some(Selection::Builtin(id)) => ui::template::show_builtin(
                        ui,
                        &id,
                        &self.doc,
                        preview_beside,
                        &mut self.ui,
                        &mut actions,
                    ),
                    Some(Selection::Source) => {
                        ui::source::show(ui, &self.doc, &mut self.ui, &mut actions)
                    }
                    None if ui::welcome::applies(&self.doc) => {
                        ui::welcome::show(ui, &self.settings.recent, &mut actions)
                    }
                    None => {
                        ui.heading("Senzor");
                        ui.label(t!("app-select-something"));
                    }
                });
        });
        if self.ui.broker_open {
            ui::broker::show(&ctx, &self.doc, &mut self.ui, &mut actions);
        }
        ui::drop_hint(&ctx);
        if self.ui.settings_open {
            let mcp = self.mcp_status();
            ui::settings::show(
                &ctx,
                &mut self.ui,
                &self.settings,
                &mcp,
                newer.as_ref(),
                &mut actions,
            );
        }

        if let Some(palette) = &mut self.ui.palette {
            let entries = ui::palette::entries(&self.doc, &self.state, newer.as_ref());
            match ui::palette::show(&ctx, palette, &entries) {
                Some(ui::palette::Outcome::Run(action)) => {
                    self.ui.palette = None;
                    actions.push(*action);
                }
                Some(ui::palette::Outcome::Close) => self.ui.palette = None,
                None => {}
            }
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
        self.publish_view();
    }

    /// Waits for the current engine and for replaced ones still stopping: dropping the runtime
    /// would cancel their shutdown halfway, and their devices would disappear through the Last
    /// Will instead of a clean `offline`.
    fn on_exit(&mut self) {
        self.stop_mcp();
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

    use egui_kittest::kittest::Queryable;
    use senzor_core::engine::InstanceStatus;
    use senzor_testkit::slow_broker;

    use super::*;
    use crate::testing::{
        App, FRAME_PAUSE, FakeDialogs, TIMEOUT, click, status, wait_online, wait_until,
        window_builder, write_project,
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
        let mut harness = window_builder().build_eframe(move |cc| {
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
        let mut harness = window_builder().build_eframe(move |cc| {
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

    /// The window's own texts follow the language of the settings.
    #[test]
    fn the_window_speaks_the_language_of_its_settings() {
        let path = write_project(
            "czech-window",
            AUTOSTART_D.replace("true", "false").as_str(),
        );
        let mut h = window_builder().build_eframe(move |cc| {
            let settings = AppSettings {
                language: crate::i18n::Language::Czech,
                ..AppSettings::default()
            };
            SenzorApp::with_settings(cc, Some(path), Box::new(FakeDialogs::default()), settings)
        });
        h.run();
        assert!(
            h.query_by_label("Vlevo vyberte zařízení, instanci nebo šablonu.")
                .is_some()
        );

        let gone = std::env::temp_dir().join("senzor-czech-window-gone.toml");
        h.state_mut().apply(Action::OpenRecent(gone.clone()));
        h.run();
        let text = format!(
            "{} už neexistuje, a proto zmizel z nedávných projektů.",
            gone.display()
        );
        assert!(h.query_by_label(&text).is_some(), "{text}");
        assert!(h.query_by_label("Něco se pokazilo").is_some());
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
