# Stavy, stáří a velikosti, ať se ukazují kdekoli.

status-stopped = Zastaveno
status-connecting = Připojuje se
status-online = Online
status-reconnecting = Znovu se připojuje
status-reconnecting-attempt = pokus { $attempt }: { $error }
status-failed = Selhalo

format-just-now = právě teď
format-seconds-ago = před { $seconds } s
format-minutes-ago = před { $minutes } min
format-hours-ago = před { $hours } h
format-bytes =
    { $count ->
        [one] { $count } bajt
        [few] { $count } bajty
       *[other] { $count } bajtů
    }
