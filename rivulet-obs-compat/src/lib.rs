use std::collections::HashMap;

pub mod plugin;
pub mod types;

// Placeholder structs so that the code compiles
#[derive(Debug)]
pub struct PluginConfig;

#[derive(Debug)]
pub struct LoadedPlugin;

/// Plugin Manager - handles OBS plugin loading
#[derive(Debug, Default)] // `Default` implemented and `Debug` added for good practice
pub struct PluginSystem {
    #[allow(dead_code)] // Allow this field to remain unused
    plugins: HashMap<String, LoadedPlugin>,
    #[allow(dead_code)] // Allow this field to remain unused
    configs: HashMap<String, PluginConfig>,
}

impl PluginSystem {
    /// Creates a new, empty PluginSystem.
    pub fn new() -> Self {
        // We can use the `Default` implementation here
        Self::default()
    }

    // Plugin loading functions etc. will be added here later
}

// Placeholder for PluginManager, in case it is used elsewhere
pub struct PluginManager;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_system_starts_empty() {
        let system = PluginSystem::new();
        assert_eq!(system.plugins.len(), 0);
        assert_eq!(system.configs.len(), 0);
    }
}
