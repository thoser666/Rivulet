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
}
