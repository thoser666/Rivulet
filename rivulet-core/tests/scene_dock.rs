//! Dock-level tests for the scene tree ([#350](https://github.com/thoser666/Rivulet/issues/350),
//! package C of [#319](https://github.com/thoser666/Rivulet/issues/319)).
//!
//! The dock's logic lives in `rivulet_core::scene_dock` precisely so it can be
//! checked without opening a window: indentation, collapse filtering, the
//! component grouping, the drop rules and the palette contrast are all pure
//! data. The rendering itself is the GUI's job and is covered by its
//! persistence test.
//!
//! Everything here is pure data — no GStreamer, no hardware, no filesystem.

use std::collections::BTreeSet;

use rivulet_core::scene_dock::{
    apply_drop, can_drop, dock_rows, drop_error_key, SceneColorLabel, SceneDock,
};
use rivulet_core::{Locale, Scene, SceneHierarchyError, SceneManager};

fn manager_with(names: &[&str]) -> (SceneManager, Vec<uuid::Uuid>) {
    let mut manager = SceneManager::new();
    let ids = names
        .iter()
        .map(|name| manager.add(Scene::new((*name).to_string())))
        .collect();
    (manager, ids)
}

fn names_of(manager: &SceneManager, rows: &[rivulet_core::SceneDockRow]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            manager
                .get(row.id)
                .map(|scene| scene.name.clone())
                .unwrap_or_default()
        })
        .collect()
}

fn depths(rows: &[rivulet_core::SceneDockRow]) -> Vec<usize> {
    rows.iter().map(|row| row.depth).collect()
}

// ── Tree rendering ────────────────────────────────────────────────────────

#[test]
fn dock_rows_indent_children_under_their_folder() {
    let (mut manager, ids) = manager_with(&["Show", "Game", "Chat"]);
    manager.set_parent(ids[1], Some(ids[0])).unwrap();
    manager.set_parent(ids[2], Some(ids[1])).unwrap();

    let rows = dock_rows(&manager, &BTreeSet::new(), &[]);
    assert_eq!(names_of(&manager, &rows), ["Show", "Game", "Chat"]);
    assert_eq!(depths(&rows), [0, 1, 2], "each level indents once more");
    assert!(rows[0].has_children && rows[1].has_children);
    assert!(!rows[2].has_children, "a leaf gets no disclosure control");
}

#[test]
fn dock_rows_keep_the_flat_order_for_root_scenes() {
    // The pre-#319 list was flat and stable; a collection without folders must
    // still read exactly like it did, just with indentation 0 everywhere.
    let (manager, _) = manager_with(&["Intro", "Game", "Chat", "BRB"]);
    let rows = dock_rows(&manager, &BTreeSet::new(), &[]);
    assert_eq!(
        names_of(&manager, &rows),
        ["Intro", "Game", "Chat", "BRB"],
        "no folders must not reorder the list"
    );
    assert_eq!(depths(&rows), [0, 0, 0, 0]);
    assert!(rows.iter().all(|row| !row.has_children));
}

// ── Collapse ──────────────────────────────────────────────────────────────

#[test]
fn collapsing_a_folder_hides_only_its_subtree() {
    let (mut manager, ids) = manager_with(&["Show", "Game", "Chat", "BRB"]);
    manager.set_parent(ids[1], Some(ids[0])).unwrap();
    manager.set_parent(ids[2], Some(ids[0])).unwrap();

    let mut collapsed = BTreeSet::new();
    collapsed.insert(ids[0]);
    let rows = dock_rows(&manager, &collapsed, &[]);
    assert_eq!(
        names_of(&manager, &rows),
        ["Show", "BRB"],
        "a collapsed folder keeps its own row and hides its children"
    );
    assert!(
        rows[0].has_children,
        "a collapsed folder still offers expand"
    );
}

