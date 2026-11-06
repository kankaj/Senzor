//! GUI flows driven headlessly through AccessKit with egui_kittest.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use senzor::SenzorApp;
use senzor::document::Document;
use senzor::testing::{
    App, FRAME_PAUSE, FakeDialogs, TIMEOUT, click, status, test_dir, wait_online, wait_until,
    write_project,
};
use senzor_core::engine::InstanceStatus;
use senzor_core::project::ProjectFile;
use senzor_testkit::{Subscriber, slow_broker, start_broker};

fn app(path: Option<PathBuf>, dialogs: FakeDialogs) -> App {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_eframe(move |cc| SenzorApp::new(cc, path, Box::new(dialogs)));
    harness.step();
    harness
}

fn project(port: u16, autostart: bool) -> String {
    format!(
        r#"
[broker]
host = "127.0.0.1"
port = {port}

[[templates]]
id = "air"
interval = "200ms"
availability = {{ topic = "gui/{{{{ device_id }}}}/status", online = "online", offline = "offline" }}
messages = [{{ topic = "gui/{{{{ device_id }}}}/state", payload = "{{{{ co2 | int }}}}" }}]
fields.co2 = {{ generator = "constant", value = 600 }}

[[devices]]
id = "office"
template = "air"
count = 2
autostart = {autostart}

[[devices]]
id = "spare"
template = "air"
autostart = false
"#
    )
}

/// Asks to close the window, as its close button does.
fn request_close(harness: &mut App) {
    harness
        .input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    harness.run_steps(2);
}

#[test]
fn opening_a_project_autostarts_its_devices() {
    let port = start_broker();
    let path = write_project("autostart", &project(port, true));
    let mut app = app(Some(path), FakeDialogs::default());

    wait_until(&mut app, "office online", |a| {
        status(a, "office-0") == InstanceStatus::Online
            && status(a, "office-1") == InstanceStatus::Online
    });
    assert_eq!(status(&app, "spare"), InstanceStatus::Stopped);

    click(&mut app, "office-0");
    wait_until(&mut app, "instance view", |a| {
        a.query_by_label("Instance office-0").is_some() && a.query_by_label("Online").is_some()
    });
}

#[test]
fn start_all_and_stop_all_ignore_autostart() {
    let port = start_broker();
    let path = write_project("start-all", &project(port, false));
    let mut app = app(Some(path), FakeDialogs::default());
    assert_eq!(status(&app, "office-0"), InstanceStatus::Stopped);

    click(&mut app, "Start all");
    wait_until(&mut app, "everything online", |a| {
        a.state()
            .engine_state()
            .instances
            .values()
            .all(|i| i.status == InstanceStatus::Online)
    });

    click(&mut app, "Stop all");
    wait_until(&mut app, "everything stopped", |a| {
        a.state()
            .engine_state()
            .instances
            .values()
            .all(|i| i.status == InstanceStatus::Stopped)
    });
}

#[test]
fn pinning_a_field_publishes_the_pinned_value() {
    let port = start_broker();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut sub = runtime.block_on(Subscriber::connect(port, "gui/office-0/state"));
    let path = write_project("pin", &project(port, true));
    let mut app = app(Some(path), FakeDialogs::default());
    wait_online(&mut app, "office-0");

    click(&mut app, "office-0");
    let input = app.get_by_label("co2");
    input.focus();
    input.type_text("2500");
    app.run_steps(2);
    click(&mut app, "Pin");

    let pinned = runtime.block_on(sub.wait_for(Duration::from_secs(5), |m| m.payload == "2500"));
    assert!(
        pinned.is_some(),
        "the broker should receive the pinned value"
    );
    wait_until(&mut app, "pinned marker", |a| {
        a.query_by_label("2500 (pinned)").is_some()
    });
}

/// The new engine uses the same client IDs, so it must wait for the old one's clean stop;
/// otherwise the broker sees the new `online` first and the old `offline` last.
#[test]
fn reopening_a_project_takes_its_devices_offline_once_before_they_come_back() {
    let port = start_broker();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let mut sub = runtime.block_on(Subscriber::connect(port, "gui/office-0/status"));
    let path = write_project("reopen", &project(port, true));
    let dialogs = FakeDialogs::opening(path.clone());
    let mut app = app(Some(path), dialogs);
    wait_online(&mut app, "office-0");
    let first = runtime.block_on(sub.wait_for(Duration::from_secs(5), |m| m.payload == "online"));
    assert!(first.is_some(), "the first start publishes online");

    click(&mut app, "Open");
    let mut payloads: Vec<String> = Vec::new();
    let mut collect = |app: &mut App, payloads: &mut Vec<String>| {
        app.step();
        let messages = runtime.block_on(sub.collect(Duration::from_millis(50)));
        payloads.extend(messages.into_iter().map(|m| m.payload));
    };
    let deadline = Instant::now() + TIMEOUT;
    while !(payloads.iter().any(|p| p == "offline")
        && payloads.last().is_some_and(|p| p == "online"))
    {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for offline, then online: {payloads:?}"
        );
        collect(&mut app, &mut payloads);
    }
    for _ in 0..10 {
        collect(&mut app, &mut payloads);
    }
    assert_eq!(payloads, ["offline", "online"]);
    assert_eq!(status(&app, "office-0"), InstanceStatus::Online);
}

