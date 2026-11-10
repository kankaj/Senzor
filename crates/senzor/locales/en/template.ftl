# The template editor and its preview.

template-heading = Template { $id }
template-duplicate = Duplicate template
template-delete = Delete template
template-delete-hint = Delete the template
template-builtin-missing = There is no built-in template `{ $id }`.
template-builtin-heading = Built-in template { $id }
template-builtin-read-only = Built-in templates are read-only. Copy one into the project to change it.
template-copy-into-project = Copy into project…

## Sections

template-settings = Settings
template-messages = Messages
template-fields = Fields
template-subscriptions = Subscriptions
template-preview = Preview

## Settings

template-id = ID
template-interval = Interval
template-retain = Retain
template-retain-checkbox = retain data messages
template-home-assistant-checkbox = announce over MQTT Discovery
template-no-device = No device uses this template yet.
template-every-field-announced = Every field is announced.
template-not-announced = Not announced, not in one place of a payload: { $fields }
template-availability = Availability
template-availability-checkbox = online/offline topic with Last Will
template-online-payload = Online payload
template-offline-payload = Offline payload
template-will-delay = Will delay

## Messages

template-message-number = Message { $number }
template-remove = Remove
template-topic = Topic
template-payload = Payload
template-mqtt5-properties = MQTT 5 properties
template-add-message = Add message
template-encoding = Encoding
template-proto-file = Proto file
template-proto-file-hint = e.g. proto/sensor.proto
template-message-type = Message type
template-message-type-hint = e.g. sensors.v1.Reading
template-message-expiry = Message expiry
template-content-type = Content type
template-content-type-hint = none
template-user-properties = User properties
template-property-name-hint = name
template-property-name-required = a name is required
template-property-name-used = `{ $name }` is used already
template-property-value-hint = value, e.g. { "{{ room }}" }
template-remove-property = Remove property
template-add-user-property = Add user property

## Fields

template-name = Name
template-unit = Unit
template-unit-hint = e.g. ppm
template-add-field = Add field

## Subscriptions

template-subscription-number = Subscription { $number }
template-remove-subscription = Remove subscription { $number }
template-script = Script
template-subscription-script = Subscription { $number } script
template-add-subscription = Add subscription

## Preview

template-rendered-as = Rendered as { $instance } with its params and sample values.
template-no-valid-device = No valid device uses this template, so params are not available.
template-preview-availability = availability: { $topic }
template-preview-availability-error = availability topic: { $error }
template-preview-not-encoded = not encoded as { $encoding }: { $error }
template-preview-content-type = content type: { $value }
template-preview-message-expiry = message expiry: { $seconds } s
template-preview-user-property = user property: { $name } = { $value }
template-preview-message-error = message { $number }: { $error }
