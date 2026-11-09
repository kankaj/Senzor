//! The example projects the window offers, the same files the documentation shows.

use senzor_core::project::{Document, ProjectFile};

/// An example project: its name in the window, what it shows, its file's text, and the files
/// it reads, by their path relative to it.
pub struct Example {
    pub name: &'static str,
    pub summary: &'static str,
    text: &'static str,
    files: &'static [(&'static str, &'static str)],
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
        summary: "Two CO2 sensors and a thermometer on the built-in templates",
        text: include_str!("../../../examples/quick-start.toml"),
        files: &[],
    },
    Example {
        name: "IKEA Zigbee2MQTT",
        summary: "IKEA Zigbee air sensors as Zigbee2MQTT publishes them",
        text: include_str!("../../../examples/ikea-zigbee2mqtt.toml"),
        files: &[],
    },
    Example {
        name: "Home Assistant Statestream",
        summary: "A sensor as Home Assistant's MQTT Statestream republishes it",
        text: include_str!("../../../examples/home-assistant-statestream.toml"),
        files: &[],
    },
    Example {
        name: "Home Assistant Discovery",
        summary: "An air sensor and a door contact that appear in Home Assistant by themselves",
        text: include_str!("../../../examples/home-assistant-discovery.toml"),
        files: &[],
    },
    Example {
        name: "Thermostat",
        summary: "A thermostat with scripts that takes setpoint and mode commands over MQTT",
        text: include_str!("../../../examples/thermostat.toml"),
        files: &[],
    },
    Example {
        name: "Binary payloads",
        summary: "One reading as a hex frame, as CBOR and as protobuf",
        text: include_str!("../../../examples/binary-payloads.toml"),
        files: &[(
            "proto/sensor.proto",
            include_str!("../../../examples/proto/sensor.proto"),
        )],
    },
    Example {
        name: "Fleet",
        summary: "Two floors with 100 sensors each, reproducible with a seed",
        text: include_str!("../../../examples/fleet.toml"),
        files: &[],
    },
    Example {
        name: "Showcase",
        summary: "The demo project of the documentation's screenshots",
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
