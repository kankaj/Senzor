//! GUI flows driven headlessly through AccessKit with egui_kittest.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui;
use egui_kittest::kittest::Queryable;
use rumqttc::ConnectReturnCode;
use senzor::SenzorApp;
use senzor::document::Document;
use senzor::testing::{
    App, FRAME_PAUSE, FakeDialogs, FakeReleases, RecordingNotifier, TIMEOUT, click, status,
    test_dir, wait_online, wait_until, window_builder, write_project,
};
use senzor_core::engine::InstanceStatus;
use senzor_core::project::ProjectFile;
use senzor_testkit::{Subscriber, refusing_broker, slow_broker, start_broker};

fn app(path: Option<PathBuf>, dialogs: FakeDialogs) -> App {
    let mut harness = window_builder()
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

#[test]
fn clear_retained_messages_empties_the_projects_retained_topics() {
    let port = start_broker();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let path = write_project("clear-retained", &project(port, true));
    let mut harness = app(Some(path), FakeDialogs::default());
    wait_online(&mut harness, "office-0");
    wait_online(&mut harness, "office-1");
    click(&mut harness, "Stop all");
    wait_until(&mut harness, "everything stopped", |h| {
        h.state()
            .engine_state()
            .instances
            .values()
            .all(|i| !i.status.is_running())
    });
    let before = runtime.block_on(async {
        let mut sub = Subscriber::connect(port, "gui/+/status").await;
        sub.collect(Duration::from_millis(500)).await
    });
    assert!(before.iter().any(|m| m.retain), "offline is retained");

    click(&mut harness, &format!("Broker: 127.0.0.1:{port}"));
    harness.run();
    click(&mut harness, "Clear retained messages…");
    harness.run();
    assert!(harness.query_by_label("Clear 3 retained topics?").is_some());
    assert!(harness.query_by_label("gui/spare/status").is_some());
    click(&mut harness, "Clear");
    wait_until(&mut harness, "the topics cleared", |h| {
        h.query_by_label("Cleared 3 retained topics").is_some()
    });

    let after = runtime.block_on(async {
        let mut sub = Subscriber::connect(port, "gui/+/status").await;
        sub.collect(Duration::from_millis(500)).await
    });
    assert!(after.iter().all(|m| !m.retain), "{after:?}");
}

#[test]
fn export_saves_the_logged_messages_as_csv() {
    let port = start_broker();
    let path = write_project("export-log", &project(port, true));
    let csv = test_dir("export-log").join("messages.csv");
    let mut harness = app(Some(path), FakeDialogs::saving(csv.clone()));
    wait_until(&mut harness, "a message logged", |h| {
        !h.state().engine_state().log.is_empty()
    });
    click(&mut harness, "Export…");
    let text = std::fs::read_to_string(&csv).unwrap();
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("time,instance,topic,qos,retained,payload")
    );
    let first = lines.next().unwrap();
    assert!(first.contains(",gui/office-"), "{first}");
    assert!(first.ends_with(",0,false,600"), "{first}");
}

#[test]
fn a_wide_window_shows_the_template_preview_beside_the_form() {
    let path = write_project("preview-beside", &project(start_broker(), false));
    let mut harness = app(Some(path), FakeDialogs::default());
    click(&mut harness, "Templates");
    click(&mut harness, "air");
    harness.run();
    let preview = harness.get_by_label("Preview").rect();
    let form = harness.get_by_label("Interval").rect();
    assert!(
        preview.left() > form.right() + 300.0,
        "preview at {preview:?}, form at {form:?}"
    );
    assert!(
        preview.top() < form.bottom() + 200.0,
        "side by side, not below"
    );
}

#[test]
fn ctrl_click_selects_several_instances_and_start_runs_them_all() {
    let port = start_broker();
    let path = write_project("bulk-start", &project(port, false));
    let mut harness = app(Some(path), FakeDialogs::default());
    click(&mut harness, "office-0");
    harness
        .get_by_label("office-1")
        .click_modifiers(egui::Modifiers::COMMAND);
    harness.run_steps(2);
    assert!(harness.query_by_label("2 instances").is_some());
    click(&mut harness, "Start");
    wait_online(&mut harness, "office-0");
    wait_online(&mut harness, "office-1");
    assert_eq!(status(&harness, "spare"), InstanceStatus::Stopped);
}

