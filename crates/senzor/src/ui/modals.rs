use std::path::{Path, PathBuf};

use eframe::egui::{self, RichText};

use super::{ERROR, Modal, Pending};
use crate::i18n::t;

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
    /// Clear these retained topics.
    ClearRetained(Vec<String>),
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
            Modal::ClearRetained { topics, broker } => clear_retained(ui, topics, broker),
            Modal::WhatsNew(releases) => whats_new(ui, releases),
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
    choices: impl IntoIterator<Item = (String, Outcome)>,
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
    ui.button(t!("modals-ok"))
        .clicked()
        .then_some(Outcome::Dismissed)
}

fn unsaved(ui: &mut egui::Ui, pending: &Pending) -> Option<Outcome> {
    ui.heading(t!("modals-unsaved-title"));
    ui.label(match pending {
        Pending::New => t!("modals-unsaved-new"),
        Pending::Open | Pending::OpenPath(_) => t!("modals-unsaved-open"),
        Pending::OpenExample(_) => t!("modals-unsaved-example"),
        Pending::Close => t!("modals-unsaved-close"),
    });
    buttons(
        ui,
        [
            (t!("modals-save"), UnsavedChoice::Save),
            (t!("modals-discard"), UnsavedChoice::Discard),
            (t!("modals-cancel"), UnsavedChoice::Cancel),
        ]
        .map(|(label, choice)| (label, Outcome::Unsaved(pending.clone(), choice))),
    )
}

fn open_failed(ui: &mut egui::Ui, path: &str, message: &str) -> Option<Outcome> {
    ui.heading(t!("modals-open-failed-title", path = path));
    ui.label(t!("modals-open-failed-text"));
    ui.label(RichText::new(message).monospace().color(ERROR));
    ok(ui)
}

fn copy_builtin(
    ui: &mut egui::Ui,
    builtin: &str,
    new_id: &mut String,
    error: Option<&str>,
) -> Option<Outcome> {
    ui.heading(t!("modals-copy-builtin-title", builtin = builtin));
    let label = ui.label(t!("modals-new-template-id"));
    ui.text_edit_singleline(new_id).labelled_by(label.id);
    if let Some(error) = error {
        ui.colored_label(ERROR, error);
    }
    buttons(
        ui,
        [
            (
                t!("modals-copy"),
                Outcome::CopyBuiltin {
                    builtin: builtin.to_string(),
                    new_id: new_id.clone(),
                },
            ),
            (t!("modals-cancel"), Outcome::Dismissed),
        ],
    )
}

fn confirm_replace(ui: &mut egui::Ui, path: &Path, then: Option<&Pending>) -> Option<Outcome> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let folder = path.parent().unwrap_or(Path::new("")).display();
    ui.heading(t!("modals-replace-title", name = name.as_ref()));
    ui.label(t!("modals-replace-text", folder = folder.to_string()));
    buttons(
        ui,
        [
            (
                t!("modals-replace"),
                Outcome::Replace {
                    path: path.to_path_buf(),
                    then: then.cloned(),
                },
            ),
            (t!("modals-cancel"), Outcome::Dismissed),
        ],
    )
}

fn clear_retained(ui: &mut egui::Ui, topics: &[String], broker: &str) -> Option<Outcome> {
    ui.heading(t!("modals-clear-title", count = topics.len()));
    ui.label(t!("modals-clear-text", broker = broker));
    egui::ScrollArea::vertical()
        .max_height(200.0)
        .show(ui, |ui| {
            for topic in topics {
                ui.label(RichText::new(topic).monospace());
            }
        });
    buttons(
        ui,
        [
            (t!("modals-clear"), Outcome::ClearRetained(topics.to_vec())),
            (t!("modals-cancel"), Outcome::Dismissed),
        ],
    )
}

