use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};

use super::{ERROR, Modal, Pending};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsavedChoice {
    Save,
    Discard,
    Cancel,
}

/// What the user did in the open modal this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Unsaved(Pending, UnsavedChoice),
    CopyBuiltin {
        builtin: String,
        new_id: String,
    },
    /// Save over `path`, then continue with `then`.
    Replace {
        path: PathBuf,
        then: Option<Pending>,
    },
    /// Take the project the file now holds.
    Reload,
    /// Keep the project in the window over the one in the file.
    KeepMine,
    Dismissed,
}

/// Shows `modal`; returns what the user chose, if anything. Edits the modal's own text in place.
pub fn show(ctx: &egui::Context, modal: &mut Modal) -> Option<Outcome> {
    let mut outcome = None;
    let response = egui::Modal::new(egui::Id::new("senzor-modal")).show(ctx, |ui| {
        ui.set_max_width(480.0);
        outcome = match modal {
            Modal::Unsaved(pending) => unsaved(ui, pending),
            Modal::OpenFailed { path, message } => open_failed(ui, path, message),
            Modal::CopyBuiltin {
                builtin,
                new_id,
                error,
            } => copy_builtin(ui, builtin, new_id, error.as_deref()),
            Modal::ConfirmReplace { path, then } => confirm_replace(ui, path, then.as_ref()),
            Modal::Error(message) => error(ui, message),
            Modal::FileChanged { name, unsaved } => file_changed(ui, name, *unsaved),
        };
    });
    if outcome.is_none() && response.should_close() {
        outcome = Some(match modal {
            Modal::Unsaved(pending) => Outcome::Unsaved(pending.clone(), UnsavedChoice::Cancel),
            // Losing nothing: Save can still write this version over the file.
            Modal::FileChanged { .. } => Outcome::KeepMine,
            _ => Outcome::Dismissed,
        });
    }
    outcome
}

/// A row of buttons; returns the outcome of the one clicked.
fn buttons(
    ui: &mut egui::Ui,
    choices: impl IntoIterator<Item = (&'static str, Outcome)>,
) -> Option<Outcome> {
    let mut clicked = None;
    ui.horizontal(|ui| {
        for (label, outcome) in choices {
            if ui.button(label).clicked() {
                clicked = Some(outcome);
            }
        }
    });
    clicked
}

fn ok(ui: &mut egui::Ui) -> Option<Outcome> {
    ui.button("OK").clicked().then_some(Outcome::Dismissed)
}

fn unsaved(ui: &mut egui::Ui, pending: &Pending) -> Option<Outcome> {
    ui.heading("Unsaved changes");
    let what = match pending {
        Pending::New => "starting a new project",
        Pending::Open | Pending::OpenPath(_) => "opening another project",
        Pending::OpenExample(_) => "opening the example",
        Pending::Close => "closing",
    };
    ui.label(format!("Save the changes to this project before {what}?"));
    buttons(
        ui,
        [
            ("Save", UnsavedChoice::Save),
            ("Discard", UnsavedChoice::Discard),
            ("Cancel", UnsavedChoice::Cancel),
        ]
        .map(|(label, choice)| (label, Outcome::Unsaved(pending.clone(), choice))),
    )
}

fn open_failed(ui: &mut egui::Ui, path: &str, message: &str) -> Option<Outcome> {
    ui.heading(format!("Cannot open {path}"));
    ui.label("Fix the file in a text editor, then open it again.");
    ui.label(RichText::new(message).monospace().color(ERROR));
    ok(ui)
}

fn copy_builtin(
    ui: &mut egui::Ui,
    builtin: &str,
    new_id: &mut String,
    error: Option<&str>,
) -> Option<Outcome> {
    ui.heading(format!("Copy built-in template {builtin}"));
    let label = ui.label("New template ID");
    ui.text_edit_singleline(new_id).labelled_by(label.id);
    if let Some(error) = error {
        ui.colored_label(ERROR, error);
    }
    buttons(
        ui,
        [
            (
                "Copy",
                Outcome::CopyBuiltin {
                    builtin: builtin.to_string(),
                    new_id: new_id.clone(),
                },
            ),
            ("Cancel", Outcome::Dismissed),
        ],
    )
}

fn confirm_replace(ui: &mut egui::Ui, path: &Path, then: Option<&Pending>) -> Option<Outcome> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let folder = path.parent().unwrap_or(Path::new("")).display();
    ui.heading(format!("Replace {name}?"));
    ui.label(format!(
        "A file with that name already exists in {folder}. \
         Replacing it overwrites its contents."
    ));
    buttons(
        ui,
        [
            (
                "Replace",
                Outcome::Replace {
                    path: path.to_path_buf(),
                    then: then.cloned(),
                },
            ),
            ("Cancel", Outcome::Dismissed),
        ],
    )
}

fn error(ui: &mut egui::Ui, message: &str) -> Option<Outcome> {
    ui.heading("Something went wrong");
    ui.label(message);
    ok(ui)
}

fn file_changed(ui: &mut egui::Ui, name: &str, unsaved: bool) -> Option<Outcome> {
    ui.heading(format!("{name} changed on disk"));
    ui.label(if unsaved {
        "Another program changed the file. Reload it to see its version; your \
         unsaved changes are replaced, and Undo brings them back."
    } else {
        "Another program changed the file. Reload it to see its version."
    });
    buttons(
        ui,
        [
            ("Reload", Outcome::Reload),
            ("Keep my version", Outcome::KeepMine),
        ],
    )
}