/// The fake broker never closes a connection, so the old engine's clean stop takes its full 2 s.
#[test]
fn reopened_autostart_devices_wait_until_the_old_engine_has_stopped_them() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let port = runtime.block_on(slow_broker(Duration::ZERO));
    let path = write_project("reopen-slow", &project(port, true));
    let dialogs = FakeDialogs::opening(path.clone());
    let mut app = app(Some(path), dialogs);
    wait_online(&mut app, "office-0");

    click(&mut app, "Open");
    let until = Instant::now() + Duration::from_millis(1000);
    while Instant::now() < until {
        app.step();
        assert_eq!(
            status(&app, "office-0"),
            InstanceStatus::Stopped,
            "started while the old engine was still stopping it"
        );
        std::thread::sleep(FRAME_PAUSE);
    }
    wait_online(&mut app, "office-0");
}

fn needs_restart(harness: &App, id: &str) -> bool {
    harness.state().engine_state().instances[id].needs_restart
}

#[test]
fn restart_applies_project_changes_to_a_running_instance() {
    let port = start_broker();
    let path = write_project("restart", &project(port, true));
    let mut app = app(Some(path), FakeDialogs::default());
    wait_online(&mut app, "office-0");

    click(&mut app, "office ×2");
    click(&mut app, "Add param");
    wait_until(&mut app, "office-0 needs a restart", |a| {
        needs_restart(a, "office-0")
    });

    click(&mut app, "office-0");
    click(&mut app, "Restart");
    wait_until(&mut app, "office-0 online with the new settings", |a| {
        status(a, "office-0") == InstanceStatus::Online && !needs_restart(a, "office-0")
    });
    assert!(needs_restart(&app, "office-1"), "only office-0 restarted");
}

#[test]
fn adding_a_device_marks_the_project_unsaved_and_updates_the_engine() {
    let mut app = app(None, FakeDialogs::default());
    assert!(app.query_by_label("untitled").is_some());

    click(&mut app, "Add device");
    assert!(app.query_by_label("untitled*").is_some());
    assert!(
        app.query_by_label("Device device").is_some(),
        "the new device is selected"
    );
    wait_until(&mut app, "engine knows the device", |a| {
        a.state().engine_state().instances.contains_key("device")
    });
}

#[test]
fn clicking_a_problem_selects_the_device() {
    let path = write_project(
        "problems",
        "[[devices]]\nid = \"bad\"\ntemplate = \"nope\"\n",
    );
    let mut app = app(Some(path), FakeDialogs::default());

    click(&mut app, "Problems (1)");
    click(&mut app, "device `bad`: unknown template `nope`");
    assert!(app.query_by_label("Device bad").is_some());
}

#[test]
fn a_file_that_does_not_parse_shows_its_errors() {
    let path = write_project("broken", "[broker]\nhots = 1\n");
    let mut app = app(Some(path), FakeDialogs::default());

    assert!(app.query_by_label_contains("Cannot open").is_some());
    assert!(app.query_by_label_contains("line 2, column 1").is_some());
    click(&mut app, "OK");
    assert!(app.query_by_label_contains("Cannot open").is_none());
    assert!(app.query_by_label("untitled").is_some());
}

#[test]
fn new_with_unsaved_changes_asks_first() {
    let mut app = app(None, FakeDialogs::default());
    click(&mut app, "Add device");

    click(&mut app, "New");
    assert!(app.query_by_label("Unsaved changes").is_some());
    click(&mut app, "Cancel");
    assert!(app.query_by_label("Unsaved changes").is_none());
    assert!(app.state().document().is_dirty());

    click(&mut app, "New");
    click(&mut app, "Discard");
    assert!(!app.state().document().is_dirty());
    assert!(app.state().document().file().devices.is_empty());
}

