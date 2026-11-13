# Co je nového

Highlights z `CHANGELOG.md` v češtině, které okno „Co je nového“ ukáže v češtině. Každá verze má
stejný nadpis jako v `CHANGELOG.md` a pod `### Highlights` stejný počet bodů ve stejném pořadí;
test `czech_has_the_highlights_of_every_version` to kontroluje. Verze, která tu chybí, se ukáže
anglicky.

## [1.0.1]

### Highlights

- `senzor-cli` je k dispozici i jako image kontejneru, `ghcr.io/kankaj/senzor-cli`, pro Linux na x86_64 i ARM.
- Okno i `senzor-cli` jsou na crates.io: `cargo install --locked senzor senzor-cli`.

## [1.0.0]

### Highlights

- Simulace senzorů MQTT ze šablon: jedna šablona dá tolik zařízení a instancí, kolik potřebujete, každou s vlastním spojením.
- Okno, ve kterém upravíte projekt, spustíte a zastavíte zařízení, připnete hodnoty a sledujete každou zprávu, a `senzor-cli`, který stejný projekt spustí v terminálu, v Dockeru nebo v CI.
- Výchozí MQTT 5 s message expiry, content type, user properties a zpožděným Last Will; MQTT 3.1.1 lze zvolit pro každý projekt.
- Vestavěné šablony a osm příkladů projektů, od kterých můžete začít.
- Skripty v Rhai počítají hodnoty polí a díky odběrům přijímají simulovaná zařízení příkazy: změní svůj stav, připnou pole nebo odpoví.
- Simulovaná zařízení se mohou sama objevit v Home Assistantu, přes MQTT Discovery.
- Problémy se ukazují vedle políčka, kterého se týkají, a zařízení s problémy se nespustí.
- AI agenti jako Claude Code mohou Senzor ovládat přes MCP, bez okna i v okně.
- Okno mluví anglicky nebo česky: jazyk zvolíte v nastavení.
