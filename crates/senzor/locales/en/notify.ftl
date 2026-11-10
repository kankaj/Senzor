# Desktop notifications about failed instances.

notify-title =
    { $count ->
        [one] Senzor: an instance failed
       *[other] Senzor: { $count } instances failed
    }
notify-more = and { $count } more
