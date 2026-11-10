//! The `?` next to a setting: what the setting does on hover, its documentation page on click.

use eframe::egui::{self, RichText};

use crate::i18n::{self, t};

/// What a setting does, and where the documentation explains it (a path under the docs site,
/// with an optional `#section`).
#[derive(Debug, Clone, Copy)]
pub struct Help {
    /// The text's key in `locales/*/help.ftl`.
    key: &'static str,
    pub page: &'static str,
}

impl Help {
    /// What the setting does, in the window's language.
    pub fn text(self) -> String {
        i18n::tr(self.key)
    }
}

/// The documentation page of `help`.
pub fn url(help: Help) -> String {
    format!("{}/{}", super::DOCS_URL, help.page)
}

/// A `?` that shows `help` on hover and opens its page on click. Screen readers and tests know it
/// as "About" followed by `name`.
pub fn mark(ui: &mut egui::Ui, name: &str, help: Help) -> egui::Response {
    let mark = ui
        .add(
            egui::Label::new(RichText::new(egui_phosphor::regular::QUESTION).weak())
                .sense(egui::Sense::click()),
        )
        .on_hover_text(format!("{}\n\n{}", help.text(), t!("help-read-more")));
    mark.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Link, true, t!("help-about", name = name))
    });
    if mark.clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url(help)));
    }
    mark
}

/// A form label followed by its `?`. Returns the label, for `labelled_by`.
pub fn label(ui: &mut egui::Ui, text: &str, help: Help) -> egui::Response {
    ui.horizontal(|ui| {
        let label = ui.label(text);
        mark(ui, text.trim(), help);
        label
    })
    .inner
}

/// A section heading followed by its `?`.
pub fn heading(ui: &mut egui::Ui, text: &str, help: Help) {
    ui.horizontal(|ui| {
        ui.strong(text);
        mark(ui, text, help);
    });
}

/// `key` is looked up when the `?` is drawn, so a test checks that each constant's key exists:
/// a new constant goes into the list `ALL` of the tests too.
const fn help(key: &'static str, page: &'static str) -> Help {
    Help { key, page }
}

// Project settings.
pub const HOST: Help = help("help-host", "gui/broker-settings");
pub const PORT: Help = help("help-port", "gui/broker-settings");
pub const MQTT_VERSION: Help = help("help-mqtt-version", "gui/broker-settings#mqtt-version");
pub const TRANSPORT: Help = help("help-transport", "gui/broker-settings#transport");
pub const WEBSOCKET_PATH: Help = help("help-websocket-path", "gui/broker-settings#transport");
pub const CLIENT_ID_PREFIX: Help = help(
    "help-client-id-prefix",
    "gui/broker-settings#client-id-prefix",
);
pub const KEEP_ALIVE: Help = help("help-keep-alive", "gui/broker-settings#keep-alive");
pub const SESSION_EXPIRY: Help = help("help-session-expiry", "gui/broker-settings#session-expiry");
pub const USERNAME: Help = help("help-username", "gui/broker-settings#username-and-password");
pub const PASSWORD: Help = help("help-password", "gui/broker-settings#username-and-password");
pub const TLS: Help = help("help-tls", "gui/broker-settings#tls");
pub const SEED: Help = help("help-seed", "concepts/reproducible-runs");

// Devices.
pub const DEVICE_ID: Help = help("help-device-id", "gui/editing-devices#settings");
pub const TEMPLATE: Help = help("help-template", "gui/editing-devices#settings");
pub const COUNT: Help = help("help-count", "concepts/templates-devices-instances#devices");
pub const INSTANCE_IDS: Help = help("help-instance-ids", "gui/editing-devices#instance-ids");
pub const AUTOSTART: Help = help("help-autostart", "gui/running-devices#autostart");
pub const DEVICE_INTERVAL: Help = help(
    "help-device-interval",
    "gui/editing-devices#interval-override",
);
pub const PARAMS: Help = help("help-params", "gui/editing-devices#params");
pub const FIELD_OVERRIDES: Help = help(
    "help-field-overrides",
    "gui/editing-devices#field-overrides",
);

// Templates.
pub const TEMPLATE_ID: Help = help("help-template-id", "gui/editing-templates#settings");
pub const TEMPLATE_INTERVAL: Help =
    help("help-template-interval", "gui/editing-templates#durations");
