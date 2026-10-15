use eframe::egui;

use super::Action;
use crate::document::Document;

/// `starting`: devices or commands are waiting for the previous project's engine to stop.
pub fn show(ui: &mut egui::Ui, doc: &Document, starting: bool, actions: &mut Vec<Action>) {
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
    });
}
