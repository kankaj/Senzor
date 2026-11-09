use eframe::egui;
use egui_phosphor::regular as icons;

use super::switch::switch;
use super::{Action, ERROR, UiState};
use crate::app::McpStatus;
use crate::settings::{AppSettings, Theme};
use crate::update::Release;

/// The app's settings window, opened from the top bar and kept in the middle of the window. Unlike the broker dialog it edits no
/// project: what it holds belongs to the app.
pub fn show(
    ctx: &egui::Context,
    ui_state: &mut UiState,
    settings: &AppSettings,
    mcp: &McpStatus,
    newer: Option<&Release>,
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
                    zoom_row(ui);
                    notify_row(ui, settings.notify_failures, actions);
                    mcp_row(ui, mcp, settings, actions);

                    updates_row(ui, settings.check_updates, actions);

                    ui.label("Version");
                    ui.horizontal(|ui| {
                        ui.label(env!("CARGO_PKG_VERSION"));
                        if let Some(release) = newer {
                            ui.label("·");
                            ui.hyperlink_to(format!("{} is out", release.version), &release.url)
                                .on_hover_text("See what's new and download it");
                        }
                    });
                    ui.end_row();

                    ui.label("Licence");
                    ui.hyperlink_to(
                        env!("CARGO_PKG_LICENSE"),
                        format!("{}/blob/main/LICENSE", super::SOURCE_URL),
                    );
                    ui.end_row();

                    ui.label("Author");
                    ui.label(format!("© 2026 {}", env!("CARGO_PKG_AUTHORS")));
                    ui.end_row();
                });
            ui.separator();
            about(ui, actions);
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

/// egui's zoom limits, which its zoom functions clamp to.
const MIN_ZOOM: f32 = 0.2;
const MAX_ZOOM: f32 = 5.0;

/// Zoom out, the zoom in percent, zoom in and reset, in egui's 10 % steps; the same as
/// Ctrl + −, Ctrl + + and Ctrl + 0.
fn zoom_row(ui: &mut egui::Ui) {
    use egui::gui_zoom::{self, kb_shortcuts};

    ui.label("Zoom");
    ui.horizontal(|ui| {
        let ctx = ui.ctx().clone();
        let zoom = ctx.zoom_factor();
        let button = |ui: &mut egui::Ui, enabled: bool, text: &str, name: &str, shortcut| {
            let response = ui
                .add_enabled(enabled, egui::Button::new(text))
                .on_hover_text(format!("{name} ({})", ctx.format_shortcut(shortcut)));
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
            response.clicked()
        };
        if button(
            ui,
            zoom > MIN_ZOOM,
            icons::MINUS,
            "Zoom out",
            &kb_shortcuts::ZOOM_OUT,
        ) {
            gui_zoom::zoom_out(&ctx);
        }
        ui.label(format!("{:.0} %", zoom * 100.0));
        if button(
            ui,
            zoom < MAX_ZOOM,
            icons::PLUS,
            "Zoom in",
            &kb_shortcuts::ZOOM_IN,
        ) {
            gui_zoom::zoom_in(&ctx);
        }
        if button(
            ui,
            zoom != 1.0,
            "Reset",
            "Reset zoom",
            &kb_shortcuts::ZOOM_RESET,
        ) {
            ctx.set_zoom_factor(1.0);
        }
    });
    ui.end_row();
}

/// The switch for desktop notifications about failed instances.
fn notify_row(ui: &mut egui::Ui, notify: bool, actions: &mut Vec<Action>) {
    ui.label(format!("{} Notifications", icons::BELL));
    let mut on = notify;
    ui.horizontal(|ui| {
        let hint = "A desktop notification when an instance fails while Senzor is in the \
                    background, for example when the broker refuses its connection";
        if switch(ui, &mut on, "Notify me when an instance fails")
            .on_hover_text(hint)
            .clicked()
        {
            actions.push(Action::SetNotifyFailures(on));
        }
        ui.label("when an instance fails in the background");
    });
    ui.end_row();
}

/// The switch for the daily look for a newer release.
fn updates_row(ui: &mut egui::Ui, check: bool, actions: &mut Vec<Action>) {
    ui.label(format!("{} Updates", icons::ARROW_CIRCLE_UP));
    let mut on = check;
    ui.horizontal(|ui| {
        let hint = "Asks GitHub once a day whether a newer release exists; nothing about you or \
                    your projects is sent";
        if switch(ui, &mut on, "Check for updates")
            .on_hover_text(hint)
            .clicked()
        {
            actions.push(Action::SetCheckUpdates(on));
        }
        ui.label("say when a newer version is out");
    });
    ui.end_row();
}