#[test]
fn shift_click_selects_a_range_of_instances() {
    let port = start_broker();
    let path = write_project("bulk-range", &project(port, false));
    let mut harness = app(Some(path), FakeDialogs::default());
    click(&mut harness, "office-0");
    harness
        .get_by_label("spare")
        .click_modifiers(egui::Modifiers::SHIFT);
    harness.run_steps(2);
    assert!(harness.query_by_label("3 instances").is_some());
    assert!(
        harness
            .query_by_label("office-0, office-1, spare")
            .is_some()
    );
}

/// Opens the command palette with Ctrl+K, types `query` and presses Enter.
fn run_from_palette(harness: &mut App, query: &str) {
    press(harness, CTRL, egui::Key::K);
    assert!(
        harness.query_by_label("Command palette").is_some(),
        "the palette opened"
    );
    harness.event(egui::Event::Text(query.into()));
    harness.run();
    press(harness, egui::Modifiers::NONE, egui::Key::Enter);
}

#[test]
fn the_command_palette_runs_an_action() {
    let mut harness = app(None, FakeDialogs::default());
    run_from_palette(&mut harness, "project sett");
    assert!(harness.query_by_label("Project settings").is_some());
    assert!(
        harness.query_by_label("Command palette").is_none(),
        "it closes"
    );
}

#[test]
fn the_command_palette_goes_to_an_instance() {
    let path = write_project("palette-go", &project(start_broker(), false));
    let mut harness = app(Some(path), FakeDialogs::default());
    run_from_palette(&mut harness, "office-1");
    assert!(harness.query_by_label("Instance office-1").is_some());
}

#[test]
fn escape_closes_the_command_palette() {
    let mut harness = app(None, FakeDialogs::default());
    press(&mut harness, CTRL, egui::Key::K);
    press(&mut harness, egui::Modifiers::NONE, egui::Key::Escape);
    assert!(harness.query_by_label("Command palette").is_none());
}

/// The text the window last put on the clipboard, if any.
fn copied(harness: &App) -> Option<String> {
    harness
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|c| match c {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
}

#[test]
fn project_settings_copy_a_github_actions_step_for_the_project() {
    let path = write_project("snippet", &project(1883, false));
    let mut harness = app(Some(path), FakeDialogs::default());
    click(&mut harness, "Broker: 127.0.0.1:1883");
    harness.run();
    harness
        .get_by_label("Copy as a GitHub Actions step")
        .click();
    harness.step();
    let text = copied(&harness).expect("something was copied");
    assert!(
        text.contains("senzor-cli run project.toml --duration 10m &"),
        "{text}"
    );
    harness.run();
    assert!(harness.query_by_label("Copied.").is_some());
}

#[test]
fn snippets_wait_until_the_project_has_a_file() {
    use egui_kittest::kittest::NodeT;
    let mut harness = app(None, FakeDialogs::default());
    click(&mut harness, "Set the broker");
    harness.run();
    assert!(
        harness
            .get_by_label("Copy as a docker-compose service")
            .accesskit_node()
            .is_disabled()
    );
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

/// A port nothing listens on right now.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn mcp_port(harness: &App) -> Option<u16> {
    match harness.state().mcp_status() {
        senzor::McpStatus::Listening(address) => Some(address.port()),
        senzor::McpStatus::Off { .. } => None,
    }
}

#[test]
fn the_mcp_port_set_in_settings_is_used_and_saved_with_mcp_on() {
    use senzor::settings::AppSettings;
    let port = free_port();
    let mut harness = app(None, FakeDialogs::default());
    click(&mut harness, "Settings");
    harness.run();
    let field = harness.get_by_label("MCP port");
    field.focus();
    harness.run();
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness
        .get_by_label("MCP port")
        .type_text(&port.to_string());
    harness.key_press(egui::Key::Enter);
    harness.run();
    click(&mut harness, "MCP");
    harness.run();
    assert_eq!(mcp_port(&harness), Some(port));

    let mut storage = MemoryStorage::default();
    eframe::App::save(harness.state_mut(), &mut storage);
    let saved = AppSettings::load(Some(&storage));
    assert_eq!((saved.mcp_port, saved.mcp_on), (port, true));

    click(&mut harness, "MCP");
    harness.run();
    eframe::App::save(harness.state_mut(), &mut storage);
    assert!(!AppSettings::load(Some(&storage)).mcp_on);
}

#[test]
fn the_mcp_flag_uses_the_port_in_the_settings() {
    let port = free_port();
    let settings = senzor::settings::AppSettings {
        mcp_port: port,
        ..Default::default()
    };
    let mut harness = window_builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_eframe(move |cc| {
            SenzorApp::with_settings(cc, None, Box::new(FakeDialogs::default()), settings)
                .with_mcp_on()
        });
    harness.step();
    assert_eq!(mcp_port(&harness), Some(port));
    assert!(!harness.state().settings().mcp_on, "the flag is not saved");
}

/// A window started with `settings`, as if they had been saved by an earlier launch.
fn app_with_settings(settings: senzor::settings::AppSettings) -> App {
    let mut harness = window_builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_eframe(move |cc| {
            SenzorApp::with_settings(cc, None, Box::new(FakeDialogs::default()), settings)
        });
    harness.step();
    harness
}

#[test]
fn the_selected_tabs_are_saved_and_come_back() {
    use senzor::settings::{AppSettings, PanelTabs};
    let mut harness = app_with_settings(AppSettings {
        tabs: PanelTabs {
            left: senzor::settings::LeftTab::Templates,
            bottom: senzor::settings::BottomTab::Problems,
        },
        ..Default::default()
    });
    assert!(harness.query_by_label("Add template").is_some());
    assert!(harness.query_by_label("No problems.").is_some());

    click(&mut harness, "Devices");
    click(&mut harness, "Message log");
    let mut storage = MemoryStorage::default();
    eframe::App::save(harness.state_mut(), &mut storage);
    let saved = AppSettings::load(Some(&storage)).tabs;
    assert_eq!(saved.left, senzor::settings::LeftTab::Devices);
    assert_eq!(saved.bottom, senzor::settings::BottomTab::Log);
}

#[test]
fn the_window_remembers_the_last_version_seen() {
    use senzor::settings::AppSettings;
    let harness = app_with_settings(AppSettings {
        last_version: Some("0.9.0".into()),
        ..Default::default()
    });
    assert_eq!(harness.state().version_seen_before(), Some("0.9.0"));
    assert_eq!(
        harness.state().settings().last_version.as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );

    let first = app_with_settings(AppSettings::default());
    assert_eq!(first.state().version_seen_before(), None, "a first launch");
}

/// A window started with `settings` whose update check asks `releases` instead of GitHub.
fn app_with_releases(settings: senzor::settings::AppSettings, releases: FakeReleases) -> App {
    let mut harness = window_builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_eframe(move |cc| {
            SenzorApp::with_settings(cc, None, Box::new(FakeDialogs::default()), settings)
                .with_release_source(Box::new(releases))
        });
    harness.step();
    harness
}

/// What the status bar says about release 99.0.0.
const NEWER: &str = "Senzor 99.0.0 is out";

fn release_99() -> senzor::update::Release {
    senzor::update::Release {
        version: "99.0.0".into(),
        url: "https://github.com/kankaj/Senzor/releases/tag/v99.0.0".into(),
    }
}

#[test]
fn a_newer_release_is_shown_in_the_status_bar_and_the_settings() {
    let releases = FakeReleases::latest("v99.0.0");
    let mut harness = app_with_releases(Default::default(), releases.clone());
    wait_until(&mut harness, "the notice", |h| {
        h.query_by_label_contains(NEWER).is_some()
    });
    assert_eq!(releases.asked(), 1);
    let settings = harness.state().settings();
    assert!(settings.update_checked.is_some(), "the check is remembered");
    assert_eq!(settings.latest_release, Some(release_99()));

    harness.get_by_label_contains(NEWER).click();
    harness.step();
    let opened: Vec<String> = harness
        .output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(opened, [release_99().url], "the release page opens");

    click(&mut harness, "Settings");
    harness.run();
    assert!(harness.query_by_label("99.0.0 is out").is_some());
}

#[test]
fn the_same_version_or_no_release_shows_no_notice() {
    let current = concat!("v", env!("CARGO_PKG_VERSION"));
    for releases in [FakeReleases::latest(current), FakeReleases::none()] {
        let mut harness = app_with_releases(Default::default(), releases.clone());
        wait_until(&mut harness, "the check", |h| {
            h.state().settings().update_checked.is_some()
        });
        harness.run();
        assert_eq!(releases.asked(), 1);
        assert!(harness.query_by_label_contains("is out").is_none());
    }
}

#[test]
fn with_update_checks_off_nothing_is_asked_or_shown() {
    let releases = FakeReleases::latest("v99.0.0");
    let mut harness = app_with_releases(
        senzor::settings::AppSettings {
            check_updates: false,
            latest_release: Some(release_99()),
            ..Default::default()
        },
        releases.clone(),
    );
    harness.run();
    std::thread::sleep(Duration::from_millis(200));
    harness.run();
    assert_eq!(releases.asked(), 0);
    assert!(
        harness.query_by_label_contains(NEWER).is_none(),
        "a release found earlier is not shown either"
    );
}

#[test]
fn a_check_within_the_last_day_is_not_repeated_and_its_release_still_shows() {
    let releases = FakeReleases::none();
    let mut harness = app_with_releases(
        senzor::settings::AppSettings {
            update_checked: Some(senzor::update::now_secs() - 3600),
            latest_release: Some(release_99()),
            ..Default::default()
        },
        releases.clone(),
    );
    harness.run();
    std::thread::sleep(Duration::from_millis(200));
    harness.run();
    assert_eq!(releases.asked(), 0);
    assert!(harness.query_by_label_contains(NEWER).is_some());
}

#[test]
fn turning_update_checks_off_hides_the_notice_and_is_saved() {
    let mut harness = app_with_releases(
        senzor::settings::AppSettings {
            update_checked: Some(senzor::update::now_secs()),
            latest_release: Some(release_99()),
            ..Default::default()
        },
        FakeReleases::none(),
    );
    assert!(harness.query_by_label_contains(NEWER).is_some());
    click(&mut harness, "Settings");
    harness.run();
    click(&mut harness, "Check for updates");
    harness.run();
    assert!(harness.query_by_label_contains(NEWER).is_none());
    let mut storage = MemoryStorage::default();
    eframe::App::save(harness.state_mut(), &mut storage);
    assert!(!senzor::settings::AppSettings::load(Some(&storage)).check_updates);
}

const WHATS_NEW: &str = concat!("What's new in Senzor ", env!("CARGO_PKG_VERSION"));

#[test]
fn the_first_launch_after_an_update_shows_whats_new() {
    use senzor::settings::AppSettings;
    let mut harness = app_with_settings(AppSettings {
        last_version: Some("0.9.0".into()),
        ..Default::default()
    });
    assert!(harness.query_by_label(WHATS_NEW).is_some());
    assert!(
        harness
            .query_by_label_contains("Simulate MQTT sensors from templates")
            .is_some()
    );
    click(&mut harness, "OK");
    assert!(harness.query_by_label(WHATS_NEW).is_none());
}

#[test]
fn a_first_launch_or_the_same_version_shows_no_whats_new() {
    use senzor::settings::AppSettings;
    for last_version in [None, Some(env!("CARGO_PKG_VERSION").to_string())] {
        let harness = app_with_settings(AppSettings {
            last_version: last_version.clone(),
            ..Default::default()
        });
        assert!(
            harness.query_by_label(WHATS_NEW).is_none(),
            "{last_version:?}"
        );
    }
}

#[test]
fn whats_new_in_the_settings_shows_this_versions_highlights() {
    let mut harness = app(None, FakeDialogs::default());
    click(&mut harness, "Settings");
    harness.run();
    click(&mut harness, "What's new");
    harness.run();
    assert!(harness.query_by_label(WHATS_NEW).is_some());
}

#[test]
fn mcp_left_on_starts_with_the_next_window() {
    let port = free_port();
    let harness = app_with_settings(senzor::settings::AppSettings {
        mcp_on: true,
        mcp_port: port,
        ..Default::default()
    });
    assert_eq!(mcp_port(&harness), Some(port));
}

#[test]
fn the_mcp_port_flag_wins_over_the_saved_port_for_this_launch() {
    let saved = free_port();
    let flag = free_port();
    let settings = senzor::settings::AppSettings {
        mcp_on: true,
        mcp_port: saved,
        ..Default::default()
    };
    let mut harness = window_builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_eframe(move |cc| {
            SenzorApp::with_settings(cc, None, Box::new(FakeDialogs::default()), settings)
                .with_mcp(flag)
        });
    harness.step();
    assert_eq!(mcp_port(&harness), Some(flag));
    assert_eq!(
        harness.state().settings().mcp_port,
        saved,
        "the flag is not saved"
    );
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

/// A file on disk, dropped on the window.
#[derive(Debug)]
struct DroppedPath(PathBuf);

impl egui::DroppedFile for DroppedPath {
    fn path(&self) -> &std::path::Path {
        &self.0
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.0).map_err(|e| e.to_string())
    }
}

/// Drops `path` on the window, as a file manager does.
fn drop_file(harness: &mut App, path: &std::path::Path) {
    harness
        .input_mut()
        .dropped_files
        .push(std::sync::Arc::new(DroppedPath(path.to_path_buf())));
    harness.run_steps(2);
}

#[test]
fn a_project_file_dropped_on_the_window_opens() {
    let path = write_project(
        "drop-open",
        "[[devices]]\nid = \"dropped\"\ntemplate = \"co2\"\n",
    );
    let mut harness = app(None, FakeDialogs::default());
    drop_file(&mut harness, &path);
    assert_eq!(harness.state().document().path(), Some(path.as_path()));
    assert!(harness.query_by_label("dropped ×1").is_some());
}

#[test]
fn dropping_a_file_asks_about_unsaved_changes_first() {
    let path = write_project("drop-unsaved", "");
    let mut harness = app(None, FakeDialogs::default());
    click(&mut harness, "Add your first device");
    drop_file(&mut harness, &path);
    assert!(harness.query_by_label("Unsaved changes").is_some());
    click(&mut harness, "Discard");
    assert_eq!(harness.state().document().path(), Some(path.as_path()));
}

#[test]
fn a_dropped_file_that_is_not_a_project_says_why() {
    let path = test_dir("drop-not-a-project").join("notes.txt");
    std::fs::write(&path, "not a project = [").unwrap();
    let mut harness = app(None, FakeDialogs::default());
    drop_file(&mut harness, &path);
    assert!(harness.query_by_label_contains("Cannot open").is_some());
    assert_eq!(harness.state().document().path(), None);
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
fn an_example_opens_from_the_welcome_screen_as_an_untitled_project() {
    let mut harness = app(None, FakeDialogs::default());
    click(&mut harness, "Quick start");
    let doc = harness.state().document();
    assert_eq!(doc.path(), None, "saving asks where");
    assert!(doc.file().devices.iter().any(|d| d.id == "office"));
    assert!(doc.validated().errors.is_empty());
    assert!(harness.query_by_label(WELCOME).is_none());
}

#[test]
fn the_examples_menu_opens_an_example_after_asking_about_unsaved_changes() {
    let mut harness = app(None, FakeDialogs::default());
    click(&mut harness, "Add your first device");
    click(&mut harness, "Examples");
    click(&mut harness, "Fleet");
    assert!(harness.query_by_label("Unsaved changes").is_some());
    click(&mut harness, "Discard");
    let ids: Vec<String> = harness
        .state()
        .document()
        .file()
        .devices
        .iter()
        .map(|d| d.id.clone())
        .collect();
    assert!(ids.iter().any(|id| id.starts_with("floor")), "{ids:?}");
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
fn delete_removes_the_selected_device_and_ctrl_z_brings_it_back() {
    let path = write_project(
        "shortcut-delete-device",
        "[[devices]]\nid = \"a\"\ntemplate = \"co2\"\n\n[[devices]]\nid = \"b\"\ntemplate = \"co2\"\n",
    );
    let mut harness = app(Some(path), FakeDialogs::default());
    click(&mut harness, "b ×1");
    press(&mut harness, egui::Modifiers::NONE, egui::Key::Delete);
    let ids = |h: &App| -> Vec<String> {
        h.state()
            .document()
            .file()
            .devices
            .iter()
            .map(|d| d.id.clone())
            .collect()
    };
    assert_eq!(ids(&harness), ["a"]);
    assert!(harness.query_by_label("Device b").is_none());

    press(&mut harness, CTRL, egui::Key::Z);
    assert_eq!(ids(&harness), ["a", "b"]);
}

#[test]
fn delete_removes_the_selected_template() {
    let path = write_project(
        "shortcut-delete-template",
        "[[templates]]\nid = \"t\"\nmessages = [{ topic = \"x\", payload = \"1\" }]\n\
         fields.v = { generator = \"constant\", value = 1 }\n",
    );
    let mut harness = app(Some(path), FakeDialogs::default());
    click(&mut harness, "Templates");
    click(&mut harness, "t");
    press(&mut harness, egui::Modifiers::NONE, egui::Key::Delete);
    assert!(harness.state().document().file().templates.is_empty());
}

#[test]
fn delete_in_a_text_box_deletes_text_not_the_device() {
    let path = write_project(
        "shortcut-delete-typing",
        "[[devices]]\nid = \"d\"\ntemplate = \"co2\"\n",
    );
    let mut harness = app(Some(path), FakeDialogs::default());
    click(&mut harness, "d ×1");
    harness.get_by_label("ID").focus();
    harness.run();
    harness.key_press(egui::Key::Home);
    press(&mut harness, egui::Modifiers::NONE, egui::Key::Delete);
    assert_eq!(device_count(&harness), 1);
}

#[test]
fn ctrl_f_shows_the_message_log_and_puts_the_cursor_in_its_filter() {
    let path = write_project(
        "shortcut-find",
        "[[devices]]\nid = \"bad\"\ntemplate = \"nope\"\n",
    );
    let mut harness = app(Some(path), FakeDialogs::default());
    click(&mut harness, "Problems (1)");
    press(&mut harness, CTRL, egui::Key::F);
    harness.event(egui::Event::Text("office".into()));
    harness.run();
    assert!(
        harness.query_all_by_value("office").next().is_some(),
        "typed into the filter"
    );
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

#[test]
fn test_connection_reports_a_working_broker_until_the_broker_settings_change() {
    let port = start_broker();
    let path = write_project("connection-test", &project(port, false));
    let mut harness = app(Some(path), FakeDialogs::default());
    click(&mut harness, &format!("Broker: 127.0.0.1:{port}"));
    click(&mut harness, "Test connection");
    wait_until(&mut harness, "the test passed", |h| {
        h.query_by_label("Connected").is_some()
    });

    harness.get_by_label("Host").focus();
    harness.run();
    harness.get_by_label("Host").type_text("x");
    harness.run();
    assert!(
        harness.query_by_label("Connected").is_none(),
        "an old result is not shown"
    );
}

#[test]
fn duplicate_device_adds_a_copy_after_it_and_selects_it() {
    let path = write_project(
        "duplicate-device",
        "[[devices]]\nid = \"d\"\ntemplate = \"co2\"\ncount = 2\n",
    );
    let mut harness = app(Some(path), FakeDialogs::default());
    click(&mut harness, "d ×2");
    click(&mut harness, "Duplicate device");
    let ids: Vec<String> = harness
        .state()
        .document()
        .file()
        .devices
        .iter()
        .map(|d| d.id.clone())
        .collect();
    assert_eq!(ids, ["d", "d-copy"]);
    assert!(harness.query_by_label("Device d-copy").is_some());
}

#[test]
fn duplicate_template_adds_a_copy_after_it_and_selects_it() {
    let path = write_project(
        "duplicate-template",
        "[[templates]]\nid = \"t\"\nmessages = [{ topic = \"x\", payload = \"{{ v }}\" }]\n\
         fields.v = { generator = \"constant\", value = 1 }\n",
    );
    let mut harness = app(Some(path), FakeDialogs::default());
    click(&mut harness, "Templates");
    click(&mut harness, "t");
    click(&mut harness, "Duplicate template");
    let ids: Vec<String> = harness
        .state()
        .document()
        .file()
        .templates
        .iter()
        .map(|t| t.id.clone())
        .collect();
    assert_eq!(ids, ["t", "t-copy"]);
    assert!(harness.query_by_label("Template t-copy").is_some());
}

/// Whether the widget labelled `label` has the keyboard.
fn has_focus(harness: &App, label: &str) -> bool {
    use egui_kittest::kittest::NodeT;
    harness.get_by_label(label).accesskit_node().is_focused()
}

#[test]
fn clicking_a_problem_puts_the_cursor_in_the_box_it_is_about() {
    let path = write_project(
        "problem-focus",
        "[[devices]]\nid = \"a\"\ntemplate = \"co2\"\n\n\
         [[devices]]\nid = \"d\"\ntemplate = \"co2\"\ncount = 2\ninstance_id = \"x\"\n",
    );
    let mut harness = app(Some(path), FakeDialogs::default());
    click(&mut harness, "Problems (1)");
    click(
        &mut harness,
        "device `d`: instance id `x` repeats; include {{ index }} in instance_id",
    );
    harness.run();
    assert!(harness.query_by_label("Device d").is_some());
    assert!(has_focus(&harness, "Instance IDs"));
}

#[test]
fn clicking_a_broker_problem_opens_the_settings_at_its_box() {
    let path = write_project("problem-focus-broker", "[broker]\nport = 0\n");
    let mut harness = app(Some(path), FakeDialogs::default());
    click(&mut harness, "Problems (1)");
    click(&mut harness, "broker: port must not be 0");
    harness.run();
    assert!(harness.query_by_label("Project settings").is_some());
    assert!(has_focus(&harness, "Port"));
}

const ONE_DEVICE: &str = "[[devices]]\nid = \"a\"\ntemplate = \"co2\"\nautostart = false\n";
const TWO_DEVICES_ON_DISK: &str = "[[devices]]\nid = \"a\"\ntemplate = \"co2\"\nautostart = false\n\n[[devices]]\nid = \"b\"\ntemplate = \"co2\"\nautostart = false\n";

/// Opens a one-device project, then rewrites its file with two devices and waits for the window
/// to ask about it.
fn changed_on_disk(name: &str) -> App {
    let path = write_project(name, ONE_DEVICE);
    let mut harness = app(Some(path.clone()), FakeDialogs::default());
    std::fs::write(&path, TWO_DEVICES_ON_DISK).unwrap();
    wait_until(&mut harness, "the change on disk noticed", |h| {
        h.query_by_label("project.toml changed on disk").is_some()
    });
    // A dialog sizes itself in its first frame and moves to the middle in the next.
    harness.run();
    harness
}

#[test]
fn a_project_changed_on_disk_can_be_reloaded() {
    let mut harness = changed_on_disk("disk-reload-gui");
    click(&mut harness, "Reload");
    assert_eq!(device_count(&harness), 2);
    assert!(!harness.state().document().is_dirty());
    assert!(
        harness
            .query_by_label("project.toml changed on disk")
            .is_none()
    );
}

#[test]
fn keeping_my_version_leaves_it_as_unsaved_changes() {
    let mut harness = changed_on_disk("disk-keep-gui");
    click(&mut harness, "Keep my version");
    assert_eq!(device_count(&harness), 1);
    assert!(harness.state().document().is_dirty());
}

/// Device `office` with `count` instances of the built-in `co2` template on the broker at `port`.
fn office_project(port: u16, count: u32, autostart: bool) -> String {
    format!(
        "[broker]\nhost = \"127.0.0.1\"\nport = {port}\n\n[[devices]]\nid = \"office\"\n\
         template = \"co2\"\ncount = {count}\nautostart = {autostart}\n"
    )
}

/// A broker that refuses every connection for a bad username or password. The runtime runs it
/// and must outlive the test.
fn refusing() -> (tokio::runtime::Runtime, u16) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let port = runtime.block_on(refusing_broker(ConnectReturnCode::BadUserNamePassword));
    (runtime, port)
}

/// A window on `path` with `settings`, whose notifications `notifier` records.
fn notifying_app(
    path: PathBuf,
    settings: senzor::settings::AppSettings,
    notifier: &RecordingNotifier,
) -> App {
    let notifier = notifier.clone();
    let mut harness = window_builder()
        .with_size(egui::vec2(1400.0, 900.0))
        .build_eframe(move |cc| {
            SenzorApp::with_settings(cc, Some(path), Box::new(FakeDialogs::default()), settings)
                .with_notifier(Box::new(notifier))
        });
    harness.step();
    harness
}

/// Gives the window the keyboard focus or takes it away, as switching to another window does.
fn set_focused(harness: &mut App, focused: bool) {
    harness
        .input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .focused = Some(focused);
}

/// Steps frames for longer than notifications gather failures, so any due is shown.
fn step_past_gathering(harness: &mut App) {
    let until = Instant::now() + senzor::notify::GATHER + Duration::from_millis(500);
    while Instant::now() < until {
        harness.step();
        std::thread::sleep(FRAME_PAUSE);
    }
}

fn failed(harness: &App, id: &str) -> bool {
    matches!(status(harness, id), InstanceStatus::Failed { .. })
}

const REFUSED: &str = "broker refused the connection: BadUserNamePassword";

#[test]
fn an_instance_failing_while_the_window_is_in_the_background_shows_a_notification() {
    let (_runtime, port) = refusing();
    let path = write_project("notify-one", &office_project(port, 1, false));
    let notifier = RecordingNotifier::default();
    let mut harness = notifying_app(path, Default::default(), &notifier);
    set_focused(&mut harness, false);
    click(&mut harness, "Start all");
    wait_until(&mut harness, "a notification", |_| {
        !notifier.shown().is_empty()
    });
    step_past_gathering(&mut harness);
    assert_eq!(
        notifier.shown(),
        [(
            "Senzor: an instance failed".to_string(),
            format!("office: {REFUSED}")
        )]
    );
}

#[test]
fn an_instance_failing_while_the_window_is_minimized_shows_a_notification() {
    let (_runtime, port) = refusing();
    let path = write_project("notify-minimized", &office_project(port, 1, false));
    let notifier = RecordingNotifier::default();
    let mut harness = notifying_app(path, Default::default(), &notifier);
    harness
        .input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .minimized = Some(true);
    click(&mut harness, "Start all");
    wait_until(&mut harness, "a notification", |_| {
        !notifier.shown().is_empty()
    });
    assert_eq!(notifier.shown()[0].0, "Senzor: an instance failed");
}

#[test]
fn instances_failing_together_show_one_notification() {
    let (_runtime, port) = refusing();
    let path = write_project("notify-two", &office_project(port, 2, false));
    let notifier = RecordingNotifier::default();
    let mut harness = notifying_app(path, Default::default(), &notifier);
    set_focused(&mut harness, false);
    click(&mut harness, "Start all");
    wait_until(&mut harness, "a notification", |_| {
        !notifier.shown().is_empty()
    });
    step_past_gathering(&mut harness);
    assert_eq!(
        notifier.shown(),
        [(
            "Senzor: 2 instances failed".to_string(),
            format!("office-0: {REFUSED}\noffice-1: {REFUSED}")
        )]
    );
}

#[test]
fn an_instance_failing_while_the_window_has_the_focus_shows_no_notification() {
    let (_runtime, port) = refusing();
    let path = write_project("notify-focused", &office_project(port, 1, false));
    let notifier = RecordingNotifier::default();
    let mut harness = notifying_app(path, Default::default(), &notifier);
    set_focused(&mut harness, true);
    click(&mut harness, "Start all");
    wait_until(&mut harness, "office failed", |h| failed(h, "office"));
    step_past_gathering(&mut harness);
    assert_eq!(notifier.shown(), []);
}

#[test]
fn a_failure_from_while_the_window_had_the_focus_is_not_told_once_it_goes_to_the_background() {
    let (_runtime, port) = refusing();
    let path = write_project("notify-before", &office_project(port, 1, false));
    let notifier = RecordingNotifier::default();
    let mut harness = notifying_app(path, Default::default(), &notifier);
    set_focused(&mut harness, true);
    click(&mut harness, "Start all");
    wait_until(&mut harness, "office failed", |h| failed(h, "office"));
    // A frame with the focus, which sees the failure: `wait_until` asks the engine directly.
    harness.step();
    set_focused(&mut harness, false);
    step_past_gathering(&mut harness);
    assert_eq!(notifier.shown(), []);
}

#[test]
fn turning_notifications_off_in_settings_keeps_failures_quiet_and_is_saved() {
    use senzor::settings::AppSettings;
    let (_runtime, port) = refusing();
    let path = write_project("notify-off", &office_project(port, 1, false));
    let notifier = RecordingNotifier::default();
    let mut harness = notifying_app(path, Default::default(), &notifier);
    click(&mut harness, "Settings");
    harness.run();
    click(&mut harness, "Notify me when an instance fails");
    harness.run();
    assert!(!harness.state().settings().notify_failures);
    let mut storage = MemoryStorage::default();
    eframe::App::save(harness.state_mut(), &mut storage);
    assert!(!AppSettings::load(Some(&storage)).notify_failures);

    set_focused(&mut harness, false);
    click(&mut harness, "Start all");
    wait_until(&mut harness, "office failed", |h| failed(h, "office"));
    step_past_gathering(&mut harness);
    assert_eq!(notifier.shown(), []);
}
