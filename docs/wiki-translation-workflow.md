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

## Push-Verifikation

Nach jedem Push (Schedule-Pfad mit gesetztem `WIKI_SYNC_TOKEN`) verifiziert der
Workflow den Remote-Hash: `sync-wiki-translations.py --publish` committet und
pusht Änderungen, holt danach den Remote (`git fetch`) und vergleicht dessen
`HEAD` mit dem lokalen Commit.

- Stimmen die Hashes überein, ist `ok: remote verified at <sha>` bestätigt.
- Bei Diskrepanz (z.&nbsp;B. paralleler Push oder Force-Push, bevor der
  eigene Commit am Remote ankam) wird ein Fehler mit `exit 1` gemeldet und der
  Job schlägt sichtbar fehl. Die Verifikation läuft unabhängig davon, ob in
diesem Lauf tatsächlich ein Commit entstand — sie deckt also auch ab, dass ein
angekündigter Sync am Ende wirklich auf dem Remote liegt.

Für einen reinen Push ohne Verifikation (z.&nbsp;B. in einem Lese-Kontext)
existiert die Option `--skip-verify`; im geplanten Workflow bleibt die
Verifikation immer aktiv. Auf dem Wegwerf-Arbeitsrepo (ohne gesetzte
Upstream-Tracking-Referenz) wird auf `origin/master` zurückgegriffen statt auf
`@{u}`.

## Lokaler Smoke-Test

Bevor man sich auf den CI-Workflow verlässt, lässt sich der komplette
Prüf-Sync lokal gegen den Wiki-Arbeits-Clone verifizieren:

```bash
scripts/wiki-sync-smoke.sh
```

Der Smoke-Test prüft vier Dinge in einem Lauf:

1. **Remote-Sync** – der Wiki-Clone existiert, `HEAD` stimmt mit dem Upstream
   überein und der Working Tree ist sauber (keine uncommitteten Änderungen).
2. **Sprach-Paare** – führt `check-wiki-translations.py` aus: jede englische
   Seite besitzt eine `*-de`-Partnerseite und den Sprachumschalter.
3. **i18n-Drift** – führt `sync-wiki-translations.py --check` aus: keine
   fehlenden Navigationsmetadaten.
4. **Link-Audit** – führt `audit-wiki-links.py --check-repo-docs` aus:
   Interwiki-Links (Seite + Anker), Repo-Dok-Links (Datei + GitHub-Anker),
   externe URLs (Erreichbarkeit, offline per `WIKI_LINK_AUDIT_EXTRA=--skip-external`)
   sowie rückwärts alle Wiki-Referenzen in `docs/*.md`, `README.md` und
   `CONTRIBUTING.md` (Deep-Links müssen auflösen, Backtick-Referenzen dürfen
   nicht vom kanonischen Seitennamen abweichen).

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
- Nach einem automatischen Push ist der Remote-Hash verifiziert; bei
  Diskrepanz schlägt der Workflow sichtbar fehl.
