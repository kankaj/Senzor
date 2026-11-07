//! The message log's filter, its export to CSV and how a selected message's payload is shown.

use senzor_core::engine::LogEntry;

/// Which messages the log shows: those whose topic contains `topic` (case-sensitive) and, when
/// set, those of one instance.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogFilter {
    pub topic: String,
    pub instance: Option<String>,
}

impl LogFilter {
    pub fn matches(&self, entry: &LogEntry) -> bool {
        entry.topic.contains(&self.topic)
            && self
                .instance
                .as_ref()
                .is_none_or(|instance| &entry.instance == instance)
    }
}

/// The messages as CSV with a header row, for a spreadsheet: time (RFC 3339, UTC), instance,
/// topic, QoS, retained and payload.
pub fn csv(entries: &[&LogEntry]) -> String {
    let mut out = String::from("time,instance,topic,qos,retained,payload\n");
    for entry in entries {
        let time = humantime::format_rfc3339_millis(entry.time).to_string();
        let qos = entry.qos.to_string();
        let retain = entry.retain.to_string();
        let cells = [
            time.as_str(),
            &entry.instance,
            &entry.topic,
            &qos,
            &retain,
            &entry.payload,
        ];
        let row: Vec<String> = cells.iter().map(|cell| csv_cell(cell)).collect();
        out.push_str(&row.join(","));
        out.push('\n');
    }
    out
}

/// A CSV cell: quoted, with quotes doubled, when it holds a comma, a quote or a line break.
fn csv_cell(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

/// A payload as the details show it: JSON objects and arrays indented, anything else as sent.
pub fn pretty_payload(payload: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(payload) {
        Ok(value @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) => {
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| payload.to_string())
        }
        _ => payload.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use super::*;

    fn entry(instance: &str, topic: &str, payload: &str) -> LogEntry {
        LogEntry {
            seq: 1,
            time: SystemTime::UNIX_EPOCH + Duration::from_millis(1_793_980_800_123),
            instance: instance.into(),
            topic: topic.into(),
            payload: payload.into(),
            qos: 1,
            retain: true,
            properties: Default::default(),
        }
    }

    #[test]
    fn the_filter_matches_topic_text_and_instance() {
        let office = entry("office-0", "b/office/state", "1");
        let lab = entry("lab-0", "b/lab/state", "1");
        let by_topic = LogFilter {
            topic: "office".into(),
            instance: None,
        };
        assert!(by_topic.matches(&office) && !by_topic.matches(&lab));
        let by_instance = LogFilter {
            topic: String::new(),
            instance: Some("lab-0".into()),
        };
        assert!(by_instance.matches(&lab) && !by_instance.matches(&office));
        assert!(LogFilter::default().matches(&office), "no filter shows all");
    }

    #[test]
    fn csv_has_a_header_and_quotes_what_needs_it() {
        let rows = [
            entry(
                "office-0",
                "b/office/state",
                r#"{"co2": 600, "note": "a ""b"""}"#,
            ),
            entry("lab-0", "b/lab/state", "plain"),
        ];
        let refs: Vec<&LogEntry> = rows.iter().collect();
        assert_eq!(
            csv(&refs),
            "time,instance,topic,qos,retained,payload\n\
             2026-11-06T16:00:00.123Z,office-0,b/office/state,1,true,\"{\"\"co2\"\": 600, \"\"note\"\": \"\"a \"\"\"\"b\"\"\"\"\"\"}\"\n\
             2026-11-06T16:00:00.123Z,lab-0,b/lab/state,1,true,plain\n"
        );
    }

    #[test]
    fn json_payloads_are_indented_and_others_kept() {
        assert_eq!(pretty_payload(r#"{"co2":600}"#), "{\n  \"co2\": 600\n}");
        assert_eq!(pretty_payload("[1,2]"), "[\n  1,\n  2\n]");
        assert_eq!(pretty_payload("600"), "600");
        assert_eq!(pretty_payload("{broken"), "{broken");
    }
}
