use std::path::PathBuf;

use eframe::egui::{self, DragValue};
use senzor_core::project::{Location, MqttVersion, TlsSpec};

use super::{
    Action, ConnectionTest, ERROR, ONLINE, ParsedEdit, UiState, duration_edit, errors_at,
    mqtt5_control, optional_text, replace_if_unchanged,
};
use crate::buffers::Buffers;
use crate::document::Document;
use crate::edits::{PasswordSource, set_password_source};

/// Shown under the seed box while its text is not a seed.
const SEED_HINT: &str = "use a whole number, or leave it empty";

/// The project's settings: the `[broker]` / `[broker.tls]` section and `[simulation]`.
pub fn show(
    ctx: &egui::Context,
    doc: &Document,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    let mut open = ui_state.broker_open;
    let original = &doc.file().broker;
    let mut broker = original.clone();
    let original_simulation = &doc.file().simulation;
    let mut simulation = original_simulation.clone();
    egui::Window::new("Project settings")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.strong("Broker");
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

                    let label = ui.label("MQTT version");
                    egui::ComboBox::from_id_salt("broker-mqtt-version")
                        .selected_text(broker.mqtt_version.label())
                        .show_ui(ui, |ui| {
                            for version in MqttVersion::ALL {
                                ui.selectable_value(
                                    &mut broker.mqtt_version,
                                    version,
                                    version.label(),
                                );
                            }
                        })
                        .response
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

                    let label = ui.label("Session expiry");
                    let mqtt5 = broker.mqtt_version == MqttVersion::V5;
                    let has_value = !broker.session_expiry.is_zero();
                    mqtt5_control(ui, mqtt5, has_value, |ui| {
                        duration_edit(
                            ui,
                            &mut ui_state.buffers,
                            "broker/session_expiry",
                            &mut broker.session_expiry,
                        )
                        .labelled_by(label.id)
                    });
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
            connection_test(ui, doc, ui_state.connection_test.as_ref(), actions);

            ui.separator();
            ui.strong("Simulation");
            egui::Grid::new("simulation-form")
                .num_columns(2)
                .show(ui, |ui| {
                    let label = ui.label("Seed");
                    seed_edit(ui, &mut ui_state.buffers, &mut simulation.seed)
                        .labelled_by(label.id);
                    ui.end_row();
                });
            ui.weak("With a seed, every run draws the same random values.");
        });
    ui_state.broker_open = open;
    actions.extend(replace_if_unchanged(original, broker, |f| {
        Some(&mut f.broker)
    }));
    actions.extend(replace_if_unchanged(original_simulation, simulation, |f| {
        Some(&mut f.simulation)
    }));
}

/// Test connection, then how the test went, on a line of its own so a long reason wraps.
fn connection_test(
    ui: &mut egui::Ui,
    doc: &Document,
    test: Option<&ConnectionTest>,
    actions: &mut Vec<Action>,
) {
    let running = matches!(test, Some(ConnectionTest::Running));
    let problems = doc
        .validated()
        .errors
        .iter()
        .any(|e| e.location == Location::Broker);
    let button = ui
        .add_enabled(!running && !problems, egui::Button::new("Test connection"))
        .on_hover_text("Connect once with these settings, then disconnect")
        .on_disabled_hover_text(if problems {
            "Fix the broker problems first"
        } else {
            "Testing…"
        });
    if button.clicked() {
        actions.push(Action::TestConnection);
    }
    match test {
        Some(ConnectionTest::Running) => {
            ui.horizontal(|ui| {
                ui.weak(egui_phosphor::regular::HOURGLASS_MEDIUM);
                ui.weak("connecting…");
            });
        }
        Some(ConnectionTest::Passed) => {
            ui.colored_label(ONLINE, "Connected");
        }
        Some(ConnectionTest::Failed(reason)) => {
            ui.colored_label(ERROR, reason);
        }
        None => {}
    }
}

