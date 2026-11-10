//! The example projects the window offers, the same files the documentation shows.

use senzor_core::project::{Document, ProjectFile};

use crate::i18n::t;

/// An example project: its name, what it shows, its file's text, and the files it reads, by
/// their path relative to it. The name is English in every language (it also names the
/// example's folder); the window shows [`Example::title`] and
/// [`Example::description`].
pub struct Example {
    pub name: &'static str,
    title: fn() -> String,
    description: fn() -> String,
    text: &'static str,
    files: &'static [(&'static str, &'static str)],
}

impl Example {
    /// The example's name in the window's language.
    pub fn title(&self) -> String {
        (self.title)()
    }

    /// What the example shows, in the window's language.
    pub fn description(&self) -> String {
        (self.description)()
    }
}

impl Example {
    /// The example's project. The examples are tested to parse.
    pub fn project(&self) -> ProjectFile {
        ProjectFile::parse(self.text).expect("a built-in example parses")
    }

    /// The example as a project without a file. An example that reads files of its own (a
    /// `.proto` file) finds them in a folder of the system's temporary directory, where they
    /// are written first.
    pub fn document(&self) -> Document {
        if self.files.is_empty() {
            return Document::untitled(self.project());
        }
        let folder = std::env::temp_dir()
            .join("senzor-examples")
            .join(self.name.to_lowercase().replace(' ', "-"));
        for (path, text) in self.files {
            let path = folder.join(path);
            // When this fails, the project still opens, and its problems name the file it
            // cannot read.
            let _ = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, text));
        }
        Document::untitled_in(self.project(), folder)
    }
}

/// Every example, the simplest first.
pub const EXAMPLES: [Example; 8] = [
    Example {
        name: "Quick start",
        title: || t!("examples-quick-start"),
        description: || t!("examples-quick-start-summary"),
        text: include_str!("../../../examples/quick-start.toml"),
        files: &[],
    },
    Example {
        name: "IKEA Zigbee2MQTT",
        title: || t!("examples-ikea"),
        description: || t!("examples-ikea-summary"),
        text: include_str!("../../../examples/ikea-zigbee2mqtt.toml"),
        files: &[],
    },
    Example {
        name: "Home Assistant Statestream",
        title: || t!("examples-statestream"),
        description: || t!("examples-statestream-summary"),
        text: include_str!("../../../examples/home-assistant-statestream.toml"),
        files: &[],
    },
    Example {
        name: "Home Assistant Discovery",
        title: || t!("examples-discovery"),
        description: || t!("examples-discovery-summary"),
        text: include_str!("../../../examples/home-assistant-discovery.toml"),
        files: &[],
    },
    Example {
        name: "Thermostat",
        title: || t!("examples-thermostat"),
        description: || t!("examples-thermostat-summary"),
        text: include_str!("../../../examples/thermostat.toml"),
        files: &[],
    },
    Example {
        name: "Binary payloads",
        title: || t!("examples-binary"),
        description: || t!("examples-binary-summary"),
        text: include_str!("../../../examples/binary-payloads.toml"),
        files: &[(
            "proto/sensor.proto",
            include_str!("../../../examples/proto/sensor.proto"),
        )],
    },
    Example {
        name: "Fleet",
        title: || t!("examples-fleet"),
        description: || t!("examples-fleet-summary"),
        text: include_str!("../../../examples/fleet.toml"),
        files: &[],
    },
    Example {
        name: "Showcase",
        title: || t!("examples-showcase"),
        description: || t!("examples-showcase-summary"),
        text: include_str!("../../../examples/showcase.toml"),
        files: &[],
    },
];

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// The documentation's screenshots show these devices and the 11 instances they make: the
    /// `window` shot's tree and status bar, `device-editor`'s "office ×5".
    #[test]
    fn the_showcase_has_the_devices_the_screenshots_show() {
        let showcase = EXAMPLES.iter().find(|e| e.name == "Showcase").unwrap();
        let devices: Vec<(String, u32)> = showcase
            .project()
            .devices
            .iter()
            .map(|d| (d.id.clone(), d.count))
            .collect();
        assert_eq!(
            devices,
            [
                ("office".to_string(), 5),
                ("meeting".to_string(), 2),
                ("front-door".to_string(), 1),
                ("lab".to_string(), 3),
            ]
        );
    }

    #[test]
    fn examples_are_named_in_the_windows_language() {
        let quick_start = &EXAMPLES[0];
        assert_eq!(quick_start.title(), "Quick start");
        crate::i18n::set_language(crate::i18n::Language::Czech);
        assert_eq!(quick_start.title(), "Rychlý start");
        assert_eq!(
            quick_start.description(),
            "Dva senzory CO2 a teploměr na vestavěných šablonách"
        );
        assert_eq!(quick_start.name, "Quick start", "the ID stays");
    }

    /// The English title is the name, which also names the example's folder.
    #[test]
    fn english_titles_are_the_names() {
        for example in &EXAMPLES {
            assert_eq!(example.title(), example.name);
        }
    }

    #[test]
    fn every_example_is_a_valid_project() {
        for example in &EXAMPLES {
            let doc = example.document();
            let errors = &doc.validated().errors;
            assert!(errors.is_empty(), "{}: {errors:?}", example.name);
        }
    }

    #[test]
    fn an_example_without_files_of_its_own_validates_anywhere() {
        for example in EXAMPLES.iter().filter(|e| e.files.is_empty()) {
            let errors = example.project().validate(Path::new(".")).errors;
            assert!(errors.is_empty(), "{}: {errors:?}", example.name);
        }
    }

    /// The window offers the files `examples/` holds, so the documentation and the window show
    /// the same.
    #[test]
    fn an_examples_files_are_those_next_to_it_in_examples() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        for example in &EXAMPLES {
            for (path, text) in example.files {
                assert_eq!(
                    std::fs::read_to_string(dir.join(path)).unwrap(),
                    *text,
                    "{}: {path}",
                    example.name
                );
            }
        }
    }
}
