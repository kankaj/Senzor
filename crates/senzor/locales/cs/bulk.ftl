# Panel pro několik instancí vybraných najednou.

bulk-heading =
    { $count ->
        [one] { $count } instance
        [few] { $count } instance
       *[other] { $count } instancí
    }
bulk-names-more =
    { $more ->
        [one] { $names } a { $more } další
        [few] { $names } a { $more } další
       *[other] { $names } a { $more } dalších
    }
bulk-selection-hint = Ctrl+klik na instanci ve stromu ji přidá nebo odebere, Shift+klik přidá rozsah.

## Kolik jich je v jakém stavu, spojeno jako „2 online, 1 zastavená“

bulk-count-connecting =
    { $count ->
        [one] { $count } se připojuje
        [few] { $count } se připojují
       *[other] { $count } se připojuje
    }
bulk-count-failed =
    { $count ->
        [one] { $count } selhala
        [few] { $count } selhaly
       *[other] { $count } selhalo
    }
bulk-count-online = { $count } online
bulk-count-reconnecting =
    { $count ->
        [one] { $count } se znovu připojuje
        [few] { $count } se znovu připojují
       *[other] { $count } se znovu připojuje
    }
bulk-count-stopped =
    { $count ->
        [one] { $count } zastavená
        [few] { $count } zastavené
       *[other] { $count } zastavených
    }

## Controls

bulk-start = Spustit
bulk-start-hint = Spustit ty, které jsou zastavené nebo selhaly
bulk-start-disabled = Všechny už běží
bulk-restart = Restartovat
bulk-restart-hint = Zastavit běžící a znovu je spustit s aktuálním nastavením projektu
bulk-restart-disabled = Žádná z nich neběží

## Pinning

bulk-no-common-field = Tyto instance nemají žádné společné pole, které by šlo připnout.
bulk-pin = Připnout
bulk-pin-to = na
bulk-pin-all = Připnout všem
bulk-pin-all-hint = Připnout pole všem vybraným instancím, které běží
bulk-pin-all-disabled = Zadejte hodnotu; aspoň jedna instance musí běžet
bulk-unpin-all = Odepnout všem
bulk-unpin-all-disabled = Žádná z nich nemá toto pole připnuté