#[test]
fn open_with_unsaved_changes_asks_first() {
    let path = write_project(
        "open-target",
        "[[devices]]\nid = \"other\"\ntemplate = \"co2\"\nautostart = false\n",
    );
    let dialogs = FakeDialogs::opening(path);
    let mut app = app(None, dialogs);
    click(&mut app, "Add device");

    click(&mut app, "Open");
    assert!(app.query_by_label("Unsaved changes").is_some());
    click(&mut app, "Discard");
    assert_eq!(app.state().document().file().devices[0].id, "other");
    assert!(app.query_by_label("project.toml").is_some());
}

#[test]
fn closing_with_unsaved_changes_asks_first() {
    let mut app = app(None, FakeDialogs::default());
    click(&mut app, "Add device");

    request_close(&mut app);
    assert!(app.query_by_label("Unsaved changes").is_some());
    click(&mut app, "Cancel");
    assert!(app.query_by_label("Unsaved changes").is_none());
}

/// Clicks a button of the unsaved-changes prompt; its "Save" shares the label with the top bar's.
/// The prompt's click lands a frame later than a top-bar click, and a modal it opens first shows
/// at the prompt's place, so one more frame lets that modal centre before the next click.
fn choose_in_prompt(harness: &mut App, choice: &str) {
    assert!(harness.query_by_label("Unsaved changes").is_some());
    let prompt_heading = harness.get_by_label("Unsaved changes").rect().top();
    harness
        .get_all_by_label(choice)
        .find(|button| button.rect().top() > prompt_heading)
        .expect("the prompt has this button")
        .click();
    harness.run_steps(3);
}

#[test]
fn saving_from_the_unsaved_changes_prompt_saves_then_continues() {
    let path = write_project(
        "prompt-save",
        "[[devices]]\nid = \"office\"\ntemplate = \"co2\"\nautostart = false\n",
    );
    let mut app = app(Some(path.clone()), FakeDialogs::default());
    click(&mut app, "Add device");

    click(&mut app, "New");
    choose_in_prompt(&mut app, "Save");
    let saved = Document::open(&path).unwrap();
    assert_eq!(saved.file().devices.len(), 2, "the added device was saved");
    assert!(
        app.query_by_label("untitled").is_some(),
        "then New went ahead"
    );
    assert!(app.query_by_label("Unsaved changes").is_none());
}

#[test]
fn cancelling_save_as_from_the_unsaved_changes_prompt_keeps_the_project() {
    let mut app = app(None, FakeDialogs::default());
    click(&mut app, "Add device");

    click(&mut app, "New");
    choose_in_prompt(&mut app, "Save");
    assert!(
        app.query_by_label("Unsaved changes").is_none(),
        "the prompt does not come back"
    );
    assert!(app.state().document().is_dirty());
    assert_eq!(app.state().document().file().devices.len(), 1);
    assert!(app.query_by_label("untitled*").is_some());
}

#[test]
fn discarding_changes_when_closing_closes_the_window() {
    let mut app = app(None, FakeDialogs::default());
    click(&mut app, "Add device");
    request_close(&mut app);

    app.get_by_label("Discard").click();
    // One frame per queued pointer event; the last one releases the button.
    app.step();
    let commands = &app.output().viewport_output[&egui::ViewportId::ROOT].commands;
    assert!(
        commands.contains(&egui::ViewportCommand::Close),
        "{commands:?}"
    );
}

#[test]
fn copying_a_builtin_template_into_the_project() {
    let mut app = app(None, FakeDialogs::default());
    click(&mut app, "Templates");
    click(&mut app, "co2");
    assert!(app.query_by_label("Built-in template co2").is_some());

    click(&mut app, "Copy into project…");
    assert_eq!(
        app.get_by_label("New template ID").value().as_deref(),
        Some("co2-copy")
    );
    click(&mut app, "Copy");
    assert_eq!(app.state().document().file().templates[0].id, "co2-copy");
    assert!(app.query_by_label("Template co2-copy").is_some());
}

#[test]
fn save_as_writes_a_file_that_opens_again() {
    let path = test_dir("save").join("saved.toml");
    let mut app = app(None, FakeDialogs::saving(path.clone()));
    click(&mut app, "Add device");

    click(&mut app, "Save as");
    assert!(
        app.query_by_label("saved.toml").is_some(),
        "title shows the file, clean"
    );
    let reopened = Document::open(&path).unwrap();
    assert_eq!(reopened.file().devices.len(), 1);
}