#[test]
fn a_collapsed_folder_keeps_the_live_scene_reachable() {
    // Studio mode requirement: the row that is on air must stay visible even
    // when its folder was folded away, otherwise the dock cannot show what is
    // live and what is in preview.
    let (mut manager, ids) = manager_with(&["Show", "Game", "BRB"]);
    manager.set_parent(ids[1], Some(ids[0])).unwrap();

    let mut collapsed = BTreeSet::new();
    collapsed.insert(ids[0]);
    let rows = dock_rows(&manager, &collapsed, &[ids[1]]);
    assert_eq!(
        names_of(&manager, &rows),
        ["Show", "Game", "BRB"],
        "the ancestor chain of an always-visible scene is kept open"
    );
    assert_eq!(depths(&rows), [0, 1, 0]);
}

// ── Persisted collapse state ──────────────────────────────────────────────

#[test]
fn collapse_state_is_kept_per_collection() {
    let mut dock = SceneDock::new();
    let game = uuid::Uuid::new_v4();

    dock.toggle("Gaming", game);
    assert!(dock.is_collapsed("Gaming", game));
    assert!(
        !dock.is_collapsed("Podcast", game),
        "another collection must not inherit the fold"
    );

    dock.toggle("Gaming", game);
    assert!(!dock.is_collapsed("Gaming", game));
    assert!(dock.collapsed_in("Gaming").is_empty());
}

#[test]
fn collapse_state_survives_a_serialization_round_trip() {
    // The dock state is persisted with the app settings, so the exact shape
    // has to survive a save/load cycle — that is what "survives a restart"
    // means for the GUI.
    let mut dock = SceneDock::new();
    let folder = uuid::Uuid::new_v4();
    dock.set_collapsed("Gaming", folder, true);

    let json = serde_json::to_string(&dock).expect("dock state must serialize");
    let restored: SceneDock = serde_json::from_str(&json).expect("dock state must deserialize");
    assert_eq!(restored, dock);
    assert!(restored.is_collapsed("Gaming", folder));
}

#[test]
fn fold_state_of_removed_scenes_can_be_pruned() {
    let mut dock = SceneDock::new();
    let kept = uuid::Uuid::new_v4();
    let gone = uuid::Uuid::new_v4();
    dock.set_collapsed("Gaming", kept, true);
    dock.set_collapsed("Gaming", gone, true);

    dock.retain_existing("Gaming", &BTreeSet::from([kept]));
    assert!(dock.is_collapsed("Gaming", kept));
    assert!(!dock.is_collapsed("Gaming", gone));
}

// ── Broken input ──────────────────────────────────────────────────────────

fn import(json: &str) -> SceneManager {
    SceneManager::import_json(json).expect("a broken collection must still load")
}

#[test]
fn a_dangling_parent_still_renders_the_scene_at_the_top_level() {
    let manager = import(
        r#"{
            "scenes": [
                {
                    "id": "11111111-1111-4111-8111-111111111111",
                    "name": "Orphan",
                    "parent": "22222222-2222-4222-8222-222222222222",
                    "color": 4294967295,
                    "kind": "primary"
                }
            ],
            "active": null,
            "history": [],
            "collection": "Default",
            "profile": "Default"
        }"#,
    );

    let rows = dock_rows(&manager, &BTreeSet::new(), &[]);
    assert_eq!(rows.len(), 1, "an orphan must not disappear from the dock");
    assert_eq!(rows[0].depth, 0, "an orphan renders as a root");
}

#[test]
fn a_cyclic_collection_renders_every_scene_exactly_once() {
    let manager = import(
        r#"{
            "scenes": [
                {
                    "id": "11111111-1111-4111-8111-111111111111",
                    "name": "A",
                    "parent": "22222222-2222-4222-8222-222222222222",
                    "color": 4294967295,
                    "kind": "primary"
                },
                {
                    "id": "22222222-2222-4222-8222-222222222222",
                    "name": "B",
                    "parent": "11111111-1111-4111-8111-111111111111",
                    "color": 4294967295,
                    "kind": "primary"
                }
            ],
            "active": null,
            "history": [],
            "collection": "Default",
            "profile": "Default"
        }"#,
    );

    // A cyclic parent chain must not make the depth or the fold walk loop.
    let mut collapsed = BTreeSet::new();
    collapsed.extend(manager.scenes().iter().map(|scene| scene.id));
    let rows = dock_rows(&manager, &collapsed, &[]);
    assert_eq!(names_of(&manager, &rows), ["A", "B"]);
    let mut unique: Vec<uuid::Uuid> = rows.iter().map(|row| row.id).collect();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 2, "no row may be listed twice");
    // Every row is collapsed, but a row never hides itself.
    assert_eq!(rows.len(), 2);
}

