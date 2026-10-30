//! Scenes, scene collections, and the folder hierarchy between them.
//!
//! A [`Scene`] holds the sources and settings that are live at once; a
//! [`SceneManager`] owns the collection, the active scene, its switch history
//! and the undo/redo stacks, and reads/writes the JSON collection format.
//!
//! Scenes form a tree through [`Scene::parent`]: a scene with children doubles
//! as a folder for the scene dock. Moves go through
//! [`SceneManager::set_parent`], which refuses cycles with a typed
//! [`SceneHierarchyError`], and [`Scene::kind`] separates the scenes a streamer
//! switches through from reusable component scenes. The full contract,
//! including how legacy and hand-edited collections load, is documented in
//! `docs/scene-management.md`.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uuid::Uuid;

/// What a scene is for in a collection ([#319](https://github.com/thoser666/Rivulet/issues/319)).
///
/// A `Primary` scene is part of the live-switching list a streamer works
/// through during a show. A `Component` scene is a reusable building block
/// (an intro layout, a BRB composition, a "just the chat" panel) that should
/// not inflate that list: it is still switchable and still exportable, it is
/// just filtered out of the default picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneKind {
    /// A normal scene, shown in the live-switching list.
    #[default]
    Primary,
    /// A reusable sub-scene, hidden from the default switching list.
    Component,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub id: Uuid,
    pub name: String,
    /// Optional parent scene ID for hierarchical folder organization
    pub parent: Option<Uuid>,
    /// Color for UI color-coding (ARGB format as u32, e.g. 0xFF1A73E8 for blue)
    pub color: u32,
    /// Whether this scene is part of the live-switching list or a reusable
    /// component. `serde(default)` keeps collections exported before #319
    /// loadable: they carry no `kind` field and every scene in them is primary.
    #[serde(default)]
    pub kind: SceneKind,
}

impl Scene {
    pub fn new(name: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            parent: None,
            color: 0xFFFFFFFF, // default white
            kind: SceneKind::Primary,
        }
    }

    pub fn with_parent(name: String, parent: Uuid) -> Self {
        Self {
            parent: Some(parent),
            ..Self::new(name)
        }
    }

    pub fn with_color(name: String, color: u32) -> Self {
        Self {
            color,
            ..Self::new(name)
        }
    }

    pub fn with_parent_and_color(name: String, parent: Uuid, color: u32) -> Self {
        Self {
            parent: Some(parent),
            color,
            ..Self::new(name)
        }
    }

    /// A reusable component scene, kept out of the live-switching list.
    pub fn component(name: String) -> Self {
        Self {
            kind: SceneKind::Component,
            ..Self::new(name)
        }
    }

    /// Whether this scene is a reusable component rather than a live scene.
    pub fn is_component(&self) -> bool {
        self.kind == SceneKind::Component
    }
}

/// Why a parent assignment was refused.
///
/// Every variant names the offending scene so an embedding application can
/// surface the refusal next to the scene it was dropped on, instead of
/// silently building a collection that renders as an empty tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SceneHierarchyError {
    /// The scene that should move does not exist.
    UnknownScene(Uuid),
    /// The proposed parent does not exist.
    UnknownParent(Uuid),
    /// A scene cannot be its own parent.
    SelfParent(Uuid),
    /// The proposed parent is inside the moved scene's own subtree, which
    /// would make the hierarchy a cycle.
    Cycle {
        /// The scene that should move.
        scene: Uuid,
        /// The proposed parent, a descendant of `scene`.
        parent: Uuid,
    },
}

impl std::fmt::Display for SceneHierarchyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownScene(id) => {
                write!(f, "scene {id} does not exist")
            }
            Self::UnknownParent(id) => {
                write!(f, "parent scene {id} does not exist")
            }
            Self::SelfParent(id) => {
                write!(f, "scene {id} cannot be its own parent")
            }
            Self::Cycle { scene, parent } => write!(
                f,
                "scene {parent} is a descendant of scene {scene}; move it to a scene outside its own subtree"
            ),
        }
    }
}

impl std::error::Error for SceneHierarchyError {}

