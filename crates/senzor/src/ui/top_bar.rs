use std::path::PathBuf;

use eframe::egui;
use egui_phosphor::regular as icons;

use super::Action;
use crate::document::Document;

/// `starting`: devices or commands are waiting for the previous project's engine to stop.
/// `recent`: the recent projects, the latest first.
pub fn show(
    ui: &mut egui::Ui,
    doc: &Document,
    recent: &[PathBuf],
    starting: bool,
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        ui.strong(doc.title());
        ui.separator();
        let broker = &doc.file().broker;
        if icon_button(
            ui,
            icons::PLUGS_CONNECTED,
            &format!("Broker: {}:{}", broker.host, broker.port),
        )
        .clicked()
        {
            actions.push(Action::OpenBroker);
        }
        ui.separator();
        if icon_button(ui, icons::FILE_PLUS, "New").clicked() {
            actions.push(Action::New);
        }
        if icon_button(ui, icons::FOLDER_OPEN, "Open").clicked() {
            actions.push(Action::Open);
        }
        recent_menu(ui, recent, actions);
        if icon_button(ui, icons::FLOPPY_DISK, "Save").clicked() {
            actions.push(Action::Save);
        }
        if icon_button(ui, icons::FLOPPY_DISK_BACK, "Save as").clicked() {
            actions.push(Action::SaveAs);
        }
        ui.separator();
        if icon_button(ui, icons::PLAY, "Start all")
            .on_hover_text("Start every device, regardless of autostart")
            .clicked()
        {
            actions.push(Action::StartAll);
        }
        if icon_button(ui, icons::STOP, "Stop all").clicked() {
            actions.push(Action::StopAll);
        }
        if starting {
            ui.weak("starting devices…")
                .on_hover_text("Waiting for the previous project's devices to go offline");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let cog = ui
                .button(egui::RichText::new(icons::GEAR_SIX).size(16.0))
                .on_hover_text("Settings");
            // Screen readers and tests find it by name, not by the glyph.
            cog.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Settings")
            });
            if cog.clicked() {
                actions.push(Action::OpenSettings);
            }
        });
    });
}

/// A button showing `icon` before `label`. Screen readers and tests know it by `label` alone.
fn icon_button(ui: &mut egui::Ui, icon: &str, label: &str) -> egui::Response {
    let response = ui.button(format!("{icon} {label}"));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response
}

/// The Recent menu, a clock next to Open: each project's file name and folder; screen readers and tests know an item
/// by its full path.
fn recent_menu(ui: &mut egui::Ui, recent: &[PathBuf], actions: &mut Vec<Action>) {
    let menu = ui.menu_button(icons::CLOCK_COUNTER_CLOCKWISE, |ui| {
        if recent.is_empty() {
            ui.weak("No recent projects");
            return;
        }
        for path in recent {
            let name = path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            let folder = path
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let item = ui.button((
                egui::RichText::new(name),
                egui::RichText::new(folder).weak().small(),
            ));
            let full = path.display().to_string();
            item.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &full));
            if item.clicked() {
                actions.push(Action::OpenRecent(path.clone()));
            }
        }
        ui.separator();
        if ui.button("Clear list").clicked() {
            actions.push(Action::ClearRecent);
        }
    });
    let response = menu.response.on_hover_text("Recent projects");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Recent"));
}
