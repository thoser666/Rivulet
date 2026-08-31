# Cloud recordings (S3-kompatibel)

Rivulet kann fertige Aufnahmen in einen **S3-kompatiblen Objektspeicher**
hochladen (AWS S3, MinIO, Cloudflare R2, Backblaze B2, …). Der Umfang umfasst
den validierten **Konfigurations-Contract** (`rivulet-core::CloudRecording`)
und die tatsächliche **Upload-Ausführung**: ein S3-`PUT` mit AWS Signature V4
über `ureq`, das nach `stop_recording` automatisch für die fertige Datei
ausgeführt wird (nur wenn `enabled` gesetzt ist).

## Konfiguration

| Feld | Bedeutung |
|---|---|
| `endpoint` | Basis-Endpunkt mit Scheme, z. B. `https://s3.eu-central-1.amazonaws.com` oder `https://minio.example.internal` |
| `bucket` | Bucket-Name (3–63 Zeichen, nur Kleinbuchstaben, Ziffern, `.`, `-`) |
| `region` | Optional, z. B. `eu-central-1` |
| `prefix` | Optionaler Objekt-Key-Präfix, z. B. `rivulet/recordings` (Slash-Normalisierung) |
| `access_key_id` / `secret_access_key` | Zugangsdaten; der Secret wird in `Debug`/Logs maskiert |
| `enabled` | Upload aus — **standardmäßig aus**, damit nie unbeabsichtigt hochgeladen wird |

## Sicherheit

- `validate()` prüft Endpoint-Scheme, Bucket-Regeln und Vorhandensein der
  Zugangsdaten (nur wenn `enabled`).
- Das manuelle `Debug`-Impl und `masked_credentials()` geben den Secret nie im
  Klartext aus — nur die ersten zwei Zeichen plus `••••`.
- Der Objekt-Key wird deterministisch über `upload_key_for(filename)` gebaut
  (`{prefix}/{dateiname}`).

## Upload (implementiert)

- `CloudRecording::upload_recording(path, http, now)` — liest die Datei und
  führt einen einzelnen, streamenden S3-`PUT` gegen den Endpoint aus
  (Path-Style-URL, AWS SigV4-Signierung mit Region `us-east-1` als Default).
  Gibt die hochgeladene Bytezahl zurück; Fehler (deaktiviert, ungültig, HTTP
  ≠ 2xx) werden als `Err` gemeldet.
- Nach `stop_recording` wird die fertige Datei automatisch hochgeladen, sobald
  eine `CloudRecording`-Konfiguration mit `enabled: true` gesetzt wurde
  (`engine.set_cloud_recording(...)`). Der Upload läuft synchron und ist
  best-effort: Fehler werden geloggt, nie fatal.

## Tests & Verifikation

- Die SigV4-Signierung wird gegen die unabhängig publizierten AWS-Testvektoren
  (Signing-Key + Header-Form) geprüft — deterministisch, ohne Netzwerk.
- `upload_recording` wird über einen Mock-Transport getestet (erfolgreicher
  `PUT` meldet Bytezahl; HTTP-Fehler werden propagiert; deaktivierte Config
  schlägt fehl).
- Engine-Tests belegen Setter-Roundtrip, Default-aus-Status und dass ein
  Stopp ohne aktive Cloud-Konfiguration keinen Netzwerkaufruf auslöst.

## Offen (Follow-up)

- **Multipart-Upload** für sehr große Dateien (derzeit ein einzelner `PUT`;
  für die meisten Aufnahme-Größen ausreichend).
- Fortschritts-/Fehleranzeige in der GUI (Upload läuft derzeit unsichtbar im
  Hintergrund; Erfolg/Fehler erscheint nur im Log).
- GUI-Einstellungen für die Cloud-Destination (Endpoint/Bucket/Region/Key).
- IAM-Rollen statt statischer Schlüssel (derzeit Access-Key/Secret-Paar).
