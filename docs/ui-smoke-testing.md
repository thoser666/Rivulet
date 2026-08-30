# UI-Smoke-Tests

Die UI-Smoke-Tests laufen headless und prüfen plattformneutral die stabilen UI-Verträge für Navigation, Tastaturbedienung, Diagnostik, Accessibility und Screenshot-Inhalte.

## Datenschutz im Screenshot-Report

Der deterministische Report darf keine Stream-Keys, Ingest-URLs oder interne Secret-Feldnamen enthalten. Der Test modelliert deshalb ausschließlich sichtbaren Text und filtert credential-bezogene Implementierungsdetails aus der Quelltext-basierten Headless-Repräsentation. Die GUI selbst verwendet für Stream-Keys ein maskiertes Passwortfeld; Keys werden weder in Screenshots noch in Status-/Presence-Daten aufgenommen.

Ein grüner Smoke-Test beweist nicht, dass ein echter Plattformstream funktioniert. Dafür ist der dokumentierte private RTMPS-Test in `docs/stream-setup.md` vorgesehen.
