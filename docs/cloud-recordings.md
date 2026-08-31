# Cloud recordings (S3-kompatibel)

Rivulet kann fertige Aufnahmen in einen **S3-kompatiblen Objektspeicher**
hochladen (AWS S3, MinIO, Cloudflare R2, Backblaze B2, …). Der aktuelle Umfang
ist ein validierter **Konfigurations-Contract** (`rivulet-core::CloudRecording`)
ohne Netzwerkzugriff — die eigentliche Upload-Ausführung (S3 `PUT`) ist als
Integration-Follow-up dokumentiert.

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

## Offen (Follow-up)

- Tatsächlicher Upload: S3 `PUT` gegen den Endpoint (z. B. via `ureq`, das
  bereits als Dependency vorhanden ist), Multipart für große Dateien.
- Automatischer Upload nach `stop_recording` mit Fortschritt/Fehleranzeige.
