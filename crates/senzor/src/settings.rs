//! The app's own settings, kept between launches in eframe's storage (on Linux under
//! `~/.local/share/`). They are not part of any project file.

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

/// Everything the settings window changes. A field missing from the stored settings, for
/// example one added in a later version, takes its default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub theme: Theme,
    /// How large everything is drawn: 1.0 is 100 %.
    pub zoom: f32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            zoom: 1.0,
        }
    }
}

impl AppSettings {
    const KEY: &str = "senzor-settings";

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
    }

    #[test]
    fn applying_settings_sets_the_theme_and_the_zoom() {
        let ctx = egui::Context::default();
        AppSettings {
            theme: Theme::Light,
            zoom: 1.5,
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
}