// ── Drag & drop ───────────────────────────────────────────────────────────

#[test]
fn dropping_into_own_subtree_is_refused_without_changing_anything() {
    let (mut manager, ids) = manager_with(&["Show", "Game", "Chat"]);
    manager.set_parent(ids[1], Some(ids[0])).unwrap();
    manager.set_parent(ids[2], Some(ids[1])).unwrap();
    let before = manager.export_json().expect("snapshot");

    let error = apply_drop(&mut manager, ids[0], Some(ids[2])).unwrap_err();
    assert_eq!(
        error,
        SceneHierarchyError::Cycle {
            scene: ids[0],
            parent: ids[2],
        }
    );
    assert_eq!(
        manager.export_json().unwrap(),
        before,
        "a refusal changes nothing"
    );
    assert_eq!(drop_error_key(&error), "scene_hierarchy_cycle");
}

#[test]
fn dropping_onto_a_scene_moves_it_into_that_folder() {
    let (mut manager, ids) = manager_with(&["Show", "Game", "Chat"]);
    manager.set_parent(ids[1], Some(ids[0])).unwrap();
    let before = manager.export_json().expect("snapshot");

    apply_drop(&mut manager, ids[2], Some(ids[1])).expect("a legal drop must move the scene");
    assert_eq!(manager.get(ids[2]).unwrap().parent, Some(ids[1]));
    let rows = dock_rows(&manager, &BTreeSet::new(), &[]);
    assert_eq!(depths(&rows), [0, 1, 2]);

    // Undo restores the pre-drop hierarchy: the move went through the
    // undoable model API, not around it.
    manager.undo();
    assert_eq!(manager.export_json().unwrap(), before);
}

#[test]
fn dropping_on_the_top_level_pulls_a_scene_out_of_its_folder() {
    let (mut manager, ids) = manager_with(&["Show", "Game"]);
    manager.set_parent(ids[1], Some(ids[0])).unwrap();

    apply_drop(&mut manager, ids[1], None).expect("top-level drops are legal");
    assert_eq!(manager.get(ids[1]).unwrap().parent, None);
    let rows = dock_rows(&manager, &BTreeSet::new(), &[]);
    assert_eq!(depths(&rows), [0, 0]);
}

#[test]
fn can_drop_reports_self_parents_unknown_scenes_and_cycles() {
    let (mut manager, ids) = manager_with(&["Show", "Game"]);
    manager.set_parent(ids[1], Some(ids[0])).unwrap();
    let ghost = uuid::Uuid::new_v4();

    assert_eq!(
        can_drop(&manager, ids[0], Some(ids[0])),
        Err(SceneHierarchyError::SelfParent(ids[0]))
    );
    assert_eq!(
        can_drop(&manager, ghost, Some(ids[0])),
        Err(SceneHierarchyError::UnknownScene(ghost))
    );
    assert_eq!(
        can_drop(&manager, ids[0], Some(ghost)),
        Err(SceneHierarchyError::UnknownParent(ghost))
    );
    assert_eq!(
        can_drop(&manager, ids[0], Some(ids[1])),
        Err(SceneHierarchyError::Cycle {
            scene: ids[0],
            parent: ids[1],
        })
    );
    assert!(can_drop(&manager, ids[1], Some(ids[0])).is_ok());
    assert!(can_drop(&manager, ids[1], None).is_ok());
}

#[test]
fn drop_refusals_are_explained_in_english_and_german() {
    // The dock shows the mapped key inline; a key that falls back to itself
    // would surface as a raw identifier in the UI.
    for error in [
        SceneHierarchyError::SelfParent(uuid::Uuid::new_v4()),
        SceneHierarchyError::UnknownParent(uuid::Uuid::new_v4()),
        SceneHierarchyError::Cycle {
            scene: uuid::Uuid::new_v4(),
            parent: uuid::Uuid::new_v4(),
        },
    ] {
        let key = drop_error_key(&error);
        assert_ne!(
            Locale::En.tr(key),
            key,
            "{key} is missing from the EN table"
        );
        assert_ne!(
            Locale::De.tr(key),
            key,
            "{key} is missing from the DE table"
        );
        assert!(!Locale::En.tr(key).is_empty());
        assert!(!Locale::De.tr(key).is_empty());
    }
}

