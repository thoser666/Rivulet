# Von der Szene zu Aufnahme und Stream

Dieses Tutorial geht den Weg einmal komplett durch: leere Szene anlegen, Quelle
hinzufügen, Layout setzen — und dann sehen, wie genau dieses Bild in der
Aufnahme, im Live-Arbeitsplatz und im Stream ankommt.

Es ergänzt das Tutorial [Szenen verwalten](scene-management-tutorial.md) (Ordner,
Farben, Komponenten, Suche) und die technische Beschreibung
[scene-management.md](scene-management.md#live-composition) (Modell, Garantien,
Testabdeckung). Dort steht, *wie* die Sammlung funktioniert; hier steht, *was du
davon im gesendeten Bild siehst*.

## Was du am Ende siehst

Eine Szene ist nicht nur eine Liste im Szenen-Tab. Sie ist die Vorlage für das
Bild, das der Encoder bekommt. Dieselbe Szene begegnet dir an vier Stellen:

| Ansicht (Sidebar) | Was du dort siehst |
| --- | --- |
| **Szenen** | Der Editor: Ebenen der aktiven Szene, Position, Größe, Zuschnitt, Deckkraft, Reihenfolge. |
| **Live** | Der Arbeitsplatz: das gesendete Bild, darunter der Schalter für die Komposition, das Szenen-Dock und der Mixer. |
| **Aufnehmen** | Die Aufnahme-Vorschau mit derselben Komposition, daneben Quelle, Encoder und Aufnahmeknöpfe. |
| **Stream** | Ziel und Ingest-Adresse. Das Bild kommt aus derselben Komposition wie die Aufnahme. |

Kurz: du baust die Szene einmal, und Aufnahme und Stream teilen sich denselben
Encoder-Pfad.

## 1. Eine Szene anlegen

1. Öffne den Tab **Szenen**.
2. Klicke auf **Szene hinzufügen**.
3. Gib einen Namen, der im laufenden Betrieb sofort erkennbar ist — „Game",
   „Chat", „Pause".

Die neue Szene wird aktiv und trägt die Markierung **Aktiv**. Eine Szene ist der
Container; die Quellen kommen im nächsten Schritt hinein.

> **Tipp:** Ordner, Farb-Labels und die Trennung von umschaltbaren Szenen sind im
> Tutorial [Szenen verwalten](scene-management-tutorial.md) beschrieben. Für den
> ersten Durchlauf hier brauchst du davon nichts.

## 2. Eine Quelle hinzufügen

Im Szenen-Tab, im Bereich der aktiven Szene:

