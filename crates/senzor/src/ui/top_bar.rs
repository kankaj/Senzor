use std::net::SocketAddr;
use std::path::PathBuf;

use eframe::egui;
use egui_phosphor::regular as icons;

use super::{Action, shortcuts};
use crate::document::Document;

/// From the left: the project, the broker, the file buttons and the run buttons with the
/// starting hint; on the right, the MCP sign and the settings.
///
/// `recent`: the recent projects, the latest first. `starting`: devices or commands are waiting
/// for the previous project's engine to stop. `mcp`: the address AI agents connect to while MCP
/// is on.
pub fn show(
    ui: &mut egui::Ui,
    doc: &Document,
    recent: &[PathBuf],
    starting: bool,
    mcp: Option<SocketAddr>,
    actions: &mut Vec<Action>,
) {
    // The settings and the MCP sign keep their place at the right; in a narrow window the rest
    // wraps onto a second row instead of running under them.
    let mut right = Vec::new();
    egui::Sides::new().shrink_left().show(
        ui,
        |ui| {
            ui.horizontal_wrapped(|ui| {
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
                if icon_button(ui, icons::FILE_PLUS, "New")
                    .on_hover_text(shortcuts::tip(ui.ctx(), "New project", &shortcuts::NEW))
                    .clicked()
                {
                    actions.push(Action::New);
                }
                if icon_button(ui, icons::FOLDER_OPEN, "Open")
                    .on_hover_text(shortcuts::tip(ui.ctx(), "Open a project", &shortcuts::OPEN))
                    .clicked()
                {
                    actions.push(Action::Open);
                }
                recent_menu(ui, recent, actions);
                if save_button(ui, doc.is_dirty()).clicked() {
                    actions.push(Action::Save);
                }
                if icon_button(ui, icons::FLOPPY_DISK_BACK, "Save as")
                    .on_hover_text(shortcuts::tip(
                        ui.ctx(),
                        "Save under another name",
                        &shortcuts::SAVE_AS,
                    ))
                    .clicked()
                {
                    actions.push(Action::SaveAs);
                }
                undo_redo(ui, doc, actions);
                ui.separator();
                if icon_button(ui, icons::PLAY, "Start all")
                    .on_hover_text(shortcuts::tip(
                        ui.ctx(),
                        "Start every device, regardless of autostart",
                        &shortcuts::START_ALL,
                    ))
                    .clicked()
                {
                    actions.push(Action::StartAll);
                }
                if icon_button(ui, icons::STOP, "Stop all")
                    .on_hover_text(shortcuts::tip(
                        ui.ctx(),
                        "Stop every device",
                        &shortcuts::STOP_ALL,
                    ))
                    .clicked()
                {
                    actions.push(Action::StopAll);
                }
                if starting {
                    ui.weak("starting devices…")
                        .on_hover_text("Waiting for the previous project's devices to go offline");
                }
            });
        },
        |ui| {
            let cog = ui
                .button(egui::RichText::new(icons::GEAR_SIX).size(16.0))
                .on_hover_text(shortcuts::tip(ui.ctx(), "Settings", &shortcuts::SETTINGS));
            // Screen readers and tests find it by name, not by the glyph.
            cog.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Settings")
            });
            if cog.clicked() {
                right.push(Action::OpenSettings);
            }
            if let Some(address) = mcp
                && mcp_sign(ui, address).clicked()
            {
                right.push(Action::OpenSettings);
            }
        },
    );
    actions.extend(right);
}

/// Save, filled with the accent colour while the project has unsaved changes.
fn save_button(ui: &mut egui::Ui, dirty: bool) -> egui::Response {
    let mut button = egui::Button::new(format!("{} Save", icons::FLOPPY_DISK));
    if dirty {
        button = button.fill(ui.visuals().selection.bg_fill);
    }
    let response = ui.add(button);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Save"));
    let text = if dirty {
        "Save the unsaved changes"
    } else {
        "Save"
    };
    response.on_hover_text(shortcuts::tip(ui.ctx(), text, &shortcuts::SAVE))
}

/// A robot in the accent colour while AI agents can control the window; it opens the settings,
/// where MCP is turned off.
fn mcp_sign(ui: &mut egui::Ui, address: SocketAddr) -> egui::Response {
    let robot = egui::RichText::new(icons::ROBOT)
        .size(16.0)
        .color(ui.visuals().selection.stroke.color);
    let response = ui.button(robot).on_hover_text(format!(
        "AI agents can control this window at http://{address}/mcp. Click for the settings."
    ));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "MCP on"));
    response
}

/// A button showing `icon` before `label`. Screen readers and tests know it by `label` alone.
fn icon_button(ui: &mut egui::Ui, icon: &str, label: &str) -> egui::Response {
    let response = ui.button(format!("{icon} {label}"));
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response
}

/// The Recent menu, a clock next to Open, listing [`super::recent_project_button`]s.
fn recent_menu(ui: &mut egui::Ui, recent: &[PathBuf], actions: &mut Vec<Action>) {
    let menu = ui.menu_button(icons::CLOCK_COUNTER_CLOCKWISE, |ui| {
        if recent.is_empty() {
            ui.weak("No recent projects");
            return;
        }
        for path in recent {
            if super::recent_project_button(ui, path).clicked() {
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

/// An Undo or Redo button: icon, name, enabled, shortcut and action.
type HistoryButton = (
    &'static str,
    &'static str,
    bool,
    &'static egui::KeyboardShortcut,
    fn() -> Action,
);

/// Undo and Redo as icons, off while there is nothing to undo or redo.
fn undo_redo(ui: &mut egui::Ui, doc: &Document, actions: &mut Vec<Action>) {
    let buttons: [HistoryButton; 2] = [
        (
            icons::ARROW_COUNTER_CLOCKWISE,
            "Undo",
            doc.can_undo(),
            &shortcuts::UNDO,
            || Action::Undo,
        ),
        (
            icons::ARROW_CLOCKWISE,
            "Redo",
            doc.can_redo(),
            &shortcuts::REDO,
            || Action::Redo,
        ),
    ];
    for (icon, name, enabled, shortcut, action) in buttons {
        let response = ui
            .add_enabled(enabled, egui::Button::new(icon))
            .on_hover_text(shortcuts::tip(ui.ctx(), name, shortcut))
            .on_disabled_hover_text(format!("Nothing to {}", name.to_lowercase()));
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
        if response.clicked() {
            actions.push(action());
        }
    }
}
