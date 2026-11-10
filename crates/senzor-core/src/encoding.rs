//! How a message's rendered payload text becomes the bytes that are published.

use std::path::Path;

use crate::i18n::{Text, text};
pub use crate::project::file::Encoding;
use crate::project::file::MessageSpec;
use crate::proto::{self, ProtoError, ProtoMessage};
use crate::render::TemplateError;

/// Turns one message's rendered payload into the bytes that are published.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Encoder {
    #[default]
    Text,
    Hex,
    Base64,
    Cbor,
    Protobuf(ProtoMessage),
}

impl Encoder {
    pub fn encoding(&self) -> Encoding {
        match self {
            Encoder::Text => Encoding::Text,
            Encoder::Hex => Encoding::Hex,
            Encoder::Base64 => Encoding::Base64,
            Encoder::Cbor => Encoding::Cbor,
            Encoder::Protobuf(_) => Encoding::Protobuf,
        }
    }

    /// The bytes to publish for the rendered `text`; the error says why the text does not
    /// encode.
    pub fn encode(&self, text: &str) -> Result<Vec<u8>, String> {
        self.encode_text(text).map_err(|e| e.to_string())
    }

    /// [`Self::encode`], with the error as a translatable text.
    pub(crate) fn encode_text(&self, text: &str) -> Result<Vec<u8>, Text> {
        match self {
            Encoder::Text => Ok(text.as_bytes().to_vec()),
            Encoder::Hex => from_hex(text),
            Encoder::Base64 => from_base64(text),
            Encoder::Cbor => json_to_cbor(text),
            // prost-reflect's own (English) text.
            Encoder::Protobuf(message) => message
                .encode_json(text)
                .map_err(|e| text!("verbatim", text = e)),
        }
    }
}

/// `bytes` as lowercase hex digits, two per byte, without spaces.
pub fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

fn from_hex(text: &str) -> Result<Vec<u8>, Text> {
    let digits = text
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .map(|c| {
            c.to_digit(16)
                .map(|d| d as u8)
                .ok_or_else(|| text!("hex-bad-digit", char = c.to_string()))
        })
        .collect::<Result<Vec<u8>, Text>>()?;
    if digits.len() % 2 != 0 {
        return Err(text!("hex-odd-digits", count = digits.len()));
    }
    Ok(digits
        .chunks(2)
        .map(|pair| (pair[0] << 4) | pair[1])
        .collect())
}

fn from_base64(text: &str) -> Result<Vec<u8>, Text> {
    use base64::{DecodeError, Engine};
    let compact: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let char = |byte: u8| char::from(byte).to_string();
    base64::engine::general_purpose::STANDARD
        .decode(&compact)
        .map_err(|e| match e {
            DecodeError::InvalidByte(_, byte) if byte != b'=' => {
                text!("base64-bad-character", char = char(byte))
            }
            DecodeError::InvalidLastSymbol { symbol, .. } => {
                text!("base64-last-character", char = char(symbol))
            }
            DecodeError::InvalidLength(_) => text!("base64-length"),
            DecodeError::InvalidByte(..) | DecodeError::InvalidPadding => {
                text!("base64-padding")
            }
        })
}

/// JSON `text` as CBOR. Integers stay integers and numbers with a fraction or exponent stay
/// floats; objects keep their key order.
fn json_to_cbor(text: &str) -> Result<Vec<u8>, Text> {
    // Read straight into CBOR's own value: serde_json's would sort the keys.
    let value: ciborium::Value =
        serde_json::from_str(text).map_err(|e| text!("cbor-not-json", reason = e.to_string()))?;
    let mut bytes = Vec::new();
    ciborium::into_writer(&value, &mut bytes)
        .map_err(|e| text!("verbatim", text = e.to_string()))?;
    Ok(bytes)
}

