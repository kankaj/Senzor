use std::path::PathBuf;

use eframe::egui::{self, DragValue};
use senzor_core::project::{BrokerSpec, Location, MqttVersion, TlsSpec};

use super::{
    Action, BoxOrRow, ClearRetained, ConnectionTest, ERROR, ONLINE, ParsedEdit, UiState, checked,
    duration_edit, errors_at, help, mqtt5_control, optional_text, replace_if_unchanged,
    retained_topics_text,
};
use crate::buffers::Buffers;
use crate::document::Document;
use crate::edits::{PasswordSource, set_password_source};

/// A broker to try Senzor with: its name in the Presets menu, host, port and what it is.
struct Preset {
    name: &'static str,
    host: &'static str,
    port: u16,
    hint: &'static str,
}

/// Anyone can read what is published to the public brokers, hence their hints.
const PRESETS: [Preset; 4] = [
    Preset {
        name: "Mosquitto on this computer",
        host: "localhost",
        port: 1883,
        hint: "A broker running on this computer, installed or in Docker",
    },
    Preset {
        name: "Mosquitto test server",
        host: "test.mosquitto.org",
        port: 1883,
        hint: "Public: anyone can read what is published there",
    },
    Preset {
        name: "HiveMQ public broker",
        host: "broker.hivemq.com",
        port: 1883,
        hint: "Public: anyone can read what is published there",
    },
    Preset {
        name: "EMQX public broker",
        host: "broker.emqx.io",
        port: 1883,
        hint: "Public: anyone can read what is published there",
    },
];

/// Points `broker` at `preset`, which needs no credentials and no TLS; the other settings stay.
fn apply_preset(broker: &mut BrokerSpec, preset: &Preset) {
    broker.host = preset.host.to_string();
    broker.port = preset.port;
    broker.username = None;
    broker.password = None;
    broker.password_env = None;
    broker.tls = None;
}

/// The Presets menu, which sets the broker's address to a known broker.
fn presets_menu(ui: &mut egui::Ui, broker: &mut BrokerSpec) {
    ui.menu_button("Presets", |ui| {
        for preset in &PRESETS {
            let item = ui
                .button(format!(
                    "{}  ({}:{})",
                    preset.name, preset.host, preset.port
                ))
                .on_hover_text(preset.hint);
            item.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, preset.name)
            });
            if item.clicked() {
                apply_preset(broker, preset);
                ui.close();
            }
        }
    })
    .response
    .on_hover_text("Fill in a known broker's address; credentials and TLS are cleared");
}

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
            ui.horizontal(|ui| {
                ui.strong("Broker");
                presets_menu(ui, &mut broker);
            });
            let at = &Location::Broker;
            errors_at(ui, doc, at, shown_inline);
            egui::Grid::new("broker-form")
                .num_columns(2)
                .show(ui, |ui| {
                    let label = help::label(ui, "Host", help::HOST);
                    checked(ui, doc, at, "host", |ui| {
                        ui.text_edit_singleline(&mut broker.host)
                            .labelled_by(label.id)
                    });
                    ui.end_row();

                    let label = help::label(ui, "Port", help::PORT);
                    checked(ui, doc, at, "port", |ui| {
                        ui.add(DragValue::new(&mut broker.port))
                            .labelled_by(label.id)
                    });
                    ui.end_row();

                    let label = help::label(ui, "MQTT version", help::MQTT_VERSION);
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

                    help::label(ui, "Client ID prefix", help::CLIENT_ID_PREFIX);
                    ui.text_edit_singleline(&mut broker.client_id_prefix);
                    ui.end_row();

                    let label = help::label(ui, "Keep alive", help::KEEP_ALIVE);
                    checked(ui, doc, at, "keep_alive", |ui| {
                        duration_edit(
                            ui,
                            &mut ui_state.buffers,
                            "broker/keep_alive",
                            &mut broker.keep_alive,
                        )
                        .labelled_by(label.id)
                    });
                    ui.end_row();

                    let label = help::label(ui, "Session expiry", help::SESSION_EXPIRY);
                    let mqtt5 = broker.mqtt_version == MqttVersion::V5;
                    let has_value = !broker.session_expiry.is_zero();
                    checked(ui, doc, at, "session_expiry", |ui| {
                        mqtt5_control(ui, mqtt5, has_value, |ui| {
                            duration_edit(
                                ui,
                                &mut ui_state.buffers,
                                "broker/session_expiry",
                                &mut broker.session_expiry,
                            )
                            .labelled_by(label.id)
                        })
                    });
                    ui.end_row();

                    help::label(ui, "Username", help::USERNAME);
                    checked(ui, doc, at, "username", |ui| {
                        optional_text(ui, &mut broker.username, "none")
                    });
                    ui.end_row();

                    help::label(ui, "Password", help::PASSWORD);
                    checked(ui, doc, at, "password_env", |ui| {
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
                            broker
                                .password_env
                                .as_mut()
                                .map(|variable| ui.text_edit_singleline(variable))
                        })
                        .box_or_row()
                    });
                    ui.end_row();

                    help::label(ui, "TLS", help::TLS);
                    let mut tls = broker.tls.is_some();
                    if ui.checkbox(&mut tls, "use TLS").changed() {
                        broker.tls = tls.then(TlsSpec::default);
                    }
                    ui.end_row();
                    if let Some(tls) = &mut broker.tls {
                        for (label, field, path) in [
                            ("  CA file", "tls.ca_file", &mut tls.ca_file),
                            (
                                "  Client certificate",
                                "tls.client_cert",
                                &mut tls.client_cert,
                            ),
                            ("  Client key", "tls.client_key", &mut tls.client_key),
                        ] {
                            ui.label(label);
                            checked(ui, doc, at, field, |ui| path_edit(ui, path));
                            ui.end_row();
                        }
                    }
                });
            ui.weak("Relative paths are resolved against the project file's folder.");
            connection_test(ui, doc, ui_state.connection_test.as_ref(), actions);
            clear_retained(ui, doc, ui_state, actions);

            ui.separator();
            ui.strong("Simulation");
            egui::Grid::new("simulation-form")
                .num_columns(2)
                .show(ui, |ui| {
                    let label = help::label(ui, "Seed", help::SEED);
                    seed_edit(ui, &mut ui_state.buffers, &mut simulation.seed)
                        .labelled_by(label.id);
                    ui.end_row();
                });
            ui.weak("With a seed, every run draws the same random values.");

            ui.separator();
            run_elsewhere(ui, doc, ui_state, actions);
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

