//! Per-folder default transitions (#368).
//!
//! The rules live in `rivulet_core::SceneManager`: a folder's default applies to
//! everything inside it, the nearest ancestor wins, and a collection without the
//! map keeps loading. These tests pin the resolution order, the undo/prune
//! interactions and the two runtime properties a switch depends on (a `Cut`
//! completes at once, a `Fade` uses the folder's duration).

use std::time::{Duration, Instant};

use rivulet_core::{Scene, SceneDock, SceneManager, StudioMode, TransitionDefault, TransitionKind};
use uuid::Uuid;

fn folder(manager: &mut SceneManager, name: &str) -> Uuid {
    manager.add(Scene::new(name.to_string()))
}

fn child(manager: &mut SceneManager, name: &str, parent: Uuid) -> Uuid {
    let id = manager.add(Scene::new(name.to_string()));
    manager.set_parent(id, Some(parent)).expect("valid parent");
    id
}

fn collection_default() -> TransitionDefault {
    TransitionDefault::new(TransitionKind::Cut, 0)
}

#[test]
fn a_folder_default_applies_to_every_scene_inside_it() {
    let mut manager = SceneManager::new();
    let game = folder(&mut manager, "Game");
    let chat = child(&mut manager, "Chat", game);
    let fade = TransitionDefault::new(TransitionKind::Fade, 800);

    assert!(manager.set_folder_transition(game, fade));
    assert_eq!(manager.resolve_transition(chat, collection_default()), fade);

    // The entry belongs to the folder's *contents*: the folder itself is a
    // scene like any other when it is switched to directly.
    assert_eq!(
        manager.resolve_transition(game, collection_default()),
        collection_default()
    );
}

/// The interesting case from the issue: a folder inside a folder where only the
/// outer one has a default, and a third where both do.
#[test]
fn the_nearest_ancestor_folder_wins_over_an_outer_one() {
    let mut manager = SceneManager::new();
    let show = folder(&mut manager, "Show");
    let game = child(&mut manager, "Game", show);
    let chat = child(&mut manager, "Chat", game);
    let brb = child(&mut manager, "BRB", show);

    let outer = TransitionDefault::new(TransitionKind::Fade, 500);
    let inner = TransitionDefault::new(TransitionKind::Cut, 500);
    assert!(manager.set_folder_transition(show, outer));
    assert!(manager.set_folder_transition(game, inner));

    assert_eq!(
        manager.resolve_transition(chat, collection_default()),
        inner,
        "the closest folder with a default wins"
    );
    assert_eq!(
        manager.resolve_transition(brb, collection_default()),
        outer,
        "the outer folder still applies where no inner one does"
    );
}

#[test]
fn a_scene_without_a_folder_default_falls_back_to_the_collection_default() {
    let mut manager = SceneManager::new();
    let top = folder(&mut manager, "Top");
    let collection = TransitionDefault::new(TransitionKind::Fade, 1_200);

    assert_eq!(manager.resolve_transition(top, collection), collection);
}

#[test]
fn setting_and_clearing_a_folder_default_is_undoable() {
    let mut manager = SceneManager::new();
    let show = folder(&mut manager, "Show");
    let fade = TransitionDefault::new(TransitionKind::Fade, 600);

    assert!(manager.set_folder_transition(show, fade));
    assert_eq!(manager.folder_transition(show), Some(fade));

    assert!(manager.undo());
    assert_eq!(manager.folder_transition(show), None);
    assert_eq!(
        manager.resolve_transition(show, collection_default()),
        collection_default()
    );

    assert!(manager.redo());
    assert_eq!(manager.folder_transition(show), Some(fade));

    assert!(manager.clear_folder_transition(show));
    assert_eq!(manager.folder_transition(show), None);
    assert!(manager.undo());
    assert_eq!(manager.folder_transition(show), Some(fade));
}

#[test]
fn clearing_a_default_that_is_not_set_is_not_an_undo_step() {
    let mut manager = SceneManager::new();
    let show = folder(&mut manager, "Show");

    assert!(!manager.clear_folder_transition(show));
    // Creating the folder is still the newest undo step: the refused clear did
    // not push a phantom one, which would make Ctrl+Z do nothing visible.
    assert!(manager.undo());
    assert!(
        manager.get(show).is_none(),
        "undo takes back the scene creation, not a phantom clear"
    );
}

#[test]
fn an_unknown_folder_is_refused() {
    let mut manager = SceneManager::new();
    let unknown = Uuid::new_v4();

    assert!(
        !manager.set_folder_transition(unknown, TransitionDefault::new(TransitionKind::Fade, 300))
    );
    assert!(!manager.clear_folder_transition(unknown));
    assert_eq!(manager.folder_transition(unknown), None);
}

#[test]
fn deleting_the_folder_prunes_its_default_and_undo_brings_it_back() {
    let mut manager = SceneManager::new();
    let show = folder(&mut manager, "Show");
    let game = child(&mut manager, "Game", show);
    let fade = TransitionDefault::new(TransitionKind::Fade, 700);
    assert!(manager.set_folder_transition(show, fade));

    assert!(manager.remove(show));
    assert_eq!(
        manager.folder_transition(show),
        None,
        "the folder is gone, so its default is too"
    );
    assert_eq!(
        manager.get(game).and_then(|scene| scene.parent),
        None,
        "children are lifted to the removed folder's parent"
    );

    assert!(manager.undo());
    assert_eq!(manager.folder_transition(show), Some(fade));
    assert_eq!(manager.get(game).and_then(|scene| scene.parent), Some(show));
}

