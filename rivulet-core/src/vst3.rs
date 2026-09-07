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

// ── Windows COM host skeleton (Z96-2) ──────────────────────────────────

/// The lifecycle stages of loading a VST3 plugin through the Windows COM
/// host interface. Each stage can fail independently, producing a
/// non-fatal [`SkipReason`]. The stages are deliberately separated so
/// macOS/Linux hosts (dlopen/dylib) can mirror the same contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostStage {
    /// Resolve the `.vst3` bundle path to a valid module file.
    BundleResolve,
    /// Obtain the VST3 module factory from the loaded DLL/COM object.
    FactoryObtain,
    /// Instantiate an audio processor from the factory.
    ProcessorCreate,
    /// Initialize the processor (set bus configuration, sample rate).
    ProcessorInit,
}

impl HostStage {
    pub fn description(&self) -> &str {
        match self {
            Self::BundleResolve => "resolving bundle",
            Self::FactoryObtain => "obtaining factory",
            Self::ProcessorCreate => "creating processor",
            Self::ProcessorInit => "initializing processor",
        }
    }
}

/// A failed stage with context for logging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageFailure {
    pub stage: HostStage,
    pub reason: SkipReason,
}

impl StageFailure {
    pub fn new(stage: HostStage, reason: SkipReason) -> Self {
        Self { stage, reason }
    }

    pub fn description(&self) -> String {
        format!(
            "{}: {}",
            self.stage.description(),
            self.reason.description()
        )
    }
}

/// Windows COM-based VST3 host skeleton.
///
/// Implements the VST3 host lifecycle: bundle → DLL → factory → processor.
/// On non-Windows platforms this is a stub that always returns `Skipped`;
/// a real macOS/Linux host (dlopen/dylib) will follow the same contract.
///
/// The skeleton separates the lifecycle into distinct stages so each
/// failure path is independently testable and loggable.
pub struct WindowsVstHost {
    /// Base directories to search for `.vst3` bundles when a plugin path
    /// is relative. Empty means only absolute paths are resolved.
    search_dirs: Vec<PathBuf>,
}

impl WindowsVstHost {
    /// Create a host with the given search directories.
    pub fn new(search_dirs: Vec<PathBuf>) -> Self {
        Self { search_dirs }
    }

    /// Create a host using the platform-standard VST3 search directories.
    pub fn with_default_dirs() -> Self {
        Self::new(vst3_search_dirs())
    }

    /// Resolve a plugin path to an absolute bundle path. If the path is
    /// already absolute and exists, it is returned as-is. If relative, the
    /// search directories are tried in order.
    pub fn resolve_bundle_path(&self, plugin: &VstPlugin) -> Result<PathBuf, SkipReason> {
        if plugin.path.is_absolute() {
            if plugin.bundle_available() {
                return Ok(plugin.path.clone());
            }
            return Err(SkipReason::BundleNotFound);
        }
        // Relative path — try search dirs.
        for dir in &self.search_dirs {
            let candidate = dir.join(&plugin.path);
            if candidate.is_dir() || candidate.is_file() {
                return Ok(candidate);
            }
        }
        Err(SkipReason::BundleNotFound)
    }

    /// Stage 1: Validate the bundle exists and can be read.
    /// Returns the resolved absolute path or a `BundleNotFound`/`BundleInvalid` skip.
    fn stage_bundle_resolve(&self, plugin: &VstPlugin) -> Result<PathBuf, StageFailure> {
        self.resolve_bundle_path(plugin)
            .map_err(|reason| StageFailure::new(HostStage::BundleResolve, reason))
    }

