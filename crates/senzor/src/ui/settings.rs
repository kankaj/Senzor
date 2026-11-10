use eframe::egui;
use egui_phosphor::regular as icons;

use super::switch::switch;
use super::{Action, ERROR, UiState};
use crate::app::McpStatus;
use crate::i18n::{Language, t};
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
    egui::Window::new(t!("settings-title"))
        .id(egui::Id::new("settings-window"))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            egui::Grid::new("settings-form")
                .num_columns(2)
                .show(ui, |ui| {
                    language_row(ui, settings.language, actions);
                    theme_row(ui, settings.theme, actions);
                    zoom_row(ui);
                    notify_row(ui, settings.notify_failures, actions);
                    mcp_row(ui, mcp, settings, actions);

                    updates_row(ui, settings.check_updates, actions);

                    ui.label(t!("settings-version"));
                    ui.horizontal(|ui| {
                        ui.label(env!("CARGO_PKG_VERSION"));
                        if let Some(release) = newer {
                            ui.label("·");
                            ui.hyperlink_to(
                                t!("settings-release-out", version = release.version.as_str()),
                                &release.url,
                            )
                            .on_hover_text(t!("settings-release-out-hint"));
                        }
                    });
                    ui.end_row();

                    ui.label(t!("settings-licence"));
                    ui.hyperlink_to(
                        env!("CARGO_PKG_LICENSE"),
                        format!("{}/blob/main/LICENSE", super::SOURCE_URL),
                    );
                    ui.end_row();

                    ui.label(t!("settings-author"));
                    ui.label(format!("© 2026 {}", env!("CARGO_PKG_AUTHORS")));
                    ui.end_row();
                });
            ui.separator();
            about(ui, actions);
        });
    ui_state.settings_open = open;
}

/// One button per language, each named in itself; the chosen one is highlighted.
fn language_row(ui: &mut egui::Ui, current: Language, actions: &mut Vec<Action>) {
    ui.label(format!("{} {}", icons::TRANSLATE, t!("settings-language")));
    ui.horizontal(|ui| {
        for language in Language::ALL {
            let chosen = language == current;
            if ui
                .selectable_label(chosen, language.native_name())
                .clicked()
                && !chosen
            {
                actions.push(Action::SetLanguage(language));
            }
        }
    });
    ui.end_row();
}

/// One button per theme; the chosen one is highlighted.
fn theme_row(ui: &mut egui::Ui, current: Theme, actions: &mut Vec<Action>) {
    ui.label(t!("settings-theme"));
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

    ui.label(t!("settings-zoom"));
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
            &t!("settings-zoom-out"),
            &kb_shortcuts::ZOOM_OUT,
        ) {
            gui_zoom::zoom_out(&ctx);
        }
        ui.label(format!("{:.0} %", zoom * 100.0));
        if button(
            ui,
            zoom < MAX_ZOOM,
            icons::PLUS,
            &t!("settings-zoom-in"),
            &kb_shortcuts::ZOOM_IN,
        ) {
            gui_zoom::zoom_in(&ctx);
        }
        if button(
            ui,
            zoom != 1.0,
            &t!("settings-zoom-reset"),
            &t!("settings-zoom-reset-name"),
            &kb_shortcuts::ZOOM_RESET,
        ) {
            ctx.set_zoom_factor(1.0);
        }
    });
    ui.end_row();
}

/// The switch for desktop notifications about failed instances.
fn notify_row(ui: &mut egui::Ui, notify: bool, actions: &mut Vec<Action>) {
    ui.label(format!("{} {}", icons::BELL, t!("settings-notifications")));
    let mut on = notify;
    ui.horizontal(|ui| {
        let hint = t!("settings-notifications-hint");
        if switch(ui, &mut on, &t!("settings-notifications-switch"))
            .on_hover_text(hint)
            .clicked()
        {
            actions.push(Action::SetNotifyFailures(on));
        }
        ui.label(t!("settings-notifications-text"));
    });
    ui.end_row();
}

