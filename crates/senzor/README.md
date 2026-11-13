# senzor

The window of [Senzor](https://senzor.kankaj.cz), an MQTT sensor simulator: edit a project, start
and stop simulated sensors, pin values and watch every message they publish.

You describe a kind of sensor once, as a template, and Senzor runs as many copies of it as you ask
for, each with its own connection to your broker. The same project file runs headless with
[`senzor-cli`](https://crates.io/crates/senzor-cli).

```bash
cargo install senzor
senzor
```

On Linux the window needs the usual desktop libraries (X11 or Wayland, xkbcommon, OpenGL); see
[Installation](https://senzor.kankaj.cz/docs/installation#building-from-source). Ready-made builds
for Linux, macOS and Windows, a Homebrew cask and the Microsoft Store are on the same page.

[Documentation](https://senzor.kankaj.cz/docs/) ·
[Source](https://github.com/kankaj/Senzor) · Apache-2.0