1. Unter **Quelle hinzufügen** einen Namen eintragen (z. B. „Desktop").
2. Im Auswahlfeld den Typ wählen. Zur Verfügung stehen **Screen Capture**,
   **Game Capture**, **Webcam**, **Browser**, **Image**, **Text**, **Media**,
   **Color** und **Audio**.
3. Bei **Screen Capture**, **Game Capture** und **Webcam** erscheint zusätzlich
   ein Gerätefeld (**Gerät**). Ohne Auswahl nimmt Rivulet das Standardgerät; meldet
   die Liste **Keine Geräte erkannt.**, fehlt der Treiber bzw. die
   Berechtigung — die Quelle wird dann trotzdem angelegt und bleibt eine Kachel.
4. Auf **Hinzufügen** klicken. Die Quelle erscheint in der Liste **Ebenen**; ihre
   Einstellungen stehen darunter unter **Quelleneigenschaften**.

> **Wichtig — zwei verschiedene „Quellen":** Die Ebene **Screen Capture** in der
> Szene ist *kein* Monitor-Auswahldialog. Welcher Bildschirm, welches Fenster oder
> welche Region tatsächlich erfasst wird, wählst du im Tab **Aufnehmen** unter
> **Quelle:**. Die Ebene bekommt genau das Bild, das die Aufnahme gerade liefert —
> inklusive Fenster- oder Regionszuschnitt.

## 3. Layout: Reihenfolge, Position, Größe, Zuschnitt

Eine Szene ist eine Fläche von **1920×1080** — das ist der Koordinatenraum, in dem
Position und Größe gelten, unabhängig von der Auflösung deines Monitors.

- **Nach oben** / **Nach unten** verschieben die ausgewählte Ebene in der
  Reihenfolge.
- **Position** und **Größe** legen fest, wo die Ebene liegt und wie groß sie ist.
- **Beschnitt** schneidet Ränder der Ebene weg.
- **Deckkraft** blendet die Ebene ein und aus, **Sichtbar** schaltet sie ganz ab.
- **Gesperrt** schützt eine Ebene vor versehentlichem Verschieben oder Löschen;
  **Quelle löschen** lässt gesperrte Ebenen stehen.
- **Quelle duplizieren** legt eine zweite Ebene an, die dieselbe Quelle mit
  eigenem Layout zeigt (z. B. Webcam groß und noch einmal klein als PiP).

Jede dieser Änderungen gilt nur für die aktive Szene. Dieselbe Quelle darf in einer
anderen Szene ein völlig anderes Layout haben.

## 4. Der Schalter „Aktive Szene in Aufnahme und Stream senden"

Damit die Szene überhaupt im gesendeten Bild landet, muss die Komposition an sein.
Sie ist **standardmäßig an** und wird mit den Einstellungen gespeichert.

Du findest den Schalter an zwei Stellen:

- **Szenen** — neben **Szenen-Snapshot speichern**.
- **Live** — direkt unter dem Bild.

Was er bewirkt:

- Die aktive Szene wird in jeden Frame komponiert, der in Aufnahme und Stream
  geht. Aufnahme und Stream zeigen damit dieselbe Szene wie der Szenen-Editor.
- Die Komposition rechnet in **1920×1080**. Solange sie an ist, zeichnet Rivulet
  in dieser Auflösung auf — ein 1440p-Monitor landet also in 1080p.
- Eine Szene ohne sichtbare Bildebene (leer oder nur **Audio**) ändert nichts:
  dann wird weiterhin das rohe Aufnahmebild aufgenommen. Ein Konfigurationsfehler
  kann die Sendung damit nicht schwarz schalten.

**Wann ausschalten?** Wenn du keine Komposition willst, sondern die native
Capture-Auflösung: für eine reine Monitor-Aufnahme in 1440p, für ein Spiel in
seiner eigenen Auflösung oder wenn du die Szenen nur als Quellenverwaltung
benutzt. Mit ausgeschaltetem Schalter geht das rohe Capture-Bild in den Encoder,
und das Szenen-Layout ist nur noch eine Vorschau.

## 5. Im Aufnehmen-Tab prüfen

1. Öffne **Aufnehmen**.
2. Wähle unter **Quelle:** den Monitor, das Fenster oder die Region, die erfasst
   werden soll (bei **Game Capture** den passenden Pfad). Vor dem Start zeigt die
   Vorschau das gewählte Capture-Ziel.
3. **Aufnahme starten** (oder **F9**). Während der Aufnahme zeigt die Vorschau den
   encodergebundenen Frame-Strom — also die Komposition, sofern der Schalter aus
   Schritt 4 an ist.
4. Im Statusbereich Timer, FPS, Encoderlast und Dateigröße kontrollieren.
5. **Aufnahme stoppen** (oder erneut **F9**). Die Datei landet am konfigurierten
   Ausgabeort.

Die Vorschau ist eine Zielkontrolle, kein Qualitätsnachweis: sie zeigt, *was*
ankommt, nicht wie gut der finale Encode ist. Prüfe für wichtige Aufnahmen
zusätzlich FPS und Dateigröße.

> **Nicht verwechseln:** **Szenen-Snapshot speichern** exportiert das Layout der
> aktiven Szene als PNG — ein Prüf- und Doku-Bild. Es ist nicht das gesendete
> Bild und ersetzt keine Probeaufnahme.

## 6. Live und Stream

Der Tab **Live** ist der Arbeitsplatz für eine laufende Sendung: gesendetes Bild,
Szenen-Dock, Mixer und die Aufnahme-/Stream-Leiste auf einer Seite.

- **Szenenwechsel:** Klick im Szenen-Dock auf den Namen — oder das für die Szene
  vergebene Tastenkürzel. Das Bild folgt sofort; die Aufnahme oder den Stream
  musst du dafür nicht neu starten.
- **Studio-Modus:** Ist er aktiv, bearbeitest du die **Vorschau**-Szene und
  überträgst sie mit **Take** ins **Programm**-Bild. Die dabei verwendete
  Transition ist der Eintrag des nächsten Ordners mit eigener Vorgabe, sonst der
  Sammlungsstandard (siehe
  [Standardtransition pro Ordner](scene-management-tutorial.md#11-standardtransition-pro-ordner)).
- **Stream:** Im Tab **Stream** wählst du Ziel und Ingest-Adresse und startest die
  Sendung. Was Zuschauer sehen, ist dieselbe Komposition wie in der
  Aufnahme-Vorschau.

## 7. Was heute noch eine Kachel ist

Die Komposition liefert nicht für jeden Ebenentyp echte Pixel. Das ist bekannt und
als [#217](https://github.com/thoser666/Rivulet/issues/217) hinterlegt, nicht
verschwiegen:

| Ebene | Im gesendeten Bild |
| --- | --- |
| **Screen Capture**, **Game Capture**, **Webcam** | Das Bild, das die Aufnahme gerade liefert — inklusive nativem Zuschnitt. |
| **Browser** | Der neueste Frame der Browser-Quelle. |
| **Image**, **Text**, **Media**, **Color**, **Audio** | Eine stabile Layout-Kachel an der Position und Größe der Ebene, bis der jeweilige Renderer existiert (#217). Nie stilles Schwarz. |

Typische Stolpersteine:

- **Bild ist schwarz oder unverändert:** keine sichtbare Bildebene in der Szene,
  oder der Schalter aus Schritt 4 ist aus. Beides führt absichtlich zum rohen
  Aufnahmebild.
- **Aufnahme ist „nur" 1080p:** die Komposition ist an. Für die native Auflösung
  den Schalter ausschalten.
- **Falscher Monitor im Bild:** die Capture-Ebene zeigt die Aufnahmequelle aus dem
  **Aufnehmen**-Tab, nicht eine eigene Auswahl in der Szene.
- **Quelle verschwindet nicht:** sie ist **Gesperrt**. Erst entsperren, dann
  **Quelle löschen**.
- **Die Live-Vorschau bleibt leer:** der Status steht auf **Waiting** — es kommt
  noch kein Frame an, weil die Aufnahme nicht läuft oder das Capture-Ziel kein Bild
  liefert. Ohne Frame gibt es auch nichts zu komponieren.

## Was (noch) nicht geht

- Native Renderer für Image-, Text-, Media- und Color-Ebenen
  ([#217](https://github.com/thoser666/Rivulet/issues/217)).
- Der GUI-Menüpunkt für den Import einer OBS-Szenensammlung
  ([#367](https://github.com/thoser666/Rivulet/issues/367)); die
  Kommandozeile (`rivulet import-obs`) liest sie bereits ein, siehe
  [Szenen verwalten](scene-management-tutorial.md#13-obs-sammlung-importieren).

## Weiterlesen

- [Szenen verwalten](scene-management-tutorial.md) — Ordner, Farben,
  Komponenten, Suche, Standardtransition, Mehrfachauswahl.
- [scene-management.md](scene-management.md#live-composition) — Modell,
  Garantien und Testabdeckung der Live-Komposition.
- [user-guide.md](user-guide.md#5-szenen-und-quellen) — die übrigen
  Arbeitsabläufe der Anwendung.
- [hotkeys.md](hotkeys.md) — Tastenkürzel inklusive Szenenwechsel.
- [first-stream-checklist.md](first-stream-checklist.md) — der erste Stream
  Schritt für Schritt.