/// Whether agents on other machines may connect, and the token they need.
fn remote_rows(ui: &mut egui::Ui, settings: &AppSettings, actions: &mut Vec<Action>) {
    ui.label("");
    let mut remote = settings.mcp_remote;
    ui.horizontal(|ui| {
        let hint = "MCP listens on every address of this machine, and agents need a token. \
                    Anyone on your network who has the token can control this window.";
        if switch(ui, &mut remote, "MCP from other machines")
            .on_hover_text(hint)
            .clicked()
        {
            actions.push(Action::SetMcpRemote(remote));
        }
        ui.label("let agents on other machines connect, with a token");
    });
    ui.end_row();
    if settings.mcp_remote {
        ui.label("Token");
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("••••••••").monospace())
                .on_hover_text("Agents send it as Authorization: Bearer <token>");
            if ui.button("Copy token").clicked() {
                actions.push(Action::CopyMcpToken);
            }
            if ui
                .button("New token")
                .on_hover_text("Agents that use the old token can no longer connect")
                .clicked()
            {
                actions.push(Action::NewMcpToken);
            }
        });
        ui.end_row();
    }
}

/// The MCP toggle, with the address agents connect to while it is on, or why it is off.
fn mcp_row(ui: &mut egui::Ui, mcp: &McpStatus, settings: &AppSettings, actions: &mut Vec<Action>) {
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

    let label = ui.label("MCP port");
    let mut port = match mcp {
        McpStatus::Listening(address) => address.port(),
        McpStatus::Off { port, .. } => *port,
    };
    let listening = matches!(mcp, McpStatus::Listening(_));
    let response = ui
        .add_enabled(
            !listening,
            egui::DragValue::new(&mut port)
                .range(1..=65535)
                .clamp_existing_to_range(false),
        )
        .labelled_by(label.id)
        .on_hover_text("The port AI agents connect to")
        .on_disabled_hover_text("Turn MCP off to change its port");
    if response.changed() {
        actions.push(Action::SetMcpPort(port));
    }
    ui.end_row();

    remote_rows(ui, settings, actions);

    match mcp {
        McpStatus::Listening(address) if address.ip().is_unspecified() => {
            ui.label("Address");
            ui.label(format!(
                "this machine's name or address, port {}",
                address.port()
            ))
            .on_hover_text(format!(
                "Agents connect to http://<this machine>:{}/mcp with the token",
                address.port()
            ));
            ui.end_row();
        }
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

/// The open-source components built into the window: name, licence and project page.
const CREDITS: &[(&str, &str, &str)] = &[
    (
        "egui and eframe",
        "MIT or Apache-2.0",
        "https://github.com/emilk/egui",
    ),
    ("Phosphor Icons", "MIT", "https://phosphoricons.com"),
    (
        "egui-phosphor",
        "MIT or Apache-2.0",
        "https://github.com/amPerl/egui-phosphor",
    ),
    (
        "Ubuntu font",
        "Ubuntu Font Licence 1.0",
        "https://design.ubuntu.com/font",
    ),
    ("Hack font", "MIT", "https://github.com/source-foundry/Hack"),
    (
        "winit",
        "Apache-2.0",
        "https://github.com/rust-windowing/winit",
    ),
    (
        "wgpu",
        "MIT or Apache-2.0",
        "https://github.com/gfx-rs/wgpu",
    ),
    ("rfd", "MIT", "https://github.com/PolyMeilex/rfd"),
    (
        "notify-rust",
        "MIT or Apache-2.0",
        "https://github.com/hoodie/notify-rust",
    ),
    (
        "rumqttc",
        "Apache-2.0",
        "https://github.com/bytebeamio/rumqtt",
    ),
    ("Tokio", "MIT", "https://github.com/tokio-rs/tokio"),
    (
        "MiniJinja",
        "Apache-2.0",
        "https://github.com/mitsuhiko/minijinja",
    ),
    (
        "rmcp",
        "Apache-2.0",
        "https://github.com/modelcontextprotocol/rust-sdk",
    ),
    ("axum", "MIT", "https://github.com/tokio-rs/axum"),
];

/// Links to the documentation and the source, and the credits, folded at first.
fn about(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        if ui.link("What's new").clicked() {
            actions.push(Action::ShowWhatsNew);
        }
        ui.label("·");
        ui.hyperlink_to("Documentation", super::DOCS_URL);
        ui.label("·");
        ui.hyperlink_to("Source code", super::SOURCE_URL);
    });
    ui.collapsing("Open-source credits", |ui| {
        ui.label("Senzor is built with these projects, and the Rust crates they depend on:");
        egui::Grid::new("credits").num_columns(2).show(ui, |ui| {
            for (name, licence, url) in CREDITS {
                ui.hyperlink_to(*name, *url);
                ui.weak(*licence);
                ui.end_row();
            }
        });
    });
}
