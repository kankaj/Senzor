//! The window's texts in English or Czech, from the Fluent files in `crates/senzor/locales/`.
//!
//! Every text the window shows goes through [`t!`]: `t!("settings-theme")`, or with variables
//! `t!("format-bytes", count = n)`. The keys of `locales/en/` and `locales/cs/` are the same; tests
//! check that, and that every key the code asks for exists. English is the default and the
//! fallback.
//!
//! The language belongs to the thread that draws the window: [`set_language`] sets it there, and
//! texts made on another thread come out in English. (Each test runs on a thread of its own, so a
//! test that switches to Czech changes no other test.)

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashMap;

use fluent_templates::fluent_bundle::FluentValue;
use fluent_templates::{Loader, static_loader};

static_loader! {
    static LOCALES = {
        locales: "./locales",
        fallback_language: "en",
        // No Unicode isolation marks around variables: egui would draw them as boxes.
        customise: |bundle| bundle.set_use_isolating(false),
    };
}

/// The language of the window's texts; the same type the problem texts of `senzor-core` take.
pub use senzor_core::i18n::Language;

thread_local! {
    static CURRENT: Cell<Language> = const { Cell::new(Language::English) };
}

/// Shows this thread's texts in `language` from now on.
pub fn set_language(language: Language) {
    CURRENT.with(|current| current.set(language));
}

/// The language of this thread's texts.
pub fn language() -> Language {
    CURRENT.with(Cell::get)
}

/// The text of `key` in this thread's language; in English when that language lacks it, and the
/// key itself when English lacks it too.
pub fn tr(key: &str) -> String {
    lookup(key, None)
}

/// The text of `key` with its variables filled in.
pub fn tr_args(key: &str, args: &[(&'static str, FluentValue<'_>)]) -> String {
    let args: HashMap<Cow<'static, str>, FluentValue<'_>> = args
        .iter()
        .map(|(name, value)| (Cow::Borrowed(*name), value.clone()))
        .collect();
    lookup(key, Some(&args))
}

fn lookup(key: &str, args: Option<&HashMap<Cow<'static, str>, FluentValue<'_>>>) -> String {
    LOCALES
        .try_lookup_complete(language().id(), key, args)
        .or_else(|| LOCALES.try_lookup_complete(Language::English.id(), key, args))
        .unwrap_or_else(|| key.to_string())
}

/// A value for a text's variable: a number (which picks plural forms) or a string.
pub fn arg<'a>(value: impl Into<FluentValue<'a>>) -> FluentValue<'a> {
    value.into()
}

