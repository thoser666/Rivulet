# Feature request (strong/observable): OBS-ähnliche Szenen-Konfigurations-Ansicht in Rivulet

## Priorität
Hohe Priorität (flowtreibend für tägliche Szenenpflege).

## Zusammenfassung
Rivulet soll eine Szenen-Konfigurations-Ansicht bekommen, die näher an einem typischen OBS-Workflow für Szenenerstellung/Organisation ist: Ordner, Farben, Suche, klare Duplikats-Semantik, Vorlagen/Sammlungen als Startpunkt, wiederverwendbare Quellenvorlagen/Bausteine, Vorab-Preview im Erstellworkflow und Theme-fähige Farbcodierung.

## Ziel
Nicht 1:1 OBS-Oberfläche kopieren. Stattdessen die Arbeits-Prinzipien übernehmen, die bei größeren Szenen-Sets Entropie reduzieren und wiederkommendes Szenen-Setup beschleunigen.

## Beobachtete Webverbesserungsideen (einfließen lassen)

Aus der aktuellen OBS-Praxis und -Dokumentation:

1. Scene Collections als Projekt-Vorlagen nutzen; Profiles getrennt für Ausgabe-Einstellungen.
   - Quelle: OBS Scene Collections KB (scene collections store scenes + sources; profiles store output settings).

2. Wiederverwendbare Bestandteile statt immer neu bauen: Kamera-Rahmen, Titel, Alert-Laybacks, Hintergrund; teilen oder gezielt abweichen.
   - Quelle: OBS Sources Guide + Diskussionen zu Duplikat vs. Referenz vs. eigenständiger Source.

3. Ordnungshilfen für größere Listen: Ordner, Farben, Suche, Sortierung.
   - Quelle: community-plugin-Beschreibungen (Ordner/Suche/Farben), Projekt diskutiert Organisationswunsch.

4. Transform-Kürzel für Positionieren/Ausrichten: fit/center/reset/edit transform.
   - Quelle: OBS Sources Guide.

5. Studio Mode als Vorab-Sicherheit beim Verändern/Übernehmen.
   - Quelle: OBS Studio Overview.

6. Workspace-Organisation: weniger genutzte Paneles kontextangepasst zusammenstellen (Docks/Tabs).
   - Quelle: community-plugin-Docs.

7. Automatisierte wiederkehrende Workflows nur sinnvoll, aber vorsichtig.
   - Quelle: community-plugin-Beschreibungen (Makro/Regelwerkzeuge).

## Konkrete UI-Veränderungen (observable)

### 1) Szenen-Organisations-Ansicht mit Ordner/Farben/Suche
- UIDarstellung: kollabierbare Ordnerknoten für Szenen mit `parent`, farbcodierte Szene-/Ordneranzeige, Suchfeld zum Filtern.
- Bezug zum Bestand: `Scene.parent`, `Scene.color`, `SceneManager::ordered_scenes()` (tieferstellung nach Parent, Kinder folgen danach).
- Observable Check:
  - Ordnerknoten sind sichtbar und auf-/zuklappbar.
  - Farbe wird pro Szene/Ordner dargestellt.
  - Suchfeld filtert die Liste auf Name.
  - Reihenfolge bleibt plausibel (Ordner zuerst, dann Kindkette).

### 2) Explizite Duplikats-Semantik im UI
- Im Szene-Menü oder beim Duplizieren: Wahl zwischen "shared Kopie" und "unabhängige Kopie".
- Bezug zum Bestand: `SceneManager::duplicate_scene`, `SourceManager`-Duplikat-/Bindungslogik.
- Observable Check:
  - Je nach gewähltem Modus entsteht unterschiedliche Szene/Kopie-Topologie.
  - UI zeigt erkennbar, ob Quellen geteilt oder neu sind.
  - Der Nutzer kann die Wahl nachvollziehen (Label/Unterschrift/Tooltip).

### 3) Vorlagen/Sammlungen als Einstiegspunkt
- Neuer Einstieg für Szenenerstellung: "Neues aus Vorlage/Sammlung" statt nur "leere Sammlung".
- Bezug zum Bestand: Scene Collections (JSON), `SceneManager::export_json()` / `import_json()`.
- Observable Check:
  - Vorlage öffnet vorkonfigurierte Szene(n)/Quellen-Konfiguration.
  - Sammlung kann als Grundgerüst genutzt werden.
  - Ergebnis ist direkt bearbeitbar.

### 4) Wiederverwendbare Quellenvorlagen / Bausteine
- UI-Option, wiederkehrende Quellensets (z. B. Kamera-Rahmen, Titel-Layback, Hintergrund, Alert) als Baustein einzufügen.
- Bezug: Composition-Ansicht (`AppView::Scenes` + Quellen-Werkzeug), Quelle zuordnen, Geräte-Auswahl.
- Observable Check:
  - Baustein fügt wiederkehrende Quelleneinstellungen ein.
  - Im Composition-Panel sind die Resultate direkt sichtbar.
  - Nutzer kann sofort anpassen.

### 5) Vorab-Preview / Studio Mode-Integration im Erstellworkflow
- Im Arbeitskontext: Vorab-Preview/Studio-Preview-Pfade dann nutzbar, wenn Szenen gearbeitet wird.
- Bezug: bereits vorhandenes Studio Mode-Konzept; Composition-Panel editiert Szeneninhalte.
- Observable Check:
  - Preview zeigt aktuelle Szene vor dem Übergang.
  - Tastatur-/Klick-Steuerung ermöglicht Wechsel und Übernahme.

