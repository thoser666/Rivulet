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

## Definition of Done

- Jede englische Kernseite besitzt eine deutsche Partnerseite.
- Beide Seiten enthalten einen Sprachumschalter.
- Neue Seiten werden im Workflow erkannt.
- Keine automatische Übersetzung wird ungeprüft veröffentlicht.
- Wiki-Änderungen sind separat vom Hauptrepository versioniert.
