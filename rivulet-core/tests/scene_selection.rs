//! Multi-selection and bulk operations for the scene dock
//! ([#366](https://github.com/thoser666/Rivulet/issues/366)).
//!
//! The selection type lives in `rivulet_core::scene_dock` and the bulk
//! mutations in `rivulet_core::SceneManager` precisely so the rules can be
//! checked without a window: which rows a shift-click reaches, what a filter
//! does to a selection, and whether a drag of six scenes is really one undo
//! step. Everything here is pure data — no GStreamer, no hardware, no
//! filesystem, and no GUI.

use std::collections::BTreeSet;

use rivulet_core::scene_dock::{
    apply_drop_selection, can_drop_selection, dock_rows, SceneSelection,
};
use rivulet_core::{
    Scene, SceneHierarchyError, SceneKind, SceneManager, TransitionDefault, TransitionKind,
};

fn manager_with(names: &[&str]) -> (SceneManager, Vec<uuid::Uuid>) {
    let mut manager = SceneManager::new();
    let ids = names
        .iter()
        .map(|name| manager.add(Scene::new((*name).to_string())))
        .collect();
    (manager, ids)
}

fn visible(manager: &SceneManager) -> Vec<uuid::Uuid> {
    dock_rows(manager, &BTreeSet::new(), &[])
        .into_iter()
        .map(|row| row.id)
        .collect()
}

// ── Click, toggle, extend ─────────────────────────────────────────────────

#[test]
fn click_selects_one_row_and_makes_it_the_anchor() {
    let (_manager, ids) = manager_with(&["A", "B", "C"]);
    let mut selection = SceneSelection::new();

    selection.select(ids[1]);
    assert_eq!(selection.ids(), [ids[1]]);
    assert_eq!(selection.anchor(), Some(ids[1]));

    // A second plain click replaces the selection instead of adding to it.
    selection.select(ids[2]);
    assert_eq!(selection.ids(), [ids[2]]);
    assert_eq!(selection.len(), 1);
}

#[test]
fn ctrl_click_toggles_membership_and_removing_the_anchor_clears_it() {
    let (_manager, ids) = manager_with(&["A", "B", "C"]);
    let mut selection = SceneSelection::new();

    selection.select(ids[0]);
    selection.toggle(ids[2]);
    assert_eq!(selection.ids(), [ids[0], ids[2]]);
    assert_eq!(selection.anchor(), Some(ids[2]));

    selection.toggle(ids[0]);
    assert_eq!(selection.ids(), [ids[2]], "toggling again removes the row");

    // Removing the anchor must not leave a stale range start behind.
    selection.toggle(ids[2]);
    assert!(selection.is_empty());
    assert_eq!(selection.anchor(), None);
}

#[test]
fn shift_click_selects_the_range_in_both_directions() {
    let (manager, _ids) = manager_with(&["A", "B", "C", "D"]);
    let rows = visible(&manager);
    let mut selection = SceneSelection::new();

    selection.select(rows[1]);
    selection.extend(rows[3], &rows);
    assert_eq!(
        selection.ids(),
        [rows[1], rows[2], rows[3]],
        "the range covers the anchor up to the clicked row"
    );

    // Extending back up narrows the same range from the same anchor.
    selection.extend(rows[0], &rows);
    assert_eq!(selection.ids(), [rows[0], rows[1]]);
    assert_eq!(
        selection.anchor(),
        Some(rows[1]),
        "the anchor survives extending"
    );
}

#[test]
fn shift_click_over_a_folded_folder_skips_the_hidden_children() {
    let (mut manager, ids) = manager_with(&["Show", "Game", "Chat", "BRB"]);
    manager.set_parent(ids[1], Some(ids[0])).unwrap();
    manager.set_parent(ids[2], Some(ids[0])).unwrap();

    // The folder is folded: the dock draws Show, BRB only.
    let folded: BTreeSet<uuid::Uuid> = [ids[0]].into_iter().collect();
    let rows: Vec<uuid::Uuid> = dock_rows(&manager, &folded, &[])
        .into_iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(rows, [ids[0], ids[3]]);

    let mut selection = SceneSelection::new();
    selection.select(rows[0]);
    selection.extend(rows[1], &rows);
    assert_eq!(
        selection.ids(),
        [ids[0], ids[3]],
        "a range cannot reach what the dock does not draw"
    );
    assert!(!selection.contains(ids[1]));
    assert!(!selection.contains(ids[2]));
}

