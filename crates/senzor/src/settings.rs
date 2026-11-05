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
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub theme: Theme,
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
}
