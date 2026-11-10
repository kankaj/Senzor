# Editor šablony a její náhled.

template-heading = Šablona { $id }
template-duplicate = Duplikovat šablonu
template-delete = Odstranit šablonu
template-delete-hint = Odstranit šablonu
template-builtin-missing = Vestavěná šablona `{ $id }` neexistuje.
template-builtin-heading = Vestavěná šablona { $id }
template-builtin-read-only = Vestavěné šablony jsou jen pro čtení. Chcete-li šablonu změnit, zkopírujte ji do projektu.
template-copy-into-project = Zkopírovat do projektu…

## Sections

template-settings = Nastavení
template-messages = Zprávy
template-fields = Pole
template-subscriptions = Odběry
template-preview = Náhled

## Settings

template-id = ID
template-interval = Interval
template-retain = Retain
template-retain-checkbox = publikovat datové zprávy jako retained
template-home-assistant-checkbox = ohlásit přes MQTT Discovery
template-no-device = Tuto šablonu zatím nepoužívá žádné zařízení.
template-every-field-announced = Všechna pole jsou ohlášena.
template-not-announced = Neohlášená pole (nejsou v payloadu na jednom místě): { $fields }
template-availability = Dostupnost
template-availability-checkbox = topic online/offline s Last Will
template-online-payload = Online payload
template-offline-payload = Offline payload
template-will-delay = Will delay

## Messages

template-message-number = Zpráva { $number }
template-remove = Odstranit
template-topic = Topic
template-payload = Payload
template-mqtt5-properties = MQTT 5 properties
template-add-message = Přidat zprávu
template-encoding = Kódování
template-proto-file = Soubor proto
template-proto-file-hint = např. proto/sensor.proto
template-message-type = Typ zprávy
template-message-type-hint = např. sensors.v1.Reading
template-message-expiry = Message expiry
template-content-type = Content type
template-content-type-hint = žádný
template-user-properties = User properties
template-property-name-hint = název
template-property-name-required = název je povinný
template-property-name-used = `{ $name }` se už používá
template-property-value-hint = hodnota, např. { "{{ room }}" }
template-remove-property = Odstranit property
template-add-user-property = Přidat user property

## Fields

template-name = Název
template-unit = Jednotka
template-unit-hint = např. ppm
template-add-field = Přidat pole

## Subscriptions

template-subscription-number = Odběr { $number }
template-remove-subscription = Odstranit odběr { $number }
template-script = Skript
template-subscription-script = Skript odběru { $number }
template-add-subscription = Přidat odběr

## Preview

template-rendered-as = Vykresleno jako { $instance } s jejími parametry a ukázkovými hodnotami.
template-no-valid-device = Tuto šablonu nepoužívá žádné platné zařízení, parametry proto nejsou k dispozici.
template-preview-availability = dostupnost: { $topic }
template-preview-availability-error = topic dostupnosti: { $error }
template-preview-not-encoded = nelze zakódovat jako { $encoding }: { $error }
template-preview-content-type = content type: { $value }
template-preview-message-expiry = message expiry: { $seconds } s
template-preview-user-property = user property: { $name } = { $value }
template-preview-message-error = zpráva { $number }: { $error }
