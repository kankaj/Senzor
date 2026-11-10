//! Translations of the texts `senzor-core` shows people: validation problems and their
//! locations. The catalogs are Fluent files in `locales/<language>/`, embedded at compile time.
//! English is the default and what the CLI, MCP and the docs show; the GUI can show Czech.

use std::collections::HashMap;
use std::fmt;

use fluent_templates::fluent_bundle::FluentValue;
pub use fluent_templates::{LanguageIdentifier, langid};
use serde::{Deserialize, Serialize};

fluent_templates::static_loader! {
    static LOCALES = {
        locales: "./locales",
        fallback_language: "en",
        // Fluent wraps every argument in Unicode isolation marks (U+2068, U+2069) by default;
        // problem texts end up in terminals, logs and JSON, where the marks are noise.
        customise: |bundle| bundle.set_use_isolating(false),
    };
}

static ENGLISH: LanguageIdentifier = langid!("en");
static CZECH: LanguageIdentifier = langid!("cs");

/// A language the texts can be shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Language {
    #[default]
    #[serde(rename = "en")]
    English,
    #[serde(rename = "cs")]
    Czech,
}

impl Language {
    /// Every language, English first.
    pub const ALL: [Language; 2] = [Language::English, Language::Czech];

    /// The language's identifier, as Fluent loaders take it.
    pub fn id(self) -> &'static LanguageIdentifier {
        match self {
            Language::English => &ENGLISH,
            Language::Czech => &CZECH,
        }
    }

    /// The language's code, as settings store it: `en`, `cs`.
    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::Czech => "cs",
        }
    }

    /// The language's name in that language, for a language picker: `English`, `Čeština`.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::English => "English",
            Language::Czech => "Čeština",
        }
    }
}

/// A translatable text: a key of the catalogs and the values of its variables. It shows in
/// English (`to_string`) or in any [`Language`] ([`Text::render`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    key: &'static str,
    args: Vec<(&'static str, Arg)>,
}

/// The value of a [`Text`]'s variable.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    /// Shown as it is, whatever the language: names, paths, TOML values, and texts that come
    /// from libraries (TOML, MiniJinja, Rhai, protox, io errors), which stay English.
    Str(String),
    /// A count or index, which plural forms can depend on.
    Number(u64),
    /// Another text, shown in the same language.
    Text(Text),
}

impl Text {
    pub fn new(key: &'static str) -> Self {
        Self {
            key,
            args: Vec::new(),
        }
    }

    /// The text with the variable `name` set to `value`.
    pub fn arg(mut self, name: &'static str, value: impl Into<Arg>) -> Self {
        self.args.push((name, value.into()));
        self
    }

    /// The catalog key, such as `device-unknown-template`.
    pub fn key(&self) -> &'static str {
        self.key
    }

    pub fn args(&self) -> &[(&'static str, Arg)] {
        &self.args
    }

    /// The text in `lang`. A key that `lang` lacks shows in English.
    pub fn render(&self, lang: Language) -> String {
        let args: HashMap<&str, FluentValue> = self
            .args
            .iter()
            .map(|(name, arg)| {
                let value = match arg {
                    Arg::Str(s) => FluentValue::from(s.as_str()),
                    Arg::Number(n) => FluentValue::from(*n),
                    Arg::Text(text) => FluentValue::from(text.render(lang)),
                };
                (*name, value)
            })
            .collect();
        LOCALES
            .lookup_single_language(lang.id(), self.key, Some(&args))
            .or_else(|_| LOCALES.lookup_single_language(&ENGLISH, self.key, Some(&args)))
            .unwrap_or_else(|_| self.key.to_string())
    }
}

/// The English text.
impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render(Language::English))
    }
}

/// Whether the English text is `other`.
impl PartialEq<&str> for Text {
    fn eq(&self, other: &&str) -> bool {
        self.render(Language::English) == *other
    }
}

impl From<String> for Arg {
    fn from(value: String) -> Self {
        Arg::Str(value)
    }
}

impl From<&str> for Arg {
    fn from(value: &str) -> Self {
        Arg::Str(value.to_string())
    }
}

impl From<&String> for Arg {
    fn from(value: &String) -> Self {
        Arg::Str(value.clone())
    }
}

impl From<Text> for Arg {
    fn from(value: Text) -> Self {
        Arg::Text(value)
    }
}

macro_rules! number_arg {
    ($($type:ty),*) => {
        $(impl From<$type> for Arg {
            fn from(value: $type) -> Self {
                Arg::Number(value as u64)
            }
        })*
    };
}
number_arg!(u8, u16, u32, u64, usize);

