# Erster Livestream mit Rivulet

Diese Checkliste führt durch einen ersten Stream mit Rivulet auf **Twitch**,
**YouTube** oder **Kick**. Die Menünamen können je nach Rivulet-Version und
Plattform leicht abweichen.

## 1. Vor dem ersten Stream

- [ ] Rivulet aus einer aktuellen, vertrauenswürdigen Version installieren.
- [ ] Grafiktreiber und Audio-Treiber aktualisieren.
- [ ] Rivulet einmal starten und unter **Einstellungen** Sprache, Theme und
      Speicherort prüfen.
- [ ] Rivulet nicht als Administrator starten, außer ein Plattformtreiber
      verlangt es ausdrücklich.
- [ ] Spiel im Fenstermodus ohne Rahmen oder exklusivem Vollbild testen.
- [ ] Benachrichtigungen, private Fenster und sensible Desktop-Inhalte schließen.
- [ ] Einen lokalen Testmitschnitt von 30–60 Sekunden erstellen.

## 2. Plattformdaten eintragen

### Twitch

- [ ] Im Creator Dashboard den Stream-Key bzw. die bevorzugte sichere
      Authentifizierung einrichten.
- [ ] Server automatisch wählen lassen oder einen nahegelegenen Ingest-Server
      auswählen.
- [ ] Stream-Key niemals in Screenshots, Logs, Issues oder Chats veröffentlichen.

### YouTube

- [ ] In YouTube Studio einen Livestream anlegen.
- [ ] Stream-URL und Stream-Key aus den Live-Control-Room-Einstellungen übernehmen.
- [ ] Sichtbarkeit zunächst auf **Nicht gelistet** oder **Privat** setzen, wenn
      zuerst getestet werden soll.
- [ ] Latenzmodus auswählen; für Interaktion ist „Niedrige Latenz“ meist ein
      guter Startpunkt.

### Kick

- [ ] Im Creator Dashboard Stream-URL und Stream-Key abrufen.
- [ ] Kategorie, Titel und Sichtbarkeit prüfen.
- [ ] Den Stream-Key wie ein Passwort behandeln und bei Verdacht sofort rotieren.

## 3. Szene und Quellen vorbereiten

- [ ] Eine Szene für den Startbildschirm anlegen.
- [ ] Eine Gameplay-Szene mit Game-Capture oder Bildschirm-Capture anlegen.
- [ ] Optional Kamera, Mikrofon, Systemaudio, Browser- und Textquelle ergänzen.
- [ ] Prüfen, dass **Source** und **Window** getrennt ausgewählt sind.
- [ ] Game-Capture-Vorschau öffnen und kontrollieren, dass das richtige Fenster
      angezeigt wird.
- [ ] Quellenreihenfolge, Sichtbarkeit, Lautstärke und Ausrichtung prüfen.
- [ ] Übergänge und Hotkeys testen.

## 4. Empfohlene Startwerte

- [ ] Ausgabeauflösung passend zur Hardware wählen, z. B. 1280×720 oder 1920×1080.
- [ ] Mit 30 FPS starten; 60 FPS erst nach einem Stabilitätstest verwenden.
- [ ] Eine moderate Bitrate wählen und die Plattformempfehlung beachten.
- [ ] Bei älterer Hardware Encoder-Preset und Auflösung reduzieren, statt das
      Spiel unnötig zu belasten.
- [ ] Replay-Puffer nur aktivieren, wenn ausreichend RAM und Speicher vorhanden
      sind.
- [ ] Status, Queue-Füllstand, Underflow-/Overflow-Zähler und Latenz im
      Stream-Tab beobachten.

## 5. Audio- und Datenschutztest

- [ ] Mikrofonpegel im Mixer prüfen; keine Übersteuerung.
- [ ] Spielaudio und Mikrofon getrennt abhören.
- [ ] Einen kurzen Testclip aufnehmen und mit Kopfhörern kontrollieren.
- [ ] Echo, Doppelmonitoring und Tastatur-/Lüftergeräusche beseitigen.
- [ ] Stream-Key, lokale Dateipfade, private Fenstertitel und Benachrichtigungen
      nicht im Stream oder in Discord Presence anzeigen.
- [ ] Optionalen Rivulet-Aktivitätsstatus und Game-Namen auf gewünschte
      Datenschutzwirkung prüfen.

## 6. Teststream

- [ ] Zuerst einen privaten/nicht gelisteten Teststream starten.
- [ ] Prüfen, ob das Plattform-Dashboard den Stream erkennt.
- [ ] Mit einem zweiten Gerät oder Browser Bild und Ton kontrollieren.
- [ ] Mindestens fünf Minuten laufen lassen.
- [ ] Stream-Tab auf steigende Queue-Fehler, fehlende Frames und hohe Latenz prüfen.
- [ ] Bei Problemen zuerst Auflösung, FPS und Bitrate reduzieren.
- [ ] Teststream sauber stoppen und die Aufzeichnung bzw. den VOD-Status prüfen.

## 7. Live gehen

- [ ] Titel, Kategorie, Sprache und Tags setzen.
- [ ] Datenschutz- und Werbehinweise der jeweiligen Plattform beachten.
- [ ] Rivulet starten, Szene und Quellen kontrollieren.
- [ ] Aufnahme optional einige Sekunden vor dem Stream starten.
- [ ] Streaming starten und im Plattform-Dashboard den Live-Status bestätigen.
- [ ] Nach dem Start 30 Sekunden lang Audio, Bild und Dropped-Frame-Anzeige prüfen.
- [ ] Während des Streams gelegentlich Stream-Health und Ressourcenverbrauch prüfen.

## 8. Nach dem Stream

- [ ] Streaming stoppen und kontrollieren, dass der Status auf **Bereit** zurückgeht.
- [ ] Aufnahme sicher beenden und die Datei prüfen.
- [ ] VOD und Chat-Aufzeichnung in der Plattformoberfläche kontrollieren.
- [ ] Fehler, Unterbrechungen und relevante Logstellen notieren.
- [ ] Bei Problemen keine Stream-Keys posten; Keys stattdessen rotieren.

## Schnelle Fehlerdiagnose

| Symptom | Erste Maßnahme |
| --- | --- |
| Plattform erkennt keinen Stream | Stream-URL, Key, Plattformprofil und Firewall prüfen |
| Kein Spielbild | Game-Capture-Fenster neu auswählen, Vorschau aktualisieren, testweise Bildschirm-Capture verwenden |
| Kein Mikrofon | Eingabegerät und Mixer-Pegel prüfen; Testaufnahme erstellen |
| Ruckeln oder Encoder-Überlastung | FPS, Auflösung oder Bitrate reduzieren |
| Hohe Queue-Latenz | Netzwerk prüfen, Bitrate senken, anderen Ingest-Server testen |
| Stream startet, bricht aber ab | Logs prüfen, Reconnect-Verhalten beobachten und Plattform-Key rotieren, falls Authentifizierung fehlschlägt |

Weitere Hintergründe: [Streaming-Dokumentation](user-guide.md),
[Aktivitätsstatus](activity-status.md) und
[Update-/Fehlerdiagnose](updater-troubleshooting.md).