#[test]
fn save_as_a_name_without_an_extension_writes_a_toml_file() {
    let dir = test_dir("no-extension");
    let mut app = app(None, FakeDialogs::saving(dir.join("myproject")));
    click(&mut app, "Add device");

    click(&mut app, "Save as");
    assert!(
        app.query_by_label("Replace myproject.toml?").is_none(),
        "no file to replace, nothing to ask"
    );
    assert!(dir.join("myproject.toml").is_file());
    assert!(!dir.join("myproject").exists());
    assert!(app.query_by_label("myproject.toml").is_some());
}

const EXISTING: &str = "[[devices]]\nid = \"existing\"\ntemplate = \"co2\"\nautostart = false\n";

/// A folder that already holds `x.toml`, and dialogs whose Save answers `x`: the dialog only
/// asked about replacing `x`, but the project is saved as `x.toml`.
fn folder_with_x_toml(name: &str) -> (PathBuf, FakeDialogs) {
    let dir = test_dir(name);
    let existing = dir.join("x.toml");
    std::fs::write(&existing, EXISTING).unwrap();
    (existing, FakeDialogs::saving(dir.join("x")))
}

fn device_ids(file: &ProjectFile) -> Vec<String> {
    file.devices.iter().map(|d| d.id.clone()).collect()
}

fn saved_device_ids(path: &std::path::Path) -> Vec<String> {
    device_ids(Document::open(path).unwrap().file())
}

#[test]
fn save_as_asks_before_replacing_the_toml_file_the_dialog_did_not_ask_about() {
    let (existing, dialogs) = folder_with_x_toml("replace");
    let mut app = app(None, dialogs);
    click(&mut app, "Add device");

    click(&mut app, "Save as");
    assert!(app.query_by_label("Replace x.toml?").is_some());
    assert_eq!(std::fs::read_to_string(&existing).unwrap(), EXISTING);

    click(&mut app, "Replace");
    assert!(app.query_by_label("Replace x.toml?").is_none());
    assert_eq!(saved_device_ids(&existing), ["device"]);
    assert!(!app.state().document().is_dirty());
    assert_eq!(app.state().document().path(), Some(existing.as_path()));
}

#[test]
fn cancelling_the_replace_question_writes_nothing() {
    let (existing, dialogs) = folder_with_x_toml("replace-cancel");
    let mut app = app(None, dialogs);
    click(&mut app, "Add device");

    click(&mut app, "Save as");
    click(&mut app, "Cancel");
    assert!(app.query_by_label("Replace x.toml?").is_none());
    assert_eq!(std::fs::read_to_string(&existing).unwrap(), EXISTING);
    assert!(app.state().document().is_dirty());
    assert_eq!(app.state().document().path(), None);
}

#[test]
fn replacing_from_the_unsaved_changes_prompt_saves_then_continues() {
    let (existing, dialogs) = folder_with_x_toml("prompt-replace");
    let mut app = app(None, dialogs);
    click(&mut app, "Add device");

    click(&mut app, "New");
    choose_in_prompt(&mut app, "Save");
    assert!(app.query_by_label("Replace x.toml?").is_some());
    click(&mut app, "Replace");
    assert_eq!(saved_device_ids(&existing), ["device"]);
    assert!(
        app.query_by_label("untitled").is_some(),
        "then New went ahead"
    );
    assert!(app.state().document().file().devices.is_empty());
}

#[test]
fn cancelling_the_replace_question_from_the_unsaved_changes_prompt_keeps_the_project() {
    let (existing, dialogs) = folder_with_x_toml("prompt-replace-cancel");
    let mut app = app(None, dialogs);
    click(&mut app, "Add device");

    click(&mut app, "New");
    choose_in_prompt(&mut app, "Save");
    click(&mut app, "Cancel");
    assert!(app.query_by_label("Replace x.toml?").is_none());
    assert!(
        app.query_by_label("Unsaved changes").is_none(),
        "the prompt does not come back"
    );
    assert_eq!(std::fs::read_to_string(&existing).unwrap(), EXISTING);
    assert!(app.state().document().is_dirty());
    assert_eq!(app.state().document().file().devices.len(), 1);
    assert!(app.query_by_label("untitled*").is_some());
}

const WITH_PARAM: &str = "[[devices]]\nid = \"office\"\ntemplate = \"co2\"\nautostart = false\nparams = { room = \"kitchen\" }\n";

fn start_param_rename(app: &mut App) {
    click(app, "office ×1");
    let name = app.get_all_by_value("room").next().unwrap();
    name.focus();
    name.type_text("2");
    app.run_steps(2);
}

#[test]
fn new_with_a_pending_rename_asks_first() {
    let path = write_project("pending-new", WITH_PARAM);
    let mut app = app(Some(path), FakeDialogs::default());
    start_param_rename(&mut app);
    click(&mut app, "New");
    assert!(app.query_by_label("Unsaved changes").is_some());
}

