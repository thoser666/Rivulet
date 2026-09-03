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
- Der **Stream-Workspace** (Meld-artige Broadcast-Seite) schaltet unterhalb von `STREAM_WORKSPACE_NARROW_WIDTH` (720 px) auf responsive Layouts um: die Action-Bar (Plattform/Preset + Start/Stop) und alle Steuerzeilen (Stream-Config, Chat-Dock, Audio-Sektion) wrappen mit `ui.horizontal_wrapped`, die Chat-/Info-Spalten stapeln statt in `columns(2)` zu klemmen, und das Chat-Sende-Eingabefeld behält eine Mindestbreite (`.max(120.0)`). So bleiben Start/Stop, Connect, Senden und Mixer-Shortcut auch in schmalen Fenstern erreichbar.
- Die **Sende-Budget-Anzeige** über dem Chat-Eingabefeld bleibt ebenfalls schmal-tauglich: sie rendert als explizit wrapendes Label (`egui::Label::new(…).wrap()`), damit lange (deutsche) Hinweise in einer schmalen Dock-Spalte nicht rechts abgeschnitten werden, und stapelt zwischen Reply-Banner und Eingabezeile. Vertikal ist sie durch den umgebenden Seiten-Scroll (ganze View im vertikalen `ScrollArea`, `auto_shrink([false, false])`) auch bei kurzen Fenstern erreichbar.

Die Verträge werden in `tests/ui_smoke.rs` (`responsive_contract_keeps_controls_reachable_on_narrow_windows`, inkl. Reihenfolge Reply-Banner → Sende-Budget → Eingabezeile und `.wrap()`-Marker), `tests/ui_accessibility.rs` (`narrow_layout_is_responsive`), `tests/ui_regression.rs` (alle Viewports inkl. 640×480), dem In-File-Test `stream_workspace_stays_reachable_on_narrow_windows_in_source` und dem ci_pinning-Guard `stream_workspace_controls_stay_reachable_on_narrow_windows` geprüft — der Guard prüft zusätzlich, dass die Budget-Zeile im Dock als wrapendes Label über der Eingabezeile liegt.
