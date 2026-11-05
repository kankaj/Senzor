# Development brokers

`compose.yaml` runs two Mosquitto brokers for development and for the tests that need a real
broker. Both listen on `127.0.0.1` only and accept anonymous clients.

| Service | Image | Port | Speaks |
| --- | --- | --- | --- |
| `mosquitto` | `eclipse-mosquitto:2` | 18831 | MQTT 5 and 3.1.1 |
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