pub const QOS: Help = help("help-qos", "gui/editing-templates#settings");
pub const RETAIN: Help = help("help-retain", "gui/editing-templates#settings");
pub const HOME_ASSISTANT: Help = help("help-home-assistant", "recipes/home-assistant-discovery");
pub const SUBSCRIPTIONS: Help = help("help-subscriptions", "concepts/scripts");
pub const SUBSCRIPTION_TOPIC: Help = help("help-subscription-topic", "concepts/scripts");
pub const SUBSCRIPTION_SCRIPT: Help = help("help-subscription-script", "concepts/scripts");
pub const FIELD_SCRIPT: Help = help("help-field-script", "concepts/scripts");
pub const AVAILABILITY: Help = help("help-availability", "concepts/availability");
pub const WILL_DELAY: Help = help("help-will-delay", "concepts/availability#will-delay");
pub const TOPIC: Help = help("help-topic", "reference/template-variables");
pub const PAYLOAD: Help = help("help-payload", "reference/template-variables");
pub const ENCODING: Help = help("help-encoding", "gui/editing-templates#encoding");
pub const PROTO_FILE: Help = help("help-proto-file", "recipes/binary-payloads#protobuf");
pub const MESSAGE_TYPE: Help = help("help-message-type", "recipes/binary-payloads#protobuf");
pub const MESSAGE_EXPIRY: Help = help(
    "help-message-expiry",
    "gui/editing-templates#mqtt-5-properties",
);
pub const CONTENT_TYPE: Help = help(
    "help-content-type",
    "gui/editing-templates#mqtt-5-properties",
);
pub const USER_PROPERTIES: Help = help(
    "help-user-properties",
    "gui/editing-templates#mqtt-5-properties",
);
pub const FIELD_NAME: Help = help("help-field-name", "reference/project-file#fields");
pub const FIELD_UNIT: Help = help("help-field-unit", "reference/project-file#fields");
pub const FIELDS: Help = help("help-fields", "reference/generators");

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;

    #[test]
    fn the_mark_explains_on_hover_and_opens_the_docs_on_click() {
        let mut h = Harness::new_ui(|ui| {
            label(ui, "Keep alive", KEEP_ALIVE);
        });
        h.run();
        h.get_by_label("About Keep alive").hover();
        for _ in 0..10 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
        assert!(
            h.query_by_label_contains("How often an instance tells")
                .is_some()
        );

        h.get_by_label("About Keep alive").click();
        h.step();
        let opened = h.output().platform_output.commands.iter().any(|c| {
            matches!(c, egui::OutputCommand::OpenUrl(open) if open.url == "https://senzor.kankaj.cz/docs/gui/broker-settings#keep-alive")
        });
        assert!(opened);
    }

    #[test]
    fn the_mark_explains_in_czech_in_a_czech_window() {
        i18n::set_language(i18n::Language::Czech);
        let mut h = Harness::new_ui(|ui| {
            label(ui, "Udržování spojení", KEEP_ALIVE);
        });
        h.run();
        h.get_by_label("Nápověda: Udržování spojení").hover();
        for _ in 0..10 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(60));
        }
        assert!(
            h.query_by_label_contains("Jak často instance dává brokerovi vědět")
                .is_some()
        );
        assert!(
            h.query_by_label_contains("Kliknutím si přečtete víc v dokumentaci")
                .is_some()
        );
    }

    /// Every constant of this file, for the tests that check them all.
    const ALL: [Help; 42] = [
        HOST,
        PORT,
        MQTT_VERSION,
        TRANSPORT,
        WEBSOCKET_PATH,
        CLIENT_ID_PREFIX,
        KEEP_ALIVE,
        SESSION_EXPIRY,
        USERNAME,
        PASSWORD,
        TLS,
        SEED,
        DEVICE_ID,
        TEMPLATE,
        COUNT,
        INSTANCE_IDS,
        AUTOSTART,
        DEVICE_INTERVAL,
        PARAMS,
        FIELD_OVERRIDES,
        TEMPLATE_ID,
        TEMPLATE_INTERVAL,
        QOS,
        RETAIN,
        HOME_ASSISTANT,
        SUBSCRIPTIONS,
        SUBSCRIPTION_TOPIC,
        SUBSCRIPTION_SCRIPT,
        FIELD_SCRIPT,
        AVAILABILITY,
        WILL_DELAY,
        TOPIC,
        PAYLOAD,
        ENCODING,
        PROTO_FILE,
        MESSAGE_TYPE,
        MESSAGE_EXPIRY,
        CONTENT_TYPE,
        USER_PROPERTIES,
        FIELD_NAME,
        FIELD_UNIT,
        FIELDS,
    ];

    #[test]
    fn all_lists_every_help_of_this_file() {
        let source = include_str!("help.rs");
        let constants = source
            .lines()
            .filter(|line| line.starts_with("pub const ") && line.contains(": Help ="))
            .count();
        assert_eq!(constants, ALL.len());
    }

    /// The keys of `help` are looked up at run time, so `t!`'s check does not see them.
    #[test]
    fn every_help_has_a_text_in_each_language() {
        let locales = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("locales");
        for language in ["en", "cs"] {
            let file = locales.join(language).join("help.ftl");
            let text = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{file:?}: {e}"));
            let keys: Vec<&str> = text
                .lines()
                .filter_map(|line| line.split_once(" = ").map(|(key, _)| key))
                .collect();
            for help in ALL {
                assert!(keys.contains(&help.key), "{language}: no {}", help.key);
            }
        }
    }

    /// Every help links to a page of the docs site, so the links are checked like the site's.
    #[test]
    fn every_help_page_exists_in_the_docs() {
        let docs =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../website/content/docs");
        for help in ALL {
            let (page, section) = help.page.split_once('#').unwrap_or((help.page, ""));
            let text = std::fs::read_to_string(docs.join(format!("{page}.mdx")))
                .unwrap_or_else(|e| panic!("{page}: {e}"));
            if !section.is_empty() {
                let has_heading = text.lines().any(|line| {
                    let heading = line.trim_start_matches('#').trim();
                    line.starts_with('#') && slug(heading) == section
                });
                assert!(has_heading, "{page} has no section #{section}");
            }
        }
    }

    /// A heading's anchor as the docs site makes it: lower case, spaces as dashes, other
    /// punctuation dropped.
    fn slug(heading: &str) -> String {
        heading
            .to_lowercase()
            .chars()
            .filter_map(|c| match c {
                ' ' => Some('-'),
                c if c.is_alphanumeric() || c == '-' => Some(c),
                _ => None,
            })
            .collect()
    }
}
