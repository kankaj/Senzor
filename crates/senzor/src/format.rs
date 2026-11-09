//! Text shown in the GUI for values, statuses and times.

use std::time::{Duration, SystemTime};

use senzor_core::engine::InstanceStatus;
use senzor_core::render::Payload;
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

/// Most bytes of a binary payload shown in one message log row.
pub const LOG_PAYLOAD_BYTES: usize = 16;

/// A payload as one log row. Text: whitespace runs (including newlines) become one space, and
/// anything past [`LOG_PAYLOAD_CHARS`] is cut with `…`. Binary: the encoding, the size and the
/// first [`LOG_PAYLOAD_BYTES`] bytes in hex.
pub fn log_payload(payload: &Payload) -> String {
    let Some(payload) = payload.as_text() else {
        let bytes = payload.bytes();
        let shown = spaced_hex(&bytes[..bytes.len().min(LOG_PAYLOAD_BYTES)]);
        let more = if bytes.len() > LOG_PAYLOAD_BYTES {
            " …"
        } else {
            ""
        };
        return format!(
            "{}, {}: {shown}{more}",
            payload.encoding().label(),
            byte_count(bytes.len())
        );
    };
    let one_line = payload.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > LOG_PAYLOAD_CHARS {
        let mut shown: String = one_line.chars().take(LOG_PAYLOAD_CHARS).collect();
        shown.push('…');
        shown
    } else {
        one_line
    }
}

/// `bytes` as a hex dump: `per_line` bytes a line, after the offset of the line's first byte.
pub fn hex_dump(bytes: &[u8], per_line: usize) -> String {
    let width = offset_width(bytes.len());
    bytes
        .chunks(per_line)
        .enumerate()
        .map(|(i, line)| format!("{:0width$x}  {}", i * per_line, spaced_hex(line)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// How many bytes a line of the dump of `len` bytes holds within `chars` characters: 16, else
/// 8, else 4.
pub fn hex_dump_line_bytes(len: usize, chars: usize) -> usize {
    let fits = |per_line: usize| offset_width(len) + 2 + per_line * 3 - 1 <= chars;
    [16, 8].into_iter().find(|&n| fits(n)).unwrap_or(4)
}

/// Four hex digits, or as many as the last offset of `len` bytes needs.
fn offset_width(len: usize) -> usize {
    format!("{:x}", len.saturating_sub(1)).len().max(4)
}

/// `bytes` as lowercase hex, a space between bytes.
pub fn spaced_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `n` bytes, as "1 byte" or "20 bytes".
pub fn byte_count(n: usize) -> String {
    if n == 1 {
        "1 byte".into()
    } else {
        format!("{n} bytes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use senzor_core::encoding::Encoding;

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
        let text = |t: &str| log_payload(&Payload::from(t));
        assert_eq!(text("{\n  \"co2\": 600\n}"), "{ \"co2\": 600 }");
        let shown = text(&"x".repeat(300));
        assert_eq!(shown.chars().count(), 201);
        assert!(shown.ends_with('…'));
    }

    fn binary(encoding: Encoding, bytes: Vec<u8>) -> Payload {
        Payload::Binary {
            encoding,
            bytes,
            text: String::new(),
        }
    }

    #[test]
    fn a_binary_log_payload_is_its_encoding_size_and_first_bytes() {
        assert_eq!(
            log_payload(&binary(Encoding::Cbor, vec![0xa1, 0x61, 0x76, 0x01])),
            "cbor, 4 bytes: a1 61 76 01"
        );
        assert_eq!(
            log_payload(&binary(Encoding::Hex, vec![0x0a])),
            "hex, 1 byte: 0a"
        );
        let long = log_payload(&binary(Encoding::Protobuf, vec![0xff; 40]));
        assert_eq!(
            long,
            format!(
                "protobuf, 40 bytes: {} …",
                vec!["ff"; LOG_PAYLOAD_BYTES].join(" ")
            )
        );
    }

    #[test]
    fn a_hex_dump_has_16_bytes_a_line_after_the_offset() {
        let bytes: Vec<u8> = (0..20).collect();
        assert_eq!(
            hex_dump(&bytes, 16),
            "0000  00 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f\n0010  10 11 12 13"
        );
        assert_eq!(hex_dump(&[], 16), "");
        assert!(
            hex_dump(&vec![0; 70_000], 16)
                .ends_with("\n11160  00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00")
        );
    }

    #[test]
    fn a_hex_dump_takes_fewer_bytes_a_line_when_16_do_not_fit() {
        let bytes: Vec<u8> = (0..20).collect();
        // "0000  " and 16 bytes need 53 characters.
        assert_eq!(hex_dump_line_bytes(bytes.len(), 53), 16);
        assert_eq!(hex_dump_line_bytes(bytes.len(), 52), 8);
        assert_eq!(hex_dump_line_bytes(bytes.len(), 10), 4);
        assert_eq!(
            hex_dump(&bytes, 8),
            "0000  00 01 02 03 04 05 06 07\n0008  08 09 0a 0b 0c 0d 0e 0f\n0010  10 11 12 13"
        );
    }
}
