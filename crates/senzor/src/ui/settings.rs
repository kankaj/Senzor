use eframe::egui;
use egui_phosphor::regular as icons;

use super::switch::switch;
use super::{Action, ERROR, UiState};
use crate::app::McpStatus;
use crate::settings::{AppSettings, Theme};

/// The app's settings window, opened from the top bar and kept in the middle of the window. Unlike the broker dialog it edits no
/// project: what it holds belongs to the app.
pub fn show(
    ctx: &egui::Context,
    ui_state: &mut UiState,
    settings: &AppSettings,
    mcp: &McpStatus,
    actions: &mut Vec<Action>,
) {
    let mut open = ui_state.settings_open;
    egui::Window::new("Settings")
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            egui::Grid::new("settings-form")
                .num_columns(2)
                .show(ui, |ui| {
                    theme_row(ui, settings.theme, actions);
                    mcp_row(ui, mcp, actions);

                    ui.label("Version");
                    ui.label(env!("CARGO_PKG_VERSION"));
                    ui.end_row();
                });
        });
    ui_state.settings_open = open;
}

/// One button per theme; the chosen one is highlighted.
fn theme_row(ui: &mut egui::Ui, current: Theme, actions: &mut Vec<Action>) {
    ui.label("Theme");
    ui.horizontal(|ui| {
        for theme in Theme::ALL {
            let icon = match theme {
                Theme::System => icons::MONITOR,
                Theme::Light => icons::SUN,
                Theme::Dark => icons::MOON,
            };
            let chosen = theme == current;
            let button = ui.selectable_label(chosen, format!("{icon} {}", theme.label()));
            button.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::SelectableLabel,
                    true,
                    chosen,
                    theme.label(),
                )
            });
            if button.clicked() && !chosen {
                actions.push(Action::SetTheme(theme));
            }
        }
    });
    ui.end_row();
}

/// The MCP toggle, with the address agents connect to while it is on, or why it is off.
fn mcp_row(ui: &mut egui::Ui, mcp: &McpStatus, actions: &mut Vec<Action>) {
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
    ui.label(format!("{} MCP", icons::ROBOT));
    let mut on = matches!(mcp, McpStatus::Listening(_));
    ui.horizontal(|ui| {
        if switch(ui, &mut on, "MCP").on_hover_text(hint).clicked() {
            actions.push(Action::ToggleMcp);
        }
        ui.label("let AI agents control this window");
    });
    ui.end_row();

    match mcp {
        McpStatus::Listening(address) => {
            ui.label("Address");
            ui.label(address.to_string());
            ui.end_row();
        }
        McpStatus::Off {
            error: Some(error), ..
        } => {
            ui.label("");
            ui.colored_label(ERROR, format!("Could not start: {error}"));
            ui.end_row();
        }
        McpStatus::Off { error: None, .. } => {}
    }
}
