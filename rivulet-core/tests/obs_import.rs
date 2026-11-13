//! Regression tests for the OBS scene-collection importer
//! ([#367](https://github.com/thoser666/Rivulet/issues/367)).
//!
//! These pin the mapping `docs/scene-management.md` documents: what becomes a
//! scene, what becomes a folder, how a scene item becomes a binding, and - just
//! as important - that everything without a Rivulet counterpart is *reported*
//! instead of disappearing. A silent drop is the failure mode a migration tool
//! must not have: the user would only notice the missing webcam weeks later.
//!
//! Everything here is pure data; the fixtures are trimmed copies of what OBS
//! writes, keeping the field names (and their OBS quirks) intact.

use rivulet_core::obs_import::{import_collection, source_kind};
use rivulet_core::{ObsImport, ObsImportError, ObsImportNote, SceneKind, SourceKind};

fn import(json: &str) -> ObsImport {
    import_collection(json).expect("fixture must import")
}

fn scene_id(import: &ObsImport, name: &str) -> uuid::Uuid {
    import
        .scenes
        .scenes()
        .iter()
        .find(|scene| scene.name == name)
        .unwrap_or_else(|| panic!("scene {name:?} was not created"))
        .id
}

fn source_id(import: &ObsImport, name: &str) -> uuid::Uuid {
    import
        .sources
        .sources()
        .iter()
        .find(|source| source.name == name)
        .unwrap_or_else(|| panic!("source {name:?} was not imported"))
        .id
}

/// A collection with two scenes, one group holding a webcam, and every item
/// property the mapper reads.
const FULL: &str = r#"{
  "name": "Stream",
  "current_scene": "Game",
  "scene_order": [{"name": "Intro"}, {"name": "Game"}, {"name": "Overlays"}],
  "transition": "Fade",
  "transition_duration": 300,
  "groups": [
    {
      "name": "Overlays",
      "items": [{"name": "Webcam"}, {"name": "Chat"}]
    }
  ],
  "sources": [
    {
      "name": "Intro",
      "id": "scene",
      "settings": {"items": [{"name": "Logo"}]}
    },
    {
      "name": "Game",
      "id": "scene",
      "settings": {
        "items": [
          {
            "name": "Gameplay",
            "visible": false,
            "locked": true,
            "rot": 12.5,
            "pos": {"x": 100.0, "y": 50.0, "alignment": 4},
            "bounds_type": 2,
            "bounds": {"x": 200.0, "y": 100.0, "alignment": 0},
            "crop_left": 1,
            "crop_top": 2,
            "crop_right": 3,
            "crop_bottom": 4
          },
          {"name": "Facecam"}
        ]
      }
    },
    {"name": "Overlays", "id": "group", "settings": {}},
    {"name": "Gameplay", "id": "monitor_capture", "settings": {"monitor": 2}},
    {"name": "Facecam", "id": "v4l2_input", "settings": {"device_id": "/dev/video0"}},
    {"name": "Logo", "id": "image_source", "settings": {"file": "logo.png"}},
    {"name": "Webcam", "id": "dshow_input", "settings": {"video_device_id": "cam-1"}},
    {"name": "Chat", "id": "browser_source", "settings": {"url": "https://example.test/chat"}}
  ]
}"#;

#[test]
fn scenes_follow_scene_order_and_the_active_scene_is_switched() {
    let imported = import(FULL);
    let names: Vec<&str> = imported
        .scenes
        .scenes()
        .iter()
        .map(|scene| scene.name.as_str())
        .collect();
    assert_eq!(names, ["Intro", "Game", "Overlays"]);
    assert_eq!(names, ["Intro", "Game", "Overlays"]);
    assert!(
        imported
            .scenes
            .scenes()
            .iter()
            .all(|scene| scene.kind == SceneKind::Primary),
        "OBS scenes and groups are primary scenes: {names:?}"
    );
    assert_eq!(imported.scenes.collection(), "Stream");
    assert_eq!(
        imported
            .scenes
            .active_scene()
            .map(|scene| scene.name.as_str()),
        Some("Game")
    );
    assert_eq!(imported.report.scenes, 2);
    assert_eq!(imported.report.folders, 1);
}