/// Folding is view state and lives in the dock, never in the collection: a fold
/// must not prune a default and must not dirty the export.
#[test]
fn folding_a_folder_does_not_touch_its_default() {
    let mut manager = SceneManager::new();
    let show = folder(&mut manager, "Show");
    let game = child(&mut manager, "Game", show);
    let fade = TransitionDefault::new(TransitionKind::Fade, 400);
    assert!(manager.set_folder_transition(show, fade));
    let exported_before = manager.export_json().expect("exportable");

    let mut dock = SceneDock::new();
    dock.set_collapsed(manager.collection(), show, true);
    assert!(dock.is_collapsed(manager.collection(), show));

    assert_eq!(
        manager.export_json().expect("exportable"),
        exported_before,
        "folding a folder is not a collection change"
    );
    assert_eq!(manager.folder_transition(show), Some(fade));
    assert_eq!(manager.resolve_transition(game, collection_default()), fade);
}

#[test]
fn export_and_import_round_trip_the_folder_defaults() {
    let mut manager = SceneManager::new();
    let show = folder(&mut manager, "Show");
    let game = child(&mut manager, "Game", show);
    let fade = TransitionDefault::new(TransitionKind::Fade, 900);
    assert!(manager.set_folder_transition(show, fade));

    let json = manager.export_json().expect("exportable");
    let restored = SceneManager::import_json(&json).expect("importable");

    assert_eq!(restored.folder_transition(show), Some(fade));
    assert_eq!(
        restored.resolve_transition(game, collection_default()),
        fade
    );
    assert!(
        !restored.can_undo(),
        "an import starts with a clean history"
    );
}

#[test]
fn a_collection_written_before_the_feature_loads_without_overrides() {
    let show = "6f9b8f6e-2b0e-4f1a-9f3f-5f7f7b0e2c11";
    let legacy = format!(
        r#"{{
  "scenes": [
    {{"id": "{show}", "name": "Show", "parent": null, "color": 4294967295, "kind": "primary"}}
  ],
  "active": "{show}",
  "history": [],
  "collection": "Default",
  "profile": "Default"
}}"#
    );

    let mut manager = SceneManager::import_json(&legacy).expect("legacy collection loads");
    let show_id = Uuid::parse_str(show).expect("uuid");
    let collection = TransitionDefault::new(TransitionKind::Fade, 250);
    assert_eq!(manager.folder_transition(show_id), None);
    assert_eq!(manager.resolve_transition(show_id, collection), collection);

    let exported = manager.export_json().expect("exportable");
    assert!(
        !exported.contains("folder_transitions"),
        "an untouched collection exports without the field, exactly as before"
    );

    // Setting one default is all it takes to start carrying the map.
    assert!(
        manager.set_folder_transition(show_id, TransitionDefault::new(TransitionKind::Fade, 500))
    );
    assert!(manager
        .export_json()
        .expect("exportable")
        .contains("folder_transitions"));
}

/// Studio mode is where transitions are most visible, so a `Take` resolves the
/// same way a direct switch does — including the runtime properties.
#[test]
fn a_studio_take_plays_the_folder_default() {
    let mut manager = SceneManager::new();
    let show = folder(&mut manager, "Show");
    let brb = child(&mut manager, "BRB", show);
    let intro = folder(&mut manager, "Intro");
    assert!(manager.set_folder_transition(show, TransitionDefault::new(TransitionKind::Fade, 750)));

    let now = Instant::now();
    let mut studio = StudioMode::new();
    studio.set_enabled(true, None);
    studio.set_preview(Some(brb));

    let resolved = manager.resolve_transition(brb, collection_default());
    let transition = studio.take_with(resolved, now).expect("takeable");
    assert_eq!(transition.kind, TransitionKind::Fade);
    assert_eq!(
        transition.duration,
        Duration::from_millis(750),
        "the folder's duration, not the collection's"
    );
    assert!(
        (transition.progress_at(now + Duration::from_millis(375)) - 0.5).abs() < 0.01,
        "a Fade interpolates over the folder's duration"
    );
    assert_eq!(studio.program(), Some(brb));

    // A scene outside the folder keeps the collection default, and a Cut never
    // shows a half-faded frame.
    studio.set_preview(Some(intro));
    let resolved = manager.resolve_transition(intro, collection_default());
    let transition = studio.take_with(resolved, now).expect("takeable");
    assert_eq!(
        transition.progress_at(now),
        1.0,
        "a Cut completes at the instant it starts"
    );
}

#[test]
fn resolution_terminates_on_a_hand_edited_cycle() {
    let a = "11111111-1111-4111-8111-111111111111";
    let b = "22222222-2222-4222-8222-222222222222";
    let json = format!(
        r#"{{"scenes":[
  {{"id":"{a}","name":"A","parent":"{b}","color":4294967295,"kind":"primary"}},
  {{"id":"{b}","name":"B","parent":"{a}","color":4294967295,"kind":"primary"}}
],"active":null,"history":[],"collection":"Default","profile":"Default"}}"#
    );
    let manager =
        SceneManager::import_json(&json).expect("a cycle loads and is repaired at render time");
    let fallback = TransitionDefault::new(TransitionKind::Cut, 100);

    assert_eq!(
        manager.resolve_transition(Uuid::parse_str(a).expect("uuid"), fallback),
        fallback,
        "a parent cycle must not hang the switch — it falls back instead"
    );
}