#[test]
fn shift_click_without_a_usable_anchor_degrades_to_a_plain_click() {
    let (manager, _ids) = manager_with(&["A", "B", "C"]);
    let rows = visible(&manager);
    let mut selection = SceneSelection::new();

    // No anchor at all.
    selection.extend(rows[2], &rows);
    assert_eq!(selection.ids(), [rows[2]]);

    // An anchor the filter hides: the range would be arbitrary, so the click
    // is treated as a fresh selection.
    let hidden_anchor: Vec<uuid::Uuid> = rows[1..].to_vec();
    selection.select(rows[0]);
    selection.extend(rows[2], &hidden_anchor);
    assert_eq!(selection.ids(), [rows[2]]);
}

#[test]
fn select_all_takes_the_visible_rows_and_keeps_a_visible_anchor() {
    let (manager, ids) = manager_with(&["A", "B", "C"]);
    let rows = visible(&manager);
    let mut selection = SceneSelection::new();

    selection.select(rows[1]);
    selection.select_all(&rows);
    assert_eq!(selection.ids(), rows);
    assert_eq!(
        selection.anchor(),
        Some(ids[1]),
        "a still-visible anchor stays put"
    );

    // An anchor the filter hid moves to the first visible row instead.
    let subset: Vec<uuid::Uuid> = rows[1..].to_vec();
    selection.select_all(&subset);
    assert_eq!(selection.ids(), subset);
    assert_eq!(selection.anchor(), Some(ids[1]));

    let without_anchor: Vec<uuid::Uuid> = rows[..1].to_vec();
    selection.select_all(&without_anchor);
    assert_eq!(selection.anchor(), Some(ids[0]));
}

// ── Filter semantics (documented in docs/scene-management.md) ─────────────

#[test]
fn a_filter_does_not_deselect_hidden_scenes_but_counts_them() {
    let (manager, ids) = manager_with(&["A", "B", "C"]);
    let rows = visible(&manager);
    let mut selection = SceneSelection::new();
    selection.select(rows[0]);
    selection.toggle(rows[2]);

    // The filter now shows only the first row.
    let filtered: Vec<uuid::Uuid> = rows[..1].to_vec();
    assert_eq!(selection.len(), 2, "hidden rows stay selected");
    assert_eq!(selection.hidden_count(&filtered), 1);
    assert!(
        selection.contains(ids[2]),
        "and a bulk action still includes them"
    );
    assert_eq!(selection.hidden_count(&rows), 0);
}

#[test]
fn retain_existing_drops_deleted_scenes_and_their_anchor() {
    let (_manager, ids) = manager_with(&["A", "B", "C"]);
    let mut selection = SceneSelection::new();
    selection.select(ids[1]);
    selection.toggle(ids[2]);

    let existing: BTreeSet<uuid::Uuid> = [ids[0], ids[1]].into_iter().collect();
    selection.retain_existing(&existing);
    assert_eq!(selection.ids(), [ids[1]]);
    assert_eq!(
        selection.anchor(),
        None,
        "the removed anchor does not point at a phantom row"
    );
}

// ── Bulk move ─────────────────────────────────────────────────────────────

#[test]
fn dragging_a_selection_moves_all_of_it_in_one_undo_step() {
    let (manager, ids) = manager_with(&["Folder", "A", "B", "C"]);
    // Start from a collection whose undo history is empty, so "one undo step"
    // is measurable rather than mixed in with the four `add` steps above.
    let mut manager = SceneManager::import_json(&manager.export_json().unwrap()).unwrap();
    assert!(!manager.can_undo());

    let moved = apply_drop_selection(&mut manager, &[ids[1], ids[2], ids[3]], Some(ids[0]))
        .expect("a legal drop");
    assert_eq!(moved, 3);
    for child in &ids[1..] {
        assert_eq!(manager.get(*child).unwrap().parent, Some(ids[0]));
    }

    // One undo takes the whole batch back, not one scene per step.
    assert!(manager.undo());
    for child in &ids[1..] {
        assert_eq!(manager.get(*child).unwrap().parent, None);
    }
    assert!(!manager.can_undo(), "the batch was exactly one undo step");
}

#[test]
fn a_bulk_move_is_atomic_and_names_the_offending_scene() {
    let (mut manager, ids) = manager_with(&["Folder", "A", "Inner", "B"]);
    manager.set_parent(ids[2], Some(ids[1])).unwrap();

    // Moving A together with its own child into that child's subtree would
    // produce a cycle: the whole batch must be refused, not half-applied.
    let result = apply_drop_selection(&mut manager, &[ids[1], ids[2]], Some(ids[2]));
    match result {
        Err(SceneHierarchyError::SelfParent(id)) => assert_eq!(id, ids[2]),
        other => panic!("expected the member to be named, got {other:?}"),
    }
    assert_eq!(manager.get(ids[1]).unwrap().parent, None, "nothing moved");
}

