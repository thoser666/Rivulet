use uuid::Uuid;

/// The kind of source (determines what data it carries and how it renders).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Image,
    Text,
    Webcam,
    Browser,
    Media,
    Color,
    GameCapture,
    ScreenCapture,
    Audio,
}

impl SourceKind {
    /// Human-readable label for UI display.
    pub fn label(&self) -> &'static str {
        match self {
            SourceKind::Image => "Image",
            SourceKind::Text => "Text",
            SourceKind::Webcam => "Webcam",
            SourceKind::Browser => "Browser",
            SourceKind::Media => "Media",
            SourceKind::Color => "Color",
            SourceKind::GameCapture => "Game Capture",
            SourceKind::ScreenCapture => "Screen Capture",
            SourceKind::Audio => "Audio",
        }
    }
}

/// 2-D transform applied to a source within a scene.
///
/// Coordinates are in scene-local pixels.  The origin is the top-left
/// corner of the scene canvas.  `opacity` ranges from 0.0 (fully
/// transparent) to 1.0 (fully opaque).
#[derive(Debug, Clone, PartialEq)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub rotation: f32, // degrees, clockwise
    pub opacity: f32,  // 0.0 – 1.0
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
            rotation: 0.0,
            opacity: 1.0,
        }
    }
}

impl Transform {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
            rotation: 0.0,
            opacity: 1.0,
        }
    }

    /// Returns true if the transform is the default (full-frame, no rotation).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Clamps opacity to the valid range [0.0, 1.0].
    pub fn clamp_opacity(&mut self) {
        self.opacity = self.opacity.clamp(0.0, 1.0);
    }

    /// Normalizes rotation to [0.0, 360.0).
    pub fn normalize_rotation(&mut self) {
        self.rotation = self.rotation.rem_euclid(360.0);
    }
}

/// A source in the application — identified by a UUID, carrying a kind,
/// a default name, and per-source metadata.
#[derive(Debug, Clone)]
pub struct Source {
    pub id: Uuid,
    pub name: String,
    pub kind: SourceKind,
    /// Default transform (used when the source is added to a scene
    /// without an explicit override).
    pub transform: Transform,
    pub visible: bool,
    pub locked: bool,
    /// Z-order for rendering within a scene (higher = on top).
    pub z_order: i32,
}

impl Source {
    pub fn new(name: String, kind: SourceKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            kind,
            transform: Transform::default(),
            visible: true,
            locked: false,
            z_order: 0,
        }
    }

    pub fn with_transform(mut self, transform: Transform) -> Self {
        self.transform = transform;
        self
    }

    pub fn with_z_order(mut self, z_order: i32) -> Self {
        self.z_order = z_order;
        self
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn locked(mut self, locked: bool) -> Self {
        self.locked = locked;
        self
    }
}

/// Binds a source to a scene with a per-scene transform override.
///
/// Different scenes can position the same source differently.  When
/// `transform_override` is `None`, the source's default transform is
/// used.
#[derive(Debug, Clone)]
pub struct SceneSource {
    pub source_id: Uuid,
    pub scene_id: Uuid,
    pub transform_override: Option<Transform>,
    pub visible: bool,
    pub locked: bool,
    pub z_order: i32,
}

impl SceneSource {
    pub fn new(source_id: Uuid, scene_id: Uuid) -> Self {
        Self {
            source_id,
            scene_id,
            transform_override: None,
            visible: true,
            locked: false,
            z_order: 0,
        }
    }

    /// Returns the effective transform: the override if set, otherwise
    /// the fallback (typically the source's default transform).
    pub fn effective_transform<'a>(&'a self, fallback: &'a Transform) -> &'a Transform {
        match &self.transform_override {
            Some(t) => t,
            None => fallback,
        }
    }
}

