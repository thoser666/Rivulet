//! Vulkan layer activation utilities.
//!
//! Provides functions to locate and activate the `VK_LAYER_RIVULET_capture`
//! layer for zero-overhead Vulkan game capture (G3).

use std::path::{Path, PathBuf};

/// Layer name as registered in the Vulkan loader.
pub const LAYER_NAME: &str = "VK_LAYER_RIVULET_capture";

/// JSON manifest filename.
pub const MANIFEST_FILENAME: &str = "VkLayer_rivulet_capture.json";

/// Layer DLL filename (Windows).
#[cfg(target_os = "windows")]
pub const LAYER_DLL_FILENAME: &str = "rivulet-vulkan-layer.dll";

/// Layer shared library filename (Linux).
#[cfg(target_os = "linux")]
pub const LAYER_DLL_FILENAME: &str = "librivulet_vulkan_layer.so";

/// Layer shared library filename (macOS). The layer is not shipped on macOS,
/// but the constant keeps `VulkanLayerConfig::find` compiling everywhere; it
/// simply never finds the files and reports the layer as unavailable.
#[cfg(target_os = "macos")]
pub const LAYER_DLL_FILENAME: &str = "librivulet_vulkan_layer.dylib";

/// Configuration for activating the Vulkan capture layer.
pub struct VulkanLayerConfig {
    /// Directory containing the layer DLL and JSON manifest.
    pub layer_dir: PathBuf,
}

impl VulkanLayerConfig {
    /// Create a config by searching for the layer files relative to the
    /// given base directory (typically the executable's directory).
    pub fn find(base_dir: &Path) -> Option<Self> {
        let manifest = base_dir.join(MANIFEST_FILENAME);
        let dll = base_dir.join(LAYER_DLL_FILENAME);
        if manifest.exists() && dll.exists() {
            Some(Self {
                layer_dir: base_dir.to_path_buf(),
            })
        } else {
            None
        }
    }

    /// Create a config for a specific layer directory.
    pub fn new(layer_dir: PathBuf) -> Self {
        Self { layer_dir }
    }

    /// Returns the `VK_LAYER_PATH` environment variable value.
    pub fn vk_layer_path(&self) -> &Path {
        &self.layer_dir
    }

    /// Returns the `VK_INSTANCE_LAYERS` value to enable our layer.
    pub fn vk_instance_layers() -> &'static str {
        LAYER_NAME
    }

    /// Apply layer environment variables to a `std::process::Command`.
    ///
    /// This sets `VK_LAYER_PATH` and `VK_INSTANCE_LAYERS` so the child
    /// process loads the Rivulet capture layer on Vulkan init.
    pub fn apply_to_command(&self, cmd: &mut std::process::Command) {
        cmd.env("VK_LAYER_PATH", &self.layer_dir);

        let existing = std::env::var("VK_INSTANCE_LAYERS").unwrap_or_default();
        let layers = if existing.is_empty() {
            LAYER_NAME.to_string()
        } else {
            format!("{existing}:{LAYER_NAME}")
        };
        cmd.env("VK_INSTANCE_LAYERS", &layers);

        eprintln!(
            "[Rivulet] Vulkan layer enabled: VK_LAYER_PATH={}, VK_INSTANCE_LAYERS={}",
            self.layer_dir.display(),
            layers,
        );
    }

    /// Apply layer environment variables to the current process.
    ///
    /// # Safety
    /// Must be called before any Vulkan initialization in the current
    /// process. Not safe in a multi-threaded context where other threads
    /// may be calling Vulkan functions.
    pub unsafe fn apply_to_current_process(&self) {
        std::env::set_var("VK_LAYER_PATH", &self.layer_dir);
        let existing = std::env::var("VK_INSTANCE_LAYERS").unwrap_or_default();
        let layers = if existing.is_empty() {
            LAYER_NAME.to_string()
        } else {
            format!("{existing}:{LAYER_NAME}")
        };
        std::env::set_var("VK_INSTANCE_LAYERS", &layers);

        eprintln!(
            "[Rivulet] Vulkan layer enabled for current process: VK_LAYER_PATH={}",
            self.layer_dir.display(),
        );
    }
}

/// Try to locate the layer files by searching multiple standard locations.
///
/// Search order:
/// 1. Directory of the current executable
/// 2. `target/debug/` relative to `CARGO_MANIFEST_DIR` (dev builds)
/// 3. `target/release/` relative to `CARGO_MANIFEST_DIR` (release builds)
pub fn find_layer_config() -> Option<VulkanLayerConfig> {
    // 1. Next to the current executable
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
    {
        if let Some(config) = VulkanLayerConfig::find(&exe_dir) {
            return Some(config);
        }
    }

    // 2. target/debug/ relative to workspace root
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let workspace = PathBuf::from(&manifest_dir)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from(&manifest_dir));

        for profile in &["debug", "release"] {
            let target_dir = workspace.join("target").join(profile);
            if let Some(config) = VulkanLayerConfig::find(&target_dir) {
                return Some(config);
            }
        }

        // Also check inside rivulet-vulkan-layer/target/{debug,release}
        let layer_crate = workspace.join("rivulet-vulkan-layer");
        for profile in &["debug", "release"] {
            let target_dir = layer_crate.join("target").join(profile);
            if let Some(config) = VulkanLayerConfig::find(&target_dir) {
                return Some(config);
            }
        }
    }

    None
}

/// Check if the Vulkan capture layer is available on this system.
pub fn is_layer_available() -> bool {
    find_layer_config().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layer_constants_are_correct() {
        assert_eq!(LAYER_NAME, "VK_LAYER_RIVULET_capture");
        assert_eq!(MANIFEST_FILENAME, "VkLayer_rivulet_capture.json");
    }

    #[test]
    fn find_requires_both_files() {
        let dir = std::env::temp_dir();
        let result = VulkanLayerConfig::find(&dir);
        let _ = result;
    }

    #[test]
    fn apply_to_command_sets_env_vars() {
        let dir = std::env::temp_dir();
        let config = VulkanLayerConfig::new(dir.clone());
        let mut cmd = std::process::Command::new("echo");
        config.apply_to_command(&mut cmd);
    }

    #[test]
    fn vk_instance_layers_returns_layer_name() {
        assert_eq!(
            VulkanLayerConfig::vk_instance_layers(),
            "VK_LAYER_RIVULET_capture"
        );
    }

    #[test]
    fn new_config_stores_layer_dir() {
        let dir = PathBuf::from("/some/path");
        let config = VulkanLayerConfig::new(dir.clone());
        assert_eq!(config.vk_layer_path(), dir.as_path());
    }

    #[test]
    fn find_layer_config_does_not_panic() {
        let _ = find_layer_config();
    }

    #[test]
    fn is_layer_available_does_not_panic() {
        let _ = is_layer_available();
    }
}
