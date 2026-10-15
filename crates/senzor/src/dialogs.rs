use std::path::PathBuf;

/// The extension of project files, which the dialogs filter on.
const PROJECT_EXTENSION: &str = "toml";

/// Native file dialogs, behind a trait so tests can answer them.
pub trait FileDialogs {
    /// The project file to open, or `None` if the user cancelled.
    fn open_project(&mut self) -> Option<PathBuf>;
    /// Where to save the project, or `None` if the user cancelled.
    fn save_project(&mut self, suggested_name: &str) -> Option<PathBuf>;
}

/// Where to save a project the user named in the save dialog. A name that does not end in
/// `.toml` (in any case) gets `.toml` appended (Linux portals return the name as typed), so the
/// file shows up in the Open dialog's `*.toml` filter: `notes.txt` is saved as `notes.txt.toml`.
pub fn project_file_path(chosen: PathBuf) -> PathBuf {
    if chosen
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(PROJECT_EXTENSION))
    {
        return chosen;
    }
    let mut name = chosen.into_os_string();
    name.push(".");
    name.push(PROJECT_EXTENSION);
    PathBuf::from(name)
}

fn project_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("Senzor project", &[PROJECT_EXTENSION])
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_without_an_extension_is_saved_as_toml() {
        assert_eq!(
            project_file_path(PathBuf::from("/home/me/myproject")),
            PathBuf::from("/home/me/myproject.toml")
        );
    }

    #[test]
    fn a_toml_name_is_kept_in_any_case() {
        for name in ["/home/me/a.toml", "/home/me/a.TOML", "/home/me/a.b.Toml"] {
            assert_eq!(project_file_path(PathBuf::from(name)), PathBuf::from(name));
        }
    }

    #[test]
    fn any_other_extension_gets_toml_appended() {
        for (name, saved) in [
            ("/home/me/a.txt", "/home/me/a.txt.toml"),
            ("/home/me/a.b.c", "/home/me/a.b.c.toml"),
        ] {
            assert_eq!(project_file_path(PathBuf::from(name)), PathBuf::from(saved));
        }
    }
}
