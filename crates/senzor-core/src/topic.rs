//! MQTT topic filters: which filters are valid, and which topics a filter matches.

/// The prefix of an MQTT 5 shared subscription: `$share/<group>/<filter>`.
const SHARED_PREFIX: &str = "$share/";

/// What is wrong with an MQTT topic filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterProblem {
    Empty,
    /// `#` is not the whole last level, as in `a/#/b` or `a#`.
    HashNotLast,
    /// `+` is not a whole level, as in `a+`.
    PlusNotWhole,
}

/// Checks an MQTT topic filter: not empty, `#` only as the whole last level, `+` only as a whole
/// level.
pub fn check_filter(filter: &str) -> Result<(), FilterProblem> {
    if filter.is_empty() {
        return Err(FilterProblem::Empty);
    }
    let levels: Vec<&str> = filter.split('/').collect();
    for (i, level) in levels.iter().enumerate() {
        if level.contains('#') && (*level != "#" || i + 1 != levels.len()) {
            return Err(FilterProblem::HashNotLast);
        }
        if level.contains('+') && *level != "+" {
            return Err(FilterProblem::PlusNotWhole);
        }
    }
    Ok(())
}

/// Whether `topic` matches the valid filter `filter`. A shared subscription
/// (`$share/<group>/<filter>`) matches what its filter matches. Wildcards at the first level do
/// not match topics that start with `$`, such as `$SYS/...`.
pub fn matches(filter: &str, topic: &str) -> bool {
    let filter = match filter.strip_prefix(SHARED_PREFIX) {
        Some(rest) => rest.split_once('/').map_or(rest, |(_, filter)| filter),
        None => filter,
    };
    if topic.starts_with('$') && filter.starts_with(['+', '#']) {
        return false;
    }
    let mut filter_levels = filter.split('/');
    let mut topic_levels = topic.split('/');
    loop {
        match (filter_levels.next(), topic_levels.next()) {
            (Some("#"), _) => return true,
            (Some("+"), Some(_)) => {}
            (Some(f), Some(t)) if f == t => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_filters_pass() {
        for filter in [
            "a",
            "a/b",
            "#",
            "+",
            "a/#",
            "a/+/c",
            "+/+",
            "/",
            "a//b",
            "$share/g/a/#",
        ] {
            assert_eq!(check_filter(filter), Ok(()), "{filter}");
        }
    }

    #[test]
    fn invalid_filters_say_why() {
        assert_eq!(check_filter(""), Err(FilterProblem::Empty));
        for filter in ["a/#/b", "a#", "a/b#", "#/a"] {
            assert_eq!(
                check_filter(filter),
                Err(FilterProblem::HashNotLast),
                "{filter}"
            );
        }
        for filter in ["a+", "a/+b", "+a/b"] {
            assert_eq!(
                check_filter(filter),
                Err(FilterProblem::PlusNotWhole),
                "{filter}"
            );
        }
    }

    #[test]
    fn exact_filters_match_only_their_topic() {
        assert!(matches("a/b", "a/b"));
        assert!(!matches("a/b", "a/c"));
        assert!(!matches("a/b", "a/b/c"));
        assert!(!matches("a/b", "a"));
        assert!(!matches("a/b", "A/b"));
    }

    #[test]
    fn plus_matches_one_level() {
        assert!(matches("a/+/c", "a/b/c"));
        assert!(matches("a/+/c", "a//c"));
        assert!(!matches("a/+/c", "a/b/x/c"));
        assert!(matches("+", "a"));
        assert!(!matches("+", "a/b"));
        assert!(matches("+/+", "/a"));
    }

    #[test]
    fn hash_matches_the_parent_and_every_level_below() {
        assert!(matches("a/#", "a"));
        assert!(matches("a/#", "a/b"));
        assert!(matches("a/#", "a/b/c"));
        assert!(!matches("a/#", "b/a"));
        assert!(matches("#", "a/b/c"));
    }

    #[test]
    fn wildcards_at_the_first_level_skip_dollar_topics() {
        assert!(!matches("#", "$SYS/broker/load"));
        assert!(!matches("+/broker/load", "$SYS/broker/load"));
        assert!(matches("$SYS/#", "$SYS/broker/load"));
        assert!(matches("$SYS/+/load", "$SYS/broker/load"));
    }

    #[test]
    fn a_shared_subscription_matches_what_its_filter_matches() {
        assert!(matches("$share/g/cmd/+", "cmd/heating"));
        assert!(!matches("$share/g/cmd/+", "state/heating"));
    }
}
