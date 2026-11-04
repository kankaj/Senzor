use eframe::egui;

use super::{Action, ERROR};
use crate::app::McpStatus;
use crate::document::Document;

/// `starting`: devices or commands are waiting for the previous project's engine to stop.
/// `mcp`: whether AI agents can connect, shown on the MCP toggle.
pub fn show(
    ui: &mut egui::Ui,
    doc: &Document,
    starting: bool,
    mcp: &McpStatus,
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        ui.strong(doc.title());
        ui.separator();
        let broker = &doc.file().broker;
        if ui
            .button(format!("Broker: {}:{}", broker.host, broker.port))
            .clicked()
        {
            actions.push(Action::OpenBroker);
        }
        ui.separator();
        if ui.button("New").clicked() {
            actions.push(Action::New);
        }
        if ui.button("Open").clicked() {
            actions.push(Action::Open);
        }
        if ui.button("Save").clicked() {
            actions.push(Action::Save);
        }
        if ui.button("Save as").clicked() {
            actions.push(Action::SaveAs);
        }
        ui.separator();
        if ui
            .button("Start all")
            .on_hover_text("Start every device, regardless of autostart")
            .clicked()
        {
            actions.push(Action::StartAll);
        }
        if ui.button("Stop all").clicked() {
            actions.push(Action::StopAll);
        }
        if starting {
            ui.weak("starting devices…")
                .on_hover_text("Waiting for the previous project's devices to go offline");
        }
        ui.separator();
        let on = matches!(mcp, McpStatus::Listening(_));
        let hint = match mcp {
            McpStatus::Listening(address) => {
                format!("AI agents can control this window at http://{address}/mcp. Click to stop.")
            }
            McpStatus::Off { port, error: None } => {
                format!("Let AI agents control this window over MCP at http://127.0.0.1:{port}/mcp")
            }
            McpStatus::Off {
                port,
                error: Some(error),
            } => format!("MCP could not start: {error}. Click to try port {port} again."),
        };
        let mut toggle = egui::Button::new("MCP").selected(on);
        if matches!(mcp, McpStatus::Off { error: Some(_), .. }) {
            toggle = egui::Button::new(egui::RichText::new("MCP").color(ERROR));
        }
        if ui.add(toggle).on_hover_text(hint).clicked() {
            actions.push(Action::ToggleMcp);
        }
        if let McpStatus::Listening(address) = mcp {
            ui.label(address.to_string());
        }
    });
}
