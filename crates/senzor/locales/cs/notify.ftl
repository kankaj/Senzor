# Oznámení na ploše o selhaných instancích.

notify-title =
    { $count ->
        [one] Senzor: instance selhala
        [few] Senzor: { $count } instance selhaly
       *[other] Senzor: { $count } instancí selhalo
    }
notify-more =
    { $count ->
        [one] a { $count } další
        [few] a { $count } další
       *[other] a { $count } dalších
    }
