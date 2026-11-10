# Otazník `?` vedle nastavení: text, který ukáže po najetí myší, a jeho název pro čtečky obrazovky.
# Složené závorky jsou syntaxe Fluentu: doslovné {{ … }} se píše jako {"{{ … }}"}.

help-read-more = Kliknutím si přečtete víc v dokumentaci (anglicky).
help-about = Nápověda: { $name }

## Nastavení projektu.

help-host = Host name nebo IP adresa brokera. Připojuje se k němu každá instance projektu.
help-port = Port brokera: pro běžné MQTT obvykle 1883, pro TLS 8883.
help-mqtt-version = MQTT 5 (výchozí), nebo 3.1.1 pro broker, který MQTT 5 nepodporuje. Platí pro všechny instance.
help-transport = TCP, běžné spojení MQTT, nebo MQTT přes WebSocket pro brokery za webovým serverem nebo proxy. S TLS se z WebSocketu stane wss://. Brokery přijímají WebSocket na vlastním portu, například 8080 nebo 8083.
help-websocket-path = Cesta, na které broker nabízí MQTT přes WebSocket, často /mqtt; musí začínat znakem /.
help-client-id-prefix = Client ID každé instance je tento prefix, za kterým následuje ID instance, například senzor-office-0. Broker povolí pro jedno client ID jen jedno spojení.
help-keep-alive = Jak často instance dává brokerovi vědět, že je stále připojená, když nemá nic jiného k odeslání, aby se mrtvé spojení poznalo. Celé sekundy, například 30s.
help-session-expiry = MQTT 5: jak dlouho broker drží session instance po skončení jejího spojení. Je-li nad nulou, opětovné připojení během Will delay šablony Last Will zruší.
help-username = Uživatelské jméno, kterým se přihlašuje každá instance. Když ho necháte prázdné, instance se připojí bez přihlašovacích údajů.
help-password = Odkud se heslo bere: žádné, zapsané v souboru projektu, nebo z proměnné prostředí, takže v souboru není.
help-tls = Připojit se přes TLS. Bez souboru CA ověří brokera kořenové certifikáty systému. Brokery přijímají TLS obvykle na jiném portu, často 8883.
help-seed = Se seedem vygeneruje každý běh stejné náhodné hodnoty; když zůstane prázdný, každý běh vygeneruje nové.

## Zařízení.

help-device-id = Název zařízení, jedinečný v projektu. Jeho instance se jmenují podle něj, pokud ID instancí neurčují jinak.
help-template = Šablona, jejíž topiky, payloady a pole zařízení publikuje.
help-count = Kolik instancí zařízení spouští. Každá má vlastní spojení MQTT a vlastní client ID.
help-instance-ids = ID instancí, která topiky používají jako {"{{ device_id }}"}: pro jednu instanci ID zařízení, pro více instancí ID zařízení s čísly od 0. Zaškrtněte vlastní a napište vlastní vzor.
help-autostart = Spustit zařízení hned, jak se projekt otevře.
help-device-interval = Dát tomuto zařízení vlastní interval publikování místo intervalu šablony.
help-params = Hodnoty, které topiky a payloady šablony používají jako {"{{ name }}"}, stejné pro všechny instance tohoto zařízení.
help-field-overrides = Změnit pole šablony jen pro toto zařízení, například na jinou počáteční hodnotu.

## Šablony.

help-template-id = ID šablony, kterým se na ni zařízení odkazují.
help-template-interval = Jak často každá instance publikuje své zprávy, například 10s.
help-qos = Kvalita služby (QoS) MQTT u datových zpráv: 0 nejvýše jednou, 1 alespoň jednou, 2 právě jednou.
help-retain = Broker uchová poslední datovou zprávu každého topicu a novým odběratelům ji předá jako první.
help-home-assistant = Každá instance se v Home Assistantu objeví jako zařízení s jednou entitou pro každé pole: po připojení publikuje zprávy MQTT Discovery pro Home Assistant jako retained.
help-subscriptions = Topiky, které každá instance odebírá, například topic pro příkazy. Pro každou zprávu, která přijde, se spustí skript odběru: může uchovávat hodnoty ve state, připínat pole a publikovat odpověď.
help-subscription-topic = Filtr topicu, vyplněný pro každou instanci stejně jako topic dostupnosti; + a # odpovídají libovolné úrovni. Odpověď publikovaná do topicu, kterému filtr odpovídá, se vrátí zpět instanci.
help-subscription-script = Kód v Rhai, který se spustí pro každou zprávu: vidí topic, payload, json, state, fields a params a může volat publish, pin a unpin.
help-field-script = Kód v Rhai, který se spustí v každém ticku; hodnota jeho posledního výrazu je hodnotou pole. Vidí value (předchozí hodnotu pole), t, seq, state, fields a params.
help-availability = Topic online/offline: online, když se instance připojí, offline, když se zastaví, a offline jako její Last Will, který broker publikuje, když instance vypadne.
help-will-delay = MQTT 5: jak dlouho broker čeká, než publikuje Last Will. Opětovné připojení během této doby ho zruší. Session expiry u brokera musí být alespoň stejně dlouhé.
help-topic = Kam se zpráva publikuje. Šablona Jinja: vyplní ji {"{{ device_id }}"}, parametry a pole.
help-payload = Co zpráva nese, jako šablona Jinja, například {"{\"co2\": {{ co2 }}}"}.
help-encoding = Jak se vykreslený payload odešle: jako text, nebo jako bajty. hex a base64 čtou z textu bajty; CBOR a protobuf ho čtou jako JSON.
help-proto-file = Soubor .proto, který definuje typ zprávy, relativně ke složce souboru projektu.
help-message-type = Plně kvalifikovaný typ zprávy protobuf, například sensors.v1.Reading: balíček, tečka a název zprávy.
help-message-expiry = MQTT 5: po této době broker zprávu zahodí, místo aby ji doručil.
help-content-type = MQTT 5: sděluje odběratelům, co payload je, například application/json.
help-user-properties = MQTT 5: dvojice názvu a hodnoty odeslané se zprávou; hodnoty mohou používat {"{{ }}"} stejně jako payload.
help-field-name = Čitelný název pole, například CO2, zobrazený v pohledu instance. Když zůstane prázdný, zobrazí se klíč pole.
help-field-unit = Jednotka hodnoty pole, například ppm, °C nebo %, zobrazená za hodnotou v pohledu instance.
help-fields = Hodnoty, které se mění v každém ticku, každá z generátoru, například random_walk nebo sine.