#[test]
fn a_group_becomes_a_folder_its_members_point_at() {
    let imported = import(FULL);
    let folder = scene_id(&imported, "Overlays");
    let webcam = source_id(&imported, "Webcam");
    assert_eq!(
        imported.sources.scene_sources(folder).len(),
        2,
        "a folder holds its members as bindings"
    );
    let webcam_binding = imported
        .sources
        .scene_sources(folder)
        .into_iter()
        .find(|binding| binding.source_id == webcam)
        .expect("the webcam is bound in the folder");
    assert!(
        webcam_binding.transform_override.is_none(),
        "a group member has no per-item transform in OBS"
    );
    assert!(
        imported
            .scenes
            .scenes()
            .iter()
            .all(|scene| scene.parent.is_none()),
        "the fixture has no nested groups, so every container is top level"
    );
}

#[test]
fn a_nested_group_points_at_the_enclosing_group() {
    let imported = import(
        r#"{
          "name": "Nested",
          "scene_order": [{"name": "Layout"}],
          "groups": [
            {"name": "Layout", "items": [{"name": "Lower"}, {"name": "Badge"}]},
            {"name": "Lower", "items": [{"name": "Badge"}]}
          ],
          "sources": [
            {"name": "Layout", "id": "group", "settings": {}},
            {"name": "Lower", "id": "group", "settings": {}},
            {"name": "Badge", "id": "color_source", "settings": {}}
          ]
        }"#,
    );
    let outer = scene_id(&imported, "Layout");
    let inner = scene_id(&imported, "Lower");
    assert_eq!(
        imported.scenes.get(inner).and_then(|scene| scene.parent),
        Some(outer),
        "the inner group hangs below the outer group"
    );
    assert_eq!(
        imported.scenes.get(outer).and_then(|scene| scene.parent),
        None
    );
    assert_eq!(imported.report.folders, 2);
    assert_eq!(imported.report.scenes, 0);
    assert!(
        !imported
            .report
            .notes
            .iter()
            .any(|note| matches!(note, ObsImportNote::GroupCycle { .. })),
        "a nested group is not a cycle: {:?}",
        imported.report.notes
    );
}

#[test]
fn a_group_that_contains_itself_is_cut_off_and_reported() {
    let imported = import(
        r#"{
          "name": "Cycle",
          "scene_order": [{"name": "A"}],
          "groups": [
            {"name": "A", "items": [{"name": "B"}]},
            {"name": "B", "items": [{"name": "A"}]}
          ],
          "sources": [
            {"name": "A", "id": "group", "settings": {}},
            {"name": "B", "id": "group", "settings": {}}
          ]
        }"#,
    );
    assert_eq!(
        imported.scenes.scenes().len(),
        2,
        "the cycle must not duplicate a group"
    );
    assert_eq!(
        imported
            .report
            .notes
            .iter()
            .filter(|note| matches!(note, ObsImportNote::GroupCycle { .. }))
            .count(),
        1,
        "closing the cycle is reported exactly once: {:?}",
        imported.report.notes
    );
}

#[test]
fn scene_items_carry_transform_crop_visibility_and_lock() {
    let imported = import(FULL);
    let scene = scene_id(&imported, "Game");
    let gameplay = source_id(&imported, "Gameplay");
    let binding = imported
        .sources
        .scene_sources(scene)
        .into_iter()
        .find(|binding| binding.source_id == gameplay)
        .expect("Gameplay is bound in Game");

    let transform = binding
        .transform_override
        .clone()
        .expect("the item has a transform");
    assert_eq!(transform.width, 200.0);
    assert_eq!(transform.height, 100.0);
    // OBS alignment 4 is the centre, so the top-left corner sits half a size
    // before the anchor.
    assert_eq!(transform.x, 100.0 - 100.0);
    assert_eq!(transform.y, 50.0 - 50.0);
    assert_eq!(transform.rotation, 12.5);
    assert_eq!(binding.crop, rivulet_core::Crop::new(1, 2, 3, 4));
    assert!(!binding.visible, "OBS `visible: false` must hide the item");
    assert!(binding.locked, "OBS `locked: true` must lock the item");
}

#[test]
fn the_first_item_in_the_obs_list_is_the_front_most_item() {
    let imported = import(FULL);
    let scene = scene_id(&imported, "Game");
    let ordered = imported.sources.scene_sources(scene);
    let names: Vec<&str> = ordered
        .iter()
        .map(|binding| {
            imported
                .sources
                .get_source(binding.source_id)
                .map(|source| source.name.as_str())
                .unwrap_or("<unknown>")
        })
        .collect();
    // `scene_sources` sorts by z-order, and higher z renders on top. OBS's
    // top item is drawn last, so it must come out last here as well.
    assert_eq!(names, ["Facecam", "Gameplay"]);
    assert!(
        ordered[1].z_order > ordered[0].z_order,
        "the front-most OBS item needs the higher Rivulet z-order"
    );
}

#[test]
fn alignment_values_are_converted_to_a_top_left_offset() {
    // 0 top-left, 4 centre, 8 bottom-right, row-major in between.
    let cases = [
        (0, (0.0, 0.0)),
        (2, (1.0, 0.0)),
        (4, (0.5, 0.5)),
        (6, (0.0, 1.0)),
        (8, (1.0, 1.0)),
    ];
    for (alignment, (fx, fy)) in cases {
        let imported = import(&format!(
            r#"{{
              "name": "Alignment",
              "scene_order": [{{"name": "Scene"}}],
              "sources": [
                {{
                  "name": "Scene",
                  "id": "scene",
                  "settings": {{
                    "items": [{{
                      "name": "Item",
                      "pos": {{"x": 500.0, "y": 400.0, "alignment": {alignment}}},
                      "bounds_type": 2,
                      "bounds": {{"x": 100.0, "y": 40.0}}
                    }}]
                  }}
                }},
                {{"name": "Item", "id": "color_source", "settings": {{}}}}
              ]
            }}"#
        ));
        let scene = scene_id(&imported, "Scene");
        let transform = imported.sources.scene_sources(scene)[0]
            .transform_override
            .clone()
            .expect("transform");
        assert_eq!(
            transform.x,
            500.0 - fx * 100.0,
            "alignment {alignment} x offset"
        );
        assert_eq!(
            transform.y,
            400.0 - fy * 40.0,
            "alignment {alignment} y offset"
        );
    }
}

#[test]
fn an_item_without_bounds_reports_the_scale_it_could_not_convert() {
    let imported = import(FULL);
    assert!(
        imported.report.notes.iter().any(|note| matches!(
            note,
            ObsImportNote::ScaleNotConverted { scene, source }
                if scene == "Game" && source == "Facecam"
        )),
        "Facecam has no `bounds`, so the scale is not convertible: {:?}",
        imported.report.notes
    );
    let scene = scene_id(&imported, "Game");
    let facecam = source_id(&imported, "Facecam");
    let binding = imported
        .sources
        .scene_sources(scene)
        .into_iter()
        .find(|binding| binding.source_id == facecam)
        .expect("Facecam is bound");
    let transform = binding.transform_override.clone().expect("transform");
    assert_eq!(
        (transform.width, transform.height),
        (1920.0, 1080.0),
        "without bounds the item keeps the default frame size"
    );
}

