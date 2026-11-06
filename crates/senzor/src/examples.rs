//! The example projects the window offers, the same files the documentation shows.

use senzor_core::project::ProjectFile;

/// An example project: its name in the window, what it shows, and its file's text.
pub struct Example {
    pub name: &'static str,
    pub summary: &'static str,
    text: &'static str,
}

impl Example {
    /// The example's project. The examples are tested to parse.
    pub fn project(&self) -> ProjectFile {
        ProjectFile::parse(self.text).expect("a built-in example parses")
    }
}

/// Every example, the simplest first.
pub const EXAMPLES: [Example; 5] = [
    Example {
        name: "Quick start",
        summary: "Two CO2 sensors and a thermometer on the built-in templates",
        text: include_str!("../../../examples/quick-start.toml"),
    },
    Example {
        name: "IKEA Zigbee2MQTT",
        summary: "IKEA Zigbee air sensors as Zigbee2MQTT publishes them",
        text: include_str!("../../../examples/ikea-zigbee2mqtt.toml"),
    },
    Example {
        name: "Home Assistant Statestream",
        summary: "A sensor as Home Assistant's MQTT Statestream republishes it",
        text: include_str!("../../../examples/home-assistant-statestream.toml"),
    },
    Example {
        name: "Fleet",
        summary: "Two floors with 100 sensors each, reproducible with a seed",
        text: include_str!("../../../examples/fleet.toml"),
    },
    Example {
        name: "Showcase",
        summary: "The demo project of the documentation's screenshots",
        text: include_str!("../../../examples/showcase.toml"),
    },
];

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn every_example_is_a_valid_project() {
        for example in &EXAMPLES {
            let errors = example.project().validate(Path::new(".")).errors;
            assert!(errors.is_empty(), "{}: {errors:?}", example.name);
        }
    }
}
