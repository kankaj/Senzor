# senzor-cli

The headless runner of [Senzor](https://senzor.kankaj.cz), an MQTT sensor simulator: run a project
of simulated sensors in a terminal, in Docker or in CI, without a window.

```bash
cargo install senzor-cli
senzor-cli validate project.toml
senzor-cli run project.toml --duration 10m
```

It also clears the retained messages a run leaves on the broker (`clear-retained`) and lets an AI
agent drive the project over MCP (`mcp`, or `run --mcp-port`). The project file is the same one
the window, [`senzor`](https://crates.io/crates/senzor), edits.

Ready-made builds, a Homebrew formula and the container image `ghcr.io/kankaj/senzor-cli` are in
[Installation](https://senzor.kankaj.cz/docs/installation); [Running headless](https://senzor.kankaj.cz/docs/headless)
explains the commands.

[Documentation](https://senzor.kankaj.cz/docs/) ·
[Source](https://github.com/kankaj/Senzor) · Apache-2.0
