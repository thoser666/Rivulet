# Rivulet-Bedienungsanleitung

## Hilfe-Menü

Die Sidebar enthält den Menüpunkt **Hilfe**. Dort sind die wichtigsten
Markdown-Dokumente gebündelt: diese Bedienungsanleitung, die Stream-Einrichtung,
die Checkliste für den ersten Stream und die Update-Fehlerbehebung. Jeder Eintrag zeigt eine vollständige GitHub-URL auf die entsprechende
Markdown-Seite und ist direkt anklickbar. Zusätzlich öffnet der Link die lokale
Datei im Standardprogramm des Betriebssystems. In einer Paketinstallation wird
der Dokumentationsordner automatisch verwendet; für portable Builds kann der
Pfad über `RIVULET_DOCS_ROOT` gesetzt werden. Externe Netzwerkzugriffe werden
nur durch einen ausdrücklichen Klick geöffnet.

Diese Anleitung beschreibt die wichtigsten Arbeitsabläufe der aktuellen Alpha-Version. Rivulet entwickelt sich weiter; einzelne Funktionen können je nach Betriebssystem und installierten GStreamer-Komponenten unterschiedlich verfügbar sein.

## 1. Installation und erster Start

Lade das passende Paket aus den [GitHub Releases](https://github.com/thoser666/Rivulet/releases) herunter:

- Windows: `.msi`
- Linux: `.AppImage`
- macOS: `.dmg`

Installiere das Paket mit den üblichen Rechten des Betriebssystems und starte Rivulet. Für Windows muss die Bildschirmaufnahme in den Systemeinstellungen erlaubt werden. Unter Linux können für Wayland ein Portal-/PipeWire-Zugriff und für X11 ein laufender X-Server erforderlich sein.

Die Anwendung prüft beim Start auf Updates. Unter **Settings → Updates** kann die Prüfung außerdem manuell gestartet werden.

## 2. Die Navigation

Die linke Sidebar ist in folgende Bereiche aufgeteilt:

- **Record** – Aufnahmequelle, Vorschau und Aufnahmeaktionen
- **Mixer** – Systemaudio, Mikrofon und Pegel
- **Scenes** – Szenen, Quellen, Ebenen und Übergänge
- **Stream** – Streaming-Ziele, Status, Queue-Telemetrie und Netzwerkdiagnose
- **Assistant** – derzeit vorbereitet
- **Settings** – Sprache, Theme, Codec, Presets, Hotkeys und Updates

Bei schmalen Fenstern kann die Sidebar eingeklappt werden. Alle wichtigen Aktionen müssen zusätzlich über sichtbare Beschriftungen und Tastaturfokus erreichbar sein.

## 3. Eine Aufnahme erstellen

1. Öffne **Record**.
2. Wähle unter **Source** einen Monitor, ein Fenster oder eine Region.
3. Nutze die Live-Vorschau, um das Ziel zu kontrollieren.
4. Bei Fensteraufnahme kannst du die Liste mit **Refresh** aktualisieren.
   Nach Auswahl eines Monitors wird die Fensterliste auf dessen Fenster eingegrenzt, und die
   Monitor-Auswahl bleibt beim Wählen eines Fensters erhalten (aufgenommen wird dann das Fenster).
5. Wähle bei Bedarf ein Aufnahme-Preset, einen Codec und die Bildrate.
6. Aktiviere optional **Audio**, **Timer/FPS-Overlay** oder den Replay Buffer.
7. Klicke **Start Recording**.
8. Kontrolliere Timer, FPS, Encoderlast und Dateigröße im Statusbereich.
9. Klicke **Stop Recording**. Die MP4-Datei wird am konfigurierten Ausgabeort gespeichert.

### Aufnahmequellen

- **Monitor:** vollständiger Bildschirm eines ausgewählten Monitors
- **Window:** einzelnes sichtbares Fenster; die Auswahl sollte nach einem Refresh erneut geprüft werden. Nach Wahl eines Monitors zeigt die Liste nur Fenster dieses Monitors, und die Monitor-Auswahl bleibt beim Wählen eines Fensters bestehen (die Aufnahme verwendet das Fenster)
- **Region:** rechteckiger Ausschnitt mit Drag-Auswahl oder X/Y/Breite/Höhe
- **Game Capture:** Windows Graphics Capture/DXGI/Vulkan/OpenGL-Hook, sofern der jeweilige Pfad verfügbar ist; bei fehlender Unterstützung zeigt Rivulet den Fallback-Status an

### Erweiterte Rate-Control (VBR/CQ/CQ-VBR)

Standardmäßig encodiert Rivulet mit fester Bitrate (**CBR**) — der einzig zuverlässige Modus für Live-Streaming. Für lokale Aufnahmen kann in den Aufnahme-Einstellungen zwischen folgenden Modis gewählt werden:

- **CBR** (constante Bitrate): vorhersagbare Dateigröße; Standard und für Streaming vorgesehen.
- **VBR** (variable Bitrate): bessere Qualität pro Dateigröße, ideal für lokale Aufnahmen (x264 via Zwei-Pass-Stil).
- **CQ** (konstante Qualität): feste Qualität unabhängig von der Größe (x264 `quantizer`, NVENC `constqp`).
- **CQ-VBR** (Qualität + Cap): Qualitätstreiber mit Obergrenze für die Bitrate.

Der Schalter **Qualität** (0–51, niedriger = besser) und ggf. die maximale Bitrate werden nur bei den passenden Modis angezeigt. Das Freitextfeld **Zusätzliche Encoder-Optionen** hängt eigene Properties (z. B. `key-int-max=250 bframes=3`) direkt an das Encoder-Element an. Bei Backends ohne saubere Rate-Control-Properties (QuickSync, AMF, VP9, Software-H.265) fällt Rivulet auf eine durchschnittliche Bitrate zurück, damit kein Zielwert verloren geht.

### Video-Effekte (Farbkorrektur, Weichzeichnen, Schärfen)

Unter **Video effects** in den Aufnahme-Einstellungen stehen einfache Bildfilter bereit, die vor der Codierung angewendet werden:

- **Helligkeit, Kontrast, Sättigung, Farbton** — Farbkorrektur über `videobalance` (jeweils −1…+1, 0 ist neutral).
- **Weichzeichnen** (`gaussianblur`) und **Schärfen** (`cas`).

Optional installierte Elemente (z. B. `cas` für Schärfen) werden automatisch übersprungen, wenn sie auf dem System fehlen — die Aufnahme bricht dadurch nie ab. LUT-Farbgrading (`.cube`) und eine Verfeinerung des Chroma-Keys sind noch nicht über diesen Pfad abgedeckt; der Chroma-Key steht weiterhin pro Quelle zur Verfügung.

Wenn keine Frames eintreffen, beendet Rivulet die Aufnahme nach dem konfigurierten No-Frame-Timeout mit einer sichtbaren Fehlermeldung.

## 4. Audio

Öffne **Mixer**, wähle Systemaudio und/oder Mikrofon und prüfe den Live-Pegel. Die Lautstärke kann je Quelle angepasst werden.

### Multi-Track-Audiotexport

Aktiviere **Getrennte Tracks**, um Systemaudio und Mikrofon nicht zu mischen, sondern je auf einen **eigenen Track** der Aufnahmedatei zu legen (jede Quelle wird durch einen eigenen AAC-Encoder geführt und an denselben Muxer angebunden). Das Export-Routing ist damit unabhängig vom Live-Mix: Über die Schalter **„Exportiere Systemaudio auf eigenem Track“** und **„Exportiere Mikrofon auf eigenem Track“** lässt sich pro Quelle entscheiden, ob sie in der Datei eine eigene Spur erhält. Ein Track wird nur exportiert, solange die Quelle auch erfasst wird. Beim Streamen (RTMP/FLV) werden die Quellen weiterhin zu einem einzigen Audio-Track gemischt, da FLV nur einen unterstützt. Unter **Master-Ausgabe** lässt sich die Gesamtlautstärke des Mixes (System + Mikrofon zusammen) einstellen; das **Ausgangs-VU-Meter** darunter zeigt den Pegel des gesamten Mixes in dB nach Anwendung der Master-Lautstärke. Zusätzlich kann das Monitoring einzelner Quellen aktiviert und dessen Lautstärke getrennt geregelt werden.

### Audio-Filter

Unter **Filters** im Mixer kannst du je Quelle (System/Mikrofon) folgende Filter in der Gruppe hintereinander schalten: **Rauschunterdrückung** (`webrtcdsp`, sofern installiert), **Noise Gate** (schließt unterhalb einer niedrigen Schwelle), **Kompressor**, **Limiter** und **Expander** (alle über `audiodynamic`). Zusätzlich kann je Quelle eine **Verstärkung** in dB (`audioamplify`) und ein **10-Band-EQ** (`equalizer-10bands`, Bänder −12…+12 dB) eingestellt werden. Fehlende GStreamer-Elemente werden übersprungen und in der GUI sowie im Log gemeldet; die Aufnahme soll dadurch nicht stillschweigend abbrechen.

## 5. Szenen und Quellen

Unter **Scenes** kannst du Szenen anlegen, umbenennen, duplizieren und löschen. Quellen werden pro Szene verwaltet und können:

- in der Reihenfolge verschoben werden,
- ein- und ausgeblendet oder gesperrt werden,
- transformiert und zugeschnitten werden,
- mit Chroma-Key-Einstellungen versehen werden,
- als Bild, Text, Webcam, Browser, Media, Farbe, Audio oder Capture-Quelle dienen.

**Ctrl+Z** macht unterstützte Szenenänderungen rückgängig; **Ctrl+Y** stellt sie wieder her. Im Studio Mode bearbeitest du die Preview-Szene und überträgst sie mit **Take** in das Program-Bild.

## 6. Live-Vorschau

Die Vorschau zeigt vor der Aufnahme das ausgewählte Capture-Ziel und während der Aufnahme den encodergebundenen Frame-Stream. Der Status unterscheidet zwischen:

- **Waiting:** noch kein Frame eingetroffen
- **Ready:** Ziel ist ausgewählt und Vorschau verfügbar
- **Active:** Aufnahme läuft und Frames werden verarbeitet
- **Fallback:** bevorzugtes Backend war nicht verfügbar; der verwendete Ersatz wird angezeigt

Eine Vorschau ist eine Zielkontrolle, kein Qualitätsnachweis für den finalen Encode. Prüfe für wichtige Aufnahmen zusätzlich FPS und Dateigröße.

## 7. Streaming einrichten

1. Öffne **Stream**.
2. Wähle Twitch, YouTube, Kick oder Custom.
3. Prüfe die angezeigte RTMPS-/RTMP-Ingest-URL.
4. Trage den Stream-Key ein. Rivulet zeigt ihn nur maskiert an und schreibt ihn nicht in Logs.
5. Wähle ein Qualitäts-Preset oder konfiguriere die Bitrate selbst.
6. Aktiviere optional Adaptive Bitrate und Stream Delay.
7. Für mehrere Ziele füge weitere benannte Targets hinzu.
8. Starte den Stream und beobachte den Zielstatus.

Für Twitch, YouTube und Kick sollten nach Möglichkeit TLS-geschützte `rtmps://`-Endpunkte verwendet werden. Der Stream-Key gehört nicht in Commits, Screenshots, Issues oder Chatnachrichten.

### Multistream (Restream)

Rivulet kann gleichzeitig zu mehreren Plattformen streamen. Im Stream-Tab findest du den aufklappbaren Abschnitt **Restream**:

1. **Ziel hinzufügen** erzeugt einen weiteren Eintrag mit Name, Plattform (Twitch, YouTube, Kick oder Custom), Ingest-URL und Stream-Key.
2. Jedes Ziel wird beim Streamstart an die Engine übergeben und hat einen eigenen Status in der Stream-Diagnose (FPS, Queue, Reconnects).
3. Ziele werden in den App-Einstellungen persistiert; bis zu 4 zusätzliche Ziele sind möglich, doppelte Namen werden abgelehnt.
4. Ein fehlerhaftes Ziel stoppt die gesunden Ziele nicht – du siehst pro Ziel, was passiert.

### Chat

Im Stream-Tab ist der Chat-Dock integriert. Er verbindet sich mit Twitch, Kick und YouTube:

- Anonymes Lesen ist ohne Login möglich; mit OAuth-Token (Berechtigung `chat:send`) kannst du auch selbst schreiben.
- Die Eingabezeile unten im Dock sendet in den verbundenen Kanal; über **Antworten** beantwortest du eine bestimmte Nachricht.
- Über der Eingabezeile zeigt Rivulet das verbleibende Sendekontingent (z. B. „20 Nachrichten pro 30 s“) und die Plattform, für die das Limit gilt – Plattform-Limits werden pro Kanal separat durchgesetzt.

### Alerts

Rivulet kann Engagement-Events (Follows, Abos, Geschenk-Abos, Spenden, Raids) **nativ im Chat-Dock** anzeigen — ohne dass ein Overlay-Dienst (Streamlabs/StreamElements-Browser-URL, siehe [`alerts.md`](alerts.md)) geladen werden muss:

- **Settings → Alerts** aktiviert die lokale Erfassung (standardmäßig an). Eingänge werden nur lokal verarbeitet — es wird **nichts übertragen**, und Alert-Ereignisse enthalten nie Tokens oder Secrets (ein EventSub-Geheimnis bleibt in den Einstellungen und dient nur der Signaturprüfung).
- Im Chat-Dock erscheinen erfasste Events als Chat-Einträge mit eigener Farbe, z. B. „Kira hat 20.00 EUR gespendet“ oder „Boosted ist mit 42 Zuschauern geraidet“.
- Über **Alerts-Vorschau** kannst du die Darstellung ohne laufenden Stream prüfen (ein Beispiel pro Event-Typ).
- **Settings → Webhook-Empfänger** (standardmäßig aus) startet einen lokalen Empfänger auf **`127.0.0.1`** für Streamlabs-Spenden-Webhooks (`/webhook/streamlabs`) und Twitch-EventSub-Notifications (`/eventsub/twitch`) inkl. HMAC-SHA-256-Signaturprüfung gegen ein maskiert hinterlegtes Secret. Echte Lieferungen der Dienste kommen über öffentliches HTTPS — setze dafür einen lokalen HTTPS-Terminator oder Tunnel davor, der an diesen Port weiterleitet (Details in [`alerts-ingest.md`](alerts-ingest.md)).

### Auto-Clips (!clip)

Rivulet kann automatisch Replay-Buffer-Speicherungen auslösen, wenn der Chat „explodiert“:

1. Aktiviere **Auto-Clip** im Stream-Tab und stelle Schwellwert (Nachrichten pro Zeitfenster), Zeitfenster und Abklingzeit ein.
2. Der Befehlsname ist konfigurierbar (Standard `!clip`) – Chatter können damit manuell einen Clip anstoßen.
3. Steigt die Nachrichtenrate über den Schwellwert oder tippt jemand `!clip`, speichert Rivulet den Replay-Buffer als Clip. Abklingzeit verhindert Clip-Fluten.

### Discord-Status (Rich Presence)

Rivulet zeigt seinen Status in deinem Discord-Profil an (siehe [`activity-status.md`](activity-status.md) und das Wiki **Discord-Setup**):

- Voraussetzung ist eine eigene Discord-Anwendung mit Rich Presence; die Application Client ID trägst du unter **Settings → Discord** ein.
- Die Karte zeigt Zeile 1 „Rivulet · <Status>“ (Bereit, Aufnahme, Streamt, …), Zeile 2 den Spiel-/Quellennamen und bei aktivierter Aufnahme die verstrichene Zeit.
- Erscheint der Status nicht, bietet der Stream-Tab bei „Nicht verbunden“ einen **Erneut verbinden**-Knopf; Details stehen im Wiki unter Discord-Troubleshooting.

### MIDI-Controller

Unter **Settings → MIDI** kannst du MIDI-Geräte verbinden und Aktionen auf Controller-Elemente legen:

- Mappings verbinden Szenenwechsel, Aufnahme/Stream-Start und -Stopp, Filter-Toggles und Fader mit Note-/CC-Nummern eines Kanals.
- Der **Learn-Mode** nimmt Mappings auf, indem du das Controller-Element bedienst – kein manuelles Eintippen von Nummern nötig.
- Mappings lassen sich als Presets pro Gerät speichern und wieder laden.

### Fernsteuerung per obs-websocket

Rivulet bietet einen OBS-WebSocket-v5-kompatiblen Server (siehe [`obs-websocket.md`](obs-websocket.md)). Damit lassen sich Rivulet-Steuerungen wie Streamdeck oder Touch Portal anschließen, und das Protokoll ist mit bestehenden OBS-Tools kompatibel. Der Server lauscht standardmäßig auf `127.0.0.1` und kann mit einem Passwort geschützt werden.

### Remote-Begleiter (Handy / Browser)

Unter **Settings → Remote-Begleiter (Handy / Browser)** kannst du Rivulet vom Smartphone oder einem Browser im selben Netzwerk steuern: Szenen wechseln, Aufnahme starten/stoppen und – mit ausdrücklicher Freigabe – den Stream starten/stoppen (siehe [`remote-companion.md`](remote-companion.md)).

- Die Seite läuft nur, wenn der **OBS-WebSocket-Server** oben aktiviert ist, und liegt standardmäßig unter `http://127.0.0.1:<Port>`.
- **Zugriff aus dem Netzwerk erlauben (LAN)** bindet Seite und WebSocket-Server an `0.0.0.0` und erfordert ein **Passwort** – ein LAN-Bind ohne Passwort wird verweigert.
- **Remote Stream Start/Stopp erlauben**: Erst mit dieser Checkbox kann das Handy den Stream steuern; Szenen- und Aufnahmesteuerung funktionieren auch ohne. Die Freigabe wird vom Server durchgesetzt, nicht nur von der Seite.
- **Seite öffnen** startet den Browser auf dem Rechner; auf dem Handy `http://<PC-LAN-IP>:<Port>` eingeben.

### Stream-Diagnose

Pro Ziel werden Status, FPS/Rate, Queue-Füllstand, Underflows, Overflows und – sofern verfügbar – Sink-Latenz angezeigt. Ein einzelnes fehlerhaftes Ziel sollte gesunde Ziele nicht stoppen. Bei Reconnects zeigt der Status den Zielzustand; Retry-Intervalle sind begrenzt.

SRT/RIST und WHIP/WebRTC befinden sich in der Integrationsphase. Ein vorhandener Konfigurationsdialog bedeutet nicht automatisch, dass jeder externe Receiver oder jede SFU bereits interoperabel ist.

## 8. Themes und Einstellungen

Unter **Settings** kannst du zwischen **System**, **Dark** und **Light** wählen. Die Auswahl wird beim Beenden gespeichert und beim nächsten Start wiederhergestellt. Wenn sich ein Theme nicht ändert, öffne die Settings erneut und prüfe, ob der Speicherort der Anwendung beschreibbar ist.

Dort findest du außerdem Sprache, Codec, Aufnahme-Preset, Ausgabeordner, Hotkeys, Replay Buffer und Update-Prüfung.

### Sprache

Rivulet ist auf Deutsch und Englisch verfügbar. Unter **Settings → Sprache** stellst du die Oberflächensprache um; **System** übernimmt die Betriebssystem-Sprache beim ersten Start. Die Umschaltung wirkt sofort, ohne Neustart. Alle Oberflächentexte werden gepflegt und per Test darauf geprüft, dass keine Übersetzung fehlt.

## 9. Updates

Bei einem verfügbaren Update lädt Rivulet das passende Plattformpaket herunter und zeigt den Fortschritt an. Unter Windows wird der MSI-Installer anschließend getrennt gestartet und Rivulet beendet sich, damit Dateien ersetzt werden können.

Der Windows-Installer-Code **3010** bedeutet: Installation erfolgreich, Neustart erforderlich. Das ist kein Fehler. Siehe auch [`update-troubleshooting.md`](update-troubleshooting.md).

## 10. Logs und Fehler melden

Rivulet schreibt tägliche strukturierte Logs in den Benutzer-Datenordner:

- Windows: `%LOCALAPPDATA%\\Rivulet\\logs\\`
- Linux: `$XDG_DATA_HOME/Rivulet/logs/` oder der systemübliche Datenordner
- macOS: systemüblicher Datenordner unter `Rivulet/logs/`

Crash-Blöcke beginnen mit `===== RIVULET CRASH =====`. Füge bei einem Fehler möglichst die relevante Logdatei und die Versionsnummer bei, entferne aber persönliche Pfade, Stream-Keys und Tokens.

Wenn die GUI startet, aber nicht reagiert:

1. Prüfe die heutige Logdatei.
2. Starte einmal mit `RUST_LOG=info`.
3. Deaktiviere testweise Vorschau, Hooks und Hardware-Encoding.
4. Prüfe, ob ein Update- oder Installerprozess noch läuft.
5. Erstelle anschließend ein Issue mit reproduzierbaren Schritten und anonymisierten Logs.

## 11. Bekannte Einschränkungen

- Native Browser-Webview-Adapter sind noch plattformabhängig.
- Vollständige Vulkan-/OpenGL-/DXGI-Performance muss auf echter Hardware gemessen werden.
- WHIP benötigt noch den vollständigen ICE/DTLS/SRTP- und SFU-End-to-End-Nachweis.
- Der VOD-Track (Twitch-Workflow) ist als deterministische Konfiguration vorhanden; die eigentliche pro-Track-GStreamer-Routing- und UI-Integration folgt noch.
- NDI-Output ist als Konfigurationsvertrag vorhanden; eine echte LAN-Interoperabilität über den NewTek-NDI-Runtime ist noch nicht verifiziert.
- VST 3.x: Konfiguration, Entdeckung und der Host-Vertrag (inkl. Windows-Skelett) sind vorhanden; das tatsächliche Audio-Routing durch geladene Plugins (Z96-4) folgt noch.
- Cloud-Recordings: Der S3-`PUT`-Upload (AWS SigV4) nach `stop_recording` ist implementiert; Multipart für sehr große Dateien und GUI-Einstellungen folgen noch.
- RIST/SRT-Smoke-Tests prüfen die CI-Interoperabilität, ersetzen aber keinen Test gegen den produktiven Receiver.
- macOS- und Linux-Funktionen können durch Berechtigungen, Wayland-Portale oder fehlende GStreamer-Plugins eingeschränkt sein.

## 12. Weiterführende Dokumentation

- [UI-/Design-Leitfaden](ui-design.md)
- [UI-Smoke- und Accessibility-Tests](ui-smoke-testing.md)
- [Update-Fehlerbehebung](update-troubleshooting.md)
- [Logging und Crash-Diagnose](logging.md)
- [M3 Streaming Quality Gate](m3-streaming-quality-gate.md)
- [Security- und CI-Hinweise](security.md)
- [Cloud-Recordings (S3-kompatibel)](cloud-recordings.md)
- [VST 3.x-Support](vst3.md)
- [obs-websocket-Server](obs-websocket.md)
- [Discord-Activity-Status](activity-status.md)
- [MIDI-Mapping](midi.md)
- [Twitch-/Multi-Platform-Chat](twitch-chat.md)
- [Alert-Overlays](alerts.md)
- [Mehrsprachigkeit (i18n)](i18n.md)
