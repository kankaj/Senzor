# Stavový řádek v dolní části okna.

status-bar-connected = připojeno
status-bar-cannot-connect = nelze se připojit
status-bar-connecting = připojuje se…
status-bar-not-connected = nepřipojeno
status-bar-broker-hint = Nastavení brokeru
status-bar-online = { $count } online
status-bar-connecting-count =
    { $count ->
        [one] { $count } se připojuje
        [few] { $count } se připojují
       *[other] { $count } se připojuje
    }
status-bar-reconnecting-count =
    { $count ->
        [one] { $count } se znovu připojuje
        [few] { $count } se znovu připojují
       *[other] { $count } se znovu připojuje
    }
status-bar-failed-count =
    { $count ->
        [one] { $count } selhala
        [few] { $count } selhaly
       *[other] { $count } selhalo
    }
status-bar-nothing-running = nic neběží
status-bar-rate = { $rate } zpr./s
status-bar-rate-hint = Odeslané zprávy za sekundu, průměr za posledních 5 sekund
status-bar-mcp-on = MCP zapnuto :{ $port }
status-bar-mcp-on-hint = AI agenti mohou ovládat toto okno na http://{ $address }/mcp
status-bar-mcp-off = MCP vypnuto
status-bar-mcp-off-hint = AI agenti nemohou ovládat toto okno; MCP zapnete v nastavení
status-bar-release = Vyšel Senzor { $version }
status-bar-release-hint = Co je nového a stažení
