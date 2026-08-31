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
5. Wähle bei Bedarf ein Aufnahme-Preset, einen Codec und die Bildrate.
6. Aktiviere optional **Audio**, **Timer/FPS-Overlay** oder den Replay Buffer.
7. Klicke **Start Recording**.
8. Kontrolliere Timer, FPS, Encoderlast und Dateigröße im Statusbereich.
9. Klicke **Stop Recording**. Die MP4-Datei wird am konfigurierten Ausgabeort gespeichert.

### Aufnahmequellen

- **Monitor:** vollständiger Bildschirm eines ausgewählten Monitors
- **Window:** einzelnes sichtbares Fenster; die Auswahl sollte nach einem Refresh erneut geprüft werden
- **Region:** rechteckiger Ausschnitt mit Drag-Auswahl oder X/Y/Breite/Höhe
- **Game Capture:** Windows Graphics Capture/DXGI/Vulkan/OpenGL-Hook, sofern der jeweilige Pfad verfügbar ist; bei fehlender Unterstützung zeigt Rivulet den Fallback-Status an

### Erweiterte Rate-Control (VBR/CQ/CQ-VBR)

Standardmäßig encodiert Rivulet mit fester Bitrate (**CBR**) — der einzig zuverlässige Modus für Live-Streaming. Für lokale Aufnahmen kann in den Aufnahme-Einstellungen zwischen folgenden Modis gewählt werden:

- **CBR** (constante Bitrate): vorhersagbare Dateigröße; Standard und für Streaming vorgesehen.
- **VBR** (variable Bitrate): bessere Qualität pro Dateigröße, ideal für lokale Aufnahmen (x264 via Zwei-Pass-Stil).
- **CQ** (konstante Qualität): feste Qualität unabhängig von der Größe (x264 `quantizer`, NVENC `constqp`).
- **CQ-VBR** (Qualität + Cap): Qualitätstreiber mit Obergrenze für die Bitrate.

Der Schalter **Qualität** (0–51, niedriger = besser) und ggf. die maximale Bitrate werden nur bei den passenden Modis angezeigt. Das Freitextfeld **Zusätzliche Encoder-Optionen** hängt eigene Properties (z. B. `key-int-max=250 bframes=3`) direkt an das Encoder-Element an. Bei Backends ohne saubere Rate-Control-Properties (QuickSync, AMF, VP9, Software-H.265) fällt Rivulet auf eine durchschnittliche Bitrate zurück, damit kein Zielwert verloren geht.

Wenn keine Frames eintreffen, beendet Rivulet die Aufnahme nach dem konfigurierten No-Frame-Timeout mit einer sichtbaren Fehlermeldung.

## 4. Audio

Öffne **Mixer**, wähle Systemaudio und/oder Mikrofon und prüfe den Live-Pegel. Die Lautstärke kann je Quelle angepasst werden. Unter **Master-Ausgabe** lässt sich die Gesamtlautstärke des Mixes (System + Mikrofon zusammen) einstellen; das **Ausgangs-VU-Meter** darunter zeigt den Pegel des gesamten Mixes in dB nach Anwendung der Master-Lautstärke. Zusätzlich kann das Monitoring einzelner Quellen aktiviert und dessen Lautstärke getrennt geregelt werden. Fehlende GStreamer-Audiofilter werden übersprungen und in der GUI sowie im Log gemeldet; die Aufnahme soll dadurch nicht stillschweigend abbrechen.

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

### Stream-Diagnose

Pro Ziel werden Status, FPS/Rate, Queue-Füllstand, Underflows, Overflows und – sofern verfügbar – Sink-Latenz angezeigt. Ein einzelnes fehlerhaftes Ziel sollte gesunde Ziele nicht stoppen. Bei Reconnects zeigt der Status den Zielzustand; Retry-Intervalle sind begrenzt.

SRT/RIST und WHIP/WebRTC befinden sich in der Integrationsphase. Ein vorhandener Konfigurationsdialog bedeutet nicht automatisch, dass jeder externe Receiver oder jede SFU bereits interoperabel ist.

## 8. Themes und Einstellungen

Unter **Settings** kannst du zwischen **System**, **Dark** und **Light** wählen. Die Auswahl wird beim Beenden gespeichert und beim nächsten Start wiederhergestellt. Wenn sich ein Theme nicht ändert, öffne die Settings erneut und prüfe, ob der Speicherort der Anwendung beschreibbar ist.

Dort findest du außerdem Sprache, Codec, Aufnahme-Preset, Ausgabeordner, Hotkeys, Replay Buffer und Update-Prüfung.

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
- RIST/SRT-Smoke-Tests prüfen die CI-Interoperabilität, ersetzen aber keinen Test gegen den produktiven Receiver.
- macOS- und Linux-Funktionen können durch Berechtigungen, Wayland-Portale oder fehlende GStreamer-Plugins eingeschränkt sein.

## 12. Weiterführende Dokumentation

- [UI-/Design-Leitfaden](ui-design.md)
- [UI-Smoke- und Accessibility-Tests](ui-smoke-testing.md)
- [Update-Fehlerbehebung](update-troubleshooting.md)
- [Logging und Crash-Diagnose](logging.md)
- [M3 Streaming Quality Gate](m3-streaming-quality-gate.md)
- [Security- und CI-Hinweise](security.md)
