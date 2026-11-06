use std::path::PathBuf;

use eframe::egui;
use egui_phosphor::regular as icons;

use super::{Action, recent_project_button};
use crate::document::Document;

/// Where the documentation lives.
pub const DOCS_URL: &str = "https://senzor.kankaj.cz/docs";

/// The welcome screen replaces the detail panel's hint for a new project that is still blank:
/// no file, no changes, no devices and no templates. That is how the window starts without a
/// file, and what New gives.
pub fn applies(doc: &Document) -> bool {
    doc.path().is_none()
        && !doc.is_dirty()
        && doc.file().devices.is_empty()
        && doc.file().templates.is_empty()
}

/// A way to start: add a device, open a project, reopen a recent one, or read the docs.
pub fn show(ui: &mut egui::Ui, recent: &[PathBuf], actions: &mut Vec<Action>) {
    ui.add_space(32.0);
    ui.vertical_centered(|ui| {
        ui.heading(egui::RichText::new("Welcome to Senzor").size(24.0));
        ui.label("Simulated MQTT sensors for testing what reads them.");
        ui.add_space(16.0);
        if start_button(ui, icons::PLUS, "Add your first device").clicked() {
            actions.push(Action::AddDevice);
        }
        if start_button(ui, icons::FOLDER_OPEN, "Open a project").clicked() {
            actions.push(Action::Open);
        }

        ui.add_space(24.0);
        ui.strong("Recent projects");
        if recent.is_empty() {
            ui.weak("No recent projects yet.");
        }
        for path in recent {
            if recent_project_button(ui, path).clicked() {
                actions.push(Action::OpenRecent(path.clone()));
            }
        }

        ui.add_space(24.0);
        ui.hyperlink_to(format!("{} Documentation", icons::BOOK_OPEN), DOCS_URL);
    });
}

/// A larger button with `icon` before `label`; screen readers and tests know it by `label`.
fn start_button(ui: &mut egui::Ui, icon: &str, label: &str) -> egui::Response {
    let response = ui.add(
        egui::Button::new(egui::RichText::new(format!("{icon} {label}")).size(16.0))
            .min_size(egui::vec2(220.0, 32.0)),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response
}
