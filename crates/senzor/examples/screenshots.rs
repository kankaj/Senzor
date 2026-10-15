//! Renders the documentation's GUI screenshots from the real window, headlessly:
//! `cargo run -p senzor --example screenshots --features screenshots [-- <output dir>]`.
//! It opens a copy of `examples/showcase.toml` on an MQTT broker at localhost:1883 (one already
//! running there, else one it starts in-process) and drives the GUI.
//!
//! When a broker already answers on localhost:1883 the program uses it, publishes the showcase
//! devices there and leaves their retained `offline` availability messages behind. Stop that
//! broker first (or use a throwaway one) if that matters.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use eframe::egui;
use eframe::egui_wgpu::{RenderState, RendererOptions, WgpuConfiguration};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use egui_kittest::wgpu::{WgpuTestRenderer, default_wgpu_setup};
use image::RgbaImage;
use senzor::SenzorApp;
use senzor::testing::{App, FRAME_PAUSE, FakeDialogs, click, wait_online};

const SIZE: egui::Vec2 = egui::vec2(1400.0, 900.0);

/// The broker port `examples/showcase.toml` names, and so the one the shots should show.
const SHOWCASE_PORT: u16 = 1883;

/// How long the devices publish before the first shot, so values, counters and the log fill up.
const WARM_UP: Duration = Duration::from_secs(5);
/// How long the devices publish after the pin, so the log shows the pinned value.
const PINNED_RUN: Duration = Duration::from_secs(3);
/// How long the devices publish before the log shot, so the taller log fills up.
const LOG_RUN: Duration = Duration::from_secs(2);
/// Frames run before each render: enough for hover highlights to fade and a new window to lay
/// itself out.
const SETTLE_FRAMES: usize = 10;

/// The value the `instance-pin` shot pins the field to.
const PINNED_VALUE: &str = "1800";
/// The typo (a missing `}`) the `problems` shot appends to the template's state topic.
const TYPO: &str = "/{{ index }";
/// How far (in points) a field's Pin button may sit above or below the middle of its pin box.
const PIN_ROW_TOLERANCE: f32 = 5.0;
/// The bottom panel's resize edge lies this far (in points) above the top of its tabs.
const PANEL_EDGE_ABOVE_TABS: f32 = 3.0;
/// How much (in points) the `message-log` shot drags the bottom panel taller.
const LOG_PANEL_GROWTH: f32 = 300.0;

/// Every GUI label this program finds a widget by. If the GUI renames one, the program stops at
/// the shot that uses it.
mod label {
    /// The instance the shots select in the device tree (also its ID).
    pub const INSTANCE: &str = "office-0";
    /// The field whose pin box the `instance-pin` shot fills, in the instance view's table.
    pub const PINNED_FIELD: &str = "co2";
    /// The instance view's button that pins a field; every field row has one.
    pub const PIN: &str = "Pin";
    /// The left panel's tabs.
    pub const DEVICES_TAB: &str = "Devices";
    pub const TEMPLATES_TAB: &str = "Templates";
    /// The template the editor shots open, in the Templates tab (also its ID).
    pub const TEMPLATE: &str = "office-air";
    /// The device the `device-editor` shot opens, as the device tree shows it.
    pub const DEVICE: &str = "office ×5";
    /// The template editor's box for a message's topic (the availability topic's label differs).
    pub const MESSAGE_TOPIC: &str = "Topic";
    /// The bottom panel's tabs; the Problems tab adds ` (<count>)` while there are problems.
    pub const LOG_TAB: &str = "Message log";
    pub const PROBLEMS_TAB: &str = "Problems";
    /// The top bar's broker button, followed by `<host>:<port>`.
    pub const BROKER_BUTTON: &str = "Broker: ";
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rendering failed (no usable wgpu adapter?): {error}");
            ExitCode::FAILURE
        }
    }
}

/// Takes every shot and writes them; fails with why nothing could be rendered. Returning, like
/// a panic, drops the project copy, which deletes it.
fn run() -> Result<(), String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = std::env::args()
        .nth(1)
        .map_or_else(|| root.join("website/public/screenshots"), PathBuf::from);
    let renderer = wgpu_renderer()?;
    let project = ProjectCopy::of_showcase(&root, broker_port());
    let path = project.path.clone();
    let mut app: App = Harness::builder()
        .with_size(SIZE)
        .renderer(renderer)
        .build_eframe(move |cc| SenzorApp::new(cc, Some(path), Box::new(FakeDialogs::default())));
    app.step();
    wait_online(&mut app, label::INSTANCE);
    run_for(&mut app, WARM_UP);

    let shots = take_shots(&mut app)?;
    std::fs::create_dir_all(&out).expect("create output folder");
    for (name, image) in shots {
        let path = out.join(format!("{name}.png"));
        image
            .save(&path)
            .unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
        println!("wrote {}", path.display());
    }
    Ok(())
}

