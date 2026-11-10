//! "What's new": the Highlights of `CHANGELOG.md`, shown the first time the window opens after an
//! update. A Czech window shows them from `locales/cs/whats-new.md`, which has the same versions
//! and Highlights in Czech.

use crate::i18n::{self, Language};

/// The changelog the window was built with.
const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");

/// The Highlights of `CHANGELOG.md` in Czech, laid out like it.
const CHANGELOG_CS: &str = include_str!("../locales/cs/whats-new.md");

/// One version's Highlights, as plain text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub highlights: Vec<String>,
}

/// Every version in `changelog` that has Highlights, in the file's order (the newest first).
fn releases(changelog: &str) -> Vec<Release> {
    let mut releases: Vec<Release> = Vec::new();
    let mut in_highlights = false;
    for line in changelog.lines() {
        if let Some(heading) = line.strip_prefix("## [") {
            in_highlights = false;
            let version = heading.split(']').next().unwrap_or_default();
            releases.push(Release {
                version: version.to_string(),
                highlights: Vec::new(),
            });
        } else if line.starts_with("### ") {
            in_highlights = line.trim() == "### Highlights";
        } else if in_highlights
            && let (Some(item), Some(release)) = (line.strip_prefix("- "), releases.last_mut())
        {
            release.highlights.push(plain_text(item));
        }
    }
    releases.retain(|r| !r.highlights.is_empty());
    releases
}

/// `english` with the Highlights of `translation` for each version it has; the others stay in
/// English.
fn translated(english: Vec<Release>, translation: &str) -> Vec<Release> {
    let translation = releases(translation);
    english
        .into_iter()
        .map(|release| {
            translation
                .iter()
                .find(|t| t.version == release.version)
                .cloned()
                .unwrap_or(release)
        })
        .collect()
}

/// Every release with Highlights, in the window's language.
fn shown_releases() -> Vec<Release> {
    let english = releases(CHANGELOG);
    match i18n::language() {
        Language::English => english,
        Language::Czech => translated(english, CHANGELOG_CS),
    }
}

/// `text` without Markdown: no `**`, no backticks, and links as their text.
fn plain_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        let link = rest[start..]
            .find("](")
            .and_then(|mid| rest[start + mid..].find(')').map(|end| (mid, mid + end)));
        let Some((mid, end)) = link else {
            break;
        };
        out.push_str(&rest[..start]);
        out.push_str(&rest[start + 1..start + mid]);
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out.replace("**", "").replace('`', "")
}

/// `major.minor.patch` as numbers; anything after a `-` or `+` is ignored.
pub(crate) fn version_number(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// Those of `releases` after `previous` up to `current`, the newest first: what changed since the
/// version that ran before. A `previous` that is not a version counts as older than every release.
fn since(releases: Vec<Release>, previous: &str, current: &str) -> Vec<Release> {
    let previous = version_number(previous).unwrap_or_default();
    let Some(current) = version_number(current) else {
        return Vec::new();
    };
    releases
        .into_iter()
        .filter(|r| version_number(&r.version).is_some_and(|v| v > previous && v <= current))
        .collect()
}

/// What is new since `previous`, the version that last ran, for this build's changelog, in the
/// window's language.
pub fn since_version(previous: &str) -> Vec<Release> {
    since(shown_releases(), previous, env!("CARGO_PKG_VERSION"))
}

/// The Highlights of this version, for the settings' What's new, in the window's language.
pub fn this_version() -> Vec<Release> {
    let current = env!("CARGO_PKG_VERSION");
    shown_releases()
        .into_iter()
        .filter(|r| r.version == current)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Changelog

Intro text.

## [Unreleased]

## [1.2.0] - 2027-03-01

### Highlights

- Charts of a field's values in the **instance view**.
- `senzor-cli` can [watch](https://example.com) the project file.

### Added

- A detail that stays out of What's new.

## [1.1.0] - 2027-01-10

### Fixed

- A fix without Highlights.

## [1.0.0] - 2026-11-20

### Highlights

- The first release.
";

    #[test]
    fn highlights_are_read_per_version_as_plain_text() {
        assert_eq!(
            releases(SAMPLE),
            [
                Release {
                    version: "1.2.0".into(),
                    highlights: vec![
                        "Charts of a field's values in the instance view.".into(),
                        "senzor-cli can watch the project file.".into(),
                    ],
                },
                Release {
                    version: "1.0.0".into(),
                    highlights: vec!["The first release.".into()],
                },
            ]
        );
    }

    #[test]
    fn what_is_new_lists_the_versions_after_the_previous_one() {
        let versions = |previous, current| -> Vec<String> {
            since(releases(SAMPLE), previous, current)
                .into_iter()
                .map(|r| r.version)
                .collect()
        };
        assert_eq!(versions("1.0.0", "1.2.0"), ["1.2.0"]);
        assert_eq!(versions("0.9.0", "1.2.0"), ["1.2.0", "1.0.0"]);
        assert_eq!(
            versions("1.0.0", "1.1.0"),
            Vec::<String>::new(),
            "1.1.0 has no Highlights"
        );
        assert_eq!(versions("1.2.0", "1.2.0"), Vec::<String>::new());
        assert_eq!(versions("not a version", "1.0.0"), ["1.0.0"]);
    }

    #[test]
    fn a_version_without_czech_highlights_keeps_the_english_ones() {
        let czech = "## [1.2.0]\n\n### Highlights\n\n- Grafy hodnot pole v **pohledu instance**.\n";
        let shown = translated(releases(SAMPLE), czech);
        assert_eq!(shown[0].version, "1.2.0");
        assert_eq!(
            shown[0].highlights,
            ["Grafy hodnot pole v pohledu instance."]
        );
        assert_eq!(shown[1].version, "1.0.0");
        assert_eq!(shown[1].highlights, ["The first release."]);
    }

    /// Each release's Highlights are translated with it, item by item.
    #[test]
    fn czech_has_the_highlights_of_every_version() {
        let czech = releases(CHANGELOG_CS);
        for english in releases(CHANGELOG) {
            let translation = czech.iter().find(|r| r.version == english.version);
            let Some(translation) = translation else {
                panic!(
                    "locales/cs/whats-new.md has no Highlights for {}",
                    english.version
                );
            };
            assert_eq!(
                translation.highlights.len(),
                english.highlights.len(),
                "the Highlights of {}",
                english.version
            );
        }
    }

    #[test]
    fn a_czech_window_shows_the_czech_highlights() {
        let english = this_version();
        crate::i18n::set_language(crate::i18n::Language::Czech);
        let czech = this_version();
        assert_eq!(czech[0].version, english[0].version);
        assert!(czech[0].highlights[0].starts_with("Simulace senzorů MQTT"));
        assert_ne!(czech[0].highlights, english[0].highlights);
    }

    /// The release step adds this version to the changelog; the window would show nothing.
    #[test]
    fn the_changelog_has_highlights_for_this_version() {
        let current = this_version();
        assert_eq!(
            current.len(),
            1,
            "CHANGELOG.md has no Highlights for this version"
        );
        assert!(!current[0].highlights.is_empty());
    }
}
