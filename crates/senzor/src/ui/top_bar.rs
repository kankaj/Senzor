use std::net::SocketAddr;
use std::path::PathBuf;

use eframe::egui;
use egui_phosphor::regular as icons;

use super::{Action, shortcuts};
use crate::document::Document;
use crate::i18n::t;

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
                ui.strong(super::project_title(doc));
                ui.separator();
                let broker = &doc.file().broker;
                let address = format!("{}:{}", broker.host, broker.port);
                let name = t!("top-bar-broker", address = address);
                if icon_button(ui, icons::PLUGS_CONNECTED, &name).clicked() {
                    actions.push(Action::OpenBroker);
                }
                ui.separator();
                file_buttons(ui, doc, recent, actions);
                ui.separator();
                run_buttons(ui, starting, actions);
            });
        },
        |ui| {
            let name = t!("top-bar-settings");
            let cog = ui
                .button(egui::RichText::new(icons::GEAR_SIX).size(16.0))
                .on_hover_text(shortcuts::tip(ui.ctx(), &name, &shortcuts::SETTINGS));
            // Screen readers and tests find it by name, not by the glyph.
            cog.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
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

/// New, Open, the recent and example projects, Save, Save as, TOML, Undo and Redo.
fn file_buttons(ui: &mut egui::Ui, doc: &Document, recent: &[PathBuf], actions: &mut Vec<Action>) {
    if icon_button(ui, icons::FILE_PLUS, &t!("top-bar-new"))
        .on_hover_text(shortcuts::tip(
            ui.ctx(),
            &t!("top-bar-new-hint"),
            &shortcuts::NEW,
        ))
        .clicked()
    {
        actions.push(Action::New);
    }
    if icon_button(ui, icons::FOLDER_OPEN, &t!("top-bar-open"))
        .on_hover_text(shortcuts::tip(
            ui.ctx(),
            &t!("top-bar-open-hint"),
            &shortcuts::OPEN,
        ))
        .clicked()
    {
        actions.push(Action::Open);
    }
    recent_menu(ui, recent, actions);
    examples_menu(ui, actions);
    if save_button(ui, doc.is_dirty()).clicked() {
        actions.push(Action::Save);
    }
    if icon_button(ui, icons::FLOPPY_DISK_BACK, &t!("top-bar-save-as"))
        .on_hover_text(shortcuts::tip(
            ui.ctx(),
            &t!("top-bar-save-as-hint"),
            &shortcuts::SAVE_AS,
        ))
        .clicked()
    {
        actions.push(Action::SaveAs);
    }
    if icon_button(ui, icons::FILE_CODE, "TOML")
        .on_hover_text(t!("top-bar-toml-hint"))
        .clicked()
    {
        actions.push(Action::Select(Some(crate::selection::Selection::Source)));
    }
    undo_redo(ui, doc, actions);
}

/// Start all and Stop all, and the hint while devices wait to start.
fn run_buttons(ui: &mut egui::Ui, starting: bool, actions: &mut Vec<Action>) {
    if icon_button(ui, icons::PLAY, &t!("top-bar-start-all"))
        .on_hover_text(shortcuts::tip(
            ui.ctx(),
            &t!("top-bar-start-all-hint"),
            &shortcuts::START_ALL,
        ))
        .clicked()
    {
        actions.push(Action::StartAll);
    }
    if icon_button(ui, icons::STOP, &t!("top-bar-stop-all"))
        .on_hover_text(shortcuts::tip(
            ui.ctx(),
            &t!("top-bar-stop-all-hint"),
            &shortcuts::STOP_ALL,
        ))
        .clicked()
    {
        actions.push(Action::StopAll);
    }
    if starting {
        ui.weak(t!("top-bar-starting"))
            .on_hover_text(t!("top-bar-starting-hint"));
    }
}

/// Save, filled with the accent colour while the project has unsaved changes.
fn save_button(ui: &mut egui::Ui, dirty: bool) -> egui::Response {
    let name = t!("top-bar-save");
    let mut button = egui::Button::new(format!("{} {name}", icons::FLOPPY_DISK));
    if dirty {
        button = button.fill(ui.visuals().selection.bg_fill);
    }
    let response = ui.add(button);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
    let text = if dirty {
        t!("top-bar-save-hint-dirty")
    } else {
        name
    };
    response.on_hover_text(shortcuts::tip(ui.ctx(), &text, &shortcuts::SAVE))
}

/// A robot in the accent colour while AI agents can control the window; it opens the settings,
/// where MCP is turned off.
fn mcp_sign(ui: &mut egui::Ui, address: SocketAddr) -> egui::Response {
    let robot = egui::RichText::new(icons::ROBOT)
        .size(16.0)
        .color(ui.visuals().selection.stroke.color);
    let address = address.to_string();
    let response = ui
        .button(robot)
        .on_hover_text(t!("top-bar-mcp-hint", address = address));
    let name = t!("top-bar-mcp-on");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
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
            ui.weak(t!("top-bar-no-recent"));
            return;
        }
        for path in recent {
            if super::recent_project_button(ui, path).clicked() {
                actions.push(Action::OpenRecent(path.clone()));
            }
        }
        ui.separator();
        if ui.button(t!("top-bar-clear-list")).clicked() {
            actions.push(Action::ClearRecent);
        }
    });
    let response = menu.response.on_hover_text(t!("top-bar-recent-hint"));
    let name = t!("top-bar-recent");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
}