/// The encoder of message `index` (from 0) of a template, with its `.proto` file read relative
/// to `base_dir`; the error gives every problem with the message's encoding, one per line.
pub fn message_encoder(
    message: &MessageSpec,
    index: usize,
    base_dir: &Path,
) -> Result<Encoder, String> {
    message_encoder_located(message, index, base_dir).map_err(|problems| {
        problems
            .into_iter()
            .map(|p| p.message.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    })
}

/// [`message_encoder`], with each problem's key path, such as `messages[1].proto`.
pub(crate) fn message_encoder_located(
    message: &MessageSpec,
    index: usize,
    base_dir: &Path,
) -> Result<Encoder, Vec<TemplateError>> {
    let n = index + 1;
    let problem = |key: &str, problem: Text| {
        TemplateError::new(
            format!("messages[{n}].{key}"),
            text!("message-problem", n = n, problem = problem),
        )
    };
    let mut problems = Vec::new();
    if message.encoding != Encoding::Protobuf {
        if message.proto.is_some() {
            problems.push(problem("proto", text!("proto-without-protobuf")));
        }
        if message.message.is_some() {
            problems.push(problem("message", text!("message-type-without-protobuf")));
        }
    }
    let encoder = match message.encoding {
        Encoding::Text => Encoder::Text,
        Encoding::Hex => Encoder::Hex,
        Encoding::Base64 => Encoder::Base64,
        Encoding::Cbor => Encoder::Cbor,
        Encoding::Protobuf => match protobuf(message) {
            Ok((proto, name)) => match proto::load(&base_dir.join(proto), name) {
                Ok(message) => Encoder::Protobuf(message),
                Err(e) => {
                    let path = proto.display().to_string();
                    problems.push(match e {
                        ProtoError::Read(e) => problem(
                            "proto",
                            text!("proto-cannot-read", path = path, reason = e.to_string()),
                        ),
                        ProtoError::Compile(e) => problem(
                            "proto",
                            text!("proto-does-not-compile", path = path, error = e),
                        ),
                        ProtoError::NoMessage { did_you_mean } => problem(
                            "message",
                            match did_you_mean {
                                Some(full) => text!(
                                    "proto-no-message-type-did-you-mean",
                                    path = path,
                                    name = name,
                                    full = full
                                ),
                                None => text!("proto-no-message-type", path = path, name = name),
                            },
                        ),
                    });
                    Encoder::Text
                }
            },
            Err((no_proto, no_name)) => {
                if no_proto {
                    problems.push(problem("proto", text!("protobuf-needs-proto")));
                }
                if no_name {
                    problems.push(problem("message", text!("protobuf-needs-message-type")));
                }
                Encoder::Text
            }
        },
    };
    if problems.is_empty() {
        Ok(encoder)
    } else {
        Err(problems)
    }
}

/// A protobuf message's proto file and message type, or which of the two it lacks.
fn protobuf(message: &MessageSpec) -> Result<(&Path, &str), (bool, bool)> {
    let proto = message
        .proto
        .as_deref()
        .filter(|p| !p.as_os_str().is_empty());
    let name = message.message.as_deref().filter(|m| !m.trim().is_empty());
    match (proto, name) {
        (Some(proto), Some(name)) => Ok((proto, name)),
        _ => Err((proto.is_none(), name.is_none())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::tests::sensor_proto;

    #[test]
    fn text_is_published_as_it_is() {
        assert_eq!(
            Encoder::Text.encode("21.5 °C"),
            Ok("21.5 °C".as_bytes().to_vec())
        );
    }

    #[test]
    fn hex_takes_two_digits_per_byte_and_ignores_whitespace() {
        assert_eq!(
            Encoder::Hex.encode("00 d7\n0A ff"),
            Ok(vec![0x00, 0xd7, 0x0a, 0xff])
        );
        assert_eq!(Encoder::Hex.encode(""), Ok(Vec::new()));
    }

    #[test]
    fn hex_with_an_odd_digit_or_a_non_hex_character_is_an_error() {
        assert_eq!(
            Encoder::Hex.encode("00d"),
            Err("odd number of hex digits (3)".into())
        );
        assert_eq!(
            Encoder::Hex.encode("0x12"),
            Err("`x` is not a hex digit".into())
        );
    }

    #[test]
    fn to_hex_is_lowercase_without_spaces() {
        assert_eq!(to_hex(&[0x00, 0xd7, 0x0a, 0xff]), "00d70aff");
        assert_eq!(to_hex(&[]), "");
    }

    #[test]
    fn base64_is_standard_with_padding_and_ignores_whitespace() {
        assert_eq!(Encoder::Base64.encode("AQID"), Ok(vec![1, 2, 3]));
        assert_eq!(Encoder::Base64.encode(" AQ\nID "), Ok(vec![1, 2, 3]));
        assert_eq!(Encoder::Base64.encode("AQ=="), Ok(vec![1]));
        assert_eq!(Encoder::Base64.encode("+/8="), Ok(vec![0xfb, 0xff]));
    }

    #[test]
    fn invalid_base64_is_an_error() {
        assert_eq!(
            Encoder::Base64.encode("AQ*D"),
            Err("`*` is not a base64 character".into())
        );
        assert_eq!(
            Encoder::Base64.encode("AQ"),
            Err("base64 needs padding (=) to a multiple of 4 characters".into())
        );
        assert!(Encoder::Base64.encode("A").is_err());
        assert!(
            Encoder::Base64.encode("-_8=").is_err(),
            "not URL-safe base64"
        );
    }

    #[test]
    fn base64_cut_short_or_changed_says_so() {
        assert_eq!(
            Encoder::Base64.encode("AR=="),
            Err("the last character `R` leaves bits over; the text is cut short or changed".into())
        );
        assert_eq!(
            Encoder::Base64.encode("AQIDB"),
            Err("the length does not fit base64; a character is missing or one too many".into())
        );
    }

    #[test]
    fn encoding_problems_show_in_czech() {
        let czech = |encoder: Encoder, text: &str| {
            encoder
                .encode_text(text)
                .unwrap_err()
                .render(crate::i18n::Language::Czech)
        };
        assert_eq!(
            czech(Encoder::Hex, "00d"),
            "lichý počet šestnáctkových číslic (3)"
        );
        assert_eq!(czech(Encoder::Base64, "AQ*D"), "`*` není znak base64");
    }

    /// CBOR as ciborium reads it back.
    fn cbor(json: &str) -> ciborium::Value {
        let bytes = Encoder::Cbor.encode(json).unwrap();
        ciborium::from_reader(bytes.as_slice()).unwrap()
    }

    #[test]
    fn cbor_keeps_integers_and_floats_apart() {
        use ciborium::Value;
        let value = cbor(r#"{"t": 21, "h": 21.5, "big": 1.0, "neg": -3}"#);
        let Value::Map(entries) = value else {
            panic!("a map: {value:?}");
        };
        let get = |key: &str| {
            entries
                .iter()
                .find(|(k, _)| k == &Value::Text(key.into()))
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(get("t"), Value::Integer(21.into()));
        assert_eq!(get("h"), Value::Float(21.5));
        assert_eq!(get("big"), Value::Float(1.0), "1.0 stays a float");
        assert_eq!(get("neg"), Value::Integer((-3).into()));
    }

    #[test]
    fn cbor_keeps_the_key_order_and_the_other_json_types() {
        assert_eq!(
            Encoder::Cbor.encode(r#"{"b": 1, "a": [true, null, "x"]}"#),
            Ok(vec![
                0xa2, // map of 2
                0x61, b'b', 0x01, // "b": 1
                0x61, b'a', 0x83, 0xf5, 0xf6, 0x61, b'x', // "a": [true, null, "x"]
            ])
        );
    }

    #[test]
    fn cbor_needs_json() {
        let error = Encoder::Cbor.encode("{co2: 600}").unwrap_err();
        assert!(error.starts_with("not JSON: "), "{error}");
        assert!(Encoder::Cbor.encode("{} {}").is_err(), "trailing text");
    }

    fn message(encoding: Encoding, proto: Option<&str>, name: Option<&str>) -> MessageSpec {
        MessageSpec {
            encoding,
            proto: proto.map(Into::into),
            message: name.map(Into::into),
            ..MessageSpec::new("t", "{}")
        }
    }

    /// `message_encoder_located` for message 2, as `(key path, problem)` pairs.
    fn problems(message: &MessageSpec, base_dir: &Path) -> Vec<(String, String)> {
        message_encoder_located(message, 1, base_dir)
            .unwrap_err()
            .into_iter()
            .map(|e| (e.field, e.message.to_string()))
            .collect()
    }

    fn pair(field: &str, message: &str) -> (String, String) {
        (field.to_string(), message.to_string())
    }

    #[test]
    fn each_encoding_has_its_encoder() {
        let here = Path::new(".");
        for (encoding, encoder) in [
            (Encoding::Text, Encoder::Text),
            (Encoding::Hex, Encoder::Hex),
            (Encoding::Base64, Encoder::Base64),
            (Encoding::Cbor, Encoder::Cbor),
        ] {
            assert_eq!(
                message_encoder_located(&message(encoding, None, None), 0, here),
                Ok(encoder)
            );
        }
        let folder = sensor_proto().parent().unwrap().to_path_buf();
        let encoder = message_encoder_located(
            &message(
                Encoding::Protobuf,
                Some("sensor.proto"),
                Some("sensors.v1.Reading"),
            ),
            0,
            &folder,
        )
        .unwrap();
        assert_eq!(encoder.encoding(), Encoding::Protobuf);
        assert_eq!(encoder.encode(r#"{"ok": true}"#), Ok(vec![0x20, 1]));
    }

    #[test]
    fn protobuf_needs_a_proto_file_and_a_message_type() {
        assert_eq!(
            problems(&message(Encoding::Protobuf, None, Some("")), Path::new(".")),
            [
                pair(
                    "messages[2].proto",
                    "message 2: encoding protobuf needs a proto file"
                ),
                pair(
                    "messages[2].message",
                    "message 2: encoding protobuf needs a message type"
                ),
            ]
        );
    }

    #[test]
    fn proto_and_message_are_only_for_protobuf() {
        assert_eq!(
            problems(
                &message(Encoding::Cbor, Some("a.proto"), Some("a.A")),
                Path::new(".")
            ),
            [
                pair(
                    "messages[2].proto",
                    "message 2: proto is used only with encoding protobuf"
                ),
                pair(
                    "messages[2].message",
                    "message 2: message is used only with encoding protobuf"
                ),
            ]
        );
    }

    #[test]
    fn a_proto_file_that_cannot_be_read_compiled_or_lacks_the_type_is_a_problem() {
        let folder = sensor_proto().parent().unwrap().to_path_buf();
        let missing = problems(
            &message(Encoding::Protobuf, Some("missing.proto"), Some("a.A")),
            &folder,
        );
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].0, "messages[2].proto");
        assert!(
            missing[0]
                .1
                .starts_with("message 2: cannot read proto `missing.proto`: "),
            "{}",
            missing[0].1
        );

        let dir = crate::proto::tests::temp_dir("encoder-broken");
        std::fs::write(dir.join("broken.proto"), "message {").unwrap();
        let broken = problems(
            &message(Encoding::Protobuf, Some("broken.proto"), Some("a.A")),
            &dir,
        );
        assert_eq!(broken.len(), 1);
        assert_eq!(broken[0].0, "messages[2].proto");
        assert!(
            broken[0].1.starts_with("message 2: proto `broken.proto`: "),
            "{}",
            broken[0].1
        );

        assert_eq!(
            problems(
                &message(Encoding::Protobuf, Some("sensor.proto"), Some("Reading")),
                &folder
            ),
            [pair(
                "messages[2].message",
                "message 2: proto `sensor.proto` has no message type `Reading`; did you mean \
                 `sensors.v1.Reading`?"
            )]
        );
    }
}