/// Manages the scene collection: adding/removing/renaming scenes and
/// switching the active scene (the one whose sources are shown on output).
/// Scene switches are tracked in a history stack so `switch_back` behaves
/// like an undo (A → B → C, back → B, back → A).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SceneManager {
    scenes: Vec<Scene>,
    active: Option<Uuid>,
    history: Vec<Uuid>,
    #[serde(skip)]
    undo_stack: Vec<SceneSnapshot>,
    #[serde(skip)]
    redo_stack: Vec<SceneSnapshot>,
    collection: String,
    profile: String,
}

#[derive(Debug, Clone, PartialEq)]
struct SceneSnapshot {
    scenes: Vec<Scene>,
    active: Option<Uuid>,
    history: Vec<Uuid>,
    collection: String,
    profile: String,
}

impl Default for SceneManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SceneManager {
    pub fn new() -> Self {
        Self {
            scenes: Vec::new(),
            active: None,
            history: Vec::new(),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            collection: "Default".to_string(),
            profile: "Default".to_string(),
        }
    }

    fn snapshot(&self) -> SceneSnapshot {
        SceneSnapshot {
            scenes: self.scenes.clone(),
            active: self.active,
            history: self.history.clone(),
            collection: self.collection.clone(),
            profile: self.profile.clone(),
        }
    }

    fn record_change(&mut self, before: SceneSnapshot) {
        if before != self.snapshot() {
            self.undo_stack.push(before);
            self.redo_stack.clear();
        }
    }

    /// Whether an undo operation is available.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Whether a redo operation is available.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Undo the most recent scene mutation or selection change.
    pub fn undo(&mut self) -> bool {
        let Some(previous) = self.undo_stack.pop() else {
            return false;
        };
        self.redo_stack.push(self.snapshot());
        self.scenes = previous.scenes;
        self.active = previous.active;
        self.history = previous.history;
        self.collection = previous.collection;
        self.profile = previous.profile;
        true
    }