### 6) Theme-fähige Farbcodierung
- Farben sollen im Theme-System fassbar sein, nicht nur festkodiert.
- Bezug: `rivulet-gui/src/theme.rs`, `ThemePalette`, `StatusColors`, `Style`.
- Observable Check:
  - Wechsel des Theme-Farbpalettes ändert Szenen-/Ordner-Anzeige konsistent.
  - Farben sind auf Bedeutung (z. B. live-fertig, setup, utility, review) abgebildet, nicht nur dekorativ.

### 7) Schnelle wiederkehrende Aktionen (Hotkeys/Steuerung)
- Hohe-Frequenz-Aktionen sollen im UI/Hotkey-System greifbar sein.
- Bezug: Hotkeys-Register, scene hotkey, globale/lokale Bindungen.
- Observable Check:
  - Frequente Aktionen erreichbar und selbsterklärend.
  - Pfade für Szene-/Quellenaktionen konsistent.

## Beispiel-Szenarien (observable Verification)

- "Szene suchen": Nutzer gibt Teilname ein, Liste filtert.
- "Ordner öffnen": Nutzer öffnet Ordner, sieht Kindszenen, Farbcodierung ist sichtbar.
- "Szene duplizieren": Nutzer wählt Modus, Ergebnis ist in Szene-Liste erkennbar.
- "Vorlage nutzen": Nutzer startet aus Vorlage, Composition-Panel zeigt vorkonfigurierte Quellen.
- "Farben prüfen": Nach Theme-Wechsel sind Farben im Szene-Kontext konsistent.

## Mehrwert (warum das vor andere Items kann)
- Reduziert manuelles Neu-Anfertigen für Szenen-Sets.
- Hilft bei größeren Sammlungen (Ordner/Farben/Suche).
- Nutzt vorhandene Basis (`Scene.parent`, `ordered_scenes`, Scene Collections, Composition-Panel, Studio Mode, Hotkeys, Theme) statt neues Grundgerüst.
- Senkt Fehleranfälligkeit bei wiederkehrenden Szenen-Setups.

## Scope-Grenzen
- Kein 1:1 OBS-Studio-UI-Klon.
- Kein Ersatz für vorhandene Kernfunktionen.
- Optional vs. zwingend: Ordner/Farben/Suche sind besonders wertvoll bei größeren Szenen-Sets.

## Vorschlag für Implementierungsschritte (geordnet)

1. Szene-Baum/Ordner-Sicht bauen (expand/collapse, depth-first Reihenfolge).
2. Suchfeld + Filter zur Szene-Liste hinzufügen.
3. Farbcodierung in der Ansicht aus Theme/Color holen und rendern.
4. Duplikats-UI mit klarer Wahl (shared vs independent) ergänzen.
5. Vorlagen/Sammlungen als Starteinstieg ergänzen.
6. Wiederverwendbare Quellenvorlagen/Bausteine einführen.
7. Studio/Preview-Pfade im Erstellworkflow verknüpfen.

## Akzeptanzkriterien (observable)

- Szenen lassen sich in Ordnern organisieren.
- Farben sind sichtbar und Theme-konsistent.
- Suche filtert die Liste.
- Duplikat-Moduswahl ist klar.
- Vorlagen/Sammlungen sparen wiederholtes Setup.
- Quellen-Bausteine sind einführbar und im Composition-Panel einsehbar.
- Vorab-/Studio-Preview ist im Erstellworkflow nutzbar.

## Optionale Richtung
- Falls gewünscht, langfristig: robuste Workspace-/Ansichten-Konfiguration.

## Tests (observabler Abdeckung)

### 1. SceneTree/Ordner-Prioritätstests
- `fn scene_tree_lists_root_scenes_first`
- `fn scene_tree_lists_children_after_parent`
- `fn scene_tree_shows_folder_node_for_parent`
- `fn scene_tree_expand_collapse`
- `fn scene_tree_filter_by_search`

### 2. Farbcodierungstests
- `fn scene_color_is_observable_in_tree`
- `fn scene_color_theme_consistent_across_theme_change`

### 3. Suche/Filter-Tests
- `fn search_filters_scenes_by_name_prefix`
- `fn search_is_case_insensitive_or_normalized`
- `fn search_with_no_match_shows_empty_state`

### 4. Duplikats-Semantik-Tests
- `fn duplicate_scene_shared_sources_preserves_binding_reference`
- `fn duplicate_scene_independent_copy_has_own_source_instances`
- `fn duplicate_scene_ui_shows_selected_mode`

### 5. Vorlagen/Sammlungen-Tests
- `fn new_from_template_loads_scene_configuration`
- `fn template_starts_editable_scene`
- `fn template_preserves_existing_collection_exports`

### 6. Vorab-/Studio-Preview-Tests
- `fn preview_shows_current_scene_before_transition`
- `fn preview_switch_and_take_workflow_works`

### 7. Theme-Farben-Tests
- `fn scene_colors_respond_to_theme_palette_changes`
- `fn scene_color_meaning_is_not_only_decorative`

EOF
echo "written"