/// The renderer `HarnessBuilder::wgpu` would build; that one panics without a GPU adapter,
/// this one reports it.
fn wgpu_renderer() -> Result<WgpuTestRenderer, String> {
    let setup = default_wgpu_setup();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|e| e.to_string())?;
    let render_state = runtime
        .block_on(async {
            let instance = setup.new_instance().await;
            let config = WgpuConfiguration {
                wgpu_setup: setup,
                ..Default::default()
            };
            RenderState::create(&config, &instance, None, RendererOptions::PREDICTABLE).await
        })
        .map_err(|e| e.to_string())?;
    Ok(WgpuTestRenderer::from_render_state(render_state))
}

/// The port of the broker the showcase runs on: one already answering on [`SHOWCASE_PORT`],
/// else a test broker started there, else a test broker on a free port (the shots then show
/// that port).
fn broker_port() -> u16 {
    if senzor_testkit::broker_answers(SHOWCASE_PORT) {
        println!("using the MQTT broker already running on localhost:{SHOWCASE_PORT}");
        return SHOWCASE_PORT;
    }
    match senzor_testkit::start_broker_on(SHOWCASE_PORT) {
        Ok(()) => {
            println!("started a test broker on localhost:{SHOWCASE_PORT}");
            SHOWCASE_PORT
        }
        Err(error) => {
            let port = senzor_testkit::start_broker();
            eprintln!(
                "warning: no broker usable on port {SHOWCASE_PORT} ({error}); started one on \
                 port {port}, so the screenshots show localhost:{port}"
            );
            port
        }
    }
}

/// A copy of `examples/showcase.toml` in a temporary folder of its own, deleted on drop.
struct ProjectCopy {
    dir: PathBuf,
    path: PathBuf,
}

