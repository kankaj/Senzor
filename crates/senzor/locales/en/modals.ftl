# The dialogs that ask something or tell what went wrong.

modals-ok = OK
modals-save = Save
modals-discard = Discard
modals-cancel = Cancel
modals-unsaved-title = Unsaved changes
modals-unsaved-new = Save the changes to this project before starting a new project?
modals-unsaved-open = Save the changes to this project before opening another project?
modals-unsaved-example = Save the changes to this project before opening the example?
modals-unsaved-close = Save the changes to this project before closing?
modals-open-failed-title = Cannot open { $path }
modals-open-failed-text = Fix the file in a text editor, then open it again.
modals-copy-builtin-title = Copy built-in template { $builtin }
modals-new-template-id = New template ID
modals-copy = Copy
modals-replace-title = Replace { $name }?
modals-replace-text = A file with that name already exists in { $folder }. Replacing it overwrites its contents.
modals-replace = Replace
modals-clear-title =
    { $count ->
        [one] Clear 1 retained topic?
       *[other] Clear { $count } retained topics?
    }
modals-clear-text = Senzor publishes an empty retained message to each topic on { $broker }, which makes the broker forget the message it keeps there. Subscribers then get nothing until the devices publish again.
modals-clear = Clear
modals-whats-new-title = What's new in Senzor { $version }
modals-full-changelog = Full changelog
modals-error-title = Something went wrong
modals-file-changed-title = { $name } changed on disk
modals-file-changed-text = Another program changed the file. Reload it to see its version.
modals-file-changed-unsaved = Another program changed the file. Reload it to see its version; your unsaved changes are replaced, and Undo brings them back.
modals-reload = Reload
modals-keep-mine = Keep my version
