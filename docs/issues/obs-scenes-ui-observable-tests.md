# Tests für den Featurerequest: OBS-ähnliche Szenen-Konfigurations-Ansicht (observable)

## 1. Szene-Baum / Ordner / Reihenfolge

### Tests im Stil einer Szenen-Organisations-Viewport-Unterstufe

- **`scene_tree_lists_root_scenes_first`**
  - Gegeben eine Szene ohne Parent und eine Szene mit Parent.
  - Dann muss die Baumreihenfolge zuerst die Root-Szenen listen und dann deren Kindkette.

- **`scene_tree_lists_children_after_parent`**
  - Gegeben: Szene A, sowie Kind B von A und Kind C von A.
  - Dann soll B und C nach A erscheinen, nicht davor.

- **`scene_tree_shows_folder_node_for_parent`**
  - Gegeben eine Szene mit Parent.
  - Dann muss die Bedienoberfläche den Parent-Knoten als Ordnerknoten darstellen können (expand/collapse-State).

- **`scene_tree_expand_collapse`**
  - Gegeben ein Baum mit mehreren Ebenen.
  - Dann muss ein Ordnerknoten expandierbar und kollabierbar sein, ohne die Baumreihenfolge zu verändern.

- **`scene_tree_filter_by_search`**
  - Gegeben eine Liste mit mehreren Szene-Namen.
  - Dann muss das Suchfeld die Liste nach Name filtern (Teilname matchbar).

- **`scene_tree_preserves_ordered_scenes_for_flat_case`**
  - Gegeben nur root-Szenen (keine Parent-Beziehung).
  - Dann muss die alte flache, stabile Reihenfolge erhalten bleiben.

## 2. Farbcodierung (observable + theme-konsistent)

- **`scene_color_is_observable_in_tree`**
  - Gegeben eine Szene mit gesetztem `color`.
  - Dann muss die Farbe in der Szene-Ansicht sichtbar sein.

- **`scene_color_theme_consistent_across_theme_change`**
  - Gegeben eine Szene mit Farbe und ein Theme-Wechsel.
  - Dann muss sich die Darstellung konsistent verhalten, nicht inhaltlich kaputt werden.

- **`scene_color_meaning_not_only_decorative`**
  - Gegeben farbkodierte Szenen mit bekannter Bedeutung.
  - Dann muss das UI die Farbe als Bedeutungsmerkmal tragen (z. B. Label/Tooltip/Selektionszustand), nicht nur als rein dekoratives Pixel.

## 3. Suche / Filter

- **`search_filters_scenes_by_name_prefix`**
  - Gegeben Szenen "Game", "Game 2", "Studio", "Studio Beam".
  - Dann muss Eingabe von "Gam" nur passende Szenen zeigen.

- **`search_is_case_insensitive_or_normalized`**
  - Gegeben Szene "Setup".
  - Dann muss Suche nach "setup" oder "SETUP" ebenfalls matchen (je nach gewählter Normalisierung).

- **`search_with_no_match_shows_empty_state`**
  - Gegeben eine Liste und eine Suche ohne Treffer.
  - Dann muss eine leere/leeranzeige Zustand gezeigt werden, nicht undefiniertes Verhalten.

## 4. Duplikats-Semantik (shared vs. independent)

- **`duplicate_scene_shared_sources_preserves_binding_reference`**
  - Gegeben eine Szene mit Quellenbindung.
  - Dann muss im "shared"-Modus die Bindung/Referenz-Platzierung erhalten bleiben können.

- **`duplicate_scene_independent_copy_has_own_source_instances`**
  - Gegeben eine Szene mit Quellen.
  - Dann muss im "unabhängig"-Modus eine Kopie mit eigenen Source-Instanzen entstehen können.

- **`duplicate_scene_ui_shows_selected_mode`**
  - Gegeben ein Duplikat-Dialogmodell.
  - Dann muss das UI erkennbar machen, welcher Modus gewählt wurde.

## 5. Vorlagen / Sammlungen als Einstiegspunkt

- **`new_from_template_loads_scene_configuration`**
  - Gegeben eine Vorlagen-Sammlung (JSON).
  - Dann muss der Startvorgang die vorkonfigurierten Szenen/Quellen laden können.

- **`template_starts_editable_scene`**
  - Gegeben eine geladene Vorlage.
  - Dann muss die Ergebnis-Sammlung direkt bearbeitbar sein.

- **`template_preserves_existing_collection_exports`**
  - Gegeben Vorlagen-Export und Import.
  - Dann muss Export/Import-Plausibilität erhalten bleiben (kein Datenverlust durch Round-Trip).

## 6. Vorab-/Studio-Preview im Erstellworkflow

- **`preview_shows_current_scene_before_transition`**
  - Gegeben eine aktive Szene.
  - Dann muss die Vorab-Preview diese Szene vor dem Übergang anzeigen.

- **`preview_switch_and_take_workflow_works`**
  - Gegeben Studio/Preview-Modell.
  - Dann muss Wechsel + Übernahme (take) im Workflow durchführbar sein.

## 7. Theme-Farben

- **`scene_colors_respond_to_theme_palette_changes`**
  - Gegeben ein Theme mit Palette.
  - Dann müssen Farbdarstellungen auf Palette-Änderung reagieren.

EOF
echo "written"