#[test]
fn unsupported_obs_sources_are_skipped_and_reported() {
    let imported = import(
        r#"{
          "name": "Plugins",
          "scene_order": [{"name": "Main"}],
          "sources": [
            {"name": "Main", "id": "scene", "settings": {"items": [
              {"name": "Denoiser"},
              {"name": "Colour"}
            ]}},
            {"name": "Denoiser", "id": "obs_nvidia_denoiser", "settings": {}},
            {"name": "Colour", "id": "color_source", "settings": {}}
          ]
        }"#,
    );
    assert_eq!(imported.report.skipped, 1);
    assert_eq!(imported.report.sources, 1);
    assert!(
        imported
            .sources
            .sources()
            .iter()
            .all(|source| source.name != "Denoiser"),
        "there is no placeholder kind for an OBS plugin source"
    );
    assert!(
        imported.report.notes.iter().any(|note| matches!(
            note,
            ObsImportNote::UnsupportedSource { name, obs_id }
                if name == "Denoiser" && obs_id == "obs_nvidia_denoiser"
        )),
        "{:?}",
        imported.report.notes
    );
    assert!(
        !imported
            .report
            .notes
            .iter()
            .any(|note| matches!(note, ObsImportNote::DanglingItem { .. })),
        "the skipped item's binding must not also be reported as dangling: {:?}",
        imported.report.notes
    );
}

#[test]
fn payloads_and_devices_are_mapped_or_reported() {
    let imported = import(FULL);
    let gameplay = source_id(&imported, "Gameplay");
    assert_eq!(
        imported.sources.get_source(gameplay).unwrap().device_id,
        "2",
        "OBS monitor indices stay usable as Rivulet device ids"
    );
    let facecam = source_id(&imported, "Facecam");
    assert_eq!(
        imported.sources.get_source(facecam).unwrap().device_id,
        "/dev/video0"
    );
    assert!(
        imported.report.notes.iter().any(|note| matches!(
            note,
            ObsImportNote::PayloadNotCarried { source, field }
                if source == "Logo" && field == "file"
        )),
        "an image path has no field on Source: {:?}",
        imported.report.notes
    );
    assert!(
        imported.report.notes.iter().any(|note| matches!(
            note,
            ObsImportNote::PayloadNotCarried { source, field }
                if source == "Chat" && field == "url"
        )),
        "a browser URL has no field on Source: {:?}",
        imported.report.notes
    );
}

#[test]
fn the_collections_global_transition_is_reported_not_applied() {
    let imported = import(FULL);
    assert!(
        imported.report.notes.iter().any(|note| matches!(
            note,
            ObsImportNote::TransitionNotCarried { name, duration_ms }
                if name == "Fade" && *duration_ms == Some(300)
        )),
        "Rivulet overrides transitions per folder: {:?}",
        imported.report.notes
    );
    assert!(imported
        .scenes
        .folder_transition(scene_id(&imported, "Overlays"))
        .is_none());
}

#[test]
fn an_unknown_active_scene_and_a_dangling_item_are_reported() {
    let imported = import(
        r#"{
          "name": "Damaged",
          "current_scene": "Missing",
          "scene_order": [{"name": "First"}],
          "sources": [
            {"name": "First", "id": "scene", "settings": {"items": [
              {"name": "Ghost"},
              {"name": "Real"}
            ]}},
            {"name": "Real", "id": "color_source", "settings": {}}
          ]
        }"#,
    );
    assert_eq!(
        imported
            .scenes
            .active_scene()
            .map(|scene| scene.name.as_str()),
        Some("First"),
        "the first scene stays active when `current_scene` does not resolve"
    );
    assert!(imported.report.notes.iter().any(|note| matches!(
        note,
        ObsImportNote::ActiveSceneUnknown { name } if name == "Missing"
    )));
    assert!(
        imported.report.notes.iter().any(|note| matches!(
            note,
            ObsImportNote::DanglingItem { scene, source }
                if scene == "First" && source == "Ghost"
        )),
        "{:?}",
        imported.report.notes
    );
    assert_eq!(imported.report.bindings, 1);
}

#[test]
fn a_collection_without_scene_order_derives_the_order_and_says_so() {
    let imported = import(
        r#"{
          "name": "Ordered",
          "sources": [
            {"name": "One", "id": "scene", "settings": {}},
            {"name": "Two", "id": "scene", "settings": {}}
          ]
        }"#,
    );
    let names: Vec<&str> = imported
        .scenes
        .scenes()
        .iter()
        .map(|scene| scene.name.as_str())
        .collect();
    assert_eq!(names, ["One", "Two"]);
    assert!(imported
        .report
        .notes
        .contains(&ObsImportNote::SceneOrderDerived));
}

