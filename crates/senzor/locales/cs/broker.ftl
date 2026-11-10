# Okno Nastavení projektu: broker a simulace.

broker-title = Nastavení projektu
broker-broker = Broker
broker-presets = Předvolby
broker-presets-hint = Vyplní adresu známého brokeru; přihlašovací údaje a TLS se vymažou
broker-preset-local = Mosquitto na tomto počítači
broker-preset-local-hint = Broker spuštěný na tomto počítači, nainstalovaný nebo v Dockeru
broker-preset-mosquitto = Testovací server Mosquitto
broker-preset-hivemq = Veřejný broker HiveMQ
broker-preset-emqx = Veřejný broker EMQX
broker-preset-public-hint = Veřejný: co se tam publikuje, může číst kdokoli
broker-host = Host
broker-port = Port
broker-mqtt-version = Verze MQTT
broker-transport = Transport
broker-path = Cesta
broker-client-id-prefix = Prefix client ID
broker-keep-alive = Keep alive
broker-session-expiry = Session expiry
broker-username = Uživatelské jméno
broker-none = nenastaveno
broker-password = Heslo
broker-tls = TLS
broker-use-tls = použít TLS
broker-ca-file = Soubor CA
broker-client-cert = Certifikát klienta
broker-client-key = Klíč klienta
broker-tls-path-hint = systémové CA / žádný
broker-relative-paths = Relativní cesty vedou od složky souboru projektu.
broker-test-connection = Vyzkoušet připojení
broker-test-connection-hint = Jednou se s tímto nastavením připojí a zase odpojí
broker-fix-problems = Nejdřív opravte problémy brokeru
broker-testing = Zkouší se…
broker-connecting = připojuje se…
broker-connected = Připojeno
broker-clear-retained = Smazat retained zprávy…
broker-clear-retained-hint = Smaže retained zprávy, které instance tohoto projektu nechávají na brokeru, například jejich poslední offline
broker-clear-while-running = Nejdřív zastavte zařízení: běžící instance by své retained zprávy publikovaly znovu
broker-clearing-hint = Maže se…
broker-clearing = maže se…
broker-cleared =
    { $count ->
        [one] Retained zpráva smazána (1 topic)
        [few] Retained zprávy smazány ({ $count } topiky)
       *[other] Retained zprávy smazány ({ $count } topiků)
    }
broker-nothing-to-clear = Není co mazat: žádná šablona nemá dostupnost, nepublikuje retained zprávy ani se neohlašuje Home Assistantu.
broker-simulation = Simulace
broker-seed = Seed
broker-seed-random = náhodný
broker-seed-hint = zadejte celé číslo, nebo pole nechte prázdné
broker-seed-text = Se seedem dá každý běh stejné náhodné hodnoty.
broker-run-elsewhere = Spustit jinde
broker-snippet-hint = Vloží do schránky text, který tento projekt spustí pomocí senzor-cli
broker-snippet-disabled = Nejdřív projekt uložte: snippet obsahuje název jeho souboru
broker-copied = Zkopírováno.
broker-run-elsewhere-text = Zbytek úlohy popisuje návod pro CI a Docker v dokumentaci.
