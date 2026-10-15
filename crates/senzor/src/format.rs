//! Text shown in the GUI for values, statuses and times.

use std::time::{Duration, SystemTime};

use senzor_core::engine::InstanceStatus;
use senzor_core::value::Value;

/// A field value as shown in the instance view: integral numbers plain (below 9e15, see
/// [`Value::as_integer`]), others with 2 decimals.
pub fn value(value: &Value) -> String {
    match value {
        Value::Number(n) if value.as_integer().is_none() => format!("{n:.2}"),
        other => other.to_input(),
    }
}

/// One word per status, plus the error or reason where there is one.
pub fn status(status: &InstanceStatus) -> (&'static str, Option<String>) {
    match status {
        InstanceStatus::Stopped => ("Stopped", None),
        InstanceStatus::Connecting => ("Connecting", None),
        InstanceStatus::Online => ("Online", None),
        InstanceStatus::Reconnecting {
            attempt,
            last_error,
        } => (
            "Reconnecting",
            Some(format!("attempt {attempt}: {last_error}")),
        ),
        InstanceStatus::Failed { reason } => ("Failed", Some(reason.clone())),
    }
}

/// "3 s ago", "4 min ago", "2 h ago"; "just now" under a second.
pub fn ago(then: SystemTime, now: SystemTime) -> String {
    let elapsed = now.duration_since(then).unwrap_or(Duration::ZERO).as_secs();
    match elapsed {
        0 => "just now".to_string(),
        1..=119 => format!("{elapsed} s ago"),
        120..=7199 => format!("{} min ago", elapsed / 60),
        _ => format!("{} h ago", elapsed / 3600),
    }
}

/// UTC wall-clock time with milliseconds, e.g. `14:02:11.123Z`.
pub fn clock(time: SystemTime) -> String {
    let millis = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis();
    let day_millis = millis % 86_400_000;
    format!(
        "{:02}:{:02}:{:02}.{:03}Z",
        day_millis / 3_600_000,
        day_millis / 60_000 % 60,
        day_millis / 1000 % 60,
        day_millis % 1000
    )
}

/// Longest payload shown in one message log row.
pub const LOG_PAYLOAD_CHARS: usize = 200;

/// A payload as one log row: whitespace runs (including newlines) become one space, and
/// anything past [`LOG_PAYLOAD_CHARS`] is cut with `…`.
pub fn log_payload(payload: &str) -> String {
    let one_line = payload.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > LOG_PAYLOAD_CHARS {
        let mut shown: String = one_line.chars().take(LOG_PAYLOAD_CHARS).collect();
        shown.push('…');
        shown
    } else {
        one_line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_compact() {
        assert_eq!(value(&Value::Number(1200.0)), "1200");
        assert_eq!(value(&Value::Number(22.6789)), "22.68");
        assert_eq!(value(&Value::Bool(true)), "true");
        assert_eq!(value(&Value::String("open".into())), "open");
    }

    #[test]
    fn statuses_carry_their_detail() {
        assert_eq!(status(&InstanceStatus::Online), ("Online", None));
        assert_eq!(
            status(&InstanceStatus::Reconnecting {
                attempt: 2,
                last_error: "refused".into()
            }),
            ("Reconnecting", Some("attempt 2: refused".into()))
        );
        assert_eq!(
            status(&InstanceStatus::Failed {
                reason: "bad credentials".into()
            }),
            ("Failed", Some("bad credentials".into()))
        );
    }

    #[test]
    fn ago_picks_a_readable_unit() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(100_000);
        let before = |secs| now - Duration::from_secs(secs);
        assert_eq!(ago(now, now), "just now");
        assert_eq!(ago(before(3), now), "3 s ago");
        assert_eq!(ago(before(600), now), "10 min ago");
        assert_eq!(ago(before(7200), now), "2 h ago");
        assert_eq!(ago(now + Duration::from_secs(5), now), "just now");
    }

    #[test]
    fn clock_is_utc_time_of_day() {
        let t = SystemTime::UNIX_EPOCH + Duration::from_millis(1_700_000_000_123);
        assert_eq!(clock(t), "22:13:20.123Z");
    }

    #[test]
    fn log_payloads_are_one_line_and_shortened() {
        assert_eq!(log_payload("{\n  \"co2\": 600\n}"), "{ \"co2\": 600 }");
        let shown = log_payload(&"x".repeat(300));
        assert_eq!(shown.chars().count(), 201);
        assert!(shown.ends_with('…'));
    }
}
