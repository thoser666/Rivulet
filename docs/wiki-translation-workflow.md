# Wiki-Übersetzungsworkflow

Das Wiki wird auf Englisch und Deutsch geführt. Englische Seiten sind die
kanonische Ausgangsbasis; deutsche Seiten verwenden das Suffix `-de`.

## Automatische Prüfung

```bash
python3 scripts/check-wiki-translations.py
python3 scripts/sync-wiki-translations.py --check
```

Die Prüfung schlägt fehl, wenn eine englische Markdown-Seite keine passende
`*-de.md`-Seite oder keinen Sprachumschalter besitzt. Der Workflow
`.github/workflows/wiki-translations.yml` führt die Prüfung wöchentlich und
manuell aus.

## Synchronisierung

Der Workflow klont das separate Wiki-Repository, prüft die Paare und ergänzt bei
Bedarf ausschließlich fehlende Navigationsmetadaten. Er übersetzt keinen Fließtext
automatisch: neue oder geänderte Inhalte erzeugen einen sichtbaren Prüfhinweis,
damit Übersetzungen von Maintainer:innen reviewt werden können.

Für den geplanten automatischen Push benötigt das Repository-Secret
`WIKI_SYNC_TOKEN` ein Fine-grained PAT mit Schreibzugriff ausschließlich auf das
Wiki-Repository. Ist das Secret nicht gesetzt, bleibt der Prüfjob grün bzw.
meldet fehlende Übersetzungen, veröffentlicht aber nichts.

## Lokaler Smoke-Test

Bevor man sich auf den CI-Workflow verlässt, lässt sich der komplette
Prüf-Sync lokal gegen den Wiki-Arbeits-Clone verifizieren:

```bash
scripts/wiki-sync-smoke.sh
```

Der Smoke-Test prüft drei Dinge in einem Lauf:

1. **Remote-Sync** – der Wiki-Clone existiert, `HEAD` stimmt mit dem Upstream
   überein und der Working Tree ist sauber (keine uncommitteten Änderungen).
2. **Sprach-Paare** – führt `check-wiki-translations.py` aus: jede englische
   Seite besitzt eine `*-de`-Partnerseite und den Sprachumschalter.
3. **i18n-Drift** – führt `sync-wiki-translations.py --check` aus: keine
   fehlenden Navigationsmetadaten.

Ausstiegscodes: `0` = alle Checks grün, `1` = ein Check fehlgeschlagen,
`2` = Umgebungs-/Clone-Fehler (fehlender Clone, fehlendes Python, fehlender
Upstream). Der Clone wird standardmäßig unter `.freebuff-rivulet-wiki`
erwartet; ein abweichender Pfad ist über `WIKI_CLONE_DIR` konfigurierbar:

```bash
WIKI_CLONE_DIR=/pfad/zum/clone scripts/wiki-sync-smoke.sh
```

Anders als der geplante CI-Job lädt und pushed der lokale Smoke-Test nichts:
Er prüft nur den bereits vorhandenen Clone. Fehlt er, hilft die Meldung des
Skripts beim Ersteinrichten (`git clone https://github.com/thoser666/Rivulet.wiki.git .freebuff-rivulet-wiki`).

## Definition of Done

- Jede englische Kernseite besitzt eine deutsche Partnerseite.
- Beide Seiten enthalten einen Sprachumschalter.
- Neue Seiten werden im Workflow erkannt.
- Keine automatische Übersetzung wird ungeprüft veröffentlicht.
- Wiki-Änderungen sind separat vom Hauptrepository versioniert.