/// The switch for the daily look for a newer release.
fn updates_row(ui: &mut egui::Ui, check: bool, actions: &mut Vec<Action>) {
    ui.label(format!(
        "{} {}",
        icons::ARROW_CIRCLE_UP,
        t!("settings-updates")
    ));
    let mut on = check;
    ui.horizontal(|ui| {
        let hint = t!("settings-updates-hint");
        if switch(ui, &mut on, &t!("settings-updates-switch"))
            .on_hover_text(hint)
            .clicked()
        {
            actions.push(Action::SetCheckUpdates(on));
        }
        ui.label(t!("settings-updates-text"));
    });
    ui.end_row();
}

/// Whether agents on other machines may connect, and the token they need.
fn remote_rows(ui: &mut egui::Ui, settings: &AppSettings, actions: &mut Vec<Action>) {
    ui.label("");
    let mut remote = settings.mcp_remote;
    ui.horizontal(|ui| {
        let hint = t!("settings-mcp-remote-hint");
        if switch(ui, &mut remote, &t!("settings-mcp-remote-switch"))
            .on_hover_text(hint)
            .clicked()
        {
            actions.push(Action::SetMcpRemote(remote));
        }
        ui.label(t!("settings-mcp-remote-text"));
    });
    ui.end_row();
    if settings.mcp_remote {
        ui.label(t!("settings-token"));
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("••••••••").monospace())
                .on_hover_text(t!("settings-token-hint"));
            if ui.button(t!("settings-copy-token")).clicked() {
                actions.push(Action::CopyMcpToken);
            }
            if ui
                .button(t!("settings-new-token"))
                .on_hover_text(t!("settings-new-token-hint"))
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
            t!("settings-mcp-hint-on", address = address.to_string())
        }
        McpStatus::Off { port, error: None } => {
            t!("settings-mcp-hint-off", port = *port)
        }
        McpStatus::Off {
            port,
            error: Some(error),
        } => t!(
            "settings-mcp-hint-failed",
            error = error.as_str(),
            port = *port
        ),
    };
    ui.label(format!("{} MCP", icons::ROBOT));
    let mut on = matches!(mcp, McpStatus::Listening(_));
    ui.horizontal(|ui| {
        if switch(ui, &mut on, "MCP").on_hover_text(hint).clicked() {
            actions.push(Action::ToggleMcp);
        }
        ui.label(t!("settings-mcp-text"));
    });
    ui.end_row();

    let label = ui.label(t!("settings-mcp-port"));
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
        .on_hover_text(t!("settings-mcp-port-hint"))
        .on_disabled_hover_text(t!("settings-mcp-port-disabled"));
    if response.changed() {
        actions.push(Action::SetMcpPort(port));
    }
    ui.end_row();

    remote_rows(ui, settings, actions);

    match mcp {
        McpStatus::Listening(address) if address.ip().is_unspecified() => {
            ui.label(t!("settings-address"));
            ui.label(t!("settings-address-any", port = address.port()))
                .on_hover_text(t!("settings-address-any-hint", port = address.port()));
            ui.end_row();
        }
        McpStatus::Listening(address) => {
            ui.label(t!("settings-address"));
            ui.label(address.to_string());
            ui.end_row();
        }
        McpStatus::Off {
            error: Some(error), ..
        } => {
            ui.label("");
            ui.colored_label(ERROR, t!("settings-mcp-failed", error = error.as_str()));
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
        if ui.link(t!("settings-whats-new")).clicked() {
            actions.push(Action::ShowWhatsNew);
        }
        ui.label("·");
        ui.hyperlink_to(t!("settings-documentation"), super::DOCS_URL);
        ui.label("·");
        ui.hyperlink_to(t!("settings-source-code"), super::SOURCE_URL);
    });
    egui::CollapsingHeader::new(t!("settings-credits"))
        .id_salt("settings-credits")
        .show(ui, |ui| {
            ui.label(t!("settings-credits-text"));
            egui::Grid::new("credits").num_columns(2).show(ui, |ui| {
                for (name, licence, url) in CREDITS {
                    ui.hyperlink_to(*name, *url);
                    // "MIT or Apache-2.0": only the "or" is a word.
                    ui.weak(licence.replace(" or ", &format!(" {} ", t!("settings-licence-or"))));
                    ui.end_row();
                }
            });
        });
}