/// The seed: a whole number, or empty for none, which makes every run draw fresh random values.
fn seed_edit(ui: &mut egui::Ui, buffers: &mut Buffers, seed: &mut Option<u64>) -> egui::Response {
    ParsedEdit::new("simulation/seed", 160.0)
        .hint("random")
        .show(
            ui,
            buffers,
            seed,
            |seed| seed.map_or_else(String::new, |s| s.to_string()),
            |text| {
                let text = text.trim();
                if text.is_empty() {
                    return Ok(None);
                }
                text.parse().map(Some).map_err(|_| SEED_HINT.to_string())
            },
        )
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
    use egui_kittest::kittest::{NodeT, Queryable};

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
    fn test_connection_asks_the_app_to_test() {
        let mut h = harness();
        h.get_by_label("Test connection").click();
        h.run();
        assert!(
            h.state()
                .other
                .iter()
                .any(|a| matches!(a, Action::TestConnection))
        );
    }

    fn harness_with_test(test: ConnectionTest) -> Harness<'static, Fixture> {
        let mut fixture = Fixture::new("");
        fixture.ui.broker_open = true;
        fixture.ui.connection_test = Some(test);
        fixture.harness(|ui, fx| {
            let ctx = ui.ctx().clone();
            show(&ctx, &fx.doc, &mut fx.ui, &mut fx.actions);
        })
    }

    #[test]
    fn the_test_result_shows_under_the_button() {
        let h = harness_with_test(ConnectionTest::Passed);
        assert!(h.query_by_label("Connected").is_some());
        let h = harness_with_test(ConnectionTest::Failed("broker refused".into()));
        assert!(h.query_by_label("broker refused").is_some());
        let h = harness_with_test(ConnectionTest::Running);
        assert!(h.query_by_label("connecting…").is_some());
        assert!(
            h.get_by_label("Test connection")
                .accesskit_node()
                .is_disabled()
        );
    }

    #[test]
    fn a_broker_with_problems_cannot_be_tested() {
        let mut fixture = Fixture::new("[broker]\nport = 0\n");
        fixture.ui.broker_open = true;
        let h = fixture.harness(|ui, fx| {
            let ctx = ui.ctx().clone();
            show(&ctx, &fx.doc, &mut fx.ui, &mut fx.actions);
        });
        assert!(
            !h.state().doc.validated().errors.is_empty(),
            "port 0 is a problem"
        );
        assert!(
            h.get_by_label("Test connection")
                .accesskit_node()
                .is_disabled()
        );
    }

    #[test]
    fn the_window_is_the_projects_settings() {
        let h = harness();
        assert!(h.query_by_label("Project settings").is_some());
        assert!(h.query_by_label("Simulation").is_some());
    }

    #[test]
    fn typing_a_seed_sets_it_in_the_project() {
        let mut h = harness();
        type_into_label(&mut h, "Seed", "42");
        assert_eq!(h.state().doc.file().simulation.seed, Some(42));
        assert!(
            h.state()
                .doc
                .file()
                .to_toml_string()
                .contains("[simulation]\nseed = 42")
        );
    }

    #[test]
    fn clearing_the_seed_takes_it_out_of_the_project() {
        let mut fixture = Fixture::new("[simulation]\nseed = 7\n");
        fixture.ui.broker_open = true;
        let mut h = fixture.harness(|ui, fx| {
            let ctx = ui.ctx().clone();
            show(&ctx, &fx.doc, &mut fx.ui, &mut fx.actions);
        });
        h.get_by_label("Seed").focus();
        h.run();
        h.key_press(egui::Key::Backspace);
        h.run();
        assert_eq!(h.state().doc.file().simulation.seed, None);
    }

    #[test]
    fn a_seed_that_is_not_a_whole_number_is_not_applied() {
        let mut h = harness();
        type_into_label(&mut h, "Seed", "4x");
        assert_eq!(h.state().doc.file().simulation.seed, None);
        assert!(h.query_by_label(SEED_HINT).is_some());
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
    fn choosing_mqtt_3_1_1_edits_the_broker_and_marks_the_project_unsaved() {
        let mut h = harness();
        assert_eq!(h.state().doc.file().broker.mqtt_version, MqttVersion::V5);
        h.get_by_label("MQTT version").click();
        h.run();
        h.get_by_label("3.1.1").click();
        h.run();
        assert_eq!(h.state().doc.file().broker.mqtt_version, MqttVersion::V311);
        assert!(h.state().doc.is_dirty());
        assert!(
            h.state()
                .doc
                .file()
                .to_toml_string()
                .contains(r#"mqtt_version = "3.1.1""#)
        );
    }

    #[test]
    fn typing_a_session_expiry_edits_the_broker() {
        let mut h = harness();
        h.get_by_label("Session expiry").focus();
        h.run();
        h.key_press(egui::Key::Backspace);
        h.key_press(egui::Key::Backspace);
        h.run();
        type_per_frame(&mut h, &["6", "0", "s"]);
        assert_eq!(
            h.state().doc.file().broker.session_expiry,
            Duration::from_secs(60)
        );
    }

    #[test]
    fn session_expiry_is_disabled_with_the_reason_on_mqtt_3_1_1() {
        let mut fixture = Fixture::new("[broker]\nmqtt_version = \"3.1.1\"\n");
        fixture.ui.broker_open = true;
        let h = fixture.harness(|ui, fx| {
            let ctx = ui.ctx().clone();
            show(&ctx, &fx.doc, &mut fx.ui, &mut fx.actions);
        });
        assert!(
            h.get_by_label("Session expiry")
                .accesskit_node()
                .is_disabled()
        );
        assert!(h.query_by_label(crate::ui::NEEDS_MQTT5).is_some());
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