#[test]
fn dropping_a_selection_onto_one_of_its_members_is_refused() {
    let (mut manager, ids) = manager_with(&["Folder", "A", "B"]);
    let verdict = can_drop_selection(&manager, &[ids[0], ids[1]], Some(ids[0]));
    assert!(matches!(
        verdict,
        Err(SceneHierarchyError::SelfParent(id)) if id == ids[0]
    ));
    assert!(apply_drop_selection(&mut manager, &[ids[0], ids[1]], Some(ids[0])).is_err());
    assert_eq!(manager.get(ids[1]).unwrap().parent, None);
}

#[test]
fn a_member_outside_the_selection_does_not_block_the_move() {
    let (mut manager, ids) = manager_with(&["Target", "A", "B", "Loose"]);
    let moved = apply_drop_selection(&mut manager, &[ids[1], ids[2]], Some(ids[0])).unwrap();
    assert_eq!(moved, 2);
    assert_eq!(manager.get(ids[3]).unwrap().parent, None);
}

// ── Bulk colour, kind and delete ──────────────────────────────────────────

#[test]
fn bulk_colour_and_kind_are_one_undo_step_each() {
    let (mut manager, ids) = manager_with(&["A", "B", "C"]);
    let batch = [ids[0], ids[1]];

    assert_eq!(manager.set_color_many(&batch, 0xFF1A73E8), 2);
    assert_eq!(manager.get(ids[0]).unwrap().color, 0xFF1A73E8);
    assert_eq!(manager.get(ids[1]).unwrap().color, 0xFF1A73E8);
    assert_eq!(manager.get(ids[2]).unwrap().color, 0xFFFF_FFFF);

    assert_eq!(manager.set_kind_many(&batch, SceneKind::Component), 2);
    assert!(manager.get(ids[1]).unwrap().is_component());

    // Two batch actions: two undo steps, each covering the whole batch.
    assert!(manager.undo());
    assert!(!manager.get(ids[0]).unwrap().is_component());
    assert_eq!(manager.get(ids[0]).unwrap().color, 0xFF1A73E8);
    assert!(manager.undo());
    assert_eq!(manager.get(ids[0]).unwrap().color, 0xFFFF_FFFF);
}

#[test]
fn bulk_delete_lifts_children_and_is_one_undo_step() {
    let (mut manager, ids) = manager_with(&["Folder", "A", "Keep"]);
    manager.set_parent(ids[2], Some(ids[0])).unwrap();
    // Undo history starts empty (see the bulk-move test above): the tree is
    // built first, then loaded again, so the one-step assertion is exact.
    let mut manager = SceneManager::import_json(&manager.export_json().unwrap()).unwrap();
    assert!(!manager.can_undo());

    let removed = manager.remove_many(&[ids[0], ids[1]]);
    assert_eq!(removed, 2, "unknown or duplicate IDs are not counted");
    assert!(manager.get(ids[0]).is_none());
    assert_eq!(
        manager.get(ids[2]).unwrap().parent,
        None,
        "an unselected child is lifted, not deleted"
    );

    assert!(manager.undo());
    assert!(manager.get(ids[0]).is_some());
    assert_eq!(manager.get(ids[2]).unwrap().parent, Some(ids[0]));
    assert!(!manager.can_undo(), "the batch was exactly one undo step");
}

#[test]
fn bulk_delete_prunes_the_folder_transition_of_a_removed_folder() {
    let (mut manager, ids) = manager_with(&["Folder", "A"]);
    manager.set_parent(ids[1], Some(ids[0])).unwrap();
    assert!(
        manager.set_folder_transition(ids[0], TransitionDefault::new(TransitionKind::Fade, 750),)
    );
    assert!(manager.folder_transition(ids[0]).is_some());

    assert_eq!(manager.remove_many(&[ids[0]]), 1);
    assert!(
        manager.folder_transition(ids[0]).is_none(),
        "a removed folder does not keep its default"
    );

    // Undo restores the folder and its default together.
    assert!(manager.undo());
    assert!(manager.folder_transition(ids[0]).is_some());
}

#[test]
fn remove_many_skips_unknown_ids() {
    let (mut manager, ids) = manager_with(&["A", "B"]);
    let phantom = uuid::Uuid::new_v4();
    assert_eq!(manager.remove_many(&[phantom, ids[0]]), 1);
    assert!(manager.get(ids[1]).is_some());
}
