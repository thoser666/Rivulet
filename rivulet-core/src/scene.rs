use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Scene {
    pub id: Uuid,
    pub name: String,
}

impl Scene {
    pub fn new(name: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
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
    }
}