#[test]
fn open_with_a_pending_rename_leaves_the_opened_project_alone() {
    let other = write_project(
        "pending-open-target",
        "[[devices]]\nid = \"x\"\ntemplate = \"co2\"\nautostart = false\n\n[[devices]]\nid = \"y\"\ntemplate = \"co2\"\nautostart = false\n",
    );
    let path = write_project("pending-open", WITH_PARAM);
    let dialogs = FakeDialogs::opening(other);
    let mut app = app(Some(path), dialogs);
    start_param_rename(&mut app);
    click(&mut app, "Open");
    click(&mut app, "Discard");
    assert_eq!(device_ids(app.state().document().file()), ["x", "y"]);
    assert!(!app.state().document().is_dirty());
}

#[test]
fn closing_with_a_pending_rename_asks_first() {
    let path = write_project("pending-close", WITH_PARAM);
    let mut app = app(Some(path), FakeDialogs::default());
    start_param_rename(&mut app);
    request_close(&mut app);
    assert!(app.query_by_label("Unsaved changes").is_some());
}

/// Where the bottom panel's tabs are: they move down when the panel gets lower.
fn bottom_tabs_top(harness: &App) -> f32 {
    harness.get_by_label("Message log").rect().top()
}

#[test]
fn the_bottom_panel_keeps_its_height_while_there_are_no_problems() {
    let mut app = app(None, FakeDialogs::default());
    let top = bottom_tabs_top(&app);

    click(&mut app, "Problems");
    assert!(app.query_by_label("No problems.").is_some());
    assert_eq!(bottom_tabs_top(&app), top, "on the Problems tab");
    click(&mut app, "Message log");
    assert_eq!(bottom_tabs_top(&app), top, "back on the log");
}

#[test]
fn the_settings_button_opens_the_settings_window() {
    let mut harness = app(None, FakeDialogs::default());
    let version = env!("CARGO_PKG_VERSION");
    assert!(harness.query_by_label(version).is_none());

    click(&mut harness, "Settings");
    assert!(harness.query_by_label(version).is_some());
}

#[test]
fn the_settings_button_is_a_cog_at_the_right_end_of_the_top_bar() {
    let harness = app(None, FakeDialogs::default());
    let button = harness.get_by_label("Settings");
    let window_right = 1400.0;
    assert!(
        button.rect().right() > window_right - 20.0,
        "the button ends at x = {}",
        button.rect().right()
    );
    assert!(
        harness.query_by_label("⚙").is_none(),
        "the cog's label is Settings"
    );
}

#[test]
fn the_settings_window_opens_in_the_middle_of_the_window() {
    let mut harness = app(None, FakeDialogs::default());
    click(&mut harness, "Settings");
    // The window measures itself in its first frame and moves to the middle in the next.
    harness.run();
    let version = harness.get_by_label(env!("CARGO_PKG_VERSION")).rect();
    // The app window is 1400 × 900; the settings window is a few hundred pixels across.
    assert!(
        (500.0..900.0).contains(&version.center().x)
            && (250.0..650.0).contains(&version.center().y),
        "the settings window is around {:?}",
        version.center()
    );
}

#[test]
fn choosing_a_theme_in_settings_applies_it_and_keeps_it_for_the_next_launch() {
    use senzor::settings::Theme;
    let mut harness = app(None, FakeDialogs::default());
    click(&mut harness, "Settings");
    harness.run();

    click(&mut harness, "Light");
    assert_eq!(harness.ctx.theme(), egui::Theme::Light);
    assert_eq!(harness.state().settings().theme, Theme::Light);

    click(&mut harness, "Dark");
    assert_eq!(harness.ctx.theme(), egui::Theme::Dark);
    assert_eq!(harness.state().settings().theme, Theme::Dark);
}

/// Settings kept in memory, standing in for eframe's file.
#[derive(Default)]
struct MemoryStorage(std::collections::HashMap<String, String>);

impl eframe::Storage for MemoryStorage {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }

    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.to_owned(), value);
    }

    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }

    fn flush(&mut self) {}
}

