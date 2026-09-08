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
`.github/workflows/wiki-translations.yml` führt die Prüfung wöchentlich,
manuell **und bei jedem PR aus, der `docs/`, README oder CONTRIBUTING
ändert**.

### Weitere Sprachen

Die Liste der geprüften Sprachen ist konfigurierbar:

```bash
python3 scripts/check-wiki-translations.py --locales de,es,fr
```

Englisch ist immer die kanonische Ausgangsbasis; jede zusätzliche Sprache
braucht ein `<Seite>-<sprache>.md`-Paar mit Sprachumschaltern in beide
Richtungen. Neue Locales werden in `LOCALE_LINK_WORDS` des Skripts ergänzt.

### Frische-Prüfung (Staleness)

Wiki-first gepflegte Seiten haben einen Spiegel im Repo (z. B.
`docs/user-guide.md` ↔ Wiki **Aufnahme-Anleitung**). Der Auditor vergleicht
per `--stale` das letzte Änderungsdatum beider Seiten und meldet Repo-Dokumente,
die älter sind als ihr Wiki-Pendant:

```bash
python3 scripts/audit-wiki-links.py .freebuff-rivulet-wiki --skip-external --stale
```

Der Report ist informativ (exit 0); mit `--strict` schlägt er fehl. Die
Spiegel-Map (`mirrors`) steht im Skript und wird ergänzt, wenn neue
Wiki-Spiegel entstehen.

### Benutzerhandbuch-Frische

Die Bedienungsanleitung (`docs/user-guide.md`) wird gegen die GUI-Quelle
geprüft: `scripts/check-user-guide-freshness.py` leitet die Navigation aus
dem `AppView`-Enum ab und verlangt, dass jede Ansicht und jede ausgelieferte
Funktion dokumentiert ist. Die Funktions-Themen werden **automatisch aus den
i18n-Keys der GUI abgeleitet**: jeder `.tr()`/`.tr_fmt()`-Key wird nach seinem
Präfix gruppiert, und ein Präfix mit mindestens fünf eigenen Keys wird zum
Pflichtthema (generische UI-Vokabeln stehen in `GENERIC_PREFIXES`, Label-
Korrekturen in `LABEL_OVERRIDES`, akzeptierte Alternativschreibweisen in
`TOPIC_ALIASES`). Ein neues GUI-Feature mit eigenem Key-Namespace erweitert
den Doku-Check damit **ohne Skript-Änderung** — vergisst der PR die Doku,
schlägt der CI-Lauf fehl. `REQUIRED_TOPICS` bleibt als explizites Minimum für
Features ohne Präfix-Konvention bestehen. Der Check läuft in jedem CI-Lauf.

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
