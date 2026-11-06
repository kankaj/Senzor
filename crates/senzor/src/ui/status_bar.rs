//! The status bar along the bottom of the window: the broker, what runs, how fast it publishes
//! and whether AI agents can control the window.

use std::net::SocketAddr;

use eframe::egui::{self, RichText};
use senzor_core::engine::{EngineState, InstanceStatus};
use senzor_core::project::BrokerSpec;

use super::{Action, ERROR, ONLINE, STOPPED, WARNING};

/// How many instances are in each state.
#[derive(Debug, Default, PartialEq, Eq)]
struct Counts {
    online: usize,
    connecting: usize,
    reconnecting: usize,
    failed: usize,
}

impl Counts {
    fn of(state: &EngineState) -> Self {
        let mut counts = Counts::default();
        for instance in state.instances.values() {
            match instance.status {
                InstanceStatus::Online => counts.online += 1,
                InstanceStatus::Connecting => counts.connecting += 1,
                InstanceStatus::Reconnecting { .. } => counts.reconnecting += 1,
                InstanceStatus::Failed { .. } => counts.failed += 1,
                InstanceStatus::Stopped => {}
            }
        }
        counts
    }

    /// How the broker looks from the instances: reachable once one is online.
    fn broker(&self) -> (&'static str, egui::Color32) {
        if self.online > 0 {
            ("connected", ONLINE)
        } else if self.reconnecting + self.failed > 0 {
            ("cannot connect", ERROR)
        } else if self.connecting > 0 {
            ("connecting…", WARNING)
        } else {
            ("not connected", STOPPED)
        }
    }
}

/// `rate`: messages per second. `mcp`: the address AI agents connect to while MCP is on.
pub fn show(
    ui: &mut egui::Ui,
    state: &EngineState,
    broker: &BrokerSpec,
    rate: f64,
    mcp: Option<SocketAddr>,
    actions: &mut Vec<Action>,
) {
    let counts = Counts::of(state);
    let mut right = Vec::new();
    egui::Sides::new().show(
        ui,
        |ui| {
            let (word, color) = counts.broker();
            let link = ui
                .add(
                    egui::Label::new(
                        RichText::new(format!(
                            "{} {}:{} {word}",
                            egui_phosphor::regular::PLUGS_CONNECTED,
                            broker.host,
                            broker.port
                        ))
                        .color(color),
                    )
                    .sense(egui::Sense::click()),
                )
                .on_hover_text("Broker settings");
            if link.clicked() {
                actions.push(Action::OpenBroker);
            }
            ui.separator();
            let parts: Vec<(String, egui::Color32)> = [
                (counts.online, "online", ONLINE),
                (counts.connecting, "connecting", WARNING),
                (counts.reconnecting, "reconnecting", WARNING),
                (counts.failed, "failed", ERROR),
            ]
            .into_iter()
            .filter(|(count, _, _)| *count > 0)
            .map(|(count, word, color)| (format!("{count} {word}"), color))
            .collect();
            if parts.is_empty() {
                ui.weak("nothing running");
            }
            for (text, color) in parts {
                ui.colored_label(color, text);
            }
            ui.separator();
            ui.label(format_rate(rate))
                .on_hover_text("Messages published per second, over the last 5 seconds");
        },
        |ui| {
            let (text, hint) = match mcp {
                Some(address) => (
                    RichText::new(format!("MCP on :{}", address.port()))
                        .color(ui.visuals().selection.stroke.color),
                    format!("AI agents can control this window at http://{address}/mcp"),
                ),
                None => (
                    RichText::new("MCP off").weak(),
                    "AI agents cannot control this window; turn MCP on in the settings".into(),
                ),
            };
            let sign = ui
                .add(egui::Label::new(text).sense(egui::Sense::click()))
                .on_hover_text(hint);
            if sign.clicked() {
                right.push(Action::OpenSettings);
            }
        },
    );
    actions.extend(right);
}

/// `12 msg/s`, with one decimal below 10.
fn format_rate(rate: f64) -> String {
    if rate < 10.0 {
        format!("{rate:.1} msg/s")
    } else {
        format!("{rate:.0} msg/s")
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::ui::test_support::instance;

    fn harness(state: EngineState, rate: f64, mcp: Option<SocketAddr>) -> Harness<'static> {
        let broker = BrokerSpec::default();
        let mut harness = Harness::new_ui(move |ui| {
            show(ui, &state, &broker, rate, mcp, &mut Vec::new());
        });
        harness.run();
        harness
    }

    fn running(statuses: &[InstanceStatus]) -> EngineState {
        let mut state = EngineState::default();
        for (i, status) in statuses.iter().enumerate() {
            state
                .instances
                .insert(format!("d-{i}"), instance("d", status.clone()));
        }
        state
    }

    #[test]
    fn nothing_running_says_so() {
        let h = harness(EngineState::default(), 0.0, None);
        assert!(h.query_by_label("nothing running").is_some());
        assert!(h.query_by_label_contains("not connected").is_some());
        assert!(h.query_by_label("0.0 msg/s").is_some());
        assert!(h.query_by_label("MCP off").is_some());
    }

    #[test]
    fn counts_each_state_and_shows_the_broker_as_connected() {
        let failed = InstanceStatus::Failed {
            reason: "refused".into(),
        };
        let reconnecting = InstanceStatus::Reconnecting {
            attempt: 2,
            last_error: "gone".into(),
        };
        let h = harness(
            running(&[
                InstanceStatus::Online,
                InstanceStatus::Online,
                reconnecting,
                failed,
                InstanceStatus::Stopped,
            ]),
            42.4,
            Some("127.0.0.1:18830".parse().unwrap()),
        );
        assert!(h.query_by_label("2 online").is_some());
        assert!(h.query_by_label("1 reconnecting").is_some());
        assert!(h.query_by_label("1 failed").is_some());
        assert!(
            h.query_by_label_contains("localhost:1883 connected")
                .is_some()
        );
        assert!(h.query_by_label("42 msg/s").is_some());
        assert!(h.query_by_label("MCP on :18830").is_some());
    }

    #[test]
    fn a_broker_no_instance_reaches_cannot_connect() {
        let h = harness(
            running(&[InstanceStatus::Failed {
                reason: "refused".into(),
            }]),
            0.0,
            None,
        );
        assert!(h.query_by_label_contains("cannot connect").is_some());
    }
}
