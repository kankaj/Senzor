# Validation problems and their locations, as the CLI, MCP and the GUI show them.
# website/content/docs/reference/validation-errors.mdx lists each problem: change both together.
# The Czech catalog (../cs/problems.ftl) has the same keys with the same variables.

## Locations

location-line = line { $line }, column { $column }
location-file = project file
location-broker = broker
location-template = template `{ $id }`
location-device = device `{ $id }`

## Shared

# A text that comes from a library (a TOML or MiniJinja error), shown as it is.
verbatim = { $text }
needs-mqtt5-note = needs MQTT 5; the broker settings use MQTT 3.1.1
needs-mqtt5 = { $key } { needs-mqtt5-note }
must-not-be-empty = { $key } must not be empty
at-least-1s = { $key } must be at least 1s
greater-than-zero = { $key } must be greater than 0
whole-seconds = { $key } must be whole seconds, got { $duration }
at-most-seconds = { $key } must be at most { $max } { $max ->
    [one] second
   *[other] seconds
}
qos-range = qos must be 0, 1 or 2, got { $qos }

## Broker

broker-port-zero = port must not be 0
keep-alive-too-long = keep_alive must be at most 65535 seconds (18h 12m 15s)
websocket-path-slash = path `{ $path }` must start with /
password-and-password-env = use either password or password_env, not both
password-env-not-set = environment variable `{ $var }` (password_env) is not set
password-without-username = a password requires a username
tls-cannot-read = cannot read tls.{ $key } `{ $path }`: { $reason }
tls-cert-and-key = tls.client_cert and tls.client_key must be set together
tls-cert-needs-ca = tls.client_cert requires tls.ca_file
tls-no-certificate = tls.{ $key } `{ $path }` holds no PEM certificate
tls-no-private-key = tls.client_key `{ $path }` holds no PEM private key
tls-client-key = tls.client_key: { $reason }
tls-no-system-certificates = no trusted certificates found on this system; set tls.ca_file
tls-setup = TLS: { $reason }

## Templates

template-builtin-id = `{ $id }` is a built-in template id; choose another id
template-duplicate-id = duplicate template id
template-no-messages = at least one message is required
template-no-fields = at least one field is required
field-reserved-name = field `{ $name }` uses a reserved name
field-problem = field `{ $name }`: { $problem }
message-problem = message { $n }: { $problem }
proto-without-protobuf = proto is used only with encoding protobuf
message-type-without-protobuf = message is used only with encoding protobuf
protobuf-needs-proto = encoding protobuf needs a proto file
protobuf-needs-message-type = encoding protobuf needs a message type
proto-cannot-read = cannot read proto `{ $path }`: { $reason }
proto-does-not-compile = proto `{ $path }`: { $error }
proto-no-message-type = proto `{ $path }` has no message type `{ $name }`
proto-no-message-type-did-you-mean = proto `{ $path }` has no message type `{ $name }`; did you mean `{ $full }`?
will-delay-after-session-expiry = availability.will_delay { $delay } is longer than the broker's session_expiry { $expiry }; set session_expiry to at least { $delay }
subscription-problem = subscription { $n }: { $problem }
script-empty = script is empty
script-does-not-compile = script does not compile: { $reason }

## Field problems

numbers-not-finite = numbers must be finite
noise-needs-numbers = noise requires numeric values
min-not-below-max = min must be less than max
start-outside-bounds = start must be between min and max

## Devices

device-duplicate-id = duplicate device id
device-unknown-template = unknown template `{ $template }`
count-zero = count must be at least 1
param-reserved-name = param `{ $name }` uses a reserved name
param-field-name = param `{ $name }` has the same name as a field
field-override-unknown = override of unknown field `{ $name }`
field-override-problem = field `{ $name }` override: { $problem }

## Rendering

availability-topic-problem = availability topic: { $problem }
message-topic-problem = message { $n } topic: { $problem }
topic-empty = topic is empty
topic-wildcard = topic `{ $topic }` contains a wildcard (+ or #)
subscription-topic-empty = subscription { $n } topic is empty
subscription-topic-invalid = subscription { $n } topic `{ $filter }`: { $problem }
filter-hash-not-last = `#` must be the whole last level, as in `a/#`
filter-plus-not-whole = `+` must be a whole level, as in `a/+/b`
message-payload-problem = message { $n } payload as { $encoding }: { $problem }
hex-odd-digits = odd number of hex digits ({ $count })
hex-bad-digit = `{ $char }` is not a hex digit
base64-bad-character = `{ $char }` is not a base64 character
base64-last-character = the last character `{ $char }` leaves bits over; the text is cut short or changed
base64-length = the length does not fit base64; a character is missing or one too many
base64-padding = base64 needs padding (=) to a multiple of 4 characters
cbor-not-json = not JSON: { $reason }
user-property-problem = { $name }: { $error }

## Instance IDs

instance-id-template = instance_id: { $error }
instance-id-empty = instance_id renders to an empty string
instance-id-repeats = instance id `{ $id }` repeats; include {"{{"} index {"}}"} in instance_id
instance-id-taken = instance id `{ $id }` is already used by device `{ $device }`
