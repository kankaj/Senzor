# Development brokers

`compose.yaml` runs two Mosquitto brokers for development and for the tests that need a real
broker. Both listen on `127.0.0.1` only and accept anonymous clients.

| Service | Image | Port | Speaks |
| --- | --- | --- | --- |
| `mosquitto` | `eclipse-mosquitto:2` | 18831 | MQTT 5 and 3.1.1 |
| `mosquitto` | `eclipse-mosquitto:2` | 18833 | MQTT 5 and 3.1.1 over WebSocket, path `/` |
| `mosquitto-1-5` | `eclipse-mosquitto:1.5` | 18832 | MQTT 3.1.1 only |

```bash
docker compose -f dev/compose.yaml up -d      # start
cargo test --workspace -- --ignored            # run the tests that need them
docker compose -f dev/compose.yaml down        # stop
```

The tests that need these brokers are marked `#[ignore = "needs the Docker brokers in
dev/compose.yaml"]`, so a plain `cargo test` skips them. They live in
`crates/senzor-core/tests/mosquitto.rs` and cover what the in-process test broker (rumqttd) does
not: the will delay, message expiry of retained messages, and a broker that supports only MQTT
3.1.1. They fail with a message naming this file when nothing answers on the ports. To use other
ports, set `SENZOR_MOSQUITTO_PORT` and `SENZOR_MOSQUITTO15_PORT`.

Watch what Senzor publishes, with MQTT 5 properties:

```bash
docker compose -f dev/compose.yaml exec mosquitto \
  mosquitto_sub -V mqttv5 -t '#' -F '%t %p %P'
```

# Driving the GUI

`gui.sh` runs the real GUI on a virtual X display (Xvfb), so it can be screenshotted and clicked
from a shell without opening a window on the desktop. It needs `Xvfb`, `xdotool`, ImageMagick's
`import` and `dbus-run-session`.

```bash
dev/gui.sh start examples/showcase.toml   # build, launch at 1200×760
dev/gui.sh shot                           # → target/gui-driver/shot.png
dev/gui.sh click 55 85                    # coordinates are pixels in the screenshot
dev/gui.sh type 127.0.0.1                 # into the focused field
dev/gui.sh key ctrl+a Return              # xdotool key names
dev/gui.sh close                          # close like the window's × (the app saves its settings)
dev/gui.sh stop                           # kill the app and the display
```

`dev/gui.sh help` lists every command. The app runs with a scratch `HOME` and its own D-Bus
session under `target/gui-driver/`, so file dialogs open on the virtual display and nothing is
written to the real home folder; its saved settings land in `target/gui-driver/home/.local/share/`.
`close` needs Python's Xlib (`python-xlib`), and a running build uses `CARGO_TARGET_DIR` if set. Edits are not saved unless you click **Save**, so an example
project opened this way stays unchanged. `SENZOR_GUI_RELEASE=1 dev/gui.sh start …` runs an optimised
build, for judging speed: a debug build lays out the window about ten times slower.

# Releasing

`release.sh` prepares a release and `release-check.sh` checks one; `macos-app.sh` turns the
window's macOS archive into `Senzor.app` (the release workflow runs it on a Mac). The `*.test.sh`
scripts test them in throwaway folders and repositories, on any system; CI runs them all.

```bash
dev/release-check.sh                 # is the version in Cargo.toml ready? (an undated section is fine)
dev/release.sh 1.0.0                 # set the version, date CHANGELOG, check, commit and tag; asks first
git push origin main v1.0.0          # starts .github/workflows/release.yml
```

Before `release.sh`, `CHANGELOG.md` needs a `## [X.Y.Z] - Unreleased` section with Highlights and
`crates/senzor/locales/cs/whats-new.md` their Czech version. To build the release archives of
one platform locally (into `target/distrib/`), with the dist version from `mise.toml`:

```bash
mise exec -- dist build --artifacts=local --target x86_64-unknown-linux-gnu
```