#[test]
fn a_collection_name_falls_back_when_obs_omits_it() {
    let imported = import(r#"{"sources":[{"name":"Main","id":"scene","settings":{}}]}"#);
    assert_eq!(imported.report.collection, "Default");
    assert_eq!(imported.scenes.collection(), "Default");
}

#[test]
fn an_unreadable_document_is_refused_with_a_typed_error() {
    assert!(matches!(
        import_collection("{ not json"),
        Err(ObsImportError::InvalidJson(_))
    ));
    let error =
        import_collection(r#"{"sources":[{"name":"Cam","id":"v4l2_input","settings":{}}]}"#)
            .expect_err("a collection without a scene or group has nothing to import");
    assert_eq!(error, ObsImportError::NoScenes);
    assert!(
        error.to_string().contains("no scenes"),
        "the error must say what is missing: {error}"
    );
}

#[test]
fn a_missing_file_reports_the_path() {
    let missing = std::env::temp_dir().join("rivulet-obs-does-not-exist.json");
    let error = rivulet_core::obs_import::import_file(&missing).unwrap_err();
    match error {
        ObsImportError::Io(message) => assert!(
            message.contains("does-not-exist"),
            "the error must name the file: {message}"
        ),
        other => panic!("expected an Io error, got {other:?}"),
    }
}

#[test]
fn a_dangling_group_entry_keeps_its_members_at_the_top_level() {
    let imported = import(
        r#"{
          "name": "Dangling group",
          "scene_order": [{"name": "Loose"}],
          "groups": [{"name": "Ghost", "items": [{"name": "Loose"}]}],
          "sources": [{"name": "Loose", "id": "scene", "settings": {}}]
        }"#,
    );
    let loose = scene_id(&imported, "Loose");
    assert_eq!(
        imported.scenes.get(loose).and_then(|scene| scene.parent),
        None,
        "a member of a group that has no group source stays reachable at the top level"
    );
    assert!(
        imported.report.notes.iter().any(|note| matches!(
            note,
            ObsImportNote::DanglingGroup { name } if name == "Ghost"
        )),
        "{:?}",
        imported.report.notes
    );
}

#[test]
fn the_report_serializes_for_the_json_interface() {
    let imported = import(FULL);
    let json = serde_json::to_string(&imported.report).expect("report serializes");
    assert!(
        json.contains("\"kind\":\"transition_not_carried\""),
        "{json}"
    );
    assert!(json.contains("\"collection\":\"Stream\""), "{json}");
    let text = imported.report.describe();
    assert!(text.contains("scenes: 2 primary, 1 folder(s)"), "{text}");
    assert!(text.contains("differences ("), "{text}");
    assert!(imported.report.has_notes());
}

#[test]
fn known_obs_types_map_to_the_matching_rivulet_kind() {
    let cases = [
        ("monitor_capture", SourceKind::ScreenCapture),
        ("window_capture", SourceKind::ScreenCapture),
        ("game_capture", SourceKind::GameCapture),
        ("dshow_input", SourceKind::Webcam),
        ("wasapi_input_capture", SourceKind::Audio),
        ("ffmpeg_source", SourceKind::Media),
        ("image_source", SourceKind::Image),
        ("browser_source", SourceKind::Browser),
        ("text_gdiplus_v2", SourceKind::Text),
        ("color_source_v3", SourceKind::Color),
    ];
    for (obs_id, expected) in cases {
        assert_eq!(
            source_kind(obs_id),
            Some(expected),
            "{obs_id} must map without guessing"
        );
    }
    for unknown in ["obs_nvidia_denoiser", "shader_filter", "", "scene"] {
        assert_eq!(
            source_kind(unknown),
            None,
            "{unknown:?} has no counterpart and must be reported instead"
        );
    }
}