    /// Stage 2: Obtain the VST3 factory from the bundle.
    ///
    /// **Windows implementation:** loads the `.vst3` DLL via `LoadLibraryW`,
    /// calls `GetFactory` to obtain the `IPluginFactory` COM interface.
    ///
    /// **Non-Windows stub:** always returns `NoFactory`.
    #[cfg(target_os = "windows")]
    fn stage_factory_obtain(&self, bundle_path: &Path) -> Result<u64, StageFailure> {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;

        // Find the DLL/CFX inside the bundle.
        // VST3 bundles on Windows contain a Contents/x86_64/<name>.vst3 file.
        let dll_path = self
            .find_vst3_dll(bundle_path)
            .map_err(|reason| StageFailure::new(HostStage::FactoryObtain, reason))?;

        // SAFETY: LoadLibraryW is the standard Windows API for loading DLLs.
        // The returned handle is stored as a u64 for the host boundary;
        // a full implementation would wrap it in a Drop type that calls
        // FreeLibrary.
        let wide: Vec<u16> = OsStr::new(&dll_path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let handle =
            unsafe { windows_sys::Win32::System::LibraryLoader::LoadLibraryW(wide.as_ptr()) };
        if handle.is_null() {
            return Err(StageFailure::new(
                HostStage::FactoryObtain,
                SkipReason::HostError(format!("LoadLibraryW failed for {}", dll_path.display())),
            ));
        }
        Ok(handle as u64)
    }

    #[cfg(not(target_os = "windows"))]
    fn stage_factory_obtain(&self, _bundle_path: &Path) -> Result<u64, StageFailure> {
        Err(StageFailure::new(
            HostStage::FactoryObtain,
            SkipReason::HostError("COM hosting is only available on Windows".into()),
        ))
    }

    /// Find the `.vst3` DLL inside a bundle directory.
    /// On Windows, VST3 bundles have the layout:
    ///   `<name>.vst3/Contents/x86_64/<name>.vst3`
    #[cfg(target_os = "windows")]
    fn find_vst3_dll(&self, bundle_path: &Path) -> Result<PathBuf, SkipReason> {
        let contents = bundle_path.join("Contents");
        if !contents.is_dir() {
            return Err(SkipReason::BundleInvalid);
        }
        // Try x86_64 first, then fall back to any architecture dir.
        for arch in &["x86_64", "x86", "aarch64"] {
            let arch_dir = contents.join(arch);
            if arch_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&arch_dir) {
                    for entry in entries.flatten() {
                        let p = entry.path();
                        if p.to_str().map(|s| s.ends_with(".vst3")).unwrap_or(false) {
                            return Ok(p);
                        }
                    }
                }
            }
        }
        Err(SkipReason::BundleInvalid)
    }

    /// Stage 3: Create a processor instance from the factory.
    ///
    /// In a full implementation this would call `IPluginFactory::createInstance`
    /// with the audio processor class ID. The skeleton returns a dummy handle.
    fn stage_processor_create(
        &self,
        dll_handle: u64,
        _plugin: &VstPlugin,
    ) -> Result<HostHandle, StageFailure> {
        if dll_handle == 0 {
            return Err(StageFailure::new(
                HostStage::ProcessorCreate,
                SkipReason::NoFactory,
            ));
        }
        // Skeleton: return an opaque handle. A real implementation would
        // call IPluginFactory::createInstance here.
        Ok(HostHandle::new(dll_handle))
    }
}

impl VstHost for WindowsVstHost {
    fn load_plugin(&self, plugin: &VstPlugin) -> HostLoadResult {
        // Stage 1: Bundle resolve.
        let bundle_path = match self.stage_bundle_resolve(plugin) {
            Ok(p) => p,
            Err(fail) => {
                tracing::warn!(
                    "VST3 load skipped at {}: {}",
                    fail.stage.description(),
                    fail.reason.description()
                );
                return HostLoadResult::Skipped {
                    plugin: plugin.clone(),
                    reason: fail.reason,
                };
            }
        };

        // Stage 2: Factory obtain.
        let dll_handle = match self.stage_factory_obtain(&bundle_path) {
            Ok(h) => h,
            Err(fail) => {
                tracing::warn!(
                    "VST3 load skipped at {}: {}",
                    fail.stage.description(),
                    fail.reason.description()
                );
                return HostLoadResult::Skipped {
                    plugin: plugin.clone(),
                    reason: fail.reason,
                };
            }
        };

        // Stage 3: Processor create.
        let handle = match self.stage_processor_create(dll_handle, plugin) {
            Ok(h) => h,
            Err(fail) => {
                tracing::warn!(
                    "VST3 load skipped at {}: {}",
                    fail.stage.description(),
                    fail.reason.description()
                );
                return HostLoadResult::Skipped {
                    plugin: plugin.clone(),
                    reason: fail.reason,
                };
            }
        };

        HostLoadResult::Loaded {
            plugin: plugin.clone(),
            handle,
        }
    }
}

