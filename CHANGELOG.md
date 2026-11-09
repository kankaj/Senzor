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
- Built-in templates and eight example projects to start from.
- Scripts in Rhai compute field values, and subscriptions let simulated devices take commands: change their state, pin a field or answer.
- Simulated devices can appear in Home Assistant by themselves, through MQTT Discovery.
- Problems are shown next to the box they are about, and a device with problems does not start.
- AI agents such as Claude Code can drive Senzor over MCP, headless or in the window.

### Added

#### Projects

- One TOML project file holds the broker settings, templates and devices. A template defines topics, payloads, fields and the interval; a device runs it `count` times.
- Topics and payloads are Jinja templates, so payloads can be JSON, plain numbers or any text format.
- Binary payloads: a message's `encoding` turns its rendered payload into bytes, from hex digits (`hex`), base64 (`base64`), or JSON as CBOR (`cbor`) or as a protobuf message of a `.proto` file (`protobuf`, with `proto` and `message`). Filters such as `u8`, `i16be` and `f32le` write a number as the hex digits of a binary integer or float. The `binary-payloads` example uses all three.
- Field generators: `constant`, `random_walk`, `sine`, `uniform` and `sequence`, with optional noise, and per-device field overrides.
- The `script` generator computes a field's value with a Rhai script, from its previous value, the time, the params, the other fields and a `state` shared by the instance's scripts. Scripts are compiled when the project is checked and limited in how long they run and how much they build.
- Subscriptions (`[[templates.subscriptions]]`): each instance subscribes to topic filters, with `+` and `#`, and runs a script for every message that arrives. The script sees the topic, the payload and the payload read as JSON, and can change `state`, pin and unpin fields, and publish replies. The `thermostat` example takes a setpoint and a mode this way.
- A field can have a unit and a readable name, such as `ppm` and `CO2`; the built-in templates set both.
- Built-in templates `co2`, `temperature` and `humidity`.
- Availability: an online and offline topic per instance, with a Last Will the broker publishes when an instance drops off.
- A seed makes every run of a project draw the same values.
- Username and password (the password can come from an environment variable), TLS and client certificates.
- MQTT over WebSocket (`transport = "websocket"`, with a `path`), also over TLS as `wss://`.
- Validation collects every problem in the project with its location; only what a problem affects is blocked.
- Saving changes only what was edited in the file: comments, the layout and how values are written stay as you wrote them.

#### Running

- Start, Stop, Restart and Kill for the whole project, a device or one instance. Kill drops the connection, so the broker publishes the Last Will.
- Pin a field of a running instance to a value until you unpin it.
- Each instance counts the messages it receives on its subscriptions, next to the ones it publishes; a script that fails counts as an error, and the instance keeps running.
- Edits to a running project reach running instances at once, without a reconnect. Only a change to the broker settings or a template's availability marks the instances that need a restart; nothing restarts by itself.
- Clear the retained messages a project leaves on the broker, in the window or with `senzor-cli clear-retained`.
- Home Assistant MQTT Discovery: with `home_assistant = true` on a template, each instance announces itself when it connects, as a device with one entity per field. Senzor finds each field in the payload itself; the template editor names the fields it cannot find.

#### Window

- A device tree with search, a status filter and "4/5 online" counts, an editor for devices, templates and the broker, and a live preview of every template.
- A message log in columns with QoS and retained, filtered by topic or by clicking an instance, with each message's details and indented payload, export to CSV and a Clear button. Binary payloads show as hex, with their encoding and size.
- A Problems tab, and a status bar with the broker state, instance counts and messages per second.
- A `?` next to every setting that explains it and opens its documentation.
- Select several instances with Ctrl+click or Shift+click to start, stop, restart, kill or pin a field on all of them.
- A command palette (Ctrl+K) that runs any action and finds any device, instance or template.
- Project settings copy the project as a GitHub Actions step or a docker-compose service that runs it with `senzor-cli`.
- The template editor folds into sections, shows its preview beside the form on a wide window, colours Jinja and JSON, and inserts and completes variables in topics and payloads.
- The instance view shows each field's name and unit, such as `CO2` and `611.77 ppm`.
- Field overrides in the device editor use the same boxes as the template editor, with a checkbox per setting to override it.
- New, Open, Save, recent projects, example projects, drag and drop to open, and a question when the file changes on disk.
- The TOML button shows the project file as text to edit by hand; Apply takes it as one undoable edit, and its comments are saved with it.
- Undo and redo, duplicate devices and templates, keyboard shortcuts, and a test of the broker connection with presets for common brokers.
- Settings for the theme, zoom and MCP, remembered between launches with the window's size and panels.
- A desktop notification when instances fail while the window is in the background, for example when the broker refuses the credentials; it can be turned off in the settings.
- The status bar says when a newer version is out on GitHub, with a link to its release page. Senzor asks at most once a day, and the check can be turned off in the settings.

#### Command line

- `senzor-cli schema` prints the project file's JSON Schema, which the docs site also publishes for editors; a `#:schema` line at the top of a project file is kept when Senzor saves it.
- `senzor-cli validate` checks a project, `senzor-cli run` runs it (with `--only` and `--duration`), and `senzor-cli clear-retained` clears its retained messages. Exit codes tell CI what happened.
- `senzor-cli run --watch` applies each save of the project file: it updates the devices that changed, restarts only those whose connection changed, starts added ones and stops removed ones.

#### AI agents

- `senzor-cli mcp` runs a project for an AI agent over MCP; the window offers the same tools once MCP is turned on in its settings.
- Agents on other machines can connect with a token: a settings switch in the window, and `senzor-cli run --mcp-port` with `--mcp-bind` and `SENZOR_MCP_TOKEN` for runs in containers.
- An agent can change one value by its path with `set` (e.g. `devices.office.count`), remove a key with `unset`, and step through the project's history with `undo` and `redo`; in the window, that history is the one Ctrl+Z uses.
- Agents can subscribe to the project, the instances and the message log instead of polling, and three prompts (`simulate-device`, `test-consumer`, `explain-problems`) start common tasks.

[1.0.0]: https://github.com/kankaj/Senzor/releases/tag/v1.0.0
