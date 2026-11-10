//! The status bar along the bottom of the window: the broker, what runs, how fast it publishes
//! whether AI agents can control the window, and whether a newer Senzor is out.

use std::net::SocketAddr;

use eframe::egui::{self, RichText};
use senzor_core::engine::{EngineState, InstanceStatus};
use senzor_core::project::BrokerSpec;

use super::{Action, ERROR, ONLINE, STOPPED, WARNING};
use crate::i18n::t;
use crate::update::Release;

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
    fn broker(&self) -> (String, egui::Color32) {
        if self.online > 0 {
            (t!("status-bar-connected"), ONLINE)
        } else if self.reconnecting + self.failed > 0 {
            (t!("status-bar-cannot-connect"), ERROR)
        } else if self.connecting > 0 {
            (t!("status-bar-connecting"), WARNING)
        } else {
            (t!("status-bar-not-connected"), STOPPED)
        }
    }
}

/// `rate`: messages per second. `mcp`: the address AI agents connect to while MCP is on.
/// `newer`: a release newer than this version.
pub fn show(
    ui: &mut egui::Ui,
    state: &EngineState,
    broker: &BrokerSpec,
    rate: f64,
    mcp: Option<SocketAddr>,
    newer: Option<&Release>,
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
                .on_hover_text(t!("status-bar-broker-hint"));
            if link.clicked() {
                actions.push(Action::OpenBroker);
            }
            ui.separator();
            let Counts {
                online,
                connecting,
                reconnecting,
                failed,
            } = counts;
            let parts: Vec<(String, egui::Color32)> = [
                (online, t!("status-bar-online", count = online), ONLINE),
                (
                    connecting,
                    t!("status-bar-connecting-count", count = connecting),
                    WARNING,
                ),
                (
                    reconnecting,
                    t!("status-bar-reconnecting-count", count = reconnecting),
                    WARNING,
                ),
                (failed, t!("status-bar-failed-count", count = failed), ERROR),
            ]
            .into_iter()
            .filter(|(count, _, _)| *count > 0)
            .map(|(_, text, color)| (text, color))
            .collect();
            if parts.is_empty() {
                ui.weak(t!("status-bar-nothing-running"));
            }
            for (text, color) in parts {
                ui.colored_label(color, text);
            }
            ui.separator();
            ui.label(format_rate(rate))
                .on_hover_text(t!("status-bar-rate-hint"));
        },
        |ui| {
            let (text, hint) = match mcp {
                Some(address) => (
                    RichText::new(t!("status-bar-mcp-on", port = address.port().to_string()))
                        .color(ui.visuals().selection.stroke.color),
                    t!("status-bar-mcp-on-hint", address = address.to_string()),
                ),
                None => (
                    RichText::new(t!("status-bar-mcp-off")).weak(),
                    t!("status-bar-mcp-off-hint"),
                ),
            };
            let sign = ui
                .add(egui::Label::new(text).sense(egui::Sense::click()))
                .on_hover_text(hint);
            if sign.clicked() {
                right.push(Action::OpenSettings);
            }
            // The right side is laid out from the right, so this comes before the MCP sign.
            if let Some(release) = newer {
                let text = format!(
                    "{} {}",
                    egui_phosphor::regular::ARROW_CIRCLE_UP,
                    t!("status-bar-release", version = release.version.as_str())
                );
                // In the links' colour, like the release link in the settings.
                let link = ui
                    .add(egui::Link::new(text))
                    .on_hover_text(t!("status-bar-release-hint"));
                if link.clicked() {
                    right.push(Action::OpenNewRelease);
                }
                ui.separator();
            }
        },
    );
    actions.extend(right);
}

/// `12 msg/s`, with one decimal below 10.
fn format_rate(rate: f64) -> String {
    let rate = if rate < 10.0 {
        format!("{rate:.1}")
    } else {
        format!("{rate:.0}")
    };
    t!("status-bar-rate", rate = rate)
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
            show(ui, &state, &broker, rate, mcp, None, &mut Vec::new());
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

    #[test]
    fn the_status_bar_speaks_czech_with_czech_plurals() {
        crate::i18n::set_language(crate::i18n::Language::Czech);
        let h = harness(EngineState::default(), 0.0, None);
        assert!(h.query_by_label("nic neběží").is_some());
        assert!(h.query_by_label_contains("nepřipojeno").is_some());
        assert!(h.query_by_label("0.0 zpr./s").is_some());
        assert!(h.query_by_label("MCP vypnuto").is_some());

        let failed = InstanceStatus::Failed {
            reason: "refused".into(),
        };
        let h = harness(
            running(&[
                InstanceStatus::Online,
                InstanceStatus::Connecting,
                InstanceStatus::Connecting,
                failed.clone(),
                failed.clone(),
                failed.clone(),
                failed.clone(),
                failed,
            ]),
            12.0,
            Some("127.0.0.1:18830".parse().unwrap()),
        );
        assert!(h.query_by_label("1 online").is_some());
        assert!(h.query_by_label("2 se připojují").is_some());
        assert!(h.query_by_label("5 selhalo").is_some());
        assert!(h.query_by_label_contains("připojeno").is_some());
        assert!(h.query_by_label("MCP zapnuto :18830").is_some());
    }
}
