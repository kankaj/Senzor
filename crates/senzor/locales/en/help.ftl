# The `?` next to a setting: its hover text, and its name for screen readers.
# Braces are Fluent syntax: a literal {{ … }} is written as {"{{ … }}"}.

help-read-more = Click to read more in the documentation.
help-about = About { $name }

## Project settings.

help-host = The broker's host name or IP address. Every instance of the project connects there.
help-port = The broker's port: 1883 is usual for plain MQTT, 8883 for TLS.
help-mqtt-version = MQTT 5, the default, or 3.1.1 for a broker that does not support MQTT 5. It applies to every instance.
help-transport = TCP, the usual MQTT connection, or MQTT over WebSocket for brokers behind a web server or proxy. With TLS, WebSocket becomes wss://. Brokers take WebSocket on their own port, such as 8080 or 8083.
help-websocket-path = The path the broker serves MQTT over WebSocket on, often /mqtt; it must start with /.
help-client-id-prefix = Each instance's client ID is this prefix followed by its instance ID, such as senzor-office-0. A broker allows one connection per client ID.
help-keep-alive = How often an instance tells the broker it is still there when it has nothing else to send, so a dead connection is noticed. Whole seconds, such as 30s.
help-session-expiry = MQTT 5: how long the broker keeps an instance's session after its connection ends. Above zero, a reconnect within the template's Will delay cancels the Last Will.
help-username = The username every instance logs in with. Leave it empty to connect without credentials.
help-password = Where the password comes from: none, typed into the project file, or an environment variable, which keeps it out of the file.
help-tls = Connect over TLS. Without a CA file, the system's root certificates check the broker. Brokers usually take TLS on another port, often 8883.
help-seed = With a seed, every run draws the same random values; left empty, each run draws fresh ones.

## Devices.

help-device-id = The device's name, unique in the project. Its instances are named after it unless Instance IDs say otherwise.
help-template = The template whose topics, payloads and fields the device publishes.
help-count = How many instances the device runs. Each has its own MQTT connection and client ID.
help-instance-ids = The instances' IDs, which topics use as {"{{ device_id }}"}: the device ID for one instance, numbered from 0 for more. Tick custom to write a pattern.
help-autostart = Start the device as soon as the project opens.
help-device-interval = Give this device its own publish interval instead of the template's.
help-params = Values the template's topics and payloads use as {"{{ name }}"}, the same for every instance of this device.
help-field-overrides = Change a field of the template for this device only, such as another start value.

## Templates.

help-template-id = The template's ID, which devices name to use it.
help-template-interval = How often each instance publishes its messages, such as 10s.
help-qos = The MQTT quality of service of the data messages: 0 at most once, 1 at least once, 2 exactly once.
help-retain = The broker keeps each topic's last data message and hands it to new subscribers first.
help-home-assistant = Each instance appears in Home Assistant as a device with one entity per field: when it connects, it publishes Home Assistant's MQTT Discovery messages, retained.
help-subscriptions = Topics each instance subscribes to, such as a command topic. For every message that arrives, the subscription's script runs: it can keep values in state, pin fields and publish a reply.
help-subscription-topic = The topic filter, rendered per instance like the availability topic; + and # match any level. A reply published to a topic it matches comes back to the instance.
help-subscription-script = Rhai code run for each message: it sees topic, payload, json, state, fields and params, and can call publish, pin and unpin.
help-field-script = Rhai code run on every tick; its last value is the field's. It sees value (the last one), t, seq, state, fields and params.
help-availability = An online/offline topic: online when an instance connects, offline when it stops, and offline as its Last Will, which the broker publishes if the instance drops off.
help-will-delay = MQTT 5: how long the broker waits before it publishes the Last Will. A reconnect within that time cancels it. The broker's Session expiry must be at least as long.
help-topic = Where the message is published. A Jinja template: {"{{ device_id }}"}, params and fields fill it in.
help-payload = What the message carries, as a Jinja template, such as {"{\"co2\": {{ co2 }}}"}.
help-encoding = How the rendered payload is sent: as text, or as bytes. hex and base64 read the text as bytes; CBOR and protobuf read it as JSON.
help-proto-file = The .proto file that defines the message type, relative to the project file's folder.
help-message-type = The fully qualified protobuf message type, such as sensors.v1.Reading: the package, a dot and the message name.
help-message-expiry = MQTT 5: after this time the broker drops the message instead of delivering it.
help-content-type = MQTT 5: tells subscribers what the payload is, such as application/json.
help-user-properties = MQTT 5: name and value pairs sent with the message; values can use {"{{ }}"} like payloads.
help-field-name = A readable name for the field, such as CO2, shown in the instance view. Left empty, the field's key is shown.
help-field-unit = The unit of the field's value, such as ppm, °C or %, shown after the value in the instance view.
help-fields = The values that change on every tick, each from a generator such as random_walk or sine.
