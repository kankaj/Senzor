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
    Dismissed,
}

/// Shows `modal`; returns what the user chose, if anything. Edits the modal's own text in place.
pub fn show(ctx: &egui::Context, modal: &mut Modal) -> Option<Outcome> {
    let mut outcome = None;
    let response = egui::Modal::new(egui::Id::new("senzor-modal")).show(ctx, |ui| {
        ui.set_max_width(480.0);
        match modal {
            Modal::Unsaved(pending) => {
                ui.heading("Unsaved changes");
                let what = match pending {
                    Pending::New => "starting a new project",
                    Pending::Open => "opening another project",
                    Pending::Close => "closing",
                };
                ui.label(format!("Save the changes to this project before {what}?"));
                ui.horizontal(|ui| {
                    for (label, choice) in [
                        ("Save", UnsavedChoice::Save),
                        ("Discard", UnsavedChoice::Discard),
                        ("Cancel", UnsavedChoice::Cancel),
                    ] {
                        if ui.button(label).clicked() {
                            outcome = Some(Outcome::Unsaved(*pending, choice));
                        }
                    }
                });
            }
            Modal::OpenFailed { path, message } => {
                ui.heading(format!("Cannot open {path}"));
                ui.label("Fix the file in a text editor, then open it again.");
                ui.label(RichText::new(message.as_str()).monospace().color(ERROR));
                if ui.button("OK").clicked() {
                    outcome = Some(Outcome::Dismissed);
                }
            }
            Modal::CopyBuiltin {
                builtin,
                new_id,
                error,
            } => {
                ui.heading(format!("Copy built-in template {builtin}"));
                let label = ui.label("New template ID");
                ui.text_edit_singleline(new_id).labelled_by(label.id);
                if let Some(error) = error {
                    ui.colored_label(ERROR, error.as_str());
                }
                ui.horizontal(|ui| {
                    if ui.button("Copy").clicked() {
                        outcome = Some(Outcome::CopyBuiltin {
                            builtin: builtin.clone(),
                            new_id: new_id.clone(),
                        });
                    }
                    if ui.button("Cancel").clicked() {
                        outcome = Some(Outcome::Dismissed);
                    }
                });
            }
            Modal::ConfirmReplace { path, then } => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let folder = path.parent().unwrap_or(Path::new("")).display();
                ui.heading(format!("Replace {name}?"));
                ui.label(format!(
                    "A file with that name already exists in {folder}. \
                     Replacing it overwrites its contents."
                ));
                ui.horizontal(|ui| {
                    if ui.button("Replace").clicked() {
                        outcome = Some(Outcome::Replace {
                            path: path.clone(),
                            then: *then,
                        });
                    }
                    if ui.button("Cancel").clicked() {
                        outcome = Some(Outcome::Dismissed);
                    }
                });
            }
            Modal::Error(message) => {
                ui.heading("Something went wrong");
                ui.label(message.as_str());
                if ui.button("OK").clicked() {
                    outcome = Some(Outcome::Dismissed);
                }
            }
        }
    });
    if outcome.is_none() && response.should_close() {
        outcome = Some(match modal {
            Modal::Unsaved(pending) => Outcome::Unsaved(*pending, UnsavedChoice::Cancel),
            _ => Outcome::Dismissed,
        });
    }
    outcome
}
