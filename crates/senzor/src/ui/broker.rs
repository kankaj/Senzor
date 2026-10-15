use std::path::PathBuf;

use eframe::egui::{self, DragValue};
use senzor_core::project::{Location, TlsSpec};

use super::{Action, UiState, duration_edit, errors_at, optional_text, replace_if_unchanged};
use crate::document::Document;
use crate::edits::{PasswordSource, set_password_source};

/// The `[broker]` / `[broker.tls]` dialog.
pub fn show(
    ctx: &egui::Context,
    doc: &Document,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    let mut open = ui_state.broker_open;
    let original = &doc.file().broker;
    let mut broker = original.clone();
    egui::Window::new("Broker")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            errors_at(ui, doc, &Location::Broker);
            egui::Grid::new("broker-form")
                .num_columns(2)
                .show(ui, |ui| {
                    let label = ui.label("Host");
                    ui.text_edit_singleline(&mut broker.host)
                        .labelled_by(label.id);
                    ui.end_row();

                    let label = ui.label("Port");
                    ui.add(DragValue::new(&mut broker.port))
                        .labelled_by(label.id);
                    ui.end_row();

                    ui.label("Client ID prefix");
                    ui.text_edit_singleline(&mut broker.client_id_prefix);
                    ui.end_row();

                    let label = ui.label("Keep alive");
                    duration_edit(
                        ui,
                        &mut ui_state.buffers,
                        "broker/keep_alive",
                        &mut broker.keep_alive,
                    )
                    .labelled_by(label.id);
                    ui.end_row();

                    ui.label("Username");
                    optional_text(ui, &mut broker.username, "none");
                    ui.end_row();

                    ui.label("Password");
                    ui.horizontal(|ui| {
                        let mut source = PasswordSource::of(&broker);
                        egui::ComboBox::from_id_salt("broker-password-source")
                            .selected_text(source.label())
                            .show_ui(ui, |ui| {
                                for option in PasswordSource::ALL {
                                    ui.selectable_value(&mut source, option, option.label());
                                }
                            });
                        set_password_source(&mut broker, source);
                        if let Some(password) = &mut broker.password {
                            ui.add(egui::TextEdit::singleline(password).password(true));
                        }
                        if let Some(variable) = &mut broker.password_env {
                            ui.text_edit_singleline(variable);
                        }
                    });
                    ui.end_row();

                    ui.label("TLS");
                    let mut tls = broker.tls.is_some();
                    if ui.checkbox(&mut tls, "use TLS").changed() {
                        broker.tls = tls.then(TlsSpec::default);
                    }
                    ui.end_row();
                    if let Some(tls) = &mut broker.tls {
                        for (label, path) in [
                            ("  CA file", &mut tls.ca_file),
                            ("  Client certificate", &mut tls.client_cert),
                            ("  Client key", &mut tls.client_key),
                        ] {
                            ui.label(label);
                            path_edit(ui, path);
                            ui.end_row();
                        }
                    }
                });
            ui.weak("Relative paths are resolved against the project file's folder.");
        });
    ui_state.broker_open = open;
    actions.extend(replace_if_unchanged(original, broker, |f| {
        Some(&mut f.broker)
    }));
}

fn path_edit(ui: &mut egui::Ui, path: &mut Option<PathBuf>) {
    let mut text = path.as_ref().map(|p| p.display().to_string());
    optional_text(ui, &mut text, "system roots / none");
    *path = text.map(PathBuf::from);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::ui::test_support::{Fixture, type_into_label, type_per_frame};

    fn harness() -> Harness<'static, Fixture> {
        let mut fixture = Fixture::new("");
        fixture.ui.broker_open = true;
        fixture.harness(|ui, fx| {
            let ctx = ui.ctx().clone();
            show(&ctx, &fx.doc, &mut fx.ui, &mut fx.actions);
        })
    }

    #[test]
    fn typing_in_host_edits_the_broker() {
        let mut h = harness();
        type_into_label(&mut h, "Host", ".lan");
        assert_eq!(h.state().doc.file().broker.host, "localhost.lan");
    }

    #[test]
    fn an_invalid_keep_alive_is_not_applied() {
        let mut h = harness();
        type_into_label(&mut h, "Keep alive", "x");
        assert_eq!(
            h.state().doc.file().broker.keep_alive,
            Duration::from_secs(30)
        );
        assert!(h.query_by_label("use e.g. 10s, 500ms, 1h").is_some());
    }

    #[test]
    fn enabling_tls_adds_an_empty_tls_section() {
        let mut h = harness();
        h.get_by_label("use TLS").click();
        h.run();
        assert_eq!(h.state().doc.file().broker.tls, Some(TlsSpec::default()));
    }

    #[test]
    fn clearing_the_host_and_typing_a_new_one_keeps_focus() {
        let mut h = harness();
        h.get_by_label("Host").focus();
        h.run();
        for _ in 0.."localhost".len() {
            h.key_press(egui::Key::Backspace);
            h.run();
        }
        type_per_frame(&mut h, &["m", "q"]);
        assert_eq!(h.state().doc.file().broker.host, "mq");
    }
}
