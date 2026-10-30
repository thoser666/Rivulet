//! Regression tests for the scene hierarchy and component model
//! ([#319](https://github.com/thoser666/Rivulet/issues/319)).
//!
//! The folder/group, color-label and component-scene work starts in the model,
//! not in the GUI: these tests pin the invariants the scene dock will rely on,
//! so a scene cannot be dropped into its own subtree, a legacy collection
//! without a `kind` field stays loadable, and a corrupt import cannot make the
//! presentation order loop or silently hide a scene.
//!
//! Everything here is pure data — no GStreamer, no hardware, no filesystem.

use rivulet_core::{SceneHierarchyError, SceneKind, SceneManager};

fn manager_with(names: &[&str]) -> (SceneManager, Vec<uuid::Uuid>) {
    let mut manager = SceneManager::new();
    let ids = names
        .iter()
        .map(|name| manager.add(rivulet_core::Scene::new((*name).to_string())))
        .collect();
    (manager, ids)
}

#[test]
fn a_new_scene_is_a_primary_scene_with_neutral_color() {
    let scene = rivulet_core::Scene::new("Game".to_string());
    assert_eq!(scene.kind, SceneKind::Primary);
    assert!(!scene.is_component());
    assert_eq!(scene.color, 0xFFFFFFFF);
    assert_eq!(scene.parent, None);
}

#[test]
fn component_constructor_marks_the_scene_as_reusable() {
    let scene = rivulet_core::Scene::component("BRB".to_string());
    assert_eq!(scene.kind, SceneKind::Component);
    assert!(scene.is_component());
    assert_eq!(scene.parent, None, "a component can still live in a folder");
}

#[test]
fn set_parent_moves_a_scene_and_preserves_the_depth_first_order() {
    let (mut manager, ids) = manager_with(&["Show", "Game", "Chat"]);
    manager.set_parent(ids[1], Some(ids[0])).unwrap();
    manager.set_parent(ids[2], Some(ids[1])).unwrap();

    assert_eq!(manager.ordered_scenes(), vec![ids[0], ids[1], ids[2]]);
    assert_eq!(manager.get(ids[2]).unwrap().parent, Some(ids[1]));
}

#[test]
fn set_parent_rejects_a_scene_as_its_own_parent() {
    let (mut manager, ids) = manager_with(&["Show"]);
    let error = manager.set_parent(ids[0], Some(ids[0])).unwrap_err();
    assert_eq!(error, SceneHierarchyError::SelfParent(ids[0]));
    assert!(error.to_string().contains("own parent"));
    assert_eq!(manager.get(ids[0]).unwrap().parent, None);
}

#[test]
fn set_parent_rejects_a_parent_inside_the_moved_subtree() {
    // Show > Game > Chat; moving Show under its grandchild would be a cycle.
    let (mut manager, ids) = manager_with(&["Show", "Game", "Chat"]);
    manager.set_parent(ids[1], Some(ids[0])).unwrap();
    manager.set_parent(ids[2], Some(ids[1])).unwrap();

    let error = manager.set_parent(ids[0], Some(ids[2])).unwrap_err();
    assert_eq!(
        error,
        SceneHierarchyError::Cycle {
            scene: ids[0],
            parent: ids[2],
        }
    );
    assert!(
        error.to_string().contains("descendant"),
        "the refusal must say why it was refused: {error}"
    );
    assert_eq!(
        manager.get(ids[0]).unwrap().parent,
        None,
        "a refused move must leave the tree untouched"
    );
}

#[test]
fn set_parent_reports_unknown_scenes_and_parents() {
    let (mut manager, ids) = manager_with(&["Show"]);
    let ghost = uuid::Uuid::new_v4();

    assert_eq!(
        manager.set_parent(ghost, Some(ids[0])),
        Err(SceneHierarchyError::UnknownScene(ghost))
    );
    assert_eq!(
        manager.set_parent(ids[0], Some(ghost)),
        Err(SceneHierarchyError::UnknownParent(ghost))
    );
}

#[test]
fn set_parent_can_move_a_scene_back_to_the_top_level() {
    let (mut manager, ids) = manager_with(&["Show", "Game"]);
    manager.set_parent(ids[1], Some(ids[0])).unwrap();
    manager.set_parent(ids[1], None).unwrap();
    assert_eq!(manager.get(ids[1]).unwrap().parent, None);
}

#[test]
fn removing_a_folder_lifts_its_children_instead_of_hiding_them() {
    let (mut manager, ids) = manager_with(&["Show", "Game"]);
    manager.set_parent(ids[1], Some(ids[0])).unwrap();

    assert!(manager.remove(ids[0]));
    let game = manager.get(ids[1]).expect("the child must survive");
    assert_eq!(
        game.parent, None,
        "the child must be lifted to the removed folder's own parent"
    );
    assert!(
        manager.ordered_scenes().contains(&ids[1]),
        "a surviving scene must stay reachable from the presentation order"
    );
}

