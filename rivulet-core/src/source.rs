use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Source {
    pub id: Uuid,
    pub name: String,
}

impl Source {
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
    fn source_new_creates_unique_id_and_name() {
        let source = Source::new("Kamera".to_string());
        assert_eq!(source.name, "Kamera");
        assert_ne!(source.id, Uuid::nil());
    }
}
