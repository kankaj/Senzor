Senzor in the application menu (Linux)

Package managers (the AUR package senzor-bin, for example) install these files for you. With
the archive from a release, put senzor on your PATH and copy them for your user:

    install -Dm755 senzor ~/.local/bin/senzor
    install -Dm644 linux/cz.kankaj.Senzor.desktop ~/.local/share/applications/cz.kankaj.Senzor.desktop
    install -Dm644 linux/cz.kankaj.Senzor.svg ~/.local/share/icons/hicolor/scalable/apps/cz.kankaj.Senzor.svg
    install -Dm644 linux/cz.kankaj.Senzor.png ~/.local/share/icons/hicolor/256x256/apps/cz.kankaj.Senzor.png

Senzor then shows in the menu under Development, with its icon, and its windows and
notifications are matched to that entry.