#[test]
fn the_zoom_buttons_in_settings_zoom_the_window_and_the_zoom_is_saved() {
    use senzor::settings::AppSettings;
    let zoom = |harness: &App| (harness.ctx.zoom_factor() * 10.0).round() / 10.0;
    let mut harness = app(None, FakeDialogs::default());
    click(&mut harness, "Settings");
    harness.run();
    assert!(harness.query_by_label("100 %").is_some());

    click(&mut harness, "Zoom in");
    harness.run();
    assert_eq!(zoom(&harness), 1.1);
    assert!(harness.query_by_label("110 %").is_some());

    click(&mut harness, "Reset zoom");
    harness.run();
    assert_eq!(zoom(&harness), 1.0);

    click(&mut harness, "Zoom out");
    harness.run();
    assert_eq!(zoom(&harness), 0.9);

    let mut storage = MemoryStorage::default();
    eframe::App::save(harness.state_mut(), &mut storage);
    assert_eq!(AppSettings::load(Some(&storage)).zoom, 0.9);
}

/// The app's recent projects, the latest first.
fn recent(harness: &App) -> Vec<PathBuf> {
    harness.state().settings().recent.clone()
}

/// `path` as the recent projects store it.
fn resolved(path: &std::path::Path) -> PathBuf {
    path.canonicalize().unwrap()
}

/// Chooses `path` from the top bar's Recent menu; its items are named by the full path.
fn open_recent(harness: &mut App, path: &std::path::Path) {
    click(harness, "Recent");
    click(harness, &path.display().to_string());
}

#[test]
fn opening_a_project_puts_it_first_in_the_recent_projects() {
    let a = write_project("recent-open-a", "");
    let b = write_project("recent-open-b", "");
    let mut harness = app(Some(a.clone()), FakeDialogs::opening(b.clone()));
    assert_eq!(recent(&harness), [resolved(&a)]);

    click(&mut harness, "Open");
    assert_eq!(recent(&harness), [resolved(&b), resolved(&a)]);
}

#[test]
fn saving_a_project_under_a_name_puts_it_in_the_recent_projects() {
    let path = test_dir("recent-save-as").join("saved.toml");
    let mut harness = app(None, FakeDialogs::saving(path.clone()));
    click(&mut harness, "Save as");
    assert_eq!(recent(&harness), [resolved(&path)]);
}

#[test]
fn choosing_a_recent_project_opens_it() {
    let a = write_project("recent-choose-a", "");
    let b = write_project("recent-choose-b", "");
    let mut harness = app(Some(a.clone()), FakeDialogs::opening(b.clone()));
    click(&mut harness, "Open");

    open_recent(&mut harness, &resolved(&a));
    let opened = harness.state().document().path().map(resolved);
    assert_eq!(opened, Some(resolved(&a)));
    assert_eq!(recent(&harness), [resolved(&a), resolved(&b)]);
}

#[test]
fn a_recent_project_that_is_gone_says_so_and_leaves_the_list() {
    let a = write_project("recent-gone-a", "");
    let b = write_project("recent-gone-b", "");
    let mut harness = app(Some(a.clone()), FakeDialogs::opening(b.clone()));
    click(&mut harness, "Open");
    let gone = resolved(&a);
    std::fs::remove_file(&a).unwrap();

    open_recent(&mut harness, &gone);
    assert_eq!(recent(&harness), [resolved(&b)]);
    let opened = harness.state().document().path().map(resolved);
    assert_eq!(opened, Some(resolved(&b)), "the open project stays");
    assert!(
        harness
            .query_by_label(&format!(
                "{} no longer exists, so it was taken off the recent projects.",
                gone.display()
            ))
            .is_some()
    );
}

/// Whether hovering over `label` shows the tooltip `tip`.
fn shows_tooltip(harness: &mut App, label: &str, tip: &str) -> bool {
    harness.get_by_label(label).hover();
    // A step is 0.25 s; tooltips wait 0.5 s for the pointer to rest.
    harness.run_steps(4);
    let shown = harness.query_by_label(tip).is_some();
    harness.get_by_label("Settings").hover();
    harness.run_steps(2);
    shown
}

#[test]
fn save_stands_out_while_there_are_unsaved_changes() {
    const TIP: &str = "Save the unsaved changes (Ctrl+S)";
    let path = write_project("save-highlight", "");
    let mut harness = app(Some(path), FakeDialogs::default());
    assert!(!shows_tooltip(&mut harness, "Save", TIP));

    click(&mut harness, "Add device");
    assert!(harness.state().document().is_dirty());
    assert!(shows_tooltip(&mut harness, "Save", TIP));
}

const WELCOME: &str = "Welcome to Senzor";

#[test]
fn starting_without_a_project_shows_the_welcome_screen() {
    let harness = app(None, FakeDialogs::default());
    assert!(harness.query_by_label(WELCOME).is_some());
    assert!(harness.query_by_label("No recent projects yet.").is_some());
}

#[test]
fn an_opened_project_shows_no_welcome_screen() {
    let path = write_project("welcome-opened", "");
    let harness = app(Some(path), FakeDialogs::default());
    assert!(harness.query_by_label(WELCOME).is_none());
}