/// The text of a key in the window's language: `t!("key")` or `t!("key", name = value, …)`.
macro_rules! t {
    ($key:literal) => {
        $crate::i18n::tr($key)
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::tr_args($key, &[$((stringify!($name), $crate::i18n::arg($value))),+])
    };
}
pub(crate) use t;

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    use super::*;

    /// A Fluent file's messages and attributes (`key`, `key.attribute`), each with the
    /// variables it uses.
    fn entries(text: &str) -> BTreeMap<String, BTreeSet<String>> {
        let mut entries: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut message = None::<String>;
        let mut current = None::<String>;
        for line in text.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let indented = line.starts_with(char::is_whitespace);
            if !indented && let Some((id, _)) = line.split_once(" =") {
                message = Some(id.trim().to_string());
                current = message.clone();
            } else if let Some(rest) = line.trim_start().strip_prefix('.')
                && let Some((attribute, _)) = rest.split_once(" =")
                && let Some(message) = &message
            {
                current = Some(format!("{message}.{}", attribute.trim()));
            }
            let Some(key) = &current else { continue };
            let variables = entries.entry(key.clone()).or_default();
            for (i, _) in line.match_indices('$') {
                let name: String = line[i + 1..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                    .collect();
                variables.insert(name);
            }
        }
        entries
    }

    /// Every file of a locale, by file name, with its entries.
    fn locale(name: &str) -> BTreeMap<String, BTreeMap<String, BTreeSet<String>>> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("locales")
            .join(name);
        std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|e| e == "ftl"))
            // English only on purpose: see `a_text_missing_in_czech_shows_in_english`.
            .filter(|path| !path.ends_with("fallback-test.ftl"))
            .map(|path| {
                let file = path.file_name().unwrap().to_string_lossy().into_owned();
                (file, entries(&std::fs::read_to_string(&path).unwrap()))
            })
            .collect()
    }

    #[test]
    fn czech_has_every_english_text_with_the_same_variables() {
        let (english, czech) = (locale("en"), locale("cs"));
        assert_eq!(
            english.keys().collect::<Vec<_>>(),
            czech.keys().collect::<Vec<_>>(),
            "the same files"
        );
        for (file, english) in &english {
            let czech = &czech[file];
            let missing: Vec<_> = english.keys().filter(|k| !czech.contains_key(*k)).collect();
            let extra: Vec<_> = czech.keys().filter(|k| !english.contains_key(*k)).collect();
            assert!(missing.is_empty(), "{file}: not in Czech: {missing:?}");
            assert!(extra.is_empty(), "{file}: only in Czech: {extra:?}");
            for (key, variables) in english {
                assert_eq!(variables, &czech[key], "{file}: the variables of {key}");
            }
        }
    }

    /// The keys the code asks for with `t!("…"`.
    fn used_keys() -> BTreeSet<String> {
        fn walk(dir: &Path, keys: &mut BTreeSet<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(&path, keys);
                // This file's `t!`s are examples and tests, with keys of their own.
                } else if path.extension().is_some_and(|e| e == "rs") && !path.ends_with("i18n.rs")
                {
                    let text = std::fs::read_to_string(&path).unwrap();
                    for (i, _) in text.match_indices("t!(") {
                        // `t!` itself, not the end of `format!` or `print!`.
                        let before = text[..i].chars().next_back();
                        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                            continue;
                        }
                        // rustfmt may put the key on the next line.
                        let Some(rest) = text[i + 3..].trim_start().strip_prefix('"') else {
                            continue;
                        };
                        keys.insert(rest.chars().take_while(|c| *c != '"').collect());
                    }
                }
            }
        }
        let mut keys = BTreeSet::new();
        walk(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut keys,
        );
        keys
    }

    #[test]
    fn every_text_the_code_asks_for_exists_in_english() {
        let english: BTreeSet<String> = locale("en")
            .into_values()
            .flat_map(|entries| entries.into_keys())
            .collect();
        let missing: Vec<_> = used_keys()
            .into_iter()
            .filter(|key| !english.contains(key))
            .collect();
        assert!(
            missing.is_empty(),
            "keys without an English text: {missing:?}"
        );
    }

    #[test]
    fn czech_texts_follow_the_language_of_the_thread() {
        assert_eq!(t!("settings-theme"), "Theme");
        set_language(Language::Czech);
        assert_eq!(t!("settings-theme"), "Motiv");
        std::thread::spawn(|| assert_eq!(t!("settings-theme"), "Theme"))
            .join()
            .unwrap();
        set_language(Language::English);
        assert_eq!(t!("settings-theme"), "Theme");
    }

    #[test]
    fn a_text_missing_in_czech_shows_in_english() {
        set_language(Language::Czech);
        assert_eq!(tr("fallback-test-english-only"), "only in English");
        assert_eq!(
            tr_args("fallback-test-english-only", &[("unused", arg(1))]),
            "only in English"
        );
        // Not even English has it: the key, rather than nothing.
        assert_eq!(tr("no-such-text"), "no-such-text");
    }

    #[test]
    fn czech_numbers_pick_one_of_three_plural_forms() {
        set_language(Language::Czech);
        let bytes = |count: usize| t!("format-bytes", count = count);
        assert_eq!(bytes(1), "1 bajt");
        assert_eq!(bytes(3), "3 bajty");
        assert_eq!(bytes(5), "5 bajtů");
        set_language(Language::English);
        assert_eq!(bytes(1), "1 byte");
        assert_eq!(bytes(5), "5 bytes");
    }
}
