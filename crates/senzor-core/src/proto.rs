//! `.proto` files for protobuf payloads, compiled with protox into a descriptor pool. Validation
//! runs on every edit in the window, so a compiled file is kept and compiled again only when it,
//! or a file it imports, changes.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use crate::project::error::line_column;
use miette::Diagnostic as _;
use prost_reflect::prost::Message as _;
use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor};
use protox::Compiler;
use protox::file::{File, FileResolver, GoogleFileResolver};

/// A message type of a compiled `.proto` file, which encodes JSON in protobuf's JSON mapping.
#[derive(Clone)]
pub struct ProtoMessage {
    descriptor: MessageDescriptor,
    compiled: Arc<Compiled>,
}

impl ProtoMessage {
    /// The message type's fully qualified name, such as `sensors.v1.Reading`.
    pub fn full_name(&self) -> &str {
        self.descriptor.full_name()
    }

    /// The message type, to decode what it encoded.
    pub fn descriptor(&self) -> &MessageDescriptor {
        &self.descriptor
    }

    /// `json`, in protobuf's JSON mapping, as the protobuf encoding of the message.
    pub fn encode_json(&self, json: &str) -> Result<Vec<u8>, String> {
        let mut deserializer = serde_json::Deserializer::from_str(json);
        let message = DynamicMessage::deserialize(self.descriptor.clone(), &mut deserializer)
            .and_then(|message| deserializer.end().map(|()| message))
            .map_err(|e| e.to_string())?;
        Ok(message.encode_to_vec())
    }
}

/// Two are equal when they are the same message type compiled from the same file contents, so a
/// running instance takes a `.proto` file that changed as an edit.
impl PartialEq for ProtoMessage {
    fn eq(&self, other: &Self) -> bool {
        self.full_name() == other.full_name()
            && (Arc::ptr_eq(&self.compiled, &other.compiled)
                || self.compiled.sources == other.compiled.sources)
    }
}

impl fmt::Debug for ProtoMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ProtoMessage")
            .field(&self.full_name())
            .finish()
    }
}

/// Why a message type of a `.proto` file cannot be used.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ProtoError {
    /// The file cannot be read.
    Read(String),
    /// The file, or a file it imports, does not compile.
    Compile(String),
    /// The file has no message type of that name; the message type whose short name it is, if
    /// there is one.
    NoMessage { did_you_mean: Option<String> },
}

/// Message type `message` of the `.proto` file at `path`. Imports are looked up in the file's
/// folder, then among protobuf's well-known types.
pub(crate) fn load(path: &Path, message: &str) -> Result<ProtoMessage, ProtoError> {
    std::fs::read(path).map_err(|e| ProtoError::Read(e.to_string()))?;
    let compiled = compiled(path);
    let pool = compiled
        .pool
        .as_ref()
        .map_err(|e| ProtoError::Compile(e.clone()))?;
    let descriptor = pool
        .get_message_by_name(message)
        .ok_or_else(|| ProtoError::NoMessage {
            did_you_mean: pool
                .all_messages()
                .find(|m| m.name() == message)
                .map(|m| m.full_name().to_string()),
        })?;
    Ok(ProtoMessage {
        descriptor,
        compiled: compiled.clone(),
    })
}

/// Each file the compiler looked for, and its contents (`None` when it was not there).
type Sources = Vec<(PathBuf, Option<Vec<u8>>)>;

/// A compiled `.proto` file, and every file the compiler read for it.
struct Compiled {
    sources: Sources,
    pool: Result<DescriptorPool, String>,
}

impl Compiled {
    /// Whether every file still reads as it did when this was compiled.
    fn is_current(&self) -> bool {
        self.sources
            .iter()
            .all(|(path, contents)| std::fs::read(path).ok() == *contents)
    }
}

/// The file at `path` compiled: from the cache when no file it was compiled from changed.
fn compiled(path: &Path) -> Arc<Compiled> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Arc<Compiled>>>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(compiled) = cache.get(path)
        && compiled.is_current()
    {
        return compiled.clone();
    }
    let compiled = Arc::new(compile(path));
    cache.insert(path.to_path_buf(), compiled.clone());
    compiled
}