impl ProjectCopy {
    /// The showcase, pointed at the broker on `port`.
    fn of_showcase(root: &Path, port: u16) -> Self {
        let showcase = std::fs::read_to_string(root.join("examples/showcase.toml"))
            .expect("read examples/showcase.toml");
        let named = format!("port = {SHOWCASE_PORT}");
        assert!(
            showcase.contains(&named),
            "examples/showcase.toml no longer sets `{named}`"
        );
        let showcase = showcase.replace(&named, &format!("port = {port}"));
        let dir = std::env::temp_dir().join(format!("senzor-screenshots-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create a temporary folder");
        let path = dir.join("showcase.toml");
        let copy = Self { dir, path };
        std::fs::write(&copy.path, showcase).expect("write the showcase project");
        copy
    }
}

impl Drop for ProjectCopy {
    fn drop(&mut self) {
        // Best effort: it is only a scratch copy.
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Runs frames for `duration` of real time, so the devices publish meanwhile.
fn run_for(app: &mut App, duration: Duration) {
    let until = Instant::now() + duration;
    while Instant::now() < until {
        app.step();
        std::thread::sleep(FRAME_PAUSE);
    }
}

/// The window as it is now, without the mouse pointer or what it hovers.
fn render(app: &mut App) -> Result<RgbaImage, String> {
    app.remove_cursor();
    app.run_steps(SETTLE_FRAMES);
    app.render()
}

/// Every shot, in order, rendered into memory: a failed render returns before any file is
/// written.
fn take_shots(app: &mut App) -> Result<Vec<(&'static str, RgbaImage)>, String> {
    Ok(vec![
        ("window", window(app)?),
        ("instance-pin", instance_pin(app)?),
        ("template-editor", template_editor(app)?),
        ("device-editor", device_editor(app)?),
        ("problems", problems(app)?),
        ("message-log", message_log(app)?),
        ("broker", broker(app)?),
    ])
}

/// `window`: the whole window with instance office-0 selected in the device tree, its values in
/// the centre and the live log below.
fn window(app: &mut App) -> Result<RgbaImage, String> {
    click(app, label::INSTANCE);
    render(app)
}

/// `instance-pin`: office-0's co2 pinned ("1800 (pinned)" and an Unpin button), and the log
/// showing office-0 publishing the pinned value.
fn instance_pin(app: &mut App) -> Result<RgbaImage, String> {
    let input = app.get_by_label(label::PINNED_FIELD);
    input.focus();
    input.type_text(PINNED_VALUE);
    let row = input.rect().center().y;
    app.run_steps(2);
    app.get_all_by_label(label::PIN)
        .find(|pin| (pin.rect().center().y - row).abs() < PIN_ROW_TOLERANCE)
        .expect("the field has a Pin button")
        .click();
    app.run_steps(2);
    run_for(app, PINNED_RUN);
    render(app)
}

/// `template-editor`: the Templates tab with office-air open in the template editor.
fn template_editor(app: &mut App) -> Result<RgbaImage, String> {
    open_template(app);
    render(app)
}

/// `device-editor`: the Devices tab with device office (×5) open in the device editor.
fn device_editor(app: &mut App) -> Result<RgbaImage, String> {
    click(app, label::DEVICES_TAB);
    click(app, label::DEVICE);
    render(app)
}

/// `problems`: a typo in office-air's state topic, in red in the template editor and listed on
/// the Problems tab. The typo is reverted afterwards, leaving the project as opened.
fn problems(app: &mut App) -> Result<RgbaImage, String> {
    open_template(app);
    let original = state_topic(app);
    let topic = app.get_by_label(label::MESSAGE_TOPIC);
    topic.focus();
    topic.type_text(TYPO);
    app.run_steps(2);
    assert_eq!(state_topic(app), format!("{original}{TYPO}"), "typo typed");
    let count = app.state().document().validated().errors.len();
    click(app, &format!("{} ({count})", label::PROBLEMS_TAB));
    let shot = render(app)?;

    app.get_by_label(label::MESSAGE_TOPIC).focus();
    app.run_steps(2);
    app.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    app.event(egui::Event::Text(original.clone()));
    app.run_steps(2);
    assert_eq!(state_topic(app), original, "typo reverted");
    assert!(!app.state().document().is_dirty(), "project back as opened");
    Ok(shot)
}

/// `message-log`: the Message log tab, dragged taller and full of the newest messages, with
/// office-0 in the centre.
fn message_log(app: &mut App) -> Result<RgbaImage, String> {
    click(app, label::LOG_TAB);
    click(app, label::DEVICES_TAB);
    click(app, label::INSTANCE);
    grow_bottom_panel(app);
    run_for(app, LOG_RUN);
    render(app)
}

/// `broker`: the Broker dialog open over the window, office-0 still in the centre.
fn broker(app: &mut App) -> Result<RgbaImage, String> {
    let broker = &app.state().document().file().broker;
    let button = format!("{}{}:{}", label::BROKER_BUTTON, broker.host, broker.port);
    click(app, &button);
    render(app)
}

/// Selects [`label::TEMPLATE`] in the Templates tab.
fn open_template(app: &mut App) {
    click(app, label::TEMPLATES_TAB);
    click(app, label::TEMPLATE);
}

/// The state topic of [`label::TEMPLATE`].
fn state_topic(app: &App) -> String {
    let file = app.state().document().file();
    let template = file.templates.iter().find(|t| t.id == label::TEMPLATE);
    template.expect("the showcase has the template").messages[0]
        .topic
        .clone()
}

/// Drags the bottom panel's top edge up by [`LOG_PANEL_GROWTH`], as a user resizing it does.
fn grow_bottom_panel(app: &mut App) {
    let before = bottom_tabs_top(app);
    let edge = egui::pos2(SIZE.x / 2.0, before - PANEL_EDGE_ABOVE_TABS);
    let target = edge - egui::vec2(0.0, LOG_PANEL_GROWTH);
    app.hover_at(edge);
    app.drag_at(edge);
    app.hover_at(target);
    app.drop_at(target);
    app.run_steps(2);
    let moved = before - bottom_tabs_top(app);
    assert!(
        (moved - LOG_PANEL_GROWTH).abs() < 2.0,
        "the bottom panel grew by {moved} instead of {LOG_PANEL_GROWTH}"
    );
}

/// Where the bottom panel's tabs start: they move with the panel's top edge.
fn bottom_tabs_top(app: &App) -> f32 {
    app.get_by_label(label::LOG_TAB).rect().top()
}
