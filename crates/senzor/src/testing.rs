//! Helpers for driving the whole window headlessly with egui_kittest, shared by the unit tests
//! and `tests/gui.rs` (which gets them through the `test-support` feature).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use senzor_core::engine::InstanceStatus;

use crate::{FileDialogs, SenzorApp};

/// File dialogs with fixed answers; by default both are cancelled.
#[derive(Default, Clone)]
pub struct FakeDialogs {
    open: Option<PathBuf>,
    save: Option<PathBuf>,
}

impl FakeDialogs {
    /// Dialogs whose Open answers `path` and whose Save is cancelled.
    pub fn opening(path: PathBuf) -> Self {
        Self {
            open: Some(path),
            save: None,
        }
    }

    /// Dialogs whose Save answers `path` and whose Open is cancelled.
    pub fn saving(path: PathBuf) -> Self {
        Self {
            open: None,
            save: Some(path),
        }
    }
}

impl FileDialogs for FakeDialogs {
    fn open_project(&mut self) -> Option<PathBuf> {
        self.open.clone()
    }

    fn save_project(&mut self, _suggested_name: &str) -> Option<PathBuf> {
        self.save.clone()
    }
}

/// The Senzor window in a headless harness.
pub type App = Harness<'static, SenzorApp>;

/// Pause between frames while waiting for the engine.
pub const FRAME_PAUSE: Duration = Duration::from_millis(20);
/// How long to wait for the engine before failing.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// A folder of its own for the test `name`, created if missing.
pub fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("senzor-gui-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Writes `content` as `project.toml` into the folder of the test `name`.
pub fn write_project(name: &str, content: &str) -> PathBuf {
    let path = test_dir(name).join("project.toml");
    std::fs::write(&path, content).unwrap();
    path
}

/// The engine's status of instance `id`.
pub fn status(harness: &App, id: &str) -> InstanceStatus {
    harness.state().engine_state().instances[id].status.clone()
}

/// Steps frames until `pred` holds, failing after [`TIMEOUT`].
pub fn wait_until(harness: &mut App, what: &str, pred: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !pred(harness) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        harness.step();
        std::thread::sleep(FRAME_PAUSE);
    }
}

/// Steps frames until instance `id` is online.
pub fn wait_online(harness: &mut App, id: &str) {
    wait_until(harness, &format!("{id} online"), |a| {
        status(a, id) == InstanceStatus::Online
    });
}

/// Clicks the widget labelled `label` and runs the frames that act on the click.
pub fn click(harness: &mut App, label: &str) {
    harness.get_by_label(label).click();
    harness.run_steps(2);
}
