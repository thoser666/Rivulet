//! VST 3.x plugin configuration contract and bundle discovery.
//!
//! Most modern audio plugins ship exclusively as VST3, so Rivulet needs a
//! stable configuration model before any hosting can exist. This module
//! provides the *contract* for that: a validated [`VstPlugin`] (label + path
//! to a `.vst3` bundle), an ordered [`VstChain`] applied to an input track, a
//! deterministic discovery of installed bundles from the platform-standard
//! VST3 search directories, and an availability probe — all testable without
//! a VST3 runtime or a plugin binary.
//!
//! **Scope note:** loading a real VST3 module into the audio graph (hosting a
//! plugin binary, instantiating its processor, routing audio through it) is a
//! separate integration follow-up that needs a VST3 host runtime; this module
//! deliberately stops at discovery + a validated, secret-free configuration so
//! the surrounding pipeline and GUI can already target the model.

use std::path::{Path, PathBuf};

/// A single VST3 plugin reference: a human label and the path of the `.vst3`
/// bundle it was discovered at (a directory on macOS/Windows, a directory or
/// `.so` file on Linux).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VstPlugin {
    /// Human-readable display name (derived from the bundle filename).
    pub name: String,
    /// Absolute path to the `.vst3` bundle.
    pub path: PathBuf,
}

impl VstPlugin {
    /// Maximum accepted display-name length (a plugin cannot be that long).
    pub const MAX_NAME_LEN: usize = 120;

    /// Derive a display name from a bundle path (e.g. `MyPlugin.vst3` →
    /// `MyPlugin`).
    pub fn name_from_path(path: &Path) -> String {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.trim_end_matches(".vst3").to_string())
            .unwrap_or_else(|| "Unnamed plugin".to_string())
    }

    /// Validate the plugin reference without touching a VST3 runtime.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.name.trim().is_empty() {
            return Err("VST plugin name must not be empty");
        }
        if self.name.trim().chars().count() > Self::MAX_NAME_LEN {
            return Err("VST plugin name is too long");
        }
        if self.path.as_os_str().is_empty() {
            return Err("VST plugin path must not be empty");
        }
        let looks_like_bundle = self
            .path
            .to_str()
            .map(|s| s.ends_with(".vst3"))
            .unwrap_or(false);
        if !looks_like_bundle {
            return Err("VST plugin path must point to a .vst3 bundle");
        }
        Ok(())
    }

    /// Whether the bundle currently exists on disk (a lightweight availability
    /// probe that needs no VST3 runtime).
    pub fn bundle_available(&self) -> bool {
        self.path.is_dir() || self.path.is_file()
    }
}

/// An ordered chain of VST3 plugins applied to one input track.
///
/// Plugin order is preserved as configured; an empty chain is valid and means
/// \"no VST processing\".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VstChain {
    pub plugins: Vec<VstPlugin>,
}

impl VstChain {
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Validate every plugin in the chain; returns the first error.
    pub fn validate(&self) -> Result<(), &'static str> {
        self.plugins.iter().try_for_each(VstPlugin::validate)
    }

    /// Display labels of the configured plugins, in order.
    pub fn labels(&self) -> Vec<&str> {
        self.plugins.iter().map(|p| p.name.as_str()).collect()
    }
}

/// The platform-standard VST3 search directories, in discovery order.
///
/// Mirrors the well-known layout:
/// - Windows: `C:\Program Files\Common Files\VST3`, `C:\Program Files\VST3`,
///   `%LOCALAPPDATA%\Programs\Common\VST3`, `%APPDATA%\VST3`
/// - macOS: `/Library/Audio/Plug-Ins/VST3`, `~/Library/Audio/Plug-Ins/VST3`
/// - Linux: `/usr/lib/vst3`, `/usr/local/lib/vst3`, `~/.vst3`
pub fn vst3_search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "windows")]
    {
        if let Some(pf) = std::env::var_os("ProgramFiles") {
            dirs.push(PathBuf::from(&pf).join("Common Files").join("VST3"));
            dirs.push(PathBuf::from(pf).join("VST3"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(
                PathBuf::from(local)
                    .join("Programs")
                    .join("Common")
                    .join("VST3"),
            );
        }
        if let Some(appdata) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("VST3"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(
                PathBuf::from(home)
                    .join("Library")
                    .join("Audio")
                    .join("Plug-Ins")
                    .join("VST3"),
            );
        }
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        dirs.push(PathBuf::from("/usr/lib/vst3"));
        dirs.push(PathBuf::from("/usr/local/lib/vst3"));
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join(".vst3"));
        }
    }
    dirs
}