/// The Examples menu, books next to Recent, listing the example projects.
fn examples_menu(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    let menu = ui.menu_button(icons::BOOKS, |ui| {
        for (index, example) in crate::examples::EXAMPLES.iter().enumerate() {
            if ui
                .button(example.title())
                .on_hover_text(example.description())
                .clicked()
            {
                actions.push(Action::OpenExample(index));
            }
        }
    });
    let response = menu.response.on_hover_text(t!("top-bar-examples-hint"));
    let name = t!("top-bar-examples");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
}

/// An Undo or Redo button: icon, name, the hint while it is off, enabled, shortcut and action.
type HistoryButton = (
    &'static str,
    String,
    String,
    bool,
    &'static egui::KeyboardShortcut,
    fn() -> Action,
);

/// Undo and Redo as icons, off while there is nothing to undo or redo.
fn undo_redo(ui: &mut egui::Ui, doc: &Document, actions: &mut Vec<Action>) {
    let buttons: [HistoryButton; 2] = [
        (
            icons::ARROW_COUNTER_CLOCKWISE,
            t!("top-bar-undo"),
            t!("top-bar-nothing-to-undo"),
            doc.can_undo(),
            &shortcuts::UNDO,
            || Action::Undo,
        ),
        (
            icons::ARROW_CLOCKWISE,
            t!("top-bar-redo"),
            t!("top-bar-nothing-to-redo"),
            doc.can_redo(),
            &shortcuts::REDO,
            || Action::Redo,
        ),
    ];
    for (icon, name, nothing, enabled, shortcut, action) in buttons {
        let response = ui
            .add_enabled(enabled, egui::Button::new(icon))
            .on_hover_text(shortcuts::tip(ui.ctx(), &name, shortcut))
            .on_disabled_hover_text(nothing);
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &name));
        if response.clicked() {
            actions.push(action());
        }
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::i18n::{Language, set_language};
    use crate::ui::test_support::Fixture;

    #[test]
    fn the_top_bar_speaks_czech() {
        set_language(Language::Czech);
        let mut h = Fixture::new("").harness(|ui, fx| {
            show(ui, &fx.doc, &[], false, None, &mut fx.actions);
        });
        for name in [
            "Nový",
            "Otevřít",
            "Uložit",
            "Uložit jako",
            "Spustit vše",
            "Nastavení",
            "Zpět",
        ] {
            assert!(h.query_by_label(name).is_some(), "{name}");
        }
        h.get_by_label("Zastavit vše").click();
        h.run();
        assert!(h.state().other.iter().any(|a| matches!(a, Action::StopAll)));
    }
}