// ── Host runtime boundary (Z96-1) ──────────────────────────────────────

/// Outcome of attempting to load a single VST3 plugin through the host
/// runtime boundary. A missing or broken bundle produces a non-fatal
/// [`HostLoadResult::Skipped`] — the same pattern as `SkippedFilter` — so
/// the chain remains valid and the pipeline continues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostLoadResult {
    /// The plugin was loaded successfully. The opaque handle identifies the
    /// loaded instance for later processing.
    Loaded {
        /// The plugin that was loaded (for display / diagnostics).
        plugin: VstPlugin,
        /// Opaque host-side handle. The concrete type is owned by the
        /// platform-specific host implementation; the boundary only
        /// guarantees it is `Send` so it can live on a dedicated audio
        /// thread.
        handle: HostHandle,
    },
    /// The plugin was skipped because its bundle is missing, corrupt, or
    /// otherwise unusable. The pipeline continues without this plugin.
    Skipped {
        plugin: VstPlugin,
        reason: SkipReason,
    },
}

impl HostLoadResult {
    /// Whether this result represents a successfully loaded plugin.
    pub fn is_loaded(&self) -> bool {
        matches!(self, HostLoadResult::Loaded { .. })
    }

    /// Whether this result represents a skipped (non-fatal) plugin.
    pub fn is_skipped(&self) -> bool {
        matches!(self, HostLoadResult::Skipped { .. })
    }

    /// The plugin reference from this result.
    pub fn plugin(&self) -> &VstPlugin {
        match self {
            HostLoadResult::Loaded { plugin, .. } | HostLoadResult::Skipped { plugin, .. } => {
                plugin
            }
        }
    }

    /// Human-readable description of what happened.
    pub fn status_description(&self) -> String {
        match self {
            HostLoadResult::Loaded { plugin, .. } => {
                format!("{} loaded successfully", plugin.name)
            }
            HostLoadResult::Skipped { plugin, reason } => {
                format!("{} skipped: {}", plugin.name, reason.description())
            }
        }
    }
}

/// Reason a plugin was skipped instead of loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// The `.vst3` bundle directory/file does not exist on disk.
    BundleNotFound,
    /// The bundle exists but cannot be read or is not a valid VST3 bundle.
    BundleInvalid,
    /// The bundle exists but the host could not obtain a valid factory.
    NoFactory,
    /// The factory exists but no audio processor could be obtained.
    NoProcessor,
    /// A platform-specific error (e.g. COM failure, dlopen error).
    HostError(String),
}

impl SkipReason {
    pub fn description(&self) -> &str {
        match self {
            Self::BundleNotFound => "bundle not found on disk",
            Self::BundleInvalid => "bundle exists but is not a valid VST3",
            Self::NoFactory => "no valid VST3 factory in bundle",
            Self::NoProcessor => "no audio processor in factory",
            Self::HostError(msg) => msg,
        }
    }
}

/// Opaque handle for a loaded VST3 plugin instance. The concrete type is
/// defined by the platform-specific host implementation; the boundary
/// only guarantees `Send` so it can be moved to an audio thread.
#[derive(Debug, Clone)]
pub struct HostHandle {
    /// Opaque identifier — the real handle lives in the host runtime.
    id: u64,
}

impl HostHandle {
    pub fn new(id: u64) -> Self {
        Self { id }
    }

    pub fn id(&self) -> u64 {
        self.id
    }
}

impl PartialEq for HostHandle {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for HostHandle {}

/// The host runtime boundary contract: load a VST3 plugin and return a
/// [`HostLoadResult`]. Implementations are platform-specific (COM on
/// Windows, dlopen on macOS/Linux) but must never panic or abort —
/// errors produce `Skipped` results.
///
/// A mock implementation is provided for testing without a real plugin
/// binary.
pub trait VstHost {
    /// Attempt to load a VST3 plugin. Returns [`HostLoadResult::Loaded`]
    /// on success or [`HostLoadResult::Skipped`] with a reason on failure.
    fn load_plugin(&self, plugin: &VstPlugin) -> HostLoadResult;
}

/// Summary of loading all plugins in a [`VstChain`] through a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainLoadResults {
    /// One result per plugin in the chain, in order.
    pub results: Vec<HostLoadResult>,
}

impl ChainLoadResults {
    /// Number of successfully loaded plugins.
    pub fn loaded_count(&self) -> usize {
        self.results.iter().filter(|r| r.is_loaded()).count()
    }

    /// Number of skipped (non-fatal) plugins.
    pub fn skipped_count(&self) -> usize {
        self.results.iter().filter(|r| r.is_skipped()).count()
    }

    /// Whether every plugin in the chain was loaded (none skipped).
    pub fn all_loaded(&self) -> bool {
        self.results.iter().all(|r| r.is_loaded())
    }

    /// Skipped plugins, for logging / diagnostics.
    pub fn skipped_plugins(&self) -> Vec<&VstPlugin> {
        self.results
            .iter()
            .filter(|r| r.is_skipped())
            .map(|r| r.plugin())
            .collect()
    }
}

/// Load every plugin in a [`VstChain`] through a [`VstHost`] and collect
/// results. Skipped plugins are logged but never cause a failure — the
/// chain always produces a [`ChainLoadResults`], even when every plugin
/// is skipped.
pub fn load_chain(host: &dyn VstHost, chain: &VstChain) -> ChainLoadResults {
    let results: Vec<HostLoadResult> = chain
        .plugins
        .iter()
        .map(|plugin| host.load_plugin(plugin))
        .collect();
    ChainLoadResults { results }
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

    // ── Host boundary tests (Z96-1) ──────────────────────────────────

    /// A mock host that always succeeds — used to test the happy path.
    struct MockSuccessHost;
    impl VstHost for MockSuccessHost {
        fn load_plugin(&self, plugin: &VstPlugin) -> HostLoadResult {
            HostLoadResult::Loaded {
                plugin: plugin.clone(),
                handle: HostHandle::new(1),
            }
        }
    }

    /// A mock host that always skips — simulates missing/broken bundles.
    struct MockSkipHost;
    impl VstHost for MockSkipHost {
        fn load_plugin(&self, plugin: &VstPlugin) -> HostLoadResult {
            HostLoadResult::Skipped {
                plugin: plugin.clone(),
                reason: SkipReason::BundleNotFound,
            }
        }
    }

    fn test_plugin(name: &str) -> VstPlugin {
        VstPlugin {
            name: name.into(),
            path: PathBuf::from(format!("/tmp/{name}.vst3")),
        }
    }

    #[test]
    fn host_load_result_is_loaded_or_skipped() {
        let loaded = HostLoadResult::Loaded {
            plugin: test_plugin("EQ"),
            handle: HostHandle::new(42),
        };
        assert!(loaded.is_loaded());
        assert!(!loaded.is_skipped());
        assert_eq!(loaded.plugin().name, "EQ");

        let skipped = HostLoadResult::Skipped {
            plugin: test_plugin("Comp"),
            reason: SkipReason::BundleNotFound,
        };
        assert!(!skipped.is_loaded());
        assert!(skipped.is_skipped());
        assert_eq!(skipped.plugin().name, "Comp");
    }

    #[test]
    fn skip_reason_descriptions_are_human_readable() {
        assert!(!SkipReason::BundleNotFound.description().is_empty());
        assert!(!SkipReason::BundleInvalid.description().is_empty());
        assert!(!SkipReason::NoFactory.description().is_empty());
        assert!(!SkipReason::NoProcessor.description().is_empty());
        assert!(SkipReason::HostError("x".into())
            .description()
            .contains("x"));
    }

    #[test]
    fn status_description_covers_both_variants() {
        let loaded = HostLoadResult::Loaded {
            plugin: test_plugin("EQ"),
            handle: HostHandle::new(1),
        };
        assert!(loaded.status_description().contains("loaded"));

        let skipped = HostLoadResult::Skipped {
            plugin: test_plugin("Comp"),
            reason: SkipReason::BundleNotFound,
        };
        assert!(skipped.status_description().contains("skipped"));
    }

    #[test]
    fn host_handle_is_send_and_identity() {
        let h1 = HostHandle::new(10);
        let h2 = HostHandle::new(10);
        let h3 = HostHandle::new(20);
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
        assert_eq!(h1.id(), 10);
    }

    #[test]
    fn mock_success_host_loads_everything() {
        let host = MockSuccessHost;
        let chain = VstChain {
            plugins: vec![test_plugin("A"), test_plugin("B")],
        };
        let results = load_chain(&host, &chain);
        assert_eq!(results.loaded_count(), 2);
        assert_eq!(results.skipped_count(), 0);
        assert!(results.all_loaded());
        assert!(results.skipped_plugins().is_empty());
    }

    #[test]
    fn mock_skip_host_skips_everything() {
        let host = MockSkipHost;
        let chain = VstChain {
            plugins: vec![test_plugin("X"), test_plugin("Y")],
        };
        let results = load_chain(&host, &chain);
        assert_eq!(results.loaded_count(), 0);
        assert_eq!(results.skipped_count(), 2);
        assert!(!results.all_loaded());
        assert_eq!(results.skipped_plugins().len(), 2);
    }

    #[test]
    fn load_chain_on_empty_chain_succeeds() {
        let host = MockSuccessHost;
        let chain = VstChain::default();
        let results = load_chain(&host, &chain);
        assert!(results.results.is_empty());
        assert!(results.all_loaded());
    }

    #[test]
    fn mixed_results_preserve_order_and_allow_partial_success() {
        struct MixedHost;
        impl VstHost for MixedHost {
            fn load_plugin(&self, plugin: &VstPlugin) -> HostLoadResult {
                if plugin.name == "Good" {
                    HostLoadResult::Loaded {
                        plugin: plugin.clone(),
                        handle: HostHandle::new(1),
                    }
                } else {
                    HostLoadResult::Skipped {
                        plugin: plugin.clone(),
                        reason: SkipReason::BundleInvalid,
                    }
                }
            }
        }
        let chain = VstChain {
            plugins: vec![test_plugin("Bad"), test_plugin("Good"), test_plugin("Bad2")],
        };
        let results = load_chain(&MixedHost, &chain);
        assert_eq!(results.loaded_count(), 1);
        assert_eq!(results.skipped_count(), 2);
        assert_eq!(results.results[1].plugin().name, "Good");
    }

    #[test]
    fn chain_stays_validatable_after_load_with_skipped() {
        let host = MockSkipHost;
        let chain = VstChain {
            plugins: vec![test_plugin("A")],
        };
        // Chain validation is independent of host loading
        assert!(chain.validate().is_ok());
        let results = load_chain(&host, &chain);
        assert!(results.skipped_count() > 0);
        // Chain is still valid after loading
        assert!(chain.validate().is_ok());
    }

    // ── Z96-2 Windows COM host skeleton tests ──────────────────────────

    #[test]
    fn resolve_bundle_path_returns_absolute_when_exists() {
        let dir = temp_dir("resolve_abs");
        let bundle = fake_bundle(&dir, "EQ");
        let host = WindowsVstHost::new(vec![]);
        let plugin = VstPlugin {
            name: "EQ".into(),
            path: bundle.clone(),
        };
        assert_eq!(host.resolve_bundle_path(&plugin), Ok(bundle));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_bundle_path_returns_not_found_for_missing_absolute() {
        let host = WindowsVstHost::new(vec![]);
        let plugin = VstPlugin {
            name: "EQ".into(),
            path: PathBuf::from("/nonexistent/EQ.vst3"),
        };
        assert_eq!(
            host.resolve_bundle_path(&plugin),
            Err(SkipReason::BundleNotFound)
        );
    }

    #[test]
    fn resolve_bundle_path_searches_dirs_for_relative_path() {
        let dir = temp_dir("resolve_rel");
        let bundle = fake_bundle(&dir, "Reverb");
        let host = WindowsVstHost::new(vec![dir.clone()]);
        let plugin = VstPlugin {
            name: "Reverb".into(),
            path: PathBuf::from("Reverb.vst3"),
        };
        assert_eq!(host.resolve_bundle_path(&plugin), Ok(bundle));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_bundle_path_fails_for_relative_not_in_any_dir() {
        let host = WindowsVstHost::new(vec![PathBuf::from("/empty")]);
        let plugin = VstPlugin {
            name: "X".into(),
            path: PathBuf::from("X.vst3"),
        };
        assert_eq!(
            host.resolve_bundle_path(&plugin),
            Err(SkipReason::BundleNotFound)
        );
    }

    #[test]
    fn stage_bundle_resolve_ok_and_err() {
        let dir = temp_dir("stage1");
        let bundle = fake_bundle(&dir, "OK");
        let host = WindowsVstHost::new(vec![]);

        let ok_plugin = VstPlugin {
            name: "OK".into(),
            path: bundle,
        };
        assert!(host.stage_bundle_resolve(&ok_plugin).is_ok());

        let bad_plugin = VstPlugin {
            name: "Bad".into(),
            path: PathBuf::from("/nope.vst3"),
        };
        let err = host.stage_bundle_resolve(&bad_plugin).unwrap_err();
        assert_eq!(err.stage, HostStage::BundleResolve);
        assert_eq!(err.reason, SkipReason::BundleNotFound);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn host_load_plugin_skips_for_missing_bundle() {
        let host = WindowsVstHost::new(vec![]);
        let plugin = VstPlugin {
            name: "Ghost".into(),
            path: PathBuf::from("/ghost.vst3"),
        };
        let result = host.load_plugin(&plugin);
        assert!(result.is_skipped());
        assert_eq!(result.plugin().name, "Ghost");
    }

    #[test]
    fn host_load_plugin_skips_for_nonexistent_relative() {
        let host = WindowsVstHost::new(vec![PathBuf::from("/empty")]);
        let plugin = VstPlugin {
            name: "Phantom".into(),
            path: PathBuf::from("Phantom.vst3"),
        };
        let result = host.load_plugin(&plugin);
        assert!(result.is_skipped());
    }

    #[test]
    fn host_stage_descriptions_are_human_readable() {
        assert!(!HostStage::BundleResolve.description().is_empty());
        assert!(!HostStage::FactoryObtain.description().is_empty());
        assert!(!HostStage::ProcessorCreate.description().is_empty());
        assert!(!HostStage::ProcessorInit.description().is_empty());
    }

    #[test]
    fn stage_failure_description_includes_stage_and_reason() {
        let fail = StageFailure::new(HostStage::FactoryObtain, SkipReason::NoFactory);
        let desc = fail.description();
        assert!(desc.contains("factory"));
        assert!(desc.contains("no valid VST3 factory"));
    }

    #[test]
    fn windows_host_with_default_dirs_loads() {
        let host = WindowsVstHost::with_default_dirs();
        let plugin = VstPlugin {
            name: "Nonexistent".into(),
            path: PathBuf::from("/nonexistent/Plugin.vst3"),
        };
        // Absolute path that doesn't exist → Skipped at BundleResolve
        let result = host.load_plugin(&plugin);
        assert!(result.is_skipped());
    }

    #[test]
    fn non_windows_host_always_skips_factory_stage() {
        // On non-Windows, stage_factory_obtain always returns NoFactory/HostError.
        // Verify that load_plugin skips when the bundle exists but factory can't be obtained.
        let dir = temp_dir("nowin_factory");
        let bundle = fake_bundle(&dir, "Plug");
        let host = WindowsVstHost::new(vec![dir.clone()]);
        let plugin = VstPlugin {
            name: "Plug".into(),
            path: bundle,
        };
        let result = host.load_plugin(&plugin);
        // On non-Windows: BundleResolve succeeds, FactoryObtain fails → Skipped
        assert!(result.is_skipped());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_chain_with_windows_host_skips_missing_bundles() {
        let host = WindowsVstHost::new(vec![]);
        let chain = VstChain {
            plugins: vec![
                VstPlugin {
                    name: "A".into(),
                    path: PathBuf::from("/a.vst3"),
                },
                VstPlugin {
                    name: "B".into(),
                    path: PathBuf::from("/b.vst3"),
                },
            ],
        };
        let results = load_chain(&host, &chain);
        assert_eq!(results.loaded_count(), 0);
        assert_eq!(results.skipped_count(), 2);
        assert!(chain.validate().is_ok());
    }
}
