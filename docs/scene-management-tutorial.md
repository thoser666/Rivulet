# Szenen verwalten: Ordner, Farben, Komponenten und Suche

Dieses Tutorial führt Schritt für Schritt durch das Szenen-Management in
Rivulet: Szenen anlegen, zu Ordnern gruppieren, mit Farben markieren,
Komponenten von umschaltbaren Szenen trennen und mit der Suche wiederfinden.
Es beschreibt die Bedienung in der aktuellen Alpha-Version; die technischen
Garantien des Modells stehen in [scene-management.md](scene-management.md).

## Was du damit lösen kannst

Eine flache Szenenliste funktioniert bis etwa ein Dutzend Szenen. Danach
scrollt man während der Live-Sendung, statt zu wechseln. Rivulet löst das mit
drei Mitteln, die zusammenwirken:

- **Ordner** gruppieren zusammengehörige Szenen und lassen sich zuklappen.
- **Farb-Labels** markieren Szenen nach Funktion (z. B. „Intro", „Game",
  „Pause").
- **Komponenten** sind wiederverwendbare Bausteine, die nicht in der
  Umschaltliste auftauchen sollen.

Dazu kommen **Suche und Filter**, damit auch eine große Sammlung in Sekunden
bedienbar bleibt.

## 1. Szenen anlegen

1. Öffne den Tab **Scenes**.
2. Klicke oben auf **Szene hinzufügen**.
3. Vergib einen Namen, der im laufenden Betrieb sofort erkennbar ist —
   „Game", „Chat", „Pause", nicht „Szene 4".

Die neue Szene landet am Ende der Liste. Ein Klick auf den Namen schaltet sie
live; die aktive Szene trägt die Markierung **Aktiv**.

> **Tipp:** Quellen (Bildschirm, Fenster, Webcam, Browser, Text, …) fügst du
> innerhalb der Szene hinzu. Eine Szene ist der Container, nicht die Quelle.

## 2. Einen Ordner bauen

Rivulet hat keinen eigenen Knopf „Neuer Ordner". Ein Ordner ist eine ganz
normale Szene, die Kinder bekommt — wie ein Verzeichnis, das auch Dateien
enthalten darf. Das ist Absicht: ein Ordner bleibt umschaltbar, behält seine
eigenen Quellen und funktioniert unverändert mit Export, Import und Undo.

So wird eine Szene zum Ordner:

1. Lege eine Szene an, die den Ordner repräsentiert, z. B. **Show**.
2. Ziehe eine andere Szene mit gedrückter Maustaste **auf den Namen** von
   **Show** und lass los.
3. Die gezogene Szene erscheint eingerückt unter **Show**; links neben dem
   Namen erscheint ein Auf-/Zuklapp-Pfeil.

Verschachteln funktioniert genauso: ziehe einen Ordner auf einen anderen
Ordner. Die Einrückung zeigt die Tiefe an.

> **Wichtig:** Eine Szene kann nicht in sich selbst oder in ihren eigenen
> Unterbaum gezogen werden. Rivulet markiert solche Ziele schon während des
> Ziehens rot und erklärt die Ablehnung in der Statuszeile. Die Sammlung bleibt
> dabei unverändert — es entsteht kein halber Zustand und kein leerer
> Undo-Schritt.

## 3. Ein- und ausklappen

- Klicke auf den Pfeil links neben einem Ordner, um ihn zuzuklappen.
- Der Zustand wird **pro Sammlung** gespeichert und übersteht einen Neustart.
- Zuklappen verändert die Sammlung nicht: es ist kein Undo-Schritt und die
  Sammlung gilt nicht als geändert.

Die Szene, die gerade live ist, und die Preview-Szene im Studio Mode bleiben
auch in einem zugeklappten Ordner sichtbar. Du siehst also immer, was on air
ist, ohne alles aufklappen zu müssen.

## 4. Farb-Labels setzen

1. Öffne das Menü **…** rechts neben der Szene.
2. Wähle unter **Farblabel** einen der benannten Einträge.

Das Label erscheint als farbiges Quadrat vor dem Namen und wird zusätzlich als
Text im Tooltip und im Menü genannt — Farbe ist nie das einzige Signal. Die
Palette ist auf ausreichenden Kontrast gegen beide Theme-Hintergründe geprüft.

Die neutrale Standardfarbe wird als Umriss statt als Füllung gezeichnet, damit
sie nicht mit einem bewusst gesetzten Label verwechselt wird.

## 5. Komponenten von umschaltbaren Szenen trennen

Eine **Komponente** ist ein wiederverwendbarer Baustein — ein Intro-Layout,
eine „Bin gleich zurück"-Komposition, ein reines Chat-Panel. Sie gehört zur
Sammlung, soll aber die Liste der live umschaltbaren Szenen nicht aufblähen.

1. Öffne das Menü **…** der Szene.
2. Wähle **Komponente**. Der Eintrag benennt immer das Ziel: bei einer
   Komponente steht dort **Primär**, um sie zurückzuschalten.

Komponenten bleiben umschaltbar, exportierbar und dürfen weiterhin Ordner
sein. Sie werden lediglich gedimmt und in einer eigenen Gruppe dargestellt,
damit sie beim Durchzappen nicht in die Quere kommen.

## 6. Szenen verschieben und wieder herausziehen

- **In einen Ordner:** Szene auf den Zielordner ziehen.
- **Zwischen Ordnern:** genauso, Ziel ist der andere Ordner.
- **Zurück auf die oberste Ebene:** beim Ziehen erscheint über der Liste die
  Ablagefläche **Auf oberste Ebene verschieben**. Dort ablegen.
- **Alternativ:** Menü **… → Auf oberste Ebene verschieben**.

Beim Ziehen zeigen gültige Ziele einen grünen, ungültige einen roten Rahmen.
Lässt du auf einem ungültigen Ziel los, ändert sich nichts und die Statuszeile
nennt den Grund (z. B. „eine Szene kann nicht ihr eigener Ordner sein").

## 7. Suchen und filtern

Über der Szenenliste sitzt die Suchzeile:

| Bedienelement | Wirkung |
|---|---|
| Suchfeld | Sucht ohne Beachtung der Groß-/Kleinschreibung im Szenennamen. |
| **Irgendwo** / **Beginnt mit** | Legt fest, ob der Suchbegriff an beliebiger Stelle oder nur am Namensanfang treffen muss. Standard ist **Irgendwo**. |
| **Komponenten anzeigen** | Blendet Komponenten samt Unterbaum ein oder aus, unabhängig von der Suche. |
| Farb-Auswahl | Zeigt nur Szenen mit diesem Farb-Label. „Beliebig" hebt den Filter auf. |

Treffer werden automatisch sichtbar gemacht: ihre Elternkette und ihr eigener
Unterbaum öffnen sich **nur für diese Anzeige**. Dein gespeicherter
Zuklapp-Zustand bleibt unangetastet — sobald du die Suche leerst, steht die
Liste wieder genau so da wie vorher.

Findet ein aktiver Filter nichts, sagt die Liste das ausdrücklich, statt leer
zu bleiben.

### Tastatur

| Taste | Wirkung |
|---|---|
| `Ctrl+F` / `Cmd+F` | Fokus ins Suchfeld. |
| `Esc` | Suchbegriff löschen, Fokus bleibt im Feld. |
| `Enter` | Ersten Treffer in Listenreihenfolge live schalten (ausgeblendete Komponenten werden übersprungen). |

## 8. Umbenennen, duplizieren, löschen

Die drei kleinen Schaltflächen rechts neben jeder Szene:

- **Umbenennen** — ändert nur den Anzeigenamen, nicht die Identität. Bindings,
  Hotkeys und Quellen bleiben zugeordnet.
- **Duplizieren** — die Kopie behält Ordner **und** Komponenten-Status. Ein
  Duplikat innerhalb eines Ordners bleibt also im Ordner.
- **Löschen** — die Kinder eines gelöschten Ordners werden eine Ebene
  hochgezogen, nicht mitgelöscht und nicht verwaist. Beim Löschen von
  `Show/Game` landet `Chat` unter `Show`.

## 9. Rückgängig

`Ctrl+Z` macht Verschieben, Farb-Label und Komponenten-Umschaltung rückgängig,
`Ctrl+Y` stellt sie wieder her. Zuklappen und die Suche sind bewusst **nicht**
Teil der Undo-Historie: sie verändern die Sammlung nicht.

## 10. Wenn eine Sammlung von Hand bearbeitet wurde

Sammlungen sind Dateien und werden gelegentlich von Hand oder durch
Zusammenführen bearbeitet. Rivulet geht damit tolerant um:

- Eine Szene, deren Ordner nicht mehr existiert, wird als oberste Szene
  angezeigt — sie verschwindet nicht.
- Eine versehentlich gebaute Schleife (A ist Ordner von B, B von A) führt nicht
  zum Hängen der Liste. Jede Szene erscheint genau einmal und bleibt
  reparierbar.
- Sammlungen aus älteren Versionen ohne Komponenten-Feld laden unverändert;
  fehlende Angaben bedeuten **Primär**.

## 11. Was (noch) nicht geht

- **Mehrfachauswahl und Sammelverschieben** — Szenen werden einzeln bewegt.
- **OBS-Sammlung importieren** — das Mapping ist dokumentiert (siehe
  [scene-management.md](scene-management.md#migrating-an-obs-collection)), der
  Importer ist ein eigenes Vorhaben.
- **Pro-Ordner-Standardtransition** — noch nicht Teil des Modells.

## Weiterlesen

- [scene-management.md](scene-management.md) — Modell, Garantien, Fehlerfälle
  und die Testabdeckung.
- [user-guide.md](user-guide.md) — die übrigen Arbeitsabläufe der Anwendung.
- [hotkeys.md](hotkeys.md) — Tastenkürzel inklusive Szenenwechsel.
