# Statuses, ages and sizes, wherever they are shown.

status-stopped = Stopped
status-connecting = Connecting
status-online = Online
status-reconnecting = Reconnecting
status-reconnecting-attempt = attempt { $attempt }: { $error }
status-failed = Failed

format-just-now = just now
format-seconds-ago = { $seconds } s ago
format-minutes-ago = { $minutes } min ago
format-hours-ago = { $hours } h ago
format-bytes =
    { $count ->
        [one] { $count } byte
       *[other] { $count } bytes
    }
