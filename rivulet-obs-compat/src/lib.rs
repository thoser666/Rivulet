#![warn(missing_docs)]
//! OBS plugin compatibility layer.
//!
//! The surface the plugin manager in `rivulet-core` needs to drive real OBS
//! plugins. Nothing loads a plugin yet: the types below are the placeholders
//! that surface is built on.

use std::collections::HashMap;

pub mod plugin;
pub mod types;

// Placeholder structs so that the code compiles
#[derive(Debug)]
/// Configuration for a single OBS plugin.
pub struct PluginConfig;

#[derive(Debug)]
/// Handle to an OBS plugin that was loaded from its shared library.
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
/// Placeholder for the plugin manager another crate may expect.
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