/// Manages the collection of sources and their scene bindings.
#[derive(Debug, Clone, Default)]
pub struct SourceManager {
    sources: Vec<Source>,
    /// Scene → ordered list of scene-sources.
    bindings: Vec<SceneSource>,
}

impl SourceManager {
    pub fn new() -> Self {
        Self::default()
    }

    // ── source CRUD ──────────────────────────────────────────────

    /// Adds a source and returns its ID.
    pub fn add_source(&mut self, source: Source) -> Uuid {
        let id = source.id;
        self.sources.push(source);
        id
    }

    /// Removes a source and all its scene bindings.  Returns the
    /// removed source, or `None` if the ID was unknown.
    pub fn remove_source(&mut self, id: Uuid) -> Option<Source> {
        let idx = self.sources.iter().position(|s| s.id == id)?;
        let source = self.sources.remove(idx);
        self.bindings.retain(|b| b.source_id != id);
        Some(source)
    }

    pub fn get_source(&self, id: Uuid) -> Option<&Source> {
        self.sources.iter().find(|s| s.id == id)
    }

    pub fn get_source_mut(&mut self, id: Uuid) -> Option<&mut Source> {
        self.sources.iter_mut().find(|s| s.id == id)
    }

    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    pub fn sources_by_kind(&self, kind: &SourceKind) -> Vec<&Source> {
        self.sources.iter().filter(|s| s.kind == *kind).collect()
    }

    // ── scene binding ────────────────────────────────────────────

    /// Adds a source to a scene.  Uses the source's default transform
    /// unless `transform_override` is provided.
    pub fn bind_source(
        &mut self,
        source_id: Uuid,
        scene_id: Uuid,
        transform_override: Option<Transform>,
    ) -> Option<Uuid> {
        if self.sources.iter().any(|s| s.id == source_id) {
            let mut binding = SceneSource::new(source_id, scene_id);
            binding.transform_override = transform_override;
            // Assign z_order as one above the current max for this scene
            let max_z = self
                .bindings
                .iter()
                .filter(|b| b.scene_id == scene_id)
                .map(|b| b.z_order)
                .max()
                .unwrap_or(0);
            binding.z_order = max_z + 1;
            let id = source_id;
            self.bindings.push(binding);
            Some(id)
        } else {
            None
        }
    }

    /// Removes a source from a scene.  Returns true if a binding was
    /// removed.
    pub fn unbind_source(&mut self, source_id: Uuid, scene_id: Uuid) -> bool {
        let before = self.bindings.len();
        self.bindings
            .retain(|b| !(b.source_id == source_id && b.scene_id == scene_id));
        self.bindings.len() < before
    }

    /// Returns all scene-sources for a given scene, ordered by
    /// z_order (lowest first = back-to-front).
    pub fn scene_sources(&self, scene_id: Uuid) -> Vec<&SceneSource> {
        let mut list: Vec<_> = self
            .bindings
            .iter()
            .filter(|b| b.scene_id == scene_id)
            .collect();
        list.sort_by_key(|b| b.z_order);
        list
    }

    /// Returns the number of sources bound to a scene.
    pub fn source_count_in_scene(&self, scene_id: Uuid) -> usize {
        self.bindings
            .iter()
            .filter(|b| b.scene_id == scene_id)
            .count()
    }

    /// Updates the transform override for a specific scene-source
    /// binding.  Returns false if the binding doesn't exist.
    pub fn set_transform(&mut self, source_id: Uuid, scene_id: Uuid, transform: Transform) -> bool {
        if let Some(binding) = self
            .bindings
            .iter_mut()
            .find(|b| b.source_id == source_id && b.scene_id == scene_id)
        {
            binding.transform_override = Some(transform);
            true
        } else {
            false
        }
    }

    /// Moves a source up/down in the z-order within a scene.
    /// Returns the new z_order, or None if the binding doesn't exist.
    pub fn reorder_source(&mut self, source_id: Uuid, scene_id: Uuid, delta: i32) -> Option<i32> {
        let binding = self
            .bindings
            .iter_mut()
            .find(|b| b.source_id == source_id && b.scene_id == scene_id)?;
        binding.z_order += delta;
        Some(binding.z_order)
    }