/// Copy buttons for running the project with senzor-cli in CI or in a container.
fn run_elsewhere(ui: &mut egui::Ui, doc: &Document, ui_state: &UiState, actions: &mut Vec<Action>) {
    use crate::snippets::Snippet;
    ui.strong("Run elsewhere");
    let saved = doc.file_name().is_some();
    ui.horizontal_wrapped(|ui| {
        for snippet in [Snippet::GitHubActions, Snippet::DockerCompose] {
            if ui
                .add_enabled(saved, egui::Button::new(snippet.label()))
                .on_hover_text("Puts text on the clipboard that runs this project with senzor-cli")
                .on_disabled_hover_text("Save the project first: the snippet names its file")
                .clicked()
            {
                actions.push(Action::CopySnippet(snippet));
            }
        }
    });
    if saved && ui_state.snippet_copied.is_some() {
        ui.colored_label(ONLINE, "Copied.");
    }
    ui.weak("See the CI and Docker recipe in the documentation for the rest of the job.");
}

/// Hover hint of Clear retained messages while the project's devices run.
const CLEAR_WHILE_RUNNING: &str =
    "Stop the devices first: running instances would publish their retained messages again";

/// Clear retained messages…, then how it went.
fn clear_retained(
    ui: &mut egui::Ui,
    doc: &Document,
    ui_state: &UiState,
    actions: &mut Vec<Action>,
) {
    let clearing = matches!(ui_state.clear_retained, Some(ClearRetained::Running));
    let problems = doc
        .validated()
        .errors
        .iter()
        .any(|e| e.location == Location::Broker);
    let hint = if ui_state.devices_running {
        CLEAR_WHILE_RUNNING
    } else if problems {
        "Fix the broker problems first"
    } else {
        "Clearing…"
    };
    let button = ui
        .add_enabled(
            !clearing && !problems && !ui_state.devices_running,
            egui::Button::new("Clear retained messages…"),
        )
        .on_hover_text(
            "Delete the retained messages this project's instances leave on the broker, \
             such as their last offline",
        )
        .on_disabled_hover_text(hint);
    if button.clicked() {
        actions.push(Action::AskClearRetained);
    }
    match &ui_state.clear_retained {
        Some(ClearRetained::Running) => {
            ui.horizontal(|ui| {
                ui.weak(egui_phosphor::regular::HOURGLASS_MEDIUM);
                ui.weak("clearing…");
            });
        }
        Some(ClearRetained::Cleared(count)) => {
            ui.colored_label(ONLINE, format!("Cleared {}", retained_topics_text(*count)));
        }
        Some(ClearRetained::Nothing) => {
            ui.weak(
                "Nothing to clear: no template has availability, retains its messages or announces \
                 to Home Assistant.",
            );
        }
        Some(ClearRetained::Failed(reason)) => {
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

fn path_edit(ui: &mut egui::Ui, path: &mut Option<PathBuf>) -> egui::Response {
    let mut text = path.as_ref().map(|p| p.display().to_string());
    let response = optional_text(ui, &mut text, "system roots / none");
    *path = text.map(PathBuf::from);
    response
}

/// The broker fields the form shows its problems next to.
fn shown_inline(field: &str) -> bool {
    matches!(
        field,
        "host" | "port" | "keep_alive" | "session_expiry" | "username" | "password_env"
    ) || field.starts_with("tls.")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};

    use super::*;
    use crate::ui::test_support::{
        Fixture, assert_disabled_with_hint, type_into_label, type_per_frame,
    };

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

    fn harness_with_running(running: bool) -> Harness<'static, Fixture> {
        let mut fixture = Fixture::new("");
        fixture.ui.broker_open = true;
        fixture.ui.devices_running = running;
        fixture.harness(|ui, fx| {
            let ctx = ui.ctx().clone();
            show(&ctx, &fx.doc, &mut fx.ui, &mut fx.actions);
        })
    }

    #[test]
    fn clear_retained_asks_the_app_to_clear() {
        let mut h = harness_with_running(false);
        h.get_by_label("Clear retained messages…").click();
        h.run();
        assert!(
            h.state()
                .other
                .iter()
                .any(|a| matches!(a, Action::AskClearRetained))
        );
    }

    #[test]
    fn clear_retained_waits_until_the_devices_stop() {
        let mut h = harness_with_running(true);
        assert_disabled_with_hint(&mut h, "Clear retained messages…", CLEAR_WHILE_RUNNING);
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
    fn a_preset_sets_the_address_and_drops_credentials_and_tls() {
        let mut fixture = Fixture::new(
            "[broker]\nhost = \"mq.lan\"\nport = 8883\nusername = \"me\"\npassword = \"pw\"\n\
             keep_alive = \"45s\"\n[broker.tls]\n",
        );
        fixture.ui.broker_open = true;
        let mut h = fixture.harness(|ui, fx| {
            let ctx = ui.ctx().clone();
            show(&ctx, &fx.doc, &mut fx.ui, &mut fx.actions);
        });
        h.get_by_label("Presets").click();
        h.run();
        h.get_by_label("Mosquitto test server").click();
        h.run();
        let broker = &h.state().doc.file().broker;
        assert_eq!(
            (broker.host.as_str(), broker.port),
            ("test.mosquitto.org", 1883)
        );
        assert_eq!(broker.username, None);
        assert_eq!(broker.password, None);
        assert_eq!(broker.tls, None);
        assert_eq!(broker.keep_alive, Duration::from_secs(45), "the rest stays");
    }

    #[test]
    fn the_local_preset_is_localhost() {
        let mut fixture = Fixture::new("[broker]\nhost = \"mq.lan\"\nport = 1884\n");
        fixture.ui.broker_open = true;
        let mut h = fixture.harness(|ui, fx| {
            let ctx = ui.ctx().clone();
            show(&ctx, &fx.doc, &mut fx.ui, &mut fx.actions);
        });
        h.get_by_label("Presets").click();
        h.run();
        h.get_by_label("Mosquitto on this computer").click();
        h.run();
        let broker = &h.state().doc.file().broker;
        assert_eq!((broker.host.as_str(), broker.port), ("localhost", 1883));
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