    /// Redo the most recently undone scene operation.
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.redo_stack.pop() else {
            return false;
        };
        self.undo_stack.push(self.snapshot());
        self.scenes = next.scenes;
        self.active = next.active;
        self.history = next.history;
        self.collection = next.collection;
        self.profile = next.profile;
        true
    }

    /// Adds a scene and activates it when it is the first scene.
    pub fn add(&mut self, scene: Scene) -> Uuid {
        let before = self.snapshot();
        let id = scene.id;
        self.scenes.push(scene);
        if self.active.is_none() {
            self.active = Some(id);
        }
        self.record_change(before);
        id
    }

    pub fn remove(&mut self, id: Uuid) -> bool {
        let before = self.snapshot();
        let Some(index) = self.scenes.iter().position(|s| s.id == id) else {
            return false;
        };
        let former_parent = self.scenes[index].parent;
        self.scenes.remove(index);
        // A child of the removed scene would keep a parent that no longer
        // exists and drop out of `ordered_scenes` (and therefore out of the
        // UI) entirely. Lifting it one level keeps it visible and switchable.
        for scene in self.scenes.iter_mut() {
            if scene.parent == Some(id) {
                scene.parent = former_parent;
            }
        }
        self.history.retain(|h| *h != id);
        if self.active == Some(id) {
            // Restore the most recent still-existing scene from history,
            // otherwise fall back to the last remaining scene (or none).
            self.active = self
                .history
                .pop()
                .filter(|h| self.scenes.iter().any(|s| s.id == *h))
                .or_else(|| self.scenes.last().map(|s| s.id));
        }
        self.record_change(before);
        true
    }

    /// Moves a scene under `parent` (`None` moves it back to the top level).
    ///
    /// Refuses assignments that would produce an unusable tree: an unknown
    /// scene or parent, a scene as its own parent, and a parent that is inside
    /// the moved scene's own subtree (which would make the hierarchy a cycle
    /// and hang a naive renderer). The rule is the same one a file manager
    /// applies when you drop a folder into itself.
    pub fn set_parent(
        &mut self,
        id: Uuid,
        parent: Option<Uuid>,
    ) -> Result<(), SceneHierarchyError> {
        if self.get(id).is_none() {
            return Err(SceneHierarchyError::UnknownScene(id));
        }
        if let Some(parent_id) = parent {
            if parent_id == id {
                return Err(SceneHierarchyError::SelfParent(id));
            }
            if self.get(parent_id).is_none() {
                return Err(SceneHierarchyError::UnknownParent(parent_id));
            }
            if self.is_descendant_of(parent_id, id) {
                return Err(SceneHierarchyError::Cycle {
                    scene: id,
                    parent: parent_id,
                });
            }
        }
        let before = self.snapshot();
        if let Some(scene) = self.scenes.iter_mut().find(|s| s.id == id) {
            scene.parent = parent;
        }
        self.record_change(before);
        Ok(())
    }

    /// Sets the color label of a scene; returns `false` for an unknown ID.
    pub fn set_color(&mut self, id: Uuid, color: u32) -> bool {
        let before = self.snapshot();
        let Some(scene) = self.scenes.iter_mut().find(|s| s.id == id) else {
            return false;
        };
        scene.color = color;
        self.record_change(before);
        true
    }

    /// Marks a scene as a live scene or a reusable component; returns `false`
    /// for an unknown ID.
    pub fn set_kind(&mut self, id: Uuid, kind: SceneKind) -> bool {
        let before = self.snapshot();
        let Some(scene) = self.scenes.iter_mut().find(|s| s.id == id) else {
            return false;
        };
        scene.kind = kind;
        self.record_change(before);
        true
    }

    /// Whether `candidate` sits anywhere below `ancestor` in the tree.
    ///
    /// Iterative and step-bounded: imported collections can be hand-edited,
    /// and a corrupt file must not turn a validation call into an endless walk.
    fn is_descendant_of(&self, candidate: Uuid, ancestor: Uuid) -> bool {
        let mut current = self.get(candidate).and_then(|scene| scene.parent);
        let mut steps = 0;
        while let Some(parent) = current {
            if parent == ancestor {
                return true;
            }
            steps += 1;
            if steps > self.scenes.len() {
                break;
            }
            current = self.get(parent).and_then(|scene| scene.parent);
        }
        false
    }

    /// Renames a scene; returns the old name or `None` if the scene is unknown.
    pub fn rename(&mut self, id: Uuid, name: String) -> Option<String> {
        let before = self.snapshot();
        let scene = self.scenes.iter_mut().find(|s| s.id == id)?;
        let old = std::mem::replace(&mut scene.name, name);
        self.record_change(before);
        Some(old)
    }

    /// Switches the active scene. Unknown IDs are ignored (returns false).
    /// The previously active scene is pushed onto the history stack.
    pub fn switch_to(&mut self, id: Uuid) -> bool {
        if !self.scenes.iter().any(|s| s.id == id) {
            return false;
        }
        let before = self.snapshot();
        if self.active != Some(id) {
            if let Some(previous) = self.active {
                self.history.push(previous);
            }
        }
        self.active = Some(id);
        self.record_change(before);
        true
    }

    /// Undo: reverts to the most recently active scene that still exists.
    pub fn switch_back(&mut self) -> bool {
        let before = self.snapshot();
        while let Some(previous) = self.history.pop() {
            if self.scenes.iter().any(|s| s.id == previous) {
                self.active = Some(previous);
                self.record_change(before);
                return true;
            }
        }
        false
    }

    pub fn collection(&self) -> &str {
        &self.collection
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }

    pub fn set_collection(&mut self, name: impl Into<String>) {
        let before = self.snapshot();
        self.collection = name.into();
        self.record_change(before);
    }

    pub fn set_profile(&mut self, name: impl Into<String>) {
        let before = self.snapshot();
        self.profile = name.into();
        self.record_change(before);
    }

    /// Serialize the current scene collection and profile context.
    pub fn export_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Restore a scene collection from JSON. Undo/redo history starts empty.
    pub fn import_json(json: &str) -> Result<Self, serde_json::Error> {
        let mut manager: Self = serde_json::from_str(json)?;
        manager.undo_stack.clear();
        manager.redo_stack.clear();
        Ok(manager)
    }

    /// Create a copy with fresh scene IDs so it can be edited independently.
    ///
    /// The copy stays in the source scene's folder and keeps its kind, so
    /// duplicating inside a folder does not eject the copy to the top level.
    pub fn duplicate_scene(&mut self, id: Uuid, name: impl Into<String>) -> Option<Uuid> {
        let source = self.get(id)?.clone();
        let before = self.snapshot();
        let mut copy = source;
        copy.id = Uuid::new_v4();
        copy.name = name.into();
        let new_id = copy.id;
        self.scenes.push(copy);
        self.record_change(before);
        Some(new_id)
    }

    pub fn scenes(&self) -> &[Scene] {
        &self.scenes
    }

    pub fn get(&self, id: Uuid) -> Option<&Scene> {
        self.scenes.iter().find(|s| s.id == id)
    }

    pub fn active(&self) -> Option<Uuid> {
        self.active
    }

    pub fn active_scene(&self) -> Option<&Scene> {
        self.active.and_then(|id| self.get(id))
    }

    /// The scene active before the current one (top of the history stack).
    pub fn last_active(&self) -> Option<Uuid> {
        self.history.last().copied()
    }

    /// All scene IDs ordered so that children directly follow their parent
    /// (depth-first, folders first) — a stable presentation order for the UI.
    ///
    /// Every scene appears exactly once, even in a collection that was
    /// imported with a dangling or cyclic `parent`: orphaned scenes count as
    /// roots, and a cycle cannot make the walk repeat or hang. A scene hidden
    /// by a broken hierarchy would otherwise be unreachable from the UI.
    pub fn ordered_scenes(&self) -> Vec<Uuid> {
        let mut ordered: Vec<Uuid> = Vec::with_capacity(self.scenes.len());
        let mut visited: HashSet<Uuid> = HashSet::with_capacity(self.scenes.len());
        for scene in &self.scenes {
            let is_root = match scene.parent {
                None => true,
                Some(parent) => self.get(parent).is_none(),
            };
            if is_root {
                self.push_subtree(scene.id, &mut ordered, &mut visited);
            }
        }
        // Anything still unvisited only exists inside a corrupt cycle.
        for scene in &self.scenes {
            self.push_subtree(scene.id, &mut ordered, &mut visited);
        }
        ordered
    }

    fn push_subtree(&self, id: Uuid, out: &mut Vec<Uuid>, visited: &mut HashSet<Uuid>) {
        if !visited.insert(id) {
            return;
        }
        out.push(id);
        for scene in &self.scenes {
            if scene.parent == Some(id) {
                self.push_subtree(scene.id, out, visited);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_new_creates_unique_id_and_name() {
        let scene = Scene::new("Hauptszene".to_string());
        assert_eq!(scene.name, "Hauptszene");
        assert_ne!(scene.id, Uuid::nil());
        assert_eq!(scene.parent, None);
        assert_eq!(scene.color, 0xFFFFFFFF);
    }

    #[test]
    fn scene_with_parent_sets_parent_and_inherits_default_color() {
        let parent = Scene::new("Parent".to_string()).id;
        let child = Scene::with_parent("Child".to_string(), parent);
        assert_eq!(child.parent, Some(parent));
        assert_eq!(child.color, 0xFFFFFFFF);
    }

    #[test]
    fn scene_with_color_sets_custom_color() {
        let scene = Scene::with_color("Colored".to_string(), 0xFF1A73E8);
        assert_eq!(scene.color, 0xFF1A73E8);
        assert_eq!(scene.parent, None);
    }

    #[test]
    fn scene_with_parent_and_color_sets_both() {
        let parent = Scene::new("Parent".to_string()).id;
        let child = Scene::with_parent_and_color("Child".to_string(), parent, 0xFF36A2EB);
        assert_eq!(child.parent, Some(parent));
        assert_eq!(child.color, 0xFF36A2EB);
    }

    #[test]
    fn manager_first_scene_becomes_active() {
        let mut mgr = SceneManager::new();
        assert_eq!(mgr.active(), None);
        let id = mgr.add(Scene::new("Game".to_string()));
        assert_eq!(mgr.active(), Some(id));
        assert_eq!(mgr.active_scene().map(|s| s.name.as_str()), Some("Game"));
    }

    #[test]
    fn manager_switch_changes_active_and_tracks_last_active() {
        let mut mgr = SceneManager::new();
        let a = mgr.add(Scene::new("A".to_string()));
        let b = mgr.add(Scene::new("B".to_string()));
        assert_eq!(mgr.active(), Some(a));

        assert!(mgr.switch_to(b));
        assert_eq!(mgr.active(), Some(b));
        assert_eq!(mgr.last_active(), Some(a));

        // Switching to the same scene is a no-op for last_active.
        assert!(mgr.switch_to(b));
        assert_eq!(mgr.last_active(), Some(a));

        // Unknown IDs are rejected.
        assert!(!mgr.switch_to(Uuid::new_v4()));
        assert_eq!(mgr.active(), Some(b));
    }

    #[test]
    fn manager_undo_redo_restores_scene_add_and_name_changes() {
        let mut mgr = SceneManager::new();
        let id = mgr.add(Scene::new("Game".to_string()));
        assert!(mgr.can_undo());
        assert!(mgr.undo());
        assert!(mgr.scenes().is_empty());
        assert_eq!(mgr.active(), None);
        assert!(mgr.can_redo());
        assert!(mgr.redo());
        assert_eq!(mgr.get(id).map(|s| s.name.as_str()), Some("Game"));

        mgr.rename(id, "Live".to_string());
        assert!(mgr.undo());
        assert_eq!(mgr.get(id).map(|s| s.name.as_str()), Some("Game"));
        assert!(mgr.redo());
        assert_eq!(mgr.get(id).map(|s| s.name.as_str()), Some("Live"));
    }

    #[test]
    fn manager_new_change_clears_redo_stack() {
        let mut mgr = SceneManager::new();
        let a = mgr.add(Scene::new("A".to_string()));
        mgr.add(Scene::new("B".to_string()));
        assert!(mgr.undo());
        assert!(mgr.can_redo());
        mgr.rename(a, "Renamed".to_string());
        assert!(!mgr.can_redo());
    }

    #[test]
    fn manager_switch_back_is_undoable() {
        let mut mgr = SceneManager::new();
        let a = mgr.add(Scene::new("A".to_string()));
        let b = mgr.add(Scene::new("B".to_string()));
        mgr.switch_to(b);
        assert!(mgr.switch_back());
        assert_eq!(mgr.active(), Some(a));
        assert!(mgr.undo());
        assert_eq!(mgr.active(), Some(b));
        assert!(mgr.redo());
        assert_eq!(mgr.active(), Some(a));
    }

    #[test]
    fn manager_switch_back_returns_to_previous_scene() {
        let mut mgr = SceneManager::new();
        let a = mgr.add(Scene::new("A".to_string()));
        let b = mgr.add(Scene::new("B".to_string()));
        let c = mgr.add(Scene::new("C".to_string()));
        mgr.switch_to(b);
        mgr.switch_to(c);
        assert_eq!(mgr.active(), Some(c));

        assert!(mgr.switch_back());
        assert_eq!(mgr.active(), Some(b));
        assert_eq!(mgr.last_active(), Some(a));

        assert!(mgr.switch_back());
        assert_eq!(mgr.active(), Some(a));
        assert_eq!(mgr.last_active(), None);

        // History exhausted → false, active unchanged.
        assert!(!mgr.switch_back());
        assert_eq!(mgr.active(), Some(a));

        // A fresh manager without switching history returns false.
        let mut fresh = SceneManager::new();
        fresh.add(Scene::new("Only".to_string()));
        assert!(!fresh.switch_back());
        assert!(fresh.active().is_some());
    }

    #[test]
    fn manager_remove_unknown_returns_false() {
        let mut mgr = SceneManager::new();
        mgr.add(Scene::new("A".to_string()));
        assert!(!mgr.remove(Uuid::new_v4()));
        assert_eq!(mgr.scenes().len(), 1);
    }

    #[test]
    fn manager_remove_active_falls_back_to_last_scene() {
        let mut mgr = SceneManager::new();
        let a = mgr.add(Scene::new("A".to_string()));
        let b = mgr.add(Scene::new("B".to_string()));
        mgr.switch_to(b);
        assert!(mgr.remove(b));
        assert_eq!(mgr.scenes().len(), 1);
        assert_eq!(mgr.active(), Some(a));
    }

    #[test]
    fn manager_remove_last_scene_clears_active() {
        let mut mgr = SceneManager::new();
        let a = mgr.add(Scene::new("A".to_string()));
        assert!(mgr.remove(a));
        assert_eq!(mgr.scenes().len(), 0);
        assert_eq!(mgr.active(), None);
    }

    #[test]
    fn manager_rename_updates_name_and_reports_old() {
        let mut mgr = SceneManager::new();
        let a = mgr.add(Scene::new("Old".to_string()));
        assert_eq!(mgr.rename(a, "New".to_string()), Some("Old".to_string()));
        assert_eq!(mgr.get(a).map(|s| s.name.as_str()), Some("New"));

        assert_eq!(mgr.rename(Uuid::new_v4(), "X".to_string()), None);
    }

    #[test]
    fn manager_duplicate_scene_has_fresh_id_and_preserves_content() {
        let mut mgr = SceneManager::new();
        let original = mgr.add(Scene::with_color("Original".to_string(), 0xFF123456));
        let duplicate = mgr.duplicate_scene(original, "Copy").unwrap();
        assert_ne!(original, duplicate);
        assert_eq!(mgr.get(duplicate).unwrap().name, "Copy");
        assert_eq!(mgr.get(duplicate).unwrap().color, 0xFF123456);
        assert_eq!(mgr.scenes().len(), 2);
    }

    #[test]
    fn manager_collection_and_profile_are_switchable() {
        let mut mgr = SceneManager::new();
        assert_eq!(mgr.collection(), "Default");
        assert_eq!(mgr.profile(), "Default");
        mgr.set_collection("Gaming");
        mgr.set_profile("Streaming");
        assert_eq!(mgr.collection(), "Gaming");
        assert_eq!(mgr.profile(), "Streaming");
        assert!(mgr.undo());
        assert_eq!(mgr.profile(), "Default");
        assert!(mgr.undo());
        assert_eq!(mgr.collection(), "Default");
        assert!(mgr.redo());
        assert_eq!(mgr.collection(), "Gaming");
    }

    #[test]
    fn manager_collection_profile_round_trip_preserves_context_and_resets_history() {
        let mut original = SceneManager::new();
        original.add(Scene::new("Gameplay".to_string()));
        original.set_collection("Gaming");
        original.set_profile("Streaming");
        let json = original.export_json().expect("export scene collection");
        let restored = SceneManager::import_json(&json).expect("import scene collection");
        assert_eq!(restored.collection(), "Gaming");
        assert_eq!(restored.profile(), "Streaming");
        assert_eq!(restored.scenes(), original.scenes());
        assert!(!restored.can_undo());
        assert!(!restored.can_redo());
    }

    #[test]
    fn manager_import_rejects_invalid_json() {
        assert!(SceneManager::import_json("not json").is_err());
    }

    #[test]
    fn manager_ordered_scenes_groups_children_under_parents() {
        let mut mgr = SceneManager::new();
        let parent = mgr.add(Scene::new("Folder".to_string()));
        let child1 = mgr.add(Scene::with_parent("Child 1".to_string(), parent));
        let other = mgr.add(Scene::new("Top".to_string()));
        let child2 = mgr.add(Scene::with_parent("Child 2".to_string(), parent));

        let ids = mgr.ordered_scenes();
        let names: Vec<&str> = ids
            .iter()
            .map(|id| mgr.get(*id).map(|s| s.name.as_str()).unwrap_or(""))
            .collect();
        assert_eq!(names, vec!["Folder", "Child 1", "Child 2", "Top"]);
        // Every added scene is present exactly once, and child ids are grouped
        // under their parent regardless of insertion order.
        assert_eq!(ids.len(), 4);
        assert!(ids.contains(&parent));
        assert!(ids.contains(&child1));
        assert!(ids.contains(&child2));
        assert!(ids.contains(&other));
    }
}
