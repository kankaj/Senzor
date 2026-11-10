# The Project settings window: the broker and the simulation.

broker-title = Project settings
broker-broker = Broker
broker-presets = Presets
broker-presets-hint = Fill in a known broker's address; credentials and TLS are cleared
broker-preset-local = Mosquitto on this computer
broker-preset-local-hint = A broker running on this computer, installed or in Docker
broker-preset-mosquitto = Mosquitto test server
broker-preset-hivemq = HiveMQ public broker
broker-preset-emqx = EMQX public broker
broker-preset-public-hint = Public: anyone can read what is published there
broker-host = Host
broker-port = Port
broker-mqtt-version = MQTT version
broker-transport = Transport
broker-path = Path
broker-client-id-prefix = Client ID prefix
broker-keep-alive = Keep alive
broker-session-expiry = Session expiry
broker-username = Username
broker-none = none
broker-password = Password
broker-tls = TLS
broker-use-tls = use TLS
broker-ca-file = CA file
broker-client-cert = Client certificate
broker-client-key = Client key
broker-tls-path-hint = system roots / none
broker-relative-paths = Relative paths are resolved against the project file's folder.
broker-test-connection = Test connection
broker-test-connection-hint = Connect once with these settings, then disconnect
broker-fix-problems = Fix the broker problems first
broker-testing = Testing…
broker-connecting = connecting…
broker-connected = Connected
broker-clear-retained = Clear retained messages…
broker-clear-retained-hint = Delete the retained messages this project's instances leave on the broker, such as their last offline
broker-clear-while-running = Stop the devices first: running instances would publish their retained messages again
broker-clearing-hint = Clearing…
broker-clearing = clearing…
broker-cleared =
    { $count ->
        [one] Cleared 1 retained topic
       *[other] Cleared { $count } retained topics
    }
broker-nothing-to-clear = Nothing to clear: no template has availability, retains its messages or announces to Home Assistant.
broker-simulation = Simulation
broker-seed = Seed
broker-seed-random = random
broker-seed-hint = use a whole number, or leave it empty
broker-seed-text = With a seed, every run draws the same random values.
broker-run-elsewhere = Run elsewhere
broker-snippet-hint = Puts text on the clipboard that runs this project with senzor-cli
broker-snippet-disabled = Save the project first: the snippet names its file
broker-copied = Copied.
broker-run-elsewhere-text = See the CI and Docker recipe in the documentation for the rest of the job.
