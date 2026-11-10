use std::path::PathBuf;

use eframe::egui;
use egui_phosphor::regular as icons;

use super::{Action, recent_project_button};
use crate::document::Document;
use crate::examples::EXAMPLES;
use crate::i18n::t;

/// The welcome screen replaces the detail panel's hint for a new project that is still blank:
/// no file, no changes, no devices and no templates. That is how the window starts without a
/// file, and what New gives.
pub fn applies(doc: &Document) -> bool {
    doc.path().is_none()
        && !doc.is_dirty()
        && doc.file().devices.is_empty()
        && doc.file().templates.is_empty()
}

/// A way to start: add a device, open a project, set the broker, try an example, reopen a recent
/// project, or read the docs.
pub fn show(ui: &mut egui::Ui, recent: &[PathBuf], actions: &mut Vec<Action>) {
    ui.add_space(16.0);
    ui.vertical_centered(|ui| {
        ui.heading(egui::RichText::new(t!("welcome-title")).size(24.0));
        ui.label(t!("welcome-tagline"));
        ui.add_space(12.0);
        centered_row(ui, "start", |ui| {
            if start_button(ui, icons::PLUS, &t!("welcome-add-device")).clicked() {
                actions.push(Action::AddDevice);
            }
            if start_button(ui, icons::FOLDER_OPEN, &t!("welcome-open")).clicked() {
                actions.push(Action::Open);
            }
            if start_button(ui, icons::PLUGS_CONNECTED, &t!("welcome-broker")).clicked() {
                actions.push(Action::OpenBroker);
            }
        });

        ui.add_space(16.0);
        ui.strong(t!("welcome-examples"));
        centered_row(ui, "examples", |ui| {
            for (index, example) in EXAMPLES.iter().enumerate() {
                if ui
                    .button(example.title())
                    .on_hover_text(example.description())
                    .clicked()
                {
                    actions.push(Action::OpenExample(index));
                }
            }
        });

        ui.add_space(16.0);
        ui.strong(t!("welcome-recent"));
        if recent.is_empty() {
            ui.weak(t!("welcome-no-recent"));
        }
        for path in recent {
            if recent_project_button(ui, path).clicked() {
                actions.push(Action::OpenRecent(path.clone()));
            }
        }

        ui.add_space(16.0);
        centered_row(ui, "links", |ui| {
            ui.hyperlink_to(
                format!("{} {}", icons::ROCKET_LAUNCH, t!("welcome-quick-start")),
                format!("{}/quick-start", super::DOCS_URL),
            );
            ui.hyperlink_to(
                format!("{} {}", icons::BOOK_OPEN, t!("welcome-documentation")),
                super::DOCS_URL,
            );
        });
    });
}

/// Widgets side by side, centred, wrapping onto more rows in a narrow window. egui lays out
/// left to right, so the row is indented by half the room its width left in the previous frame.
fn centered_row(ui: &mut egui::Ui, salt: &str, add: impl FnOnce(&mut egui::Ui)) {
    let id = ui.id().with(salt);
    let width: f32 = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
    let indent = ((ui.available_width() - width) / 2.0).max(0.0);
    let row = ui.horizontal_wrapped(|ui| {
        ui.add_space(indent);
        let start = ui.cursor().min.x;
        add(ui);
        ui.min_rect().max.x - start
    });
    ui.ctx().data_mut(|d| d.insert_temp(id, row.inner));
}

/// A larger button with `icon` before `label`; screen readers and tests know it by `label`.
fn start_button(ui: &mut egui::Ui, icon: &str, label: &str) -> egui::Response {
    let response = ui.add(
        egui::Button::new(egui::RichText::new(format!("{icon} {label}")).size(15.0))
            .min_size(egui::vec2(0.0, 28.0)),
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::ui::test_support::Fixture;

    #[test]
    fn the_welcome_screen_speaks_czech() {
        crate::i18n::set_language(crate::i18n::Language::Czech);
        let mut h = Fixture::new("").harness(|ui, fx| show(ui, &[], &mut fx.actions));
        for name in [
            "Vítá vás Senzor",
            "Příklady",
            "Nedávné projekty",
            "Zatím žádné nedávné projekty.",
        ] {
            assert!(h.query_by_label(name).is_some(), "{name}");
        }
        h.get_by_label("Přidat první zařízení").click();
        h.run();
        assert!(
            h.state()
                .other
                .iter()
                .any(|a| matches!(a, Action::AddDevice))
        );
    }
}
