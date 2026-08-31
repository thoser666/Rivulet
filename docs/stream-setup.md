# Stream-Setup in der GUI

Der Stream-Tab unterstützt Plattform-Presets für Twitch, YouTube und Kick sowie
benutzerdefinierte RTMP/RTMPS-Ziele.

## First-time-Assistent und Verbindungstest

Der Stream-Tab enthält einen manuell startbaren Einrichtungsassistenten. Er führt durch Plattformwahl, maskierte Key-Eingabe, Vorbereitung eines kurzen privaten Teststreams und Zusammenfassung. Der Assistent startet niemals automatisch einen öffentlichen Stream; dafür ist ausdrücklich **Stream starten** zu betätigen.

Die Verbindungsauswertung unterscheidet lokale Validierungsfehler (`Rejected`) von Transportfehlern (`Failed`) und erfolgreicher Vorprüfung (`Connected`). Ein echter Plattformtest benötigt weiterhin einen eigenen privaten oder nicht gelisteten Stream sowie einen vom Nutzer eingegebenen Key. Unit- und CI-Tests kontaktieren keine Streaming-Plattform.

## Plattform und Preset

- **Twitch:** `rtmps://live.twitch.tv/app`
- **YouTube:** `rtmps://a.rtmp.youtube.com/live2`
- **Kick:** keine globale Standard-URL; die konto-/regionsabhängige Stream-URL aus dem Creator Dashboard eintragen
- **Custom:** beliebiger eigener `rtmp://`- oder `rtmps://`-Endpoint

Für Teststreams verwendet Rivulet Twitch und YouTube automatisch diese Defaults.
Bei Kick bleibt das URL-Feld absichtlich leer und der Start wird bis zur Eingabe
der Dashboard-URL blockiert. So wird kein veralteter oder falscher Kick-Ingest
als scheinbar gültiger Testwert verwendet.

Die Qualitäts-Presets Low, Standard, High und Custom werden an die
`StreamSettings` des Engines übergeben. Plattform-Presets akzeptieren nur
TLS-geschützte `rtmps://`-URLs. Unverschlüsseltes `rtmp://` ist ausschließlich
für Custom-Ziele vorgesehen.

## Stream-Key-Sicherheit

- Der Key wird im GUI als Passwortfeld eingegeben.
- Der vollständige Key wird nie angezeigt oder in Presence-/Statusmeldungen
  übertragen.
- Über **Key sicher speichern** wird der Key ausdrücklich im nativen
  Betriebssystem-Schlüsselbund gespeichert (Windows Credential Manager, macOS
  Keychain bzw. Linux Secret Service über `keyring`). **Gespeicherten Key
  löschen** entfernt ihn wieder. Der Key wird nicht in `eframe::Storage`
  gespeichert. Ist der Schlüsselbund nicht verfügbar, bleibt der Start möglich,
  aber die GUI meldet den Fehler und speichert den Key nicht.
- Keys gehören nicht in Git, Logs, Screenshots, Issues oder Chatnachrichten.
- Bei Verdacht auf Offenlegung den Key sofort im jeweiligen Creator Dashboard
  rotieren.
- Die Validierung blockiert Start, wenn Endpoint oder Key ungültig sind.

## Keyring und privater Teststream

Der Assistent unterstützt einen ausdrücklich gestarteten, zeitlich begrenzten
privaten Testlauf. Nach dem Countdown läuft die konfigurierte Pipeline bis zur
Maximaldauer und wechselt anschließend automatisch in den Status „beendet“.
Die GUI bietet außerdem das explizite Speichern und Löschen des Keys im
Betriebssystem-Schlüsselbund; diese Aktionen sind nicht Teil der eframe-
Layoutpersistenz.
Der Test veröffentlicht nur dann Daten, wenn der Nutzer zuvor selbst einen
privaten oder nicht gelisteten Stream im Plattform-Dashboard angelegt hat.

## Start und Stop

1. Plattform auswählen.
2. Bei Kick oder Custom den Ingest-Endpunkt eintragen. Für Kick die aktuelle
   URL aus dem Creator Dashboard verwenden; Twitch und YouTube sind vorbefüllt.
3. Stream-Key eingeben.
4. Qualitäts-Preset auswählen.
5. Optional **Verbindung testen** klicken. Dieser Test prüft DNS/TCP-Erreichbarkeit im Hintergrund, veröffentlicht nichts und prüft den Stream-Key nicht.
6. **Stream starten** klicken.
6. Im Stream-Tab den Verbindungsstatus und die Queue-/Fehlerzähler beobachten.
7. Zum Beenden **Stream stoppen** klicken.

Start/Stop ist idempotent auf Engine-Ebene: Ein Stream wird nur mit gültiger
Konfiguration gestartet; beim Stop werden Streaming-Zustand und aktive
Transportüberwachung beendet.

Der Verbindungstest ist ein begrenzter Preflight: Er verwendet die konfigurierte
Host-/Port-Kombination, führt keinen RTMPS-Publish und keinen Authentifizierungs-
Handshake mit Stream-Key aus. Ein positives Ergebnis bedeutet daher nur, dass
der Ingest-Endpunkt erreichbar ist; ein privater Teststream bleibt für die
vollständige Verifikation erforderlich. Der Test läuft asynchron und darf die
GUI nicht blockieren; Timeout und Abbruch müssen als Fehlerzustand sichtbar sein.

## Lokaler RTMPS-Smoke-Test

Der Test `scripts/rtmps-smoke.sh` baut eine kleine Live-GStreamer-Pipeline mit
`videotestsrc`, `x264enc`, `flvmux` und `rtmpsink` auf. Als Ziel dient ein lokaler
TCP-Listener auf `127.0.0.1`; dadurch werden Pipeline-Aufbau, Encoding, FLV-Muxing
und die Verbindung zum Sink realistisch geprüft, ohne einen öffentlichen Stream
zu starten.

```bash
bash scripts/test-rtmps-smoke.sh   # statischer Sicherheits-/Vertragstest
bash scripts/rtmps-smoke.sh        # benötigt lokal GStreamer
```

Alternativ kann ein vorbereitetes Container-Image verwendet werden:

```bash
RTMPS_SMOKE_IMAGE=rivulet-rtmps-smoke:ci bash scripts/rtmps-smoke.sh
```

Der Smoke-Test beweist **nicht**, dass Twitch, YouTube oder Kick den Stream
akzeptieren. Nicht abgedeckt sind TLS-Zertifikatsprüfung des Plattform-Endpoints,
Authentifizierung, Account-/Kanalberechtigungen, tatsächliche CDN-Erreichbarkeit,
Plattformlimits, Zuschauer-VOD-Verarbeitung und End-to-End-Audio. Dafür ist ein
privater bzw. nicht gelisteter Teststream mit einem kurzlebigen oder anschließend
rotierten Key erforderlich.

## Vor dem ersten öffentlichen Stream

Einen nicht gelisteten oder privaten Teststream mindestens fünf Minuten laufen
lassen und Bild, Ton, Bitrate, Latenz sowie Reconnect-Verhalten prüfen. Die
vollständige Plattform-Checkliste steht in
[`docs/first-stream-checklist.md`](first-stream-checklist.md).
