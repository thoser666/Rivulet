# GitHub-Wiki-Konzept und Content-Policy

Das GitHub-Wiki dient als leicht zugängliche Wissensbasis für Nutzer und
Community. Es ergänzt die versionierte Dokumentation im Repository, ersetzt sie
aber nicht.

## Zweck des Wikis

Ins Wiki gehören Inhalte, die sich durch praktische Erfahrungen häufig ändern
oder von der Community erweitert werden können:

- Einsteiger-Tutorials und kurze How-tos
- Twitch-, YouTube- und Kick-Setups
- FAQ und Troubleshooting aus realen Supportfällen
- Plattform- und Hardware-Tipps
- Performance-Erfahrungen mit Encodern und Capture-Backends
- Community-Beiträge und getestete Workarounds

## Was im Repository bleiben muss

Die folgenden Inhalte bleiben ausschließlich oder verbindlich im Repository:

- Sicherheits-, Secret- und Signing-Regeln
- Release- und Update-Prozesse
- CI-, Branch- und Quality-Gate-Anforderungen
- Architekturentscheidungen und normative API-Verträge
- reproduzierbare Testanleitungen
- versionsabhängige Installations- und Kompatibilitätsinformationen

Wiki-Seiten dürfen diese Inhalte zusammenfassen, müssen aber auf die kanonische
Datei im Repository verlinken.

## Vorgeschlagene Wiki-Struktur

```text
Home
├── Getting Started
├── Aufnahme-Anleitung
├── Streaming
│   ├── Twitch
│   ├── YouTube
│   ├── Kick
│   └── SRT-RIST
├── Szenen und Quellen
├── Audio und Filter
├── Windows
├── Linux
├── macOS
├── Updates und Rollback
├── Fehlerbehebung und FAQ
├── Performance-Tuning
└── Known Limitations
```

Die ausführliche, versionierte Startreferenz ist [`docs/user-guide.md`](user-guide.md).
Das Wiki sollte für neue Nutzer einfacher lesbare Einstiege anbieten und bei
technischen Details dorthin zurückverweisen.

## Mehrsprachigkeit

Das Wiki ist bilingual: Englisch ist die kanonische Wiki-Sprache, Deutsch wird
für die zentralen Nutzerseiten parallel gepflegt. Sprachpaare verwenden das
Suffix `-de` (zum Beispiel `Home.md` und `Home-de.md`) und enthalten am Anfang
einen Sprachumschalter. Technische Begriffe, Befehle, Pfade, Variablennamen und
Fehlercodes bleiben unverändert. Fehlt eine Übersetzung, verweist die Seite
sichtbar auf Englisch, statt eine veraltete Übersetzung zu liefern. Die
Navigationsregeln stehen zusätzlich auf der Wiki-Seite `Languages`.

## Pflege-Regeln

1. Jede Wiki-Seite nennt den getesteten Rivulet-Kanal oder Versionsstand.
2. Veraltete Workarounds werden mit `Deprecated` markiert und entfernt, sobald
   eine stabile Lösung verfügbar ist.
3. Keine Stream-Keys, Tokens, privaten Pfade oder personenbezogenen Logs in
   Beispielen verwenden.
4. Änderungen an Sicherheits- oder Release-Aussagen müssen zuerst im Repository
   erfolgen.
5. Neue Seiten sollten einen Abschnitt **Voraussetzungen**, **Schritte** und
   **Fehlerbehebung** enthalten.
6. Bei widersprüchlichen Aussagen gilt die versionierte Repository-Doku.
7. Wiki-Inhalte werden vor Beta-Releases auf tote Links und veraltete Screenshots
   geprüft.

## Aktivierung und initiale Einrichtung

Das Wiki wird in den GitHub-Repository-Settings unter **Features → Wikis**
aktiviert. Danach werden die Seiten `Home`, `Getting Started`, `Streaming`,
`Fehlerbehebung und FAQ` sowie die drei Plattformseiten angelegt. Die Wiki-URL
lautet anschließend:

```text
https://github.com/thoser666/Rivulet/wiki
```

Die Aktivierung ist eine Repository-Einstellung und wird nicht durch einen
Commit in diesem Repository vorgenommen. Dieser Leitfaden hält daher Struktur
und Verantwortungsgrenzen fest, während die eigentliche Aktivierung durch einen
Repository-Administrator erfolgt.

## Definition of Done

- Wiki ist in den Repository-Features aktiviert.
- `Home` verlinkt auf `docs/user-guide.md` und die aktuellen Releases.
- Die sieben Kernseiten aus der Struktur existieren.
- Alle Seiten enthalten Versions-/Kanalhinweis und keine Secrets.
- Repository-Doku bleibt für sicherheits- und releasekritische Aussagen
  kanonisch.
