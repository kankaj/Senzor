# Changelog

Every release of Senzor, the newest first. Each version starts with **Highlights**, the short list
the app shows under **What's new** the first time it opens after an update; the sections after it
list the changes in full.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and Senzor uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] - Unreleased

The first release.

### Highlights

- Simulate MQTT sensors from templates: one template gives as many devices and instances as you need, each with its own connection.
- A desktop window to edit a project, start and stop devices, pin values and watch every message, and `senzor-cli` to run the same project in a terminal, in Docker or in CI.
- MQTT 5 by default, with message expiry, content type, user properties and a delayed Last Will; MQTT 3.1.1 can be chosen per project.
- Built-in templates and five example projects to start from.
- Problems are shown next to the box they are about, and a device with problems does not start.
- AI agents such as Claude Code can drive Senzor over MCP, headless or in the window.

### Added

#### Projects

- One TOML project file holds the broker settings, templates and devices. A template defines topics, payloads, fields and the interval; a device runs it `count` times.
- Topics and payloads are Jinja templates, so payloads can be JSON, plain numbers or any text format.
- Field generators: `constant`, `random_walk`, `sine`, `uniform` and `sequence`, with optional noise, and per-device field overrides.
- A field can have a unit and a readable name, such as `ppm` and `CO2`; the built-in templates set both.
- Built-in templates `co2`, `temperature` and `humidity`.
- Availability: an online and offline topic per instance, with a Last Will the broker publishes when an instance drops off.
- A seed makes every run of a project draw the same values.
- Username and password (the password can come from an environment variable), TLS and client certificates.
- Validation collects every problem in the project with its location; only what a problem affects is blocked.

#### Running

- Start, Stop, Restart and Kill for the whole project, a device or one instance. Kill drops the connection, so the broker publishes the Last Will.
- Pin a field of a running instance to a value until you unpin it.
- Edits to a running project mark the instances that need a restart; nothing restarts by itself.
- Clear the retained messages a project leaves on the broker, in the window or with `senzor-cli clear-retained`.

#### Window

- A device tree with search, a status filter and "4/5 online" counts, an editor for devices, templates and the broker, and a live preview of every template.
- A message log in columns with QoS and retained, filtered by topic or by clicking an instance, with each message's details and indented payload, and export to CSV.
- A Problems tab, and a status bar with the broker state, instance counts and messages per second.
- A `?` next to every setting that explains it and opens its documentation.
- The template editor folds into sections, shows its preview beside the form on a wide window, colours Jinja and JSON, and inserts and completes variables in topics and payloads.
- The instance view shows each field's name and unit, such as `CO2` and `611.77 ppm`.
- Field overrides in the device editor use the same boxes as the template editor, with a checkbox per setting to override it.
- New, Open, Save, recent projects, example projects, drag and drop to open, and a question when the file changes on disk.
- Undo and redo, duplicate devices and templates, keyboard shortcuts, and a test of the broker connection with presets for common brokers.
- Settings for the theme, zoom and MCP, remembered between launches with the window's size and panels.

#### Command line

- `senzor-cli validate` checks a project, `senzor-cli run` runs it (with `--only` and `--duration`), and `senzor-cli clear-retained` clears its retained messages. Exit codes tell CI what happened.

#### AI agents

- `senzor-cli mcp` runs a project for an AI agent over MCP; the window offers the same tools once MCP is turned on in its settings.

[1.0.0]: https://github.com/kankaj/Senzor/releases/tag/v1.0.0