    /// Removes all bindings for a given scene (e.g. when deleting a
    /// scene).
    pub fn clear_scene(&mut self, scene_id: Uuid) {
        self.bindings.retain(|b| b.scene_id != scene_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── SourceKind ───────────────────────────────────────────────

    #[test]
    fn source_kind_label_returns_correct_string() {
        assert_eq!(SourceKind::Image.label(), "Image");
        assert_eq!(SourceKind::Text.label(), "Text");
        assert_eq!(SourceKind::Webcam.label(), "Webcam");
        assert_eq!(SourceKind::Browser.label(), "Browser");
        assert_eq!(SourceKind::Media.label(), "Media");
        assert_eq!(SourceKind::Color.label(), "Color");
        assert_eq!(SourceKind::GameCapture.label(), "Game Capture");
        assert_eq!(SourceKind::ScreenCapture.label(), "Screen Capture");
        assert_eq!(SourceKind::Audio.label(), "Audio");
    }

    #[test]
    fn source_kind_all_variants_are_unique() {
        let kinds = [
            SourceKind::Image,
            SourceKind::Text,
            SourceKind::Webcam,
            SourceKind::Browser,
            SourceKind::Media,
            SourceKind::Color,
            SourceKind::GameCapture,
            SourceKind::ScreenCapture,
            SourceKind::Audio,
        ];
        let distinct_labels: Vec<&str> = kinds.iter().map(|k| k.label()).collect();
        assert_eq!(distinct_labels.len(), 9);
    }

    // ── Transform ────────────────────────────────────────────────

    #[test]
    fn transform_default_is_full_frame_no_rotation() {
        let t = Transform::default();
        assert_eq!(t.x, 0.0);
        assert_eq!(t.y, 0.0);
        assert_eq!(t.width, 1920.0);
        assert_eq!(t.height, 1080.0);
        assert_eq!(t.rotation, 0.0);
        assert_eq!(t.opacity, 1.0);
    }

    #[test]
    fn transform_new_sets_position_and_size() {
        let t = Transform::new(100.0, 200.0, 640.0, 480.0);
        assert_eq!(t.x, 100.0);
        assert_eq!(t.y, 200.0);
        assert_eq!(t.width, 640.0);
        assert_eq!(t.height, 480.0);
        assert_eq!(t.rotation, 0.0);
        assert_eq!(t.opacity, 1.0);
    }

    #[test]
    fn transform_is_default_true_for_default() {
        assert!(Transform::default().is_default());
    }

    #[test]
    fn transform_is_default_false_for_custom() {
        assert!(!Transform::new(10.0, 20.0, 100.0, 100.0).is_default());
    }

    #[test]
    fn transform_clamp_opacity() {
        let mut t = Transform::default();
        t.opacity = -0.5;
        t.clamp_opacity();
        assert_eq!(t.opacity, 0.0);

        t.opacity = 1.5;
        t.clamp_opacity();
        assert_eq!(t.opacity, 1.0);

        t.opacity = 0.7;
        t.clamp_opacity();
        assert_eq!(t.opacity, 0.7);
    }

    #[test]
    fn transform_normalize_rotation() {
        let mut t = Transform::default();
        t.rotation = 370.0;
        t.normalize_rotation();
        assert!((t.rotation - 10.0).abs() < f32::EPSILON);

        t.rotation = -10.0;
        t.normalize_rotation();
        assert!((t.rotation - 350.0).abs() < f32::EPSILON);

        t.rotation = 720.0;
        t.normalize_rotation();
        assert!((t.rotation - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn transform_clone_and_eq() {
        let a = Transform::new(10.0, 20.0, 640.0, 480.0);
        let b = a.clone();
        assert_eq!(a, b);
    }

    // ── Source ───────────────────────────────────────────────────

    #[test]
    fn source_new_creates_unique_id_and_name() {
        let s = Source::new("My Image".to_string(), SourceKind::Image);
        assert_eq!(s.name, "My Image");
        assert_eq!(s.kind, SourceKind::Image);
        assert_ne!(s.id, Uuid::nil());
        assert!(s.visible);
        assert!(!s.locked);
        assert_eq!(s.z_order, 0);
        assert_eq!(s.transform, Transform::default());
    }

    #[test]
    fn source_builder_methods() {
        let custom_transform = Transform::new(50.0, 50.0, 320.0, 240.0);
        let s = Source::new("Cam".to_string(), SourceKind::Webcam)
            .with_transform(custom_transform.clone())
            .with_z_order(5)
            .visible(false)
            .locked(true);
        assert_eq!(s.transform, custom_transform);
        assert_eq!(s.z_order, 5);
        assert!(!s.visible);
        assert!(s.locked);
    }

    #[test]
    fn source_two_instances_have_different_ids() {
        let a = Source::new("A".to_string(), SourceKind::Color);
        let b = Source::new("B".to_string(), SourceKind::Color);
        assert_ne!(a.id, b.id);
    }

    // ── SceneSource ──────────────────────────────────────────────

    #[test]
    fn scene_source_new_uses_defaults() {
        let src = Uuid::new_v4();
        let scene = Uuid::new_v4();
        let ss = SceneSource::new(src, scene);
        assert_eq!(ss.source_id, src);
        assert_eq!(ss.scene_id, scene);
        assert!(ss.transform_override.is_none());
        assert!(ss.visible);
        assert!(!ss.locked);
        assert_eq!(ss.z_order, 0);
    }

    #[test]
    fn scene_source_effective_transform_uses_fallback_when_no_override() {
        let ss = SceneSource::new(Uuid::new_v4(), Uuid::new_v4());
        let fallback = Transform::new(0.0, 0.0, 800.0, 600.0);
        assert_eq!(ss.effective_transform(&fallback), &fallback);
    }

    #[test]
    fn scene_source_effective_transform_uses_override_when_set() {
        let mut ss = SceneSource::new(Uuid::new_v4(), Uuid::new_v4());
        let override_t = Transform::new(10.0, 20.0, 320.0, 240.0);
        ss.transform_override = Some(override_t.clone());
        let fallback = Transform::default();
        assert_eq!(ss.effective_transform(&fallback), &override_t);
    }

    // ── SourceManager: source CRUD ──────────────────────────────

    #[test]
    fn manager_add_and_get_source() {
        let mut mgr = SourceManager::new();
        let s = Source::new("Test".to_string(), SourceKind::Text);
        let id = s.id;
        mgr.add_source(s);
        assert!(mgr.get_source(id).is_some());
        assert_eq!(mgr.sources().len(), 1);
    }

    #[test]
    fn manager_remove_source_removes_bindings_too() {
        let mut mgr = SourceManager::new();
        let sid = mgr.add_source(Source::new("S".to_string(), SourceKind::Color));
        let scene = Uuid::new_v4();
        mgr.bind_source(sid, scene, None);
        assert_eq!(mgr.source_count_in_scene(scene), 1);

        let removed = mgr.remove_source(sid);
        assert!(removed.is_some());
        assert_eq!(mgr.sources().len(), 0);
        assert_eq!(mgr.source_count_in_scene(scene), 0);
    }

    #[test]
    fn manager_remove_unknown_source_returns_none() {
        let mut mgr = SourceManager::new();
        assert!(mgr.remove_source(Uuid::new_v4()).is_none());
    }

    #[test]
    fn manager_sources_by_kind() {
        let mut mgr = SourceManager::new();
        mgr.add_source(Source::new("Img".to_string(), SourceKind::Image));
        mgr.add_source(Source::new("Txt".to_string(), SourceKind::Text));
        mgr.add_source(Source::new("Img2".to_string(), SourceKind::Image));

        let imgs = mgr.sources_by_kind(&SourceKind::Image);
        assert_eq!(imgs.len(), 2);
        let txts = mgr.sources_by_kind(&SourceKind::Text);
        assert_eq!(txts.len(), 1);
    }

    #[test]
    fn manager_get_source_mut() {
        let mut mgr = SourceManager::new();
        let id = mgr.add_source(Source::new("Old".to_string(), SourceKind::Text));
        if let Some(s) = mgr.get_source_mut(id) {
            s.name = "New".to_string();
        }
        assert_eq!(mgr.get_source(id).unwrap().name, "New");
    }

    // ── SourceManager: scene binding ─────────────────────────────

    #[test]
    fn manager_bind_and_scene_sources() {
        let mut mgr = SourceManager::new();
        let scene = Uuid::new_v4();
        let s1 = mgr.add_source(Source::new("A".to_string(), SourceKind::Image));
        let s2 = mgr.add_source(Source::new("B".to_string(), SourceKind::Text));

        mgr.bind_source(s1, scene, None);
        mgr.bind_source(s2, scene, None);

        let list = mgr.scene_sources(scene);
        assert_eq!(list.len(), 2);
        // z_order should be 1, 2 (incremented)
        assert_eq!(list[0].z_order, 1);
        assert_eq!(list[1].z_order, 2);
    }

    #[test]
    fn manager_bind_unknown_source_returns_none() {
        let mut mgr = SourceManager::new();
        let scene = Uuid::new_v4();
        assert!(mgr.bind_source(Uuid::new_v4(), scene, None).is_none());
    }

    #[test]
    fn manager_unbind_source() {
        let mut mgr = SourceManager::new();
        let scene = Uuid::new_v4();
        let sid = mgr.add_source(Source::new("X".to_string(), SourceKind::Media));
        mgr.bind_source(sid, scene, None);
        assert!(mgr.unbind_source(sid, scene));
        assert_eq!(mgr.source_count_in_scene(scene), 0);
        // Unbinding again returns false.
        assert!(!mgr.unbind_source(sid, scene));
    }

    #[test]
    fn manager_bind_same_source_to_multiple_scenes() {
        let mut mgr = SourceManager::new();
        let scene_a = Uuid::new_v4();
        let scene_b = Uuid::new_v4();
        let sid = mgr.add_source(Source::new("Shared".to_string(), SourceKind::Color));

        mgr.bind_source(sid, scene_a, None);
        mgr.bind_source(sid, scene_b, None);

        assert_eq!(mgr.source_count_in_scene(scene_a), 1);
        assert_eq!(mgr.source_count_in_scene(scene_b), 1);
    }

    #[test]
    fn manager_set_transform() {
        let mut mgr = SourceManager::new();
        let scene = Uuid::new_v4();
        let sid = mgr.add_source(Source::new("S".to_string(), SourceKind::Image));
        mgr.bind_source(sid, scene, None);

        let new_t = Transform::new(100.0, 200.0, 320.0, 240.0);
        assert!(mgr.set_transform(sid, scene, new_t.clone()));

        let list = mgr.scene_sources(scene);
        assert_eq!(list[0].transform_override, Some(new_t));
    }

    #[test]
    fn manager_set_transform_unknown_binding() {
        let mut mgr = SourceManager::new();
        assert!(!mgr.set_transform(Uuid::new_v4(), Uuid::new_v4(), Transform::default()));
    }

    #[test]
    fn manager_reorder_source() {
        let mut mgr = SourceManager::new();
        let scene = Uuid::new_v4();
        let sid = mgr.add_source(Source::new("S".to_string(), SourceKind::Text));
        mgr.bind_source(sid, scene, None);

        let z = mgr.scene_sources(scene)[0].z_order;
        let new_z = mgr.reorder_source(sid, scene, 10).unwrap();
        assert_eq!(new_z, z + 10);
    }

    #[test]
    fn manager_reorder_unknown_binding() {
        let mut mgr = SourceManager::new();
        assert!(mgr
            .reorder_source(Uuid::new_v4(), Uuid::new_v4(), 1)
            .is_none());
    }

    #[test]
    fn manager_clear_scene_removes_all_bindings() {
        let mut mgr = SourceManager::new();
        let scene = Uuid::new_v4();
        let s1 = mgr.add_source(Source::new("A".to_string(), SourceKind::Image));
        let s2 = mgr.add_source(Source::new("B".to_string(), SourceKind::Text));
        mgr.bind_source(s1, scene, None);
        mgr.bind_source(s2, scene, None);

        mgr.clear_scene(scene);
        assert_eq!(mgr.source_count_in_scene(scene), 0);
        // Sources themselves should still exist.
        assert_eq!(mgr.sources().len(), 2);
    }

    #[test]
    fn manager_scene_sources_sorted_by_z_order() {
        let mut mgr = SourceManager::new();
        let scene = Uuid::new_v4();
        let s1 = mgr.add_source(Source::new("Back".to_string(), SourceKind::Image));
        let s2 = mgr.add_source(Source::new("Mid".to_string(), SourceKind::Text));
        let s3 = mgr.add_source(Source::new("Front".to_string(), SourceKind::Color));

        mgr.bind_source(s1, scene, None);
        mgr.bind_source(s2, scene, None);
        mgr.bind_source(s3, scene, None);

        let list = mgr.scene_sources(scene);
        assert_eq!(list.len(), 3);
        assert!(list[0].z_order <= list[1].z_order);
        assert!(list[1].z_order <= list[2].z_order);
    }

    #[test]
    fn manager_source_id_stable_after_bind() {
        let mut mgr = SourceManager::new();
        let scene = Uuid::new_v4();
        let sid = mgr.add_source(Source::new("S".to_string(), SourceKind::Webcam));
        mgr.bind_source(sid, scene, Some(Transform::new(10.0, 20.0, 640.0, 480.0)));

        let list = mgr.scene_sources(scene);
        assert_eq!(list[0].source_id, sid);

        let src = mgr.get_source(sid).unwrap();
        assert_eq!(src.name, "S");
        assert_eq!(src.kind, SourceKind::Webcam);
    }

    #[test]
    fn manager_bind_with_transform_override() {
        let mut mgr = SourceManager::new();
        let scene = Uuid::new_v4();
        let sid = mgr.add_source(Source::new("S".to_string(), SourceKind::Image));
        let t = Transform::new(50.0, 60.0, 320.0, 240.0);
        mgr.bind_source(sid, scene, Some(t.clone()));

        let list = mgr.scene_sources(scene);
        let ss = list[0];
        let src = mgr.get_source(sid).unwrap();
        // effective_transform should return the override, not the source default
        assert_eq!(ss.effective_transform(&src.transform), &t);
    }

    // ── SourceManager: default value ─────────────────────────────

    #[test]
    fn manager_default_is_empty() {
        let mgr = SourceManager::new();
        assert_eq!(mgr.sources().len(), 0);
        assert_eq!(mgr.scene_sources(Uuid::new_v4()).len(), 0);
    }

    #[test]
    fn manager_clone_preserves_data() {
        let mut mgr = SourceManager::new();
        let scene = Uuid::new_v4();
        let sid = mgr.add_source(Source::new("X".to_string(), SourceKind::Media));
        mgr.bind_source(sid, scene, None);

        let mgr2 = mgr.clone();
        assert_eq!(mgr2.sources().len(), 1);
        assert_eq!(mgr2.source_count_in_scene(scene), 1);
    }
}
