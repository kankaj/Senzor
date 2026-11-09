//! The `?` next to a setting: what the setting does on hover, its documentation page on click.

use eframe::egui::{self, RichText};

/// What a setting does, and where the documentation explains it (a path under the docs site,
/// with an optional `#section`).
#[derive(Debug, Clone, Copy)]
pub struct Help {
    pub text: &'static str,
    pub page: &'static str,
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
        .on_hover_text(format!(
            "{}\n\nClick to read more in the documentation.",
            help.text
        ));
    mark.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Link, true, format!("About {name}"))
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

const fn help(text: &'static str, page: &'static str) -> Help {
    Help { text, page }
}

// Project settings.
pub const HOST: Help = help(
    "The broker's host name or IP address. Every instance of the project connects there.",
    "gui/broker-settings",
);
pub const PORT: Help = help(
    "The broker's port: 1883 is usual for plain MQTT, 8883 for TLS.",
    "gui/broker-settings",
);
pub const MQTT_VERSION: Help = help(
    "MQTT 5, the default, or 3.1.1 for a broker that does not support MQTT 5. It applies to \
     every instance.",
    "gui/broker-settings#mqtt-version",
);
pub const TRANSPORT: Help = help(
    "TCP, the usual MQTT connection, or MQTT over WebSocket for brokers behind a web server or \
     proxy. With TLS, WebSocket becomes wss://. Brokers take WebSocket on their own port, such \
     as 8080 or 8083.",
    "gui/broker-settings#transport",
);
pub const WEBSOCKET_PATH: Help = help(
    "The path the broker serves MQTT over WebSocket on, often /mqtt; it must start with /.",
    "gui/broker-settings#transport",
);
pub const CLIENT_ID_PREFIX: Help = help(
    "Each instance's client ID is this prefix followed by its instance ID, such as \
     senzor-office-0. A broker allows one connection per client ID.",
    "gui/broker-settings#client-id-prefix",
);
pub const KEEP_ALIVE: Help = help(
    "How often an instance tells the broker it is still there when it has nothing else to \
     send, so a dead connection is noticed. Whole seconds, such as 30s.",
    "gui/broker-settings#keep-alive",
);
pub const SESSION_EXPIRY: Help = help(
    "MQTT 5: how long the broker keeps an instance's session after its connection ends. Above \
     zero, a reconnect within the template's Will delay cancels the Last Will.",
    "gui/broker-settings#session-expiry",
);
pub const USERNAME: Help = help(
    "The username every instance logs in with. Leave it empty to connect without credentials.",
    "gui/broker-settings#username-and-password",
);
pub const PASSWORD: Help = help(
    "Where the password comes from: none, typed into the project file, or an environment \
     variable, which keeps it out of the file.",
    "gui/broker-settings#username-and-password",
);
pub const TLS: Help = help(
    "Connect over TLS. Without a CA file, the system's root certificates check the broker. \
     Brokers usually take TLS on another port, often 8883.",
    "gui/broker-settings#tls",
);
pub const SEED: Help = help(
    "With a seed, every run draws the same random values; left empty, each run draws fresh ones.",
    "concepts/reproducible-runs",
);

// Devices.
pub const DEVICE_ID: Help = help(
    "The device's name, unique in the project. Its instances are named after it unless \
     Instance IDs say otherwise.",
    "gui/editing-devices#settings",
);
pub const TEMPLATE: Help = help(
    "The template whose topics, payloads and fields the device publishes.",
    "gui/editing-devices#settings",
);
pub const COUNT: Help = help(
    "How many instances the device runs. Each has its own MQTT connection and client ID.",
    "concepts/templates-devices-instances#devices",
);
pub const INSTANCE_IDS: Help = help(
    "The instances' IDs, which topics use as {{ device_id }}: the device ID for one instance, \
     numbered from 0 for more. Tick custom to write a pattern.",
    "gui/editing-devices#instance-ids",
);
pub const AUTOSTART: Help = help(
    "Start the device as soon as the project opens.",
    "gui/running-devices#autostart",
);
pub const DEVICE_INTERVAL: Help = help(
    "Give this device its own publish interval instead of the template's.",
    "gui/editing-devices#interval-override",
);
pub const PARAMS: Help = help(
    "Values the template's topics and payloads use as {{ name }}, the same for every instance \
     of this device.",
    "gui/editing-devices#params",
);
pub const FIELD_OVERRIDES: Help = help(
    "Change a field of the template for this device only, such as another start value.",
    "gui/editing-devices#field-overrides",
);

