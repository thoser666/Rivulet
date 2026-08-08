#[derive(Debug, Clone)]
pub struct Config {
    pub output_dir: std::path::PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            output_dir: std::path::PathBuf::from("./output"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_points_to_output_dir() {
        assert_eq!(
            Config::default().output_dir,
            std::path::PathBuf::from("./output")
        );
    }
}
