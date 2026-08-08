use std::collections::HashMap;

pub mod plugin;
pub mod types;

// Placeholder-Strukturen, damit der Code kompiliert
#[derive(Debug)]
pub struct PluginConfig;

#[derive(Debug)]
pub struct LoadedPlugin;

/// Plugin Manager - handles OBS plugin loading
#[derive(Debug, Default)] // `Default` implementiert und `Debug` für gute Praxis hinzugefügt
pub struct PluginSystem {
    #[allow(dead_code)] // Erlaubt, dass dieses Feld unbenutzt ist
    plugins: HashMap<String, LoadedPlugin>,
    #[allow(dead_code)] // Erlaubt, dass dieses Feld unbenutzt ist
    configs: HashMap<String, PluginConfig>,
}

impl PluginSystem {
    /// Creates a new, empty PluginSystem.
    pub fn new() -> Self {
        // Jetzt können wir die `Default`-Implementierung verwenden
        Self::default()
    }

    // Hier kommen später die Funktionen zum Laden von Plugins etc. hin
}

// Placeholder für PluginManager, falls er woanders verwendet wird
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