#[test]
fn the_welcome_screen_lists_the_recent_projects_and_opens_one() {
    let a = write_project("welcome-recent", "");
    let mut harness = app(None, FakeDialogs::opening(a.clone()));
    click(&mut harness, "Open");
    assert!(harness.query_by_label(WELCOME).is_none());

    click(&mut harness, "New");
    assert!(harness.query_by_label(WELCOME).is_some());
    click(&mut harness, &resolved(&a).display().to_string());
    let opened = harness.state().document().path().map(resolved);
    assert_eq!(opened, Some(resolved(&a)));
}

#[test]
fn adding_the_first_device_from_the_welcome_screen_leaves_it() {
    let mut harness = app(None, FakeDialogs::default());
    click(&mut harness, "Add your first device");
    assert_eq!(harness.state().document().file().devices.len(), 1);
    assert!(harness.query_by_label(WELCOME).is_none());
}

#[test]
fn the_settings_link_to_the_docs_and_the_source_and_credit_the_components() {
    let mut harness = app(None, FakeDialogs::default());
    click(&mut harness, "Settings");
    harness.run();
    assert!(harness.query_by_label("Source code").is_some());
    assert!(
        harness.query_by_label("Phosphor Icons").is_none(),
        "credits start folded"
    );

    click(&mut harness, "Open-source credits");
    harness.run();
    assert!(harness.query_by_label("Phosphor Icons").is_some());
    assert!(harness.query_by_label("egui and eframe").is_some());
}

/// Presses `key` with `modifiers` and lets the window handle it.
fn press(harness: &mut App, modifiers: egui::Modifiers, key: egui::Key) {
    harness.key_press_modifiers(modifiers, key);
    harness.run_steps(2);
}

const CTRL: egui::Modifiers = egui::Modifiers::COMMAND;

#[test]
fn ctrl_s_saves_and_ctrl_shift_s_saves_under_another_name() {
    let path = write_project("shortcut-save", "");
    let other = test_dir("shortcut-save").join("other.toml");
    let mut harness = app(Some(path.clone()), FakeDialogs::saving(other.clone()));
    click(&mut harness, "Add device");
    assert!(harness.state().document().is_dirty());

    press(&mut harness, CTRL, egui::Key::S);
    assert!(!harness.state().document().is_dirty());
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("[[devices]]")
    );
    assert_eq!(harness.state().document().path(), Some(path.as_path()));

    press(&mut harness, CTRL | egui::Modifiers::SHIFT, egui::Key::S);
    assert_eq!(harness.state().document().path(), Some(other.as_path()));
}

#[test]
fn ctrl_o_opens_a_project_and_ctrl_n_starts_a_new_one() {
    let a = write_project("shortcut-open", "");
    let mut harness = app(None, FakeDialogs::opening(a.clone()));
    press(&mut harness, CTRL, egui::Key::O);
    assert_eq!(harness.state().document().path(), Some(a.as_path()));

    press(&mut harness, CTRL, egui::Key::N);
    assert_eq!(harness.state().document().path(), None);
}

#[test]
fn ctrl_comma_opens_the_settings() {
    let mut harness = app(None, FakeDialogs::default());
    assert!(harness.query_by_label("Open-source credits").is_none());
    press(&mut harness, CTRL, egui::Key::Comma);
    assert!(harness.query_by_label("Open-source credits").is_some());
}

#[test]
fn f5_starts_every_device_and_shift_f5_stops_them() {
    let port = start_broker();
    let path = write_project("shortcut-run", &project(port, false));
    let mut harness = app(Some(path), FakeDialogs::default());
    let all = |a: &App, status: InstanceStatus| {
        a.state()
            .engine_state()
            .instances
            .values()
            .all(|i| i.status == status)
    };
    press(&mut harness, egui::Modifiers::NONE, egui::Key::F5);
    wait_until(&mut harness, "everything online", |a| {
        all(a, InstanceStatus::Online)
    });
    press(&mut harness, egui::Modifiers::SHIFT, egui::Key::F5);
    wait_until(&mut harness, "everything stopped", |a| {
        all(a, InstanceStatus::Stopped)
    });
}

#[test]
fn shortcuts_wait_while_a_dialog_is_open() {
    let a = write_project("shortcut-dialog-a", "");
    let b = write_project("shortcut-dialog-b", "");
    let mut harness = app(Some(a.clone()), FakeDialogs::opening(b));
    click(&mut harness, "Add device");
    press(&mut harness, CTRL, egui::Key::N);
    assert!(harness.query_by_label("Unsaved changes").is_some());

    press(&mut harness, CTRL, egui::Key::O);
    assert_eq!(harness.state().document().path(), Some(a.as_path()));
    assert!(harness.query_by_label("Unsaved changes").is_some());
}

