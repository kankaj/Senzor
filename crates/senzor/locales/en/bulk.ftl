# The panel for several instances selected together.

bulk-heading =
    { $count ->
        [one] { $count } instance
       *[other] { $count } instances
    }
bulk-names-more = { $names } and { $more } more
bulk-selection-hint = Ctrl+click an instance in the tree to add or remove it, Shift+click to add a range.

## How many are in each state, joined as "2 online, 1 stopped"

bulk-count-connecting = { $count } connecting
bulk-count-failed = { $count } failed
bulk-count-online = { $count } online
bulk-count-reconnecting = { $count } reconnecting
bulk-count-stopped = { $count } stopped

## Controls

bulk-start = Start
bulk-start-hint = Start those that are stopped or failed
bulk-start-disabled = All of them run already
bulk-restart = Restart
bulk-restart-hint = Stop those that run, then start them with the project's current settings
bulk-restart-disabled = None of them runs

## Pinning

bulk-no-common-field = These instances have no field in common to pin.
bulk-pin = Pin
bulk-pin-to = to
bulk-pin-all = Pin all
bulk-pin-all-hint = Pin the field on every selected instance that runs
bulk-pin-all-disabled = Type a value, with at least one instance running
bulk-unpin-all = Unpin all
bulk-unpin-all-disabled = None of them has this field pinned