/// The Highlights of each release, the newest first; the version is named when there are several.
/// The Highlights come from the changelog, which is in English.
fn whats_new(ui: &mut egui::Ui, releases: &[crate::whats_new::Release]) -> Option<Outcome> {
    ui.heading(t!(
        "modals-whats-new-title",
        version = env!("CARGO_PKG_VERSION")
    ));
    egui::ScrollArea::vertical()
        .max_height(360.0)
        .show(ui, |ui| {
            for release in releases {
                if releases.len() > 1 {
                    ui.add_space(4.0);
                    ui.strong(&release.version);
                }
                for highlight in &release.highlights {
                    ui.horizontal_top(|ui| {
                        ui.label("•");
                        // A row does not wrap its labels unless told to.
                        ui.add(egui::Label::new(highlight).wrap());
                    });
                }
            }
        });
    ui.add_space(4.0);
    ui.hyperlink_to(
        t!("modals-full-changelog"),
        format!("{}/blob/main/CHANGELOG.md", super::SOURCE_URL),
    );
    ok(ui)
}

fn error(ui: &mut egui::Ui, message: &str) -> Option<Outcome> {
    ui.heading(t!("modals-error-title"));
    ui.label(message);
    ok(ui)
}

fn file_changed(ui: &mut egui::Ui, name: &str, unsaved: bool) -> Option<Outcome> {
    ui.heading(t!("modals-file-changed-title", name = name));
    ui.label(if unsaved {
        t!("modals-file-changed-unsaved")
    } else {
        t!("modals-file-changed-text")
    });
    buttons(
        ui,
        [
            (t!("modals-reload"), Outcome::Reload),
            (t!("modals-keep-mine"), Outcome::KeepMine),
        ],
    )
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::i18n::{Language, set_language};

    /// `modal`, drawn in Czech, after its first frames.
    fn czech(modal: Modal) -> Harness<'static, Modal> {
        set_language(Language::Czech);
        let mut harness = Harness::new_ui_state(
            |ui, modal: &mut Modal| {
                let ctx = ui.ctx().clone();
                show(&ctx, modal);
            },
            modal,
        );
        harness.run();
        harness
    }

    fn shows(harness: &Harness<'static, Modal>, texts: &[&str]) {
        for text in texts {
            assert!(harness.query_by_label(text).is_some(), "{text}");
        }
    }

    #[test]
    fn the_unsaved_changes_dialog_speaks_czech() {
        let h = czech(Modal::Unsaved(Pending::Close));
        shows(
            &h,
            &[
                "Neuložené změny",
                "Uložit změny v tomto projektu před zavřením?",
                "Uložit",
                "Zahodit",
                "Zrušit",
            ],
        );
    }

    #[test]
    fn clearing_retained_messages_counts_the_topics_in_czech() {
        let topics = |n: usize| (0..n).map(|i| format!("t/{i}")).collect::<Vec<_>>();
        let clear = |n| Modal::ClearRetained {
            topics: topics(n),
            broker: "localhost:1883".into(),
        };
        shows(&czech(clear(1)), &["Smazat retained zprávu (1 topic)?"]);
        shows(&czech(clear(3)), &["Smazat retained zprávy (3 topiky)?"]);
        shows(
            &czech(clear(5)),
            &["Smazat retained zprávy (5 topiků)?", "Smazat", "Zrušit"],
        );
    }

    #[test]
    fn the_other_dialogs_speak_czech() {
        shows(
            &czech(Modal::FileChanged {
                name: "plant.toml".into(),
                unsaved: true,
            }),
            &[
                "Soubor plant.toml se na disku změnil",
                "Načíst znovu",
                "Ponechat mou verzi",
            ],
        );
        shows(
            &czech(Modal::ConfirmReplace {
                path: PathBuf::from("/projects/x.toml"),
                then: None,
            }),
            &["Nahradit x.toml?", "Nahradit"],
        );
        shows(
            &czech(Modal::CopyBuiltin {
                builtin: "co2".into(),
                new_id: "co2-copy".into(),
                error: None,
            }),
            &[
                "Kopírovat vestavěnou šablonu co2",
                "ID nové šablony",
                "Kopírovat",
            ],
        );
        shows(
            &czech(Modal::Error("disk full".into())),
            &["Něco se pokazilo", "OK"],
        );
        shows(
            &czech(Modal::OpenFailed {
                path: "x.toml".into(),
                message: "bad".into(),
            }),
            &["Nelze otevřít x.toml"],
        );
    }
}