fn compile(path: &Path) -> Compiled {
    #[cfg(test)]
    compile_count::record(path);
    let folder = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let resolver = Recording {
        folder: folder.to_path_buf(),
        sources: Arc::default(),
    };
    let sources = resolver.sources.clone();
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut compiler = Compiler::with_file_resolver(resolver);
    let pool = compiler
        .open_file(&name)
        .map(|compiler| compiler.descriptor_pool());
    let sources = std::mem::take(&mut *sources.lock().unwrap_or_else(PoisonError::into_inner));
    let pool = pool.map_err(|e| describe(&e, &sources));
    Compiled { sources, pool }
}

/// A protox error as one line, after the file, line and column it points at when it points at
/// one: `sensor.proto:4:1: expected ';' or '[', but found '}'`.
fn describe(error: &protox::Error, sources: &Sources) -> String {
    let message = error.to_string();
    let Some(file) = error.file() else {
        return message;
    };
    let offset = error
        .labels()
        .and_then(|mut labels| labels.next())
        .map(|label| label.offset());
    let text = sources
        .iter()
        .find(|(path, _)| path.ends_with(file))
        .and_then(|(_, contents)| contents.as_deref())
        .map(String::from_utf8_lossy);
    match offset.zip(text) {
        Some((offset, text)) => {
            let (line, column) = line_column(&text, offset);
            format!("{file}:{line}:{column}: {message}")
        }
        None if message.contains(file) => message,
        None => format!("{file}: {message}"),
    }
}

/// Opens files from one folder, then protobuf's well-known types, and records every file it
/// looked for in the folder.
struct Recording {
    folder: PathBuf,
    sources: Arc<Mutex<Sources>>,
}

impl FileResolver for Recording {
    fn open_file(&self, name: &str) -> Result<File, protox::Error> {
        let path = self.folder.join(name);
        let read = std::fs::read(&path);
        self.sources
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((path, read.as_ref().ok().cloned()));
        match read {
            Ok(bytes) => {
                let source = String::from_utf8(bytes)
                    .map_err(|_| protox::Error::new(format!("file '{name}' is not valid UTF-8")))?;
                File::from_source(name, &source)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                GoogleFileResolver::new().open_file(name)
            }
            Err(e) => Err(protox::Error::new(format!("cannot read '{name}': {e}"))),
        }
    }
}