// Templates.
pub const TEMPLATE_ID: Help = help(
    "The template's ID, which devices name to use it.",
    "gui/editing-templates#settings",
);
pub const TEMPLATE_INTERVAL: Help = help(
    "How often each instance publishes its messages, such as 10s.",
    "gui/editing-templates#durations",
);
pub const QOS: Help = help(
    "The MQTT quality of service of the data messages: 0 at most once, 1 at least once, \
     2 exactly once.",
    "gui/editing-templates#settings",
);
pub const RETAIN: Help = help(
    "The broker keeps each topic's last data message and hands it to new subscribers first.",
    "gui/editing-templates#settings",
);
pub const HOME_ASSISTANT: Help = help(
    "Each instance appears in Home Assistant as a device with one entity per field: when it \
     connects, it publishes Home Assistant's MQTT Discovery messages, retained.",
    "recipes/home-assistant-discovery",
);
pub const SUBSCRIPTIONS: Help = help(
    "Topics each instance subscribes to, such as a command topic. For every message that \
     arrives, the subscription's script runs: it can keep values in state, pin fields and \
     publish a reply.",
    "concepts/scripts",
);
pub const SUBSCRIPTION_TOPIC: Help = help(
    "The topic filter, rendered per instance like the availability topic; + and # match any \
     level. A reply published to a topic it matches comes back to the instance.",
    "concepts/scripts",
);
pub const SUBSCRIPTION_SCRIPT: Help = help(
    "Rhai code run for each message: it sees topic, payload, json, state, fields and params, \
     and can call publish, pin and unpin.",
    "concepts/scripts",
);
pub const FIELD_SCRIPT: Help = help(
    "Rhai code run on every tick; its last value is the field's. It sees value (the last one), \
     t, seq, state, fields and params.",
    "concepts/scripts",
);
pub const AVAILABILITY: Help = help(
    "An online/offline topic: online when an instance connects, offline when it stops, and \
     offline as its Last Will, which the broker publishes if the instance drops off.",
    "concepts/availability",
);
pub const WILL_DELAY: Help = help(
    "MQTT 5: how long the broker waits before it publishes the Last Will. A reconnect within \
     that time cancels it. The broker's Session expiry must be at least as long.",
    "concepts/availability#will-delay",
);
pub const TOPIC: Help = help(
    "Where the message is published. A Jinja template: {{ device_id }}, params and fields \
     fill it in.",
    "reference/template-variables",
);
pub const PAYLOAD: Help = help(
    "What the message carries, as a Jinja template, such as {\"co2\": {{ co2 }}}.",
    "reference/template-variables",
);
pub const ENCODING: Help = help(
    "How the rendered payload is sent: as text, or as bytes. hex and base64 read the text as \
     bytes; CBOR and protobuf read it as JSON.",
    "gui/editing-templates#encoding",
);
pub const PROTO_FILE: Help = help(
    "The .proto file that defines the message type, relative to the project file's folder.",
    "recipes/binary-payloads#protobuf",
);
pub const MESSAGE_TYPE: Help = help(
    "The fully qualified protobuf message type, such as sensors.v1.Reading: the package, a \
     dot and the message name.",
    "recipes/binary-payloads#protobuf",
);
pub const MESSAGE_EXPIRY: Help = help(
    "MQTT 5: after this time the broker drops the message instead of delivering it.",
    "gui/editing-templates#mqtt-5-properties",
);
pub const CONTENT_TYPE: Help = help(
    "MQTT 5: tells subscribers what the payload is, such as application/json.",
    "gui/editing-templates#mqtt-5-properties",
);
pub const USER_PROPERTIES: Help = help(
    "MQTT 5: name and value pairs sent with the message; values can use {{ }} like payloads.",
    "gui/editing-templates#mqtt-5-properties",
);
pub const FIELD_NAME: Help = help(
    "A readable name for the field, such as CO2, shown in the instance view. Left empty, the \
     field's key is shown.",
    "reference/project-file#fields",
);
pub const FIELD_UNIT: Help = help(
    "The unit of the field's value, such as ppm, °C or %, shown after the value in the \
     instance view.",
    "reference/project-file#fields",
);
pub const FIELDS: Help = help(
    "The values that change on every tick, each from a generator such as random_walk or sine.",
    "reference/generators",
);

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

    /// Every help links to a page of the docs site, so the links are checked like the site's.
    #[test]
    fn every_help_page_exists_in_the_docs() {
        let docs =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../website/content/docs");
        for help in [
            HOST,
            PORT,
            MQTT_VERSION,
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
        ] {
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
