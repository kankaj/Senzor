use std::path::PathBuf;

use senzor_core::project::edit::PROJECT_EXTENSION;

use crate::i18n::t;
/// Where to save a project the user named in the save dialog (Linux portals return the name as
/// typed, so `.toml` may be missing).
pub use senzor_core::project::edit::project_file_path;

/// Native file dialogs, behind a trait so tests can answer them.
pub trait FileDialogs {
    /// The project file to open, or `None` if the user cancelled.
    fn open_project(&mut self) -> Option<PathBuf>;
    /// Where to save the project, or `None` if the user cancelled.
    fn save_project(&mut self, suggested_name: &str) -> Option<PathBuf>;
    /// Where to save the exported messages, or `None` if the user cancelled.
    fn save_log(&mut self, suggested_name: &str) -> Option<PathBuf>;
}

fn project_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter(t!("dialogs-project-filter"), &[PROJECT_EXTENSION])
}

/// The operating system's dialogs (xdg-desktop-portal on Linux).
pub struct NativeDialogs;

impl FileDialogs for NativeDialogs {
    fn open_project(&mut self) -> Option<PathBuf> {
        project_dialog().pick_file()
    }

    fn save_project(&mut self, suggested_name: &str) -> Option<PathBuf> {
        project_dialog().set_file_name(suggested_name).save_file()
    }

    fn save_log(&mut self, suggested_name: &str) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .add_filter("CSV", &["csv"])
            .set_file_name(suggested_name)
            .save_file()
    }
}
