# Problémy validace a jejich umístění v češtině. Klíče a proměnné jsou stejné jako v
# ../en/problems.ftl. Klíče TOML, hodnoty a identifikátory v obrácených apostrofech zůstávají
# anglicky; texty z knihoven (chyby TOML, MiniJinja, Rhai, protox, souborů) také.

## Umístění

location-line = řádek { $line }, sloupec { $column }
location-file = soubor projektu
location-broker = broker
location-template = šablona `{ $id }`
location-device = zařízení `{ $id }`

## Společné

# Text z knihovny (chyba TOML nebo MiniJinja), zobrazený tak, jak je.
verbatim = { $text }
needs-mqtt5-note = vyžaduje MQTT 5; nastavení brokeru používá MQTT 3.1.1
needs-mqtt5 = { $key } { needs-mqtt5-note }
must-not-be-empty = hodnota { $key } nesmí být prázdná
at-least-1s = { $key } musí být alespoň 1s
greater-than-zero = { $key } musí být větší než 0
whole-seconds = { $key } musí být v celých sekundách, zadáno { $duration }
at-most-seconds = { $key } smí být nejvýše { $max } { $max ->
    [one] sekunda
    [few] sekundy
   *[other] sekund
}
qos-range = qos musí být 0, 1 nebo 2, zadáno { $qos }

## Broker

broker-port-zero = port nesmí být 0
keep-alive-too-long = keep_alive smí být nejvýše 65535 sekund (18h 12m 15s)
websocket-path-slash = cesta `{ $path }` musí začínat znakem /
password-and-password-env = použijte buď password, nebo password_env, ne obojí
password-env-not-set = proměnná prostředí `{ $var }` (password_env) není nastavená
password-without-username = heslo vyžaduje uživatelské jméno (username)
tls-cannot-read = nelze přečíst tls.{ $key } `{ $path }`: { $reason }
tls-cert-and-key = tls.client_cert a tls.client_key musí být nastavené společně
tls-cert-needs-ca = tls.client_cert vyžaduje tls.ca_file
tls-no-certificate = tls.{ $key } `{ $path }` neobsahuje žádný certifikát PEM
tls-no-private-key = tls.client_key `{ $path }` neobsahuje žádný soukromý klíč PEM
tls-client-key = tls.client_key: { $reason }
tls-no-system-certificates = v tomto systému nejsou žádné důvěryhodné certifikáty; nastavte tls.ca_file
tls-setup = TLS: { $reason }

## Šablony

template-builtin-id = `{ $id }` je ID vestavěné šablony; zvolte jiné ID
template-duplicate-id = duplicitní ID šablony
template-no-messages = je potřeba alespoň jedna zpráva
template-no-fields = je potřeba alespoň jedno pole
field-reserved-name = pole `{ $name }` používá vyhrazený název
field-problem = pole `{ $name }`: { $problem }
message-problem = zpráva { $n }: { $problem }
proto-without-protobuf = proto se používá jen s kódováním protobuf
message-type-without-protobuf = message se používá jen s kódováním protobuf
protobuf-needs-proto = kódování protobuf vyžaduje soubor proto
protobuf-needs-message-type = kódování protobuf vyžaduje typ zprávy
proto-cannot-read = nelze přečíst proto `{ $path }`: { $reason }
proto-does-not-compile = proto `{ $path }`: { $error }
proto-no-message-type = proto `{ $path }` nemá typ zprávy `{ $name }`
proto-no-message-type-did-you-mean = proto `{ $path }` nemá typ zprávy `{ $name }`; nemysleli jste `{ $full }`?
will-delay-after-session-expiry = availability.will_delay { $delay } je delší než session_expiry brokeru { $expiry }; nastavte session_expiry alespoň na { $delay }
subscription-problem = odběr { $n }: { $problem }
script-empty = skript je prázdný
script-does-not-compile = skript nejde zkompilovat: { $reason }

## Problémy polí

numbers-not-finite = čísla musí být konečná
noise-needs-numbers = noise vyžaduje číselné hodnoty
min-not-below-max = min musí být menší než max
start-outside-bounds = start musí ležet mezi min a max

## Zařízení

device-duplicate-id = duplicitní ID zařízení
device-unknown-template = neznámá šablona `{ $template }`
count-zero = count musí být alespoň 1
param-reserved-name = parametr `{ $name }` používá vyhrazený název
param-field-name = parametr `{ $name }` se jmenuje stejně jako pole
field-override-unknown = přepsání neznámého pole `{ $name }`
field-override-problem = přepsání pole `{ $name }`: { $problem }

## Vykreslení

availability-topic-problem = topic dostupnosti: { $problem }
message-topic-problem = topic zprávy { $n }: { $problem }
topic-empty = topic je prázdný
topic-wildcard = topic `{ $topic }` obsahuje wildcard (+ nebo #)
subscription-topic-empty = topic odběru { $n } je prázdný
subscription-topic-invalid = topic odběru { $n } `{ $filter }`: { $problem }
filter-hash-not-last = `#` musí tvořit celou poslední úroveň, jako v `a/#`
filter-plus-not-whole = `+` musí tvořit celou úroveň, jako v `a/+/b`
message-payload-problem = payload zprávy { $n } jako { $encoding }: { $problem }
hex-odd-digits = lichý počet šestnáctkových číslic ({ $count })
hex-bad-digit = `{ $char }` není šestnáctková číslice
base64-bad-character = `{ $char }` není znak base64
base64-last-character = poslední znak `{ $char }` nechává přebytečné bity; text je zkrácený nebo změněný
base64-length = délka neodpovídá base64; jeden znak chybí nebo přebývá
base64-padding = base64 potřebuje doplnění (=) na násobek 4 znaků
cbor-not-json = není JSON: { $reason }
user-property-problem = { $name }: { $error }

## ID instancí

instance-id-template = instance_id: { $error }
instance-id-empty = instance_id dává prázdný řetězec
instance-id-repeats = ID instance `{ $id }` se opakuje; vložte {"{{"} index {"}}"} do instance_id
instance-id-taken = ID instance `{ $id }` už používá zařízení `{ $device }`