/// A [`Text`] with the catalog key `$key` and the variables `name = value`:
/// `text!("device-unknown-template", template = &spec.template)`. Every key goes through this
/// macro, so a test can check that the catalogs have each one.
macro_rules! text {
    ($key:literal $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::i18n::Text::new($key)$(.arg(stringify!($name), $value))*
    };
}
pub(crate) use text;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    const EN: &str = include_str!("../locales/en/problems.ftl");
    const CS: &str = include_str!("../locales/cs/problems.ftl");

    /// Each message of a catalog with the variables (`$name`) and other messages (`{ name }`)
    /// its value uses.
    fn catalog(ftl: &str) -> BTreeMap<String, BTreeSet<String>> {
        let mut messages: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut current = None;
        for line in ftl.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let value = if line.starts_with([' ', '}']) {
                line
            } else {
                let (key, value) = line.split_once('=').expect("a message is `key = value`");
                let key = key.trim().to_string();
                assert!(
                    messages.insert(key.clone(), BTreeSet::new()).is_none(),
                    "duplicate key {key}"
                );
                current = Some(key);
                value
            };
            let used = messages.get_mut(current.as_ref().unwrap()).unwrap();
            for (i, _) in value.match_indices('{') {
                let rest = value[i + 1..].trim_start();
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '$')
                    .collect();
                if !name.is_empty() {
                    used.insert(name);
                }
            }
        }
        messages
    }

    /// The keys of every `text!` call in the crate's sources.
    fn keys_in_code() -> BTreeSet<String> {
        fn visit(dir: &Path, keys: &mut BTreeSet<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    visit(&path, keys);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let source = std::fs::read_to_string(&path).unwrap();
                    const CALL: &str = "text!(";
                    for (i, _) in source.match_indices(CALL) {
                        // The key follows on the same line or, in a long call, on the next.
                        let Some(rest) = source[i + CALL.len()..].trim_start().strip_prefix('"')
                        else {
                            continue;
                        };
                        let key: String = rest
                            .chars()
                            .take_while(|c| {
                                c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-'
                            })
                            .collect();
                        if !key.is_empty() && rest[key.len()..].starts_with('"') {
                            keys.insert(key);
                        }
                    }
                }
            }
        }
        let mut keys = BTreeSet::new();
        visit(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut keys,
        );
        keys
    }

    #[test]
    fn czech_has_every_english_key_with_the_same_variables() {
        let (en, cs) = (catalog(EN), catalog(CS));
        assert!(!en.is_empty());
        let en_keys: BTreeSet<_> = en.keys().collect();
        let cs_keys: BTreeSet<_> = cs.keys().collect();
        assert_eq!(
            en_keys.symmetric_difference(&cs_keys).collect::<Vec<_>>(),
            Vec::<&&String>::new(),
            "keys in only one catalog"
        );
        for (key, used) in &en {
            assert_eq!(&cs[key], used, "variables of {key}");
        }
    }

    #[test]
    fn every_key_in_the_code_is_in_the_catalog_and_every_catalog_key_is_used() {
        let en = catalog(EN);
        let in_code = keys_in_code();
        // The fallback test's own keys: one only in fallback-test.ftl, one nowhere.
        let in_code: BTreeSet<_> = in_code
            .into_iter()
            .filter(|k| k != "fallback-test-english-only" && k != "no-such-text")
            .collect();
        let missing: Vec<_> = in_code.iter().filter(|k| !en.contains_key(*k)).collect();
        assert!(
            missing.is_empty(),
            "keys not in en/problems.ftl: {missing:?}"
        );
        let unused: Vec<_> = en.keys().filter(|k| !in_code.contains(*k)).collect();
        assert!(unused.is_empty(), "keys no code uses: {unused:?}");
    }

    #[test]
    fn a_text_missing_in_czech_shows_in_english() {
        let text = text!("fallback-test-english-only");
        assert_eq!(text.render(Language::Czech), "only in English");
        let missing = text!("no-such-text");
        assert_eq!(missing.render(Language::Czech), "no-such-text");
    }

    #[test]
    fn english_has_no_isolation_marks() {
        let text = text!("device-unknown-template", template = "co-2");
        assert_eq!(text.to_string(), "unknown template `co-2`");
        assert_eq!(text.render(Language::English), "unknown template `co-2`");
    }

    #[test]
    fn czech_renders_with_its_variables_and_nested_texts() {
        let text = text!("device-unknown-template", template = "co-2");
        assert_eq!(text.render(Language::Czech), "neznámá šablona `co-2`");
        let nested = text!(
            "field-problem",
            name = "co2",
            problem = text!("min-not-below-max")
        );
        assert_eq!(nested.to_string(), "field `co2`: min must be less than max");
        assert_eq!(
            nested.render(Language::Czech),
            "pole `co2`: min musí být menší než max"
        );
    }

    #[test]
    fn czech_plurals_have_three_forms() {
        let seconds = |max: u64| {
            text!("at-most-seconds", key = "keep_alive", max = max).render(Language::Czech)
        };
        assert_eq!(seconds(1), "keep_alive smí být nejvýše 1 sekunda");
        assert_eq!(seconds(3), "keep_alive smí být nejvýše 3 sekundy");
        assert_eq!(seconds(5), "keep_alive smí být nejvýše 5 sekund");
        assert_eq!(
            text!("at-most-seconds", key = "x", max = 1u64).to_string(),
            "x must be at most 1 second"
        );
        assert_eq!(
            text!("at-most-seconds", key = "x", max = u32::MAX).to_string(),
            "x must be at most 4294967295 seconds"
        );
    }

    #[test]
    fn languages_are_stored_as_their_codes_and_default_to_english() {
        assert_eq!(Language::default(), Language::English);
        for lang in Language::ALL {
            let json = serde_json::to_string(&lang).unwrap();
            assert_eq!(json, format!("\"{}\"", lang.code()));
            assert_eq!(serde_json::from_str::<Language>(&json).unwrap(), lang);
            assert_eq!(lang.id().to_string(), lang.code());
        }
    }
}
