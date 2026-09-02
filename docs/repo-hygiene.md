# Repository-Hygiene-Policy

Dieses Dokument legt fest, welche Dateitypen und Verzeichnisse **ausschließlich**
über `.gitattributes` und `.gitignore` gesteuert werden dürfen — und welche
Dateien **nicht** committet werden. Es ist die normative Quelle für alles, was
Git-bezogen zur Pflege des Working Trees gehört, und ergänzt die anderen
qualitätsorientierten Policies (z.&nbsp;B.
[`docs/milestone-quality-gates.md`](milestone-quality-gates.md) und
[`docs/security.md`](security.md)).

Kanoniche Regeln leben in zwei Dateien:

- `.gitattributes` — **semantische** Git-Regeln pro Pfad/Pfadmuster
  (Zeilenenden, Binary-Kennzeichnung, Rename-/Diff-Verhalten).
- `.gitignore` — **Ausschlüsse** pro Pfad/Pfadmuster (Workspace-Artefakte,
  Caches, Tool-Clones).

Anpassungen an diesen beiden Dateien sind **mindestens Review-pflichtig** und
gehören in einen eigenen, klar betitelten Commit (z.&nbsp;B. `chore:` oder
`docs:`), nie still in Funktions-Commits.

---

## Grundprinzipien

1. **Reproduzierbarkeit über Plattform-Erwartung.**
   Zeilenenden sind durch `.gitattributes` deterministisch vorgegeben und dürfen
   nicht vom `core.autocrlf`- oder Betriebssystem-Default abhängen.
2. **Binär ist binär.**
   Jede binäre oder byte-sensitive Datei ist in `.gitattributes` als `binary`
   markiert, damit Git sie niemals umschreibt.
3. **Kein Gerümpel im Repo.**
   Werkzeuge, Caches, verschachtelte Repos und Editor-Artefakte gehören in
   `.gitignore` und bleiben ungetrackt.
4. **Keine Ausnahmen durch „nur dieses eine Mal“.**
   Eine Datei, die nicht ins Repo gehört, wird ignoriert — nicht committet,
   nicht mit `git add -f` erzwungen.

---

## Über `.gitattributes` geregelt (semantisch)

| Pfad / Muster | Regel | Begründung |
| --- | --- | --- |
| `*.md` | `text eol=lf` | Alle Markdown-Quellen werden im Repo als LF normalisiert, unabhängig von der Plattform. Verhindert CRLF/LF-Wechseldiffs (siehe unten). |
| Icons, Fonts, Bilder, Zertifikate | `binary` | `packaging/rivulet.icns`, `rivulet-gui/assets/fonts/*.ttf`, `docs/*.png`, `rivulet-gui/assets/rivulet_logo.*` etc. dürfen **niemals** zeilenender-normalisiert oder diff-umschrieben werden. |

Regeln für **neue** binäre Assets: Bei Hinzufügen eines neuen Binär-Pfads ist
eine `binary`-Zeile in `.gitattributes` mit zu committen. Bestehende Binär-Dateien
**vor** dem Hinzufügen neu zu konvertieren ist verboten.

### CRLF/LF-Hintergrund

Der README war historisch mit wörtlichem CRLF im Blob (949 `\r`-Bytes)
gespeichert, während `core.autocrlf=true` LF erwartet. Jede Editierung erzeugte
dadurch einen Full-File-Diff („949 hinzugefügt / 948 entfernt") — selbst bei
einer einzigen geänderten Zeile. Seit der Einführung von `*.md text eol=lf`
(Commit `d3d22f2`) ist der Deltamerker wieder minimal. Neue Markdown-Dateien
werden automatisch über das `*.md`-Muster abgedeckt.

---

## Über `.gitignore` geregelt (Ausgeschlossen)

| Pfad / Muster | Art | Begründung |
| --- | --- | --- |
| `target/` | Rust-Build | Kompilierartefakte, niemals versionieren. |
| `ffmpeg/`, `ffmpeg.exe` … | Tool-Binaries | Lokale Werkzeuge, die Nutzer selbst installieren. |
| `*.log`, `logs/` | Laufzeit-Logs | Tägliche Logs gehören in das Systemdaten-Verzeichnis, nicht ins Repo. |
| `*.mp4`, `*.mkv`, `*.mov`, `*.flv` u.&nbsp;a. | Test-Aufnahmen | Generierte Medien, nie committen. |
| `.env`, `.env.local` | Secrets | Umgebungsvariablen mit potenziellen Stream-Keys/Tokens. |
| `.freebuff/` | Tool-Artefakt | Working-Verzeichnis des Assistenz-Tools. |
| `.freebuff-rivulet-wiki/` | verschachteltes Repo | Working-Clone des GitHub-Wikis für den Wiki-Sync (eigenes Git-Repo unter `Rivulet.wiki.git`). Wird lokal ignoriert; Sync läuft ausschließlich über den Wiki-Workflow. Siehe [`docs/wiki-translation-workflow.md`](wiki-translation-workflow.md). |
| `__pycache__/`, `*.pyc` | Python-Cache | Bytecode-Caches der Skripte unter `scripts/`. |
| `*.rs.bk`, `*.pdb`, `*.swp`, `.DS_Store`, `Thumbs.db`, `~*`, `*.tmp`, `*.temp` | Editor/OS-Artefakte | Backup- und Cache-Dateien. |

Regel für **neue** generierte oder werkzeugbezogene Dateien: Beim ersten
Auftreten den Ausschluss in `.gitignore` aufnehmen und nichts davon committen.

---

## Hausregeln für Commits und Checks

- **Vor jedem Push:** `git status --short` prüfen. Untracked Artefakte müssen
  entweder ignoriert oder, wenn nötig, entfernt sein.
- **Kein `git add -A`** über den gesamten Baum. Gezielt die zum Arbeitspaket
  gehörenden Pfade stagen.
- **CRLF/LF:** Bearbeitung einer `.md`-Datei darf keine Full-File-Diff erzeugen.
  Falls doch, blobs und Working-Tree-Zeilenenden vergleichen und nur den
  echten Inhalt committen (Kontrollcheck `git diff -w`).
- **Neue Binärdateien** immer mit `.gitattributes`-`binary`-Zeile committen.
- **CI**: Die Pinning- und Hygiene-Checks (siehe [`docs/ci-action-pins.md`](ci-action-pins.md))
  laufen als Teil der erforderlichen Checks; ein aufgeräumter Baum ist
  Voraussetzung dafür, dass `Lints (Fmt & Clippy)` und die Repo-Checks grün
  bleiben.

---

## Definition of Done

- `.gitattributes` normalisiert `*.md` auf LF und markiert alle Binär-Assets.
- `.gitignore` schließt alle Werkzeug-/Cache-/Rechenartefakte aus.
- Kein untracked Artefakt bleibt dauerhaft im Working Tree.
- Änderungen an beiden Dateien landen in eigenen Commits mit klarer Message.
- Diese Policy ist aus der README verlinkt und damit auffindbar.