/// Discover installed VST3 bundles inside the given directories (pure variant,
/// used by tests). Non-existing directories are ignored; the returned paths
/// are sorted for determinism.
pub fn discover_in(dirs: &[PathBuf]) -> Vec<VstPlugin> {
    let mut plugins = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_bundle = path.to_str().map(|s| s.ends_with(".vst3")).unwrap_or(false);
            if is_bundle {
                let name = VstPlugin::name_from_path(&path);
                plugins.push(VstPlugin { name, path });
            }
        }
    }
    plugins.sort_by(|a, b| a.name.cmp(&b.name));
    plugins.dedup_by(|a, b| a.path == b.path);
    plugins
}

/// Discover installed VST3 bundles from the platform-standard search
/// directories.
pub fn discover_vst3_plugins() -> Vec<VstPlugin> {
    discover_in(&vst3_search_dirs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_bundle(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(format!("{name}.vst3"));
        std::fs::create_dir_all(&path).expect("create fake bundle dir");
        path
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rivulet_vst3_test_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn plugin_validation_rejects_empty_and_non_bundle_paths() {
        let p = VstPlugin {
            name: "EQ".into(),
            path: PathBuf::new(),
        };
        assert!(p.validate().is_err());

        let p = VstPlugin {
            name: "EQ".into(),
            path: PathBuf::from("/tmp/eq.so"),
        };
        assert!(p.validate().is_err(), "non-.vst3 path must be rejected");

        let p = VstPlugin {
            name: "EQ".into(),
            path: PathBuf::from("/tmp/EQ.vst3"),
        };
        assert!(p.validate().is_ok());
    }

    #[test]
    fn name_is_derived_from_bundle_stem() {
        let name = VstPlugin::name_from_path(Path::new("/usr/lib/vst3/FabFilter Pro-Q 3.vst3"));
        assert_eq!(name, "FabFilter Pro-Q 3");
        let name = VstPlugin::name_from_path(Path::new("Other.vst3"));
        assert_eq!(name, "Other");
    }

    #[test]
    fn chain_is_empty_by_default_and_validates_each_plugin() {
        let chain = VstChain::default();
        assert!(chain.is_empty());
        assert!(chain.validate().is_ok());
        let chain = VstChain {
            plugins: vec![
                VstPlugin {
                    name: "EQ".into(),
                    path: PathBuf::from("/x/EQ.vst3"),
                },
                VstPlugin {
                    name: "Comp".into(),
                    path: PathBuf::from("/x/Comp.vst3"),
                },
            ],
        };
        assert!(chain.validate().is_ok());
        assert_eq!(chain.labels(), vec!["EQ", "Comp"]);
        let broken = VstChain {
            plugins: vec![VstPlugin {
                name: "".into(),
                path: PathBuf::from("/x/EQ.vst3"),
            }],
        };
        assert!(broken.validate().is_err());
    }

    #[test]
    fn discovery_finds_bundles_in_given_dirs_and_is_deterministic() {
        let dir = temp_dir("discover");
        fake_bundle(&dir, "Alpha");
        fake_bundle(&dir, "Beta");
        let plugins = discover_in(std::slice::from_ref(&dir));
        let names: Vec<_> = plugins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["Alpha", "Beta"], "sorted by name");
        assert!(plugins.iter().all(|p| p.validate().is_ok()));
        assert!(plugins.iter().all(|p| p.bundle_available()));
        // Non-existent dirs are ignored.
        assert!(discover_in(&[PathBuf::from("/nonexistent/vst3")]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn discovery_skips_non_bundle_entries() {
        let dir = temp_dir("mixed");
        fake_bundle(&dir, "Real");
        std::fs::write(dir.join("README.txt"), "ignore").expect("write file");
        std::fs::create_dir_all(dir.join("data")).expect("create dir");
        let plugins = discover_in(std::slice::from_ref(&dir));
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "Real");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