#[test]
fn the_top_bar_buttons_name_their_shortcuts() {
    let mut harness = app(None, FakeDialogs::default());
    for (button, tip) in [
        ("New", "New project (Ctrl+N)"),
        ("Open", "Open a project (Ctrl+O)"),
        ("Save", "Save (Ctrl+S)"),
        ("Save as", "Save under another name (Ctrl+Shift+S)"),
        (
            "Start all",
            "Start every device, regardless of autostart (F5)",
        ),
        ("Stop all", "Stop every device (Shift+F5)"),
        ("Settings", "Settings (Ctrl+Comma)"),
    ] {
        assert!(shows_tooltip(&mut harness, button, tip), "{button}: {tip}");
    }
}

fn device_count(harness: &App) -> usize {
    harness.state().document().file().devices.len()
}

#[test]
fn ctrl_z_undoes_and_ctrl_shift_z_or_ctrl_y_redoes() {
    let path = write_project("undo-keys", "");
    let mut harness = app(Some(path), FakeDialogs::default());
    click(&mut harness, "Add device");
    assert_eq!(device_count(&harness), 1);

    press(&mut harness, CTRL, egui::Key::Z);
    assert_eq!(device_count(&harness), 0);
    assert!(!harness.state().document().is_dirty());
    press(&mut harness, CTRL | egui::Modifiers::SHIFT, egui::Key::Z);
    assert_eq!(device_count(&harness), 1);
    press(&mut harness, CTRL, egui::Key::Z);
    press(&mut harness, CTRL, egui::Key::Y);
    assert_eq!(device_count(&harness), 1);
}

#[test]
fn the_undo_and_redo_buttons_are_off_when_there_is_nothing_to_do() {
    use egui_kittest::kittest::NodeT;
    let path = write_project("undo-buttons", "");
    let mut harness = app(Some(path), FakeDialogs::default());
    let disabled = |h: &App, label: &str| h.get_by_label(label).accesskit_node().is_disabled();
    assert!(disabled(&harness, "Undo") && disabled(&harness, "Redo"));

    click(&mut harness, "Add device");
    assert!(!disabled(&harness, "Undo"));
    click(&mut harness, "Undo");
    assert_eq!(device_count(&harness), 0);
    assert!(disabled(&harness, "Undo") && !disabled(&harness, "Redo"));
    click(&mut harness, "Redo");
    assert_eq!(device_count(&harness), 1);
}

#[test]
fn ctrl_z_in_a_name_box_undoes_the_typing_not_the_project() {
    let path = write_project(
        "undo-name-box",
        "[[devices]]\nid = \"d\"\ntemplate = \"co2\"\n",
    );
    let mut harness = app(Some(path), FakeDialogs::default());
    click(&mut harness, "Add device");
    click(&mut harness, "d ×1");
    harness.get_by_label("ID").focus();
    harness.run();
    harness.key_press(egui::Key::End);
    harness.get_by_label("ID").type_text("x");
    harness.run();

    press(&mut harness, CTRL, egui::Key::Z);
    assert_eq!(device_count(&harness), 2, "the added device stays");
}

/// In a field that edits the project as you type, Ctrl+Z is the field's own undo, and the
/// project follows the field back.
#[test]
fn ctrl_z_in_a_field_takes_back_the_typing_and_the_project_follows() {
    let path = write_project("undo-typing", "");
    let mut harness = app(Some(path), FakeDialogs::default());
    click(&mut harness, "Broker: localhost:1883");
    harness.run();
    harness.get_by_label("Host").focus();
    harness.run();
    harness.get_by_label("Host").type_text(".lan");
    harness.run();
    assert_eq!(
        harness.state().document().file().broker.host,
        "localhost.lan"
    );

    press(&mut harness, CTRL, egui::Key::Z);
    assert_eq!(harness.state().document().file().broker.host, "localhost");
    assert!(!harness.state().document().is_dirty());
    assert!(
        !harness.state().document().can_undo(),
        "typing taken back leaves no undo step"
    );
}

#[test]
fn the_settings_name_senzors_licence_and_author() {
    let mut harness = app(None, FakeDialogs::default());
    click(&mut harness, "Settings");
    harness.run();
    assert!(harness.query_by_label("Apache-2.0").is_some());
    assert!(harness.query_by_label("© 2026 Jaroslav Kaňka").is_some());
}
