# UI-Smoke-Tests

Die UI-Smoke-Tests laufen headless und prüfen plattformneutral die stabilen UI-Verträge für Navigation, Tastaturbedienung, Diagnostik, Accessibility und Screenshot-Inhalte.

## Datenschutz im Screenshot-Report

Der deterministische Report darf keine Stream-Keys, Ingest-URLs oder interne Secret-Feldnamen enthalten. Der Test modelliert deshalb ausschließlich sichtbaren Text und filtert credential-bezogene Implementierungsdetails aus der Quelltext-basierten Headless-Repräsentation. Die GUI selbst verwendet für Stream-Keys ein maskiertes Passwortfeld; Keys werden weder in Screenshots noch in Status-/Presence-Daten aufgenommen.

Ein grüner Smoke-Test beweist nicht, dass ein echter Plattformstream funktioniert. Dafür ist der dokumentierte private RTMPS-Test in `docs/stream-setup.md` vorgesehen.

## Responsive Layouts

Beim Verkleinern des Fensters dürfen Bedienelemente nicht unerreichbar werden:

- Der gesamte Inhalt der Hauptansicht (Record, Mixer, Szenen, Stream, …) liegt in einer vertikalen `ScrollArea` (`auto_shrink([false, false])`), sodass Buttons am unteren Rand einer Ansicht erreichbar bleiben.
- Die Navigations-Sidebar (`nav_panel`) scrollt ebenfalls vertikal und schneidet bei sehr niedrigen Fenstern keine Einträge ab.
- `main.rs` setzt eine Mindestfenstergröße (`with_min_inner_size(480×360)`) als Boden, damit das Layout nie unter eine sinnvolle Größe schrumpft.

Die Verträge werden in `tests/ui_smoke.rs` (`responsive_contract_keeps_controls_reachable_on_narrow_windows`), `tests/ui_accessibility.rs` (`narrow_layout_is_responsive`) und `tests/ui_regression.rs` (alle Viewports inkl. 640×480) geprüft.