// ── Components ────────────────────────────────────────────────────────────

#[test]
fn a_component_and_its_children_form_one_group() {
    let (mut manager, ids) = manager_with(&["Game", "BRB"]);
    manager.set_kind(ids[1], rivulet_core::SceneKind::Component);
    let panel = manager.add(Scene::new("BRB Chat".to_string()));
    manager.set_parent(panel, Some(ids[1])).unwrap();

    let rows = dock_rows(&manager, &BTreeSet::new(), &[]);
    assert!(
        !rows[0].component,
        "a primary scene stays in the switching list"
    );
    assert!(rows[1].component, "the component itself is grouped");
    assert!(
        rows[2].component,
        "everything below a component belongs to the same building block"
    );
}

#[test]
fn toggling_the_kind_only_moves_the_scene_between_groups() {
    let (mut manager, ids) = manager_with(&["Game"]);
    assert!(!dock_rows(&manager, &BTreeSet::new(), &[])[0].component);

    manager.set_kind(ids[0], rivulet_core::SceneKind::Component);
    let rows = dock_rows(&manager, &BTreeSet::new(), &[]);
    assert_eq!(rows.len(), 1, "the scene stays switchable from the dock");
    assert!(rows[0].component);

    manager.undo();
    assert!(!dock_rows(&manager, &BTreeSet::new(), &[])[0].component);
}

// ── Colour labels ─────────────────────────────────────────────────────────

/// WCAG relative luminance of an ARGB colour.
fn luminance(argb: u32) -> f64 {
    let channel = |shift: u32| {
        let value = ((argb >> shift) & 0xFF) as f64 / 255.0;
        if value <= 0.039_28 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

fn contrast(a: u32, b: u32) -> f64 {
    let (lighter, darker) = {
        let (la, lb) = (luminance(a), luminance(b));
        if la > lb {
            (la, lb)
        } else {
            (lb, la)
        }
    };
    (lighter + 0.05) / (darker + 0.05)
}

#[test]
fn colour_labels_stay_legible_on_both_theme_backgrounds() {
    // The swatch is a graphical indicator, so WCAG AA 1.4.11 (3:1 against the
    // adjacent colour) is the bar. The theme backgrounds are egui's panel
    // fills: #1b1b1b dark, #f8f8f8 light.
    const DARK_BG: u32 = 0xFF1B_1B1B;
    const LIGHT_BG: u32 = 0xFFF8_F8F8;
    for label in SceneColorLabel::ALL {
        if label.is_neutral() {
            // "No colour" is drawn as an outline, not as a fill.
            assert_eq!(label.argb(), 0xFFFF_FFFF);
            continue;
        }
        for (background, name) in [(DARK_BG, "dark"), (LIGHT_BG, "light")] {
            let ratio = contrast(label.argb(), background);
            assert!(
                ratio >= 3.0,
                "{label:?} is {ratio:.2}:1 on the {name} background (WCAG AA needs 3:1)"
            );
        }
    }
}

#[test]
fn the_palette_round_trips_and_names_are_translated() {
    for label in SceneColorLabel::ALL {
        assert_eq!(SceneColorLabel::from_argb(label.argb()), Some(label));
        let key = label.key();
        assert_ne!(
            Locale::En.tr(key),
            key,
            "{key} is missing from the EN table"
        );
        assert_ne!(
            Locale::De.tr(key),
            key,
            "{key} is missing from the DE table"
        );
    }
    assert_eq!(
        SceneColorLabel::from_argb(0xFF12_3456),
        None,
        "a hand-edited colour is not a palette entry"
    );
    assert_eq!(
        SceneColorLabel::from_argb(Scene::new("x".to_string()).color),
        Some(SceneColorLabel::Neutral),
        "a fresh scene is neutral"
    );
}