/// Counts compiles by path, so a test sees only the compiles of its own files.
#[cfg(test)]
pub(crate) mod compile_count {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, OnceLock};

    fn counts() -> &'static Mutex<HashMap<PathBuf, usize>> {
        static COUNTS: OnceLock<Mutex<HashMap<PathBuf, usize>>> = OnceLock::new();
        COUNTS.get_or_init(Mutex::default)
    }

    pub(crate) fn record(path: &Path) {
        *counts()
            .lock()
            .unwrap()
            .entry(path.to_path_buf())
            .or_default() += 1;
    }

    /// How often the file at `path` was compiled.
    pub(crate) fn of(path: &Path) -> usize {
        counts().lock().unwrap().get(path).copied().unwrap_or(0)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use prost_reflect::Value;

    /// `testdata/proto/sensor.proto`: `sensors.v1.Reading`, which imports `unit.proto`.
    pub(crate) fn sensor_proto() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/proto/sensor.proto")
    }

    /// A folder of its own for one test, emptied first.
    pub(crate) fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("senzor-proto-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn encodes_json_as_the_message_and_decodes_back() {
        let reading = load(&sensor_proto(), "sensors.v1.Reading").unwrap();
        let bytes = reading
            .encode_json(
                r#"{"device": "d-1", "temperature": 21.5, "humidity": 40, "ok": true, "unit": "CELSIUS"}"#,
            )
            .unwrap();
        let decoded =
            DynamicMessage::decode(reading.descriptor().clone(), bytes.as_slice()).unwrap();
        let field = |name: &str| decoded.get_field_by_name(name).unwrap().into_owned();
        assert_eq!(field("device"), Value::String("d-1".into()));
        assert_eq!(field("temperature"), Value::F64(21.5));
        assert_eq!(field("humidity"), Value::I32(40));
        assert_eq!(field("ok"), Value::Bool(true));
        assert_eq!(
            field("unit"),
            Value::EnumNumber(1),
            "from unit.proto, an import"
        );
    }

    #[test]
    fn the_exact_bytes_of_a_small_message() {
        let reading = load(&sensor_proto(), "sensors.v1.Reading").unwrap();
        // Field 3 (varint) = 40, field 4 (varint) = 1.
        assert_eq!(
            reading.encode_json(r#"{"humidity": 40, "ok": true}"#),
            Ok(vec![0x18, 40, 0x20, 1])
        );
    }

    #[test]
    fn json_that_does_not_fit_the_message_is_an_error() {
        let reading = load(&sensor_proto(), "sensors.v1.Reading").unwrap();
        let error = reading.encode_json(r#"{"humidity": "wet"}"#).unwrap_err();
        assert!(error.contains("line 1 column 18"), "{error}");
        let error = reading.encode_json(r#"{"pressure": 1}"#).unwrap_err();
        assert!(error.contains("pressure"), "{error}");
        assert!(reading.encode_json("not json").is_err());
        assert!(reading.encode_json("{} {}").is_err(), "trailing text");
    }

    #[test]
    fn a_missing_file_a_broken_file_and_an_unknown_type_are_told_apart() {
        let dir = temp_dir("problems");
        assert!(matches!(
            load(&dir.join("missing.proto"), "a.B"),
            Err(ProtoError::Read(_))
        ));
        std::fs::write(
            dir.join("broken.proto"),
            "syntax = \"proto3\";\nmessage {\n",
        )
        .unwrap();
        let Err(ProtoError::Compile(error)) = load(&dir.join("broken.proto"), "a.B") else {
            panic!("a broken file does not compile");
        };
        assert!(!error.is_empty());
        assert_eq!(
            load(&sensor_proto(), "sensors.v1.Nope").err(),
            Some(ProtoError::NoMessage { did_you_mean: None })
        );
        assert_eq!(
            load(&sensor_proto(), "Reading").err(),
            Some(ProtoError::NoMessage {
                did_you_mean: Some("sensors.v1.Reading".into())
            })
        );
    }

    #[test]
    fn a_compile_error_names_the_file_line_and_column() {
        let dir = temp_dir("position");
        std::fs::write(
            dir.join("broken.proto"),
            "syntax = \"proto3\";\nmessage Reading {\n  int32 v = 1\n}\n",
        )
        .unwrap();
        assert_eq!(
            load(&dir.join("broken.proto"), "Reading").err(),
            Some(ProtoError::Compile(
                "broken.proto:4:1: expected ';' or '[', but found '}'".into()
            ))
        );
    }

    #[test]
    fn a_missing_import_does_not_compile() {
        let dir = temp_dir("missing-import");
        std::fs::write(
            dir.join("a.proto"),
            "syntax = \"proto3\";\nimport \"b.proto\";\nmessage A { B b = 1; }\n",
        )
        .unwrap();
        let Err(ProtoError::Compile(error)) = load(&dir.join("a.proto"), "A") else {
            panic!("the import is missing");
        };
        assert!(error.contains("b.proto"), "{error}");
    }

    #[test]
    fn well_known_types_can_be_imported() {
        let dir = temp_dir("well-known");
        std::fs::write(
            dir.join("a.proto"),
            "syntax = \"proto3\";\nimport \"google/protobuf/timestamp.proto\";\n\
             message A { google.protobuf.Timestamp at = 1; }\n",
        )
        .unwrap();
        let a = load(&dir.join("a.proto"), "A").unwrap();
        assert!(a.encode_json(r#"{"at": "2026-11-09T10:00:00Z"}"#).is_ok());
    }

    #[test]
    fn an_unchanged_file_is_compiled_once() {
        let dir = temp_dir("cache");
        let path = dir.join("a.proto");
        std::fs::write(dir.join("b.proto"), "syntax = \"proto3\";\nmessage B {}\n").unwrap();
        std::fs::write(
            &path,
            "syntax = \"proto3\";\nimport \"b.proto\";\nmessage A { B b = 1; }\n",
        )
        .unwrap();
        let first = load(&path, "A").unwrap();
        let again = load(&path, "A").unwrap();
        assert_eq!(compile_count::of(&path), 1, "not compiled again");
        assert_eq!(first, again);

        std::fs::write(
            dir.join("b.proto"),
            "syntax = \"proto3\";\nmessage B { int32 x = 1; }\n",
        )
        .unwrap();
        let changed = load(&path, "A").unwrap();
        assert_eq!(compile_count::of(&path), 2, "an import changed");
        assert_ne!(changed, first);

        std::fs::write(&path, "syntax = \"proto3\";\nmessage A { int32 y = 1; }\n").unwrap();
        load(&path, "A").unwrap();
        assert_eq!(compile_count::of(&path), 3, "the file changed");
    }
}
