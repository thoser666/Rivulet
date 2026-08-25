use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub id: Uuid,
    pub name: String,
    /// Optional parent scene ID for hierarchical folder organization
    pub parent: Option<Uuid>,
    /// Color for UI color-coding (ARGB format as u32, e.g. 0xFF1A73E8 for blue)
    pub color: u32,
}

impl Scene {
    pub fn new(name: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            parent: None,
            color: 0xFFFFFFFF, // default white
        }
    }

    pub fn with_parent(name: String, parent: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            parent: Some(parent),
            color: 0xFFFFFFFF,
        }
    }

    pub fn with_color(name: String, color: u32) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            parent: None,
            color,
        }
    }

    pub fn with_parent_and_color(name: String, parent: Uuid, color: u32) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            parent: Some(parent),
            color,
        }
    }
}

/// Manages the scene collection: adding/removing/renaming scenes and
/// switching the active scene (the one whose sources are shown on output).
/// Scene switches are tracked in a history stack so `switch_back` behaves
/// like an undo (A → B → C, back → B, back → A).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneManager {
    scenes: Vec<Scene>,
    active: Option<Uuid>,
    history: Vec<Uuid>,
    undo_stack: Vec<SceneSnapshot>,
    redo_stack: Vec<SceneSnapshot>,
}

#[derive(Debug, Clone, PartialEq)]
struct SceneSnapshot {
    scenes: Vec<Scene>,
    active: Option<Uuid>,
    history: Vec<Uuid>,
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
        }
    }

    fn snapshot(&self) -> SceneSnapshot {
        SceneSnapshot {
            scenes: self.scenes.clone(),
            active: self.active,
            history: self.history.clone(),
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
        self.scenes.remove(index);
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
    pub fn ordered_scenes(&self) -> Vec<Uuid> {
        let mut ordered: Vec<Uuid> = Vec::with_capacity(self.scenes.len());
        for scene in &self.scenes {
            if scene.parent.is_none() {
                ordered.push(scene.id);
                self.push_children(scene.id, &mut ordered);
            }
        }
        ordered
    }

    fn push_children(&self, parent: Uuid, out: &mut Vec<Uuid>) {
        for scene in &self.scenes {
            if scene.parent == Some(parent) {
                out.push(scene.id);
                self.push_children(scene.id, out);
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
