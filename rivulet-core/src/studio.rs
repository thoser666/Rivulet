use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::{SceneTransition, TransitionKind};

/// Separates the scene being prepared from the scene currently on program.
///
/// Studio Mode deliberately does not mutate the scene collection when a user
/// selects a preview scene. Only [`Self::take`] promotes the preview scene to
/// program and creates the transition metadata for the renderer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StudioMode {
    enabled: bool,
    preview: Option<Uuid>,
    program: Option<Uuid>,
}

impl StudioMode {
    /// Create a disabled Studio Mode state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the separate preview/program workflow is enabled.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Current preview scene, if one is selected.
    pub fn preview(&self) -> Option<Uuid> {
        self.preview
    }

    /// Current program scene, if one is live.
    pub fn program(&self) -> Option<Uuid> {
        self.program
    }

    /// Enable Studio Mode using the current live scene as both preview and
    /// program. Disabling it clears the two-role state so the next enable
    /// starts from the then-current live scene.
    pub fn set_enabled(&mut self, enabled: bool, current_scene: Option<Uuid>) {
        if enabled && !self.enabled {
            self.program = current_scene;
            self.preview = current_scene;
        } else if !enabled {
            self.preview = None;
            self.program = None;
        }
        self.enabled = enabled;
    }

    /// Select a scene for preview without changing program output.
    pub fn set_preview(&mut self, scene: Option<Uuid>) {
        if self.enabled {
            self.preview = scene;
        }
    }

    /// Remove a scene from the Preview/Program roles when it no longer exists.
    pub fn remove_scene(&mut self, scene: Uuid) {
        if self.preview == Some(scene) {
            self.preview = self.program.filter(|id| *id != scene);
        }
        if self.program == Some(scene) {
            self.program = None;
        }
    }

    /// Whether the selected preview scene can be taken live.
    pub fn can_take(&self) -> bool {
        self.enabled && self.preview.is_some() && self.preview != self.program
    }

    /// Promote preview to program and return the transition to render.
    pub fn take(
        &mut self,
        kind: TransitionKind,
        duration: Duration,
        now: Instant,
    ) -> Option<SceneTransition> {
        if !self.can_take() {
            return None;
        }
        let from = self.program;
        let to = self.preview?;
        self.program = Some(to);

        let mut transition = SceneTransition::new(kind, duration);
        transition.start(from, Some(to), now);
        Some(transition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene() -> Uuid {
        Uuid::new_v4()
    }

    #[test]
    fn enabling_uses_current_scene_for_both_roles() {
        let current = scene();
        let mut studio = StudioMode::new();
        studio.set_enabled(true, Some(current));

        assert!(studio.enabled());
        assert_eq!(studio.preview(), Some(current));
        assert_eq!(studio.program(), Some(current));
        assert!(!studio.can_take());
    }

    #[test]
    fn preview_selection_does_not_change_program() {
        let program = scene();
        let preview = scene();
        let mut studio = StudioMode::new();
        studio.set_enabled(true, Some(program));
        studio.set_preview(Some(preview));

        assert_eq!(studio.preview(), Some(preview));
        assert_eq!(studio.program(), Some(program));
        assert!(studio.can_take());
    }

    #[test]
    fn take_promotes_preview_and_preserves_transition_endpoints() {
        let program = scene();
        let preview = scene();
        let now = Instant::now();
        let mut studio = StudioMode::new();
        studio.set_enabled(true, Some(program));
        studio.set_preview(Some(preview));

        let transition = studio
            .take(TransitionKind::Fade, Duration::from_millis(500), now)
            .expect("preview should be takeable");

        assert_eq!(studio.program(), Some(preview));
        assert_eq!(transition.from, Some(program));
        assert_eq!(transition.to, Some(preview));
        assert_eq!(transition.progress_at(now), 0.0);
    }

    #[test]
    fn take_is_idempotent_until_a_new_preview_is_selected() {
        let program = scene();
        let preview = scene();
        let mut studio = StudioMode::new();
        studio.set_enabled(true, Some(program));
        studio.set_preview(Some(preview));

        assert!(studio
            .take(TransitionKind::Cut, Duration::ZERO, Instant::now())
            .is_some());
        assert!(studio
            .take(TransitionKind::Cut, Duration::ZERO, Instant::now())
            .is_none());
    }

    #[test]
    fn removing_a_scene_clears_its_studio_roles() {
        let program = scene();
        let preview = scene();
        let mut studio = StudioMode::new();
        studio.set_enabled(true, Some(program));
        studio.set_preview(Some(preview));

        studio.remove_scene(preview);
        assert_eq!(studio.preview(), Some(program));
        studio.remove_scene(program);
        assert_eq!(studio.preview(), None);
        assert_eq!(studio.program(), None);
    }

    #[test]
    fn disabling_clears_roles_and_prevents_take() {
        let program = scene();
        let mut studio = StudioMode::new();
        studio.set_enabled(true, Some(program));
        studio.set_enabled(false, Some(program));

        assert!(!studio.enabled());
        assert_eq!(studio.preview(), None);
        assert_eq!(studio.program(), None);
        assert!(!studio.can_take());
    }

    #[test]
    fn preview_selection_is_ignored_when_disabled() {
        let preview = scene();
        let mut studio = StudioMode::new();
        studio.set_preview(Some(preview));

        assert_eq!(studio.preview(), None);
        assert_eq!(studio.program(), None);
    }
}
