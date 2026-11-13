//! Reference pages offered as MCP resources. They are the documentation site's pages, built into
//! the binary, so improving the docs improves what agents read. The crate reads its copies in
//! `docs/`, because crates.io takes only the crate's folder: dev/crate-files.sh writes them.

pub(crate) struct Reference {
    pub uri: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub mime_type: &'static str,
    text: &'static str,
}

impl Reference {
    /// The page without its MDX frontmatter.
    pub fn text(&self) -> &'static str {
        strip_frontmatter(self.text)
    }
}

pub(crate) const REFERENCES: [Reference; 5] = [
    Reference {
        uri: "senzor://reference/project-file",
        name: "project-file",
        description: "Every key of the project file: broker, simulation, templates, devices",
        mime_type: "text/markdown",
        text: include_str!("../docs/project-file.mdx"),
    },
    Reference {
        uri: "senzor://reference/generators",
        name: "generators",
        description: "The field generators (constant, random_walk, sine, uniform, sequence, script) and noise",
        mime_type: "text/markdown",
        text: include_str!("../docs/generators.mdx"),
    },
    Reference {
        uri: "senzor://reference/template-variables",
        name: "template-variables",
        description: "Variables and filters available in topics and payloads",
        mime_type: "text/markdown",
        text: include_str!("../docs/template-variables.mdx"),
    },
    Reference {
        uri: "senzor://reference/scripts",
        name: "scripts",
        description: "Rhai scripts: field scripts, subscriptions to command topics, state, functions and limits",
        mime_type: "text/markdown",
        text: include_str!("../docs/scripts.mdx"),
    },
    Reference {
        uri: "senzor://reference/builtin-templates",
        name: "builtin-templates",
        description: "The built-in templates co2, temperature and humidity, as TOML",
        mime_type: "application/toml",
        text: senzor_core::project::builtin::BUILTIN_TOML,
    },
];

pub(crate) fn find(uri: &str) -> Option<&'static Reference> {
    REFERENCES.iter().find(|r| r.uri == uri)
}

fn strip_frontmatter(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("---\n") else {
        return text;
    };
    match rest.find("\n---\n") {
        Some(end) => rest[end + "\n---\n".len()..].trim_start(),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_is_stripped() {
        assert_eq!(strip_frontmatter("---\ntitle: X\n---\n\n# Hi\n"), "# Hi\n");
        assert_eq!(strip_frontmatter("# No frontmatter"), "# No frontmatter");
    }

    #[test]
    fn every_reference_has_text() {
        for reference in &REFERENCES {
            assert!(!reference.text().trim().is_empty(), "{}", reference.uri);
            assert!(!reference.text().starts_with("---"), "{}", reference.uri);
        }
    }

    #[test]
    fn the_generator_reference_names_every_generator() {
        let text = find("senzor://reference/generators").unwrap().text();
        for kind in ["constant", "random_walk", "sine", "uniform", "sequence"] {
            assert!(text.contains(kind), "{kind}");
        }
    }
}