#[test]
fn removing_a_nested_folder_reparents_its_children_to_the_grandparent() {
    // Show > Game > Chat; removing Game must leave Chat directly under Show.
    let (mut manager, ids) = manager_with(&["Show", "Game", "Chat"]);
    manager.set_parent(ids[1], Some(ids[0])).unwrap();
    manager.set_parent(ids[2], Some(ids[1])).unwrap();

    assert!(manager.remove(ids[1]));
    assert_eq!(manager.get(ids[2]).unwrap().parent, Some(ids[0]));
    assert_eq!(manager.ordered_scenes(), vec![ids[0], ids[2]]);
}

#[test]
fn duplicating_a_scene_keeps_it_in_its_folder_and_component_kind() {
    let mut manager = SceneManager::new();
    let folder = manager.add(rivulet_core::Scene::new("Shows".to_string()));
    let mut component = rivulet_core::Scene::component("Overlay".to_string());
    component.parent = Some(folder);
    let source = manager.add(component);

    let copy_id = manager
        .duplicate_scene(source, "Overlay copy")
        .expect("duplicating a known scene must succeed");
    let copy = manager.get(copy_id).unwrap();
    assert_eq!(copy.parent, Some(folder));
    assert_eq!(copy.kind, SceneKind::Component);
    assert_ne!(copy.id, source);
}

#[test]
fn set_color_and_set_kind_are_undoable_and_report_unknown_ids() {
    let (mut manager, ids) = manager_with(&["Game"]);
    let ghost = uuid::Uuid::new_v4();

    assert!(manager.set_color(ids[0], 0xFF1A73E8));
    assert_eq!(manager.get(ids[0]).unwrap().color, 0xFF1A73E8);
    assert!(manager.set_kind(ids[0], SceneKind::Component));
    assert!(manager.get(ids[0]).unwrap().is_component());

    assert!(!manager.set_color(ghost, 0xFF000000));
    assert!(!manager.set_kind(ghost, SceneKind::Primary));

    assert!(manager.undo(), "the kind change must be undoable");
    assert!(!manager.get(ids[0]).unwrap().is_component());
    assert!(manager.undo(), "the color change must be undoable");
    assert_eq!(manager.get(ids[0]).unwrap().color, 0xFFFFFFFF);
}

#[test]
fn export_import_roundtrip_preserves_folder_color_and_component_kind() {
    let mut manager = SceneManager::new();
    let folder = manager.add(rivulet_core::Scene::new("Shows".to_string()));
    let mut child = rivulet_core::Scene::with_color("Game".to_string(), 0xFF00FF00);
    child.parent = Some(folder);
    let child_id = manager.add(child);
    let mut component = rivulet_core::Scene::component("Overlay".to_string());
    component.parent = Some(child_id);
    let component_id = manager.add(component);

    let json = manager.export_json().expect("export must succeed");
    let restored = SceneManager::import_json(&json).expect("import must succeed");

    assert_eq!(restored.get(child_id).unwrap().parent, Some(folder));
    assert_eq!(restored.get(child_id).unwrap().color, 0xFF00FF00);
    assert!(restored.get(component_id).unwrap().is_component());
    assert_eq!(restored.ordered_scenes(), manager.ordered_scenes());
}

#[test]
fn a_collection_exported_before_the_component_flag_still_loads_as_primary() {
    // #319 adds `kind` to the serialized scene. A collection written by an
    // older build has no such field; refusing it would strand every existing
    // user's scenes, so the field defaults to `primary`.
    let json = r#"{
        "scenes": [
            {
                "id": "11111111-1111-4111-8111-111111111111",
                "name": "Legacy",
                "parent": null,
                "color": 4294967295
            }
        ],
        "active": "11111111-1111-4111-8111-111111111111",
        "history": [],
        "collection": "Default",
        "profile": "Default"
    }"#;

    let manager = SceneManager::import_json(json).expect("legacy collections must still load");
    let scene = &manager.scenes()[0];
    assert_eq!(scene.kind, SceneKind::Primary);
    assert_eq!(scene.name, "Legacy");
}

#[test]
fn a_dangling_parent_from_a_hand_edited_import_stays_visible() {
    let json = r#"{
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
    }"#;

    let manager = SceneManager::import_json(json).expect("import must not refuse the collection");
    assert_eq!(
        manager.ordered_scenes().len(),
        1,
        "a scene whose parent is gone must still be reachable"
    );
}

#[test]
fn a_cyclic_import_cannot_make_the_order_loop_or_drop_scenes() {
    // Hand-edited collection: A -> B -> A. A naive depth-first walk would
    // recurse forever; every scene must still be listed exactly once.
    let json = r#"{
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
    }"#;

    let manager = SceneManager::import_json(json).expect("import must not refuse the collection");
    let ordered = manager.ordered_scenes();
    assert_eq!(ordered.len(), 2, "both scenes must be listed: {ordered:?}");
    let mut unique = ordered.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 2, "no scene may be listed twice: {ordered:?}");
}
