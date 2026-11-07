//! The app's own settings, kept between launches in eframe's storage (on Linux under
//! `~/.local/share/`). They are not part of any project file.

use std::path::{Path, PathBuf};

use eframe::egui;
use serde::{Deserialize, Serialize};

/// Light or dark colours, or whatever the operating system uses.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::System, Theme::Light, Theme::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Theme::System => "System",
            Theme::Light => "Light",
            Theme::Dark => "Dark",
        }
    }

    pub fn preference(self) -> egui::ThemePreference {
        match self {
            Theme::System => egui::ThemePreference::System,
            Theme::Light => egui::ThemePreference::Light,
            Theme::Dark => egui::ThemePreference::Dark,
        }
    }
}

/// The left panel's tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LeftTab {
    #[default]
    Devices,
    Templates,
}

/// The bottom panel's tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BottomTab {
    #[default]
    Log,
    Problems,
}

/// The tabs selected in the side panels when the window closed. (egui keeps the panels' sizes.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PanelTabs {
    pub left: LeftTab,
    pub bottom: BottomTab,
}

/// Everything the settings window changes. A field missing from the stored settings, for
/// example one added in a later version, takes its default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub theme: Theme,
    /// How large everything is drawn: 1.0 is 100 %.
    pub zoom: f32,
    /// Project files opened or saved lately, the latest first, at most [`AppSettings::RECENT`].
    pub recent: Vec<PathBuf>,
    /// The port MCP listens on when it is turned on.
    pub mcp_port: u16,
    /// MCP was on when the window closed, so the next window turns it on again.
    pub mcp_on: bool,
    /// The tabs selected when the window closed.
    pub tabs: PanelTabs,
    /// The version of Senzor that last ran with these settings, for telling what is new.
    pub last_version: Option<String>,
    /// Show a desktop notification when an instance fails while the window is in the background.
    pub notify_failures: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            zoom: 1.0,
            recent: Vec::new(),
            mcp_port: senzor_mcp::DEFAULT_HTTP_PORT,
            mcp_on: false,
            tabs: PanelTabs::default(),
            last_version: None,
            notify_failures: true,
        }
    }
}

impl AppSettings {
    const KEY: &str = "senzor-settings";
    /// How many recent projects the list keeps.
    pub const RECENT: usize = 10;

    /// Puts `path` first in the recent projects, as an absolute path when it can be resolved.
    pub fn remember(&mut self, path: &Path) {
        let path = resolved(path);
        self.recent.retain(|p| *p != path);
        self.recent.insert(0, path);
        self.recent.truncate(Self::RECENT);
    }

    /// Takes `path` off the recent projects.
    pub fn forget(&mut self, path: &Path) {
        let resolved = resolved(path);
        self.recent.retain(|p| p != path && *p != resolved);
    }

    /// The stored settings, or the defaults when there are none or they cannot be read.
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        storage
            .and_then(|storage| eframe::get_value(storage, Self::KEY))
            .unwrap_or_default()
    }

    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, Self::KEY, self);
    }

    /// Sets the theme and zoom of `ctx` to these settings.
    pub fn apply(&self, ctx: &egui::Context) {
        ctx.set_theme(self.theme.preference());
        ctx.set_zoom_factor(self.zoom);
    }
}

/// `path` with symbolic links and `.` or `..` resolved; for a file that does not exist, just
/// made absolute.
fn resolved(path: &Path) -> PathBuf {
    path.canonicalize()
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[derive(Default)]
    struct MemoryStorage(HashMap<String, String>);

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
    fn saved_settings_load_back() {
        let mut storage = MemoryStorage::default();
        let settings = AppSettings {
            theme: Theme::Light,
            zoom: 1.3,
            recent: vec![PathBuf::from("/projects/a.toml")],
            mcp_port: 18900,
            mcp_on: true,
            tabs: PanelTabs {
                left: LeftTab::Templates,
                bottom: BottomTab::Problems,
            },
            last_version: Some("1.0.0".into()),
            notify_failures: false,
        };
        settings.save(&mut storage);
        assert_eq!(AppSettings::load(Some(&storage)), settings);
    }

    #[test]
    fn without_stored_settings_the_theme_follows_the_system() {
        assert_eq!(AppSettings::load(None).theme, Theme::System);
        assert_eq!(
            AppSettings::load(Some(&MemoryStorage::default())),
            AppSettings::default()
        );
    }

    #[test]
    fn unreadable_settings_fall_back_to_the_defaults() {
        let mut storage = MemoryStorage::default();
        storage
            .0
            .insert(AppSettings::KEY.into(), "not ron at all".into());
        assert_eq!(AppSettings::load(Some(&storage)), AppSettings::default());
    }

    #[test]
    fn settings_saved_before_zoom_existed_load_at_100_percent() {
        let mut storage = MemoryStorage::default();
        storage
            .0
            .insert(AppSettings::KEY.into(), "(theme:Dark)".into());
        let settings = AppSettings::load(Some(&storage));
        assert_eq!(settings.theme, Theme::Dark);
        assert_eq!(settings.zoom, 1.0);
        assert_eq!(AppSettings::default().zoom, 1.0);
        assert_eq!(settings.mcp_port, 18830, "MCP's usual port");
        assert!(!settings.mcp_on, "MCP stays off until turned on");
        assert!(settings.notify_failures, "failures are told about");
    }

    #[test]
    fn applying_settings_sets_the_theme_and_the_zoom() {
        let ctx = egui::Context::default();
        AppSettings {
            theme: Theme::Light,
            zoom: 1.5,
            ..AppSettings::default()
        }
        .apply(&ctx);
        // egui changes the zoom when the next frame begins.
        let mut output = ctx.run_ui(egui::RawInput::default(), |_| {});
        output.textures_delta.clear();
        assert_eq!(ctx.zoom_factor(), 1.5);
        assert_eq!(
            ctx.options(|o| o.theme_preference),
            egui::ThemePreference::Light
        );
    }

    #[test]
    fn a_remembered_project_goes_first_and_appears_once() {
        let mut settings = AppSettings::default();
        for name in ["a", "b", "a"] {
            settings.remember(Path::new(&format!("/projects/{name}.toml")));
        }
        assert_eq!(
            settings.recent,
            [Path::new("/projects/a.toml"), Path::new("/projects/b.toml")]
        );
    }

    #[test]
    fn the_recent_list_keeps_the_latest_ten() {
        let mut settings = AppSettings::default();
        for n in 0..12 {
            settings.remember(Path::new(&format!("/projects/{n}.toml")));
        }
        assert_eq!(settings.recent.len(), AppSettings::RECENT);
        assert_eq!(settings.recent[0], Path::new("/projects/11.toml"));
        assert_eq!(settings.recent[9], Path::new("/projects/2.toml"));
    }

    #[test]
    fn a_relative_path_is_remembered_as_an_absolute_one() {
        let dir = std::env::temp_dir().join(format!("senzor-recent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("p.toml");
        std::fs::write(&file, "").unwrap();
        let mut settings = AppSettings::default();
        settings.remember(&file);
        settings.remember(&dir.join(".").join("p.toml"));
        assert_eq!(settings.recent, [file.canonicalize().unwrap()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_forgotten_project_leaves_the_list() {
        let mut settings = AppSettings::default();
        settings.remember(Path::new("/projects/a.toml"));
        settings.remember(Path::new("/projects/b.toml"));
        settings.forget(Path::new("/projects/a.toml"));
        assert_eq!(settings.recent, [Path::new("/projects/b.toml")]);
    }
}
