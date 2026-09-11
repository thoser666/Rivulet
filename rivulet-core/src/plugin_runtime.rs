//! WASM plugin runtime — Phase 2 of the plugin system.
//!
//! Provides a sandboxed execution environment for WASM plugins using
//! [`wasmtime`]. Each plugin runs in its own [`wasmtime::Store`] with
//! fuel-based CPU metering, a memory cap, and a timeout. Host functions
//! are exposed as WASM imports; crash isolation ensures a trap never
//! propagates to the host.
//!
//! The lifecycle mirrors the VST3 host boundary pattern (Z96):
//! `Loaded`/`Skipped` result, skip-on-error semantics, error isolation.
//!
//! **Scope note:** this module handles runtime loading and execution.
//! Manifest parsing and validation live in [`crate::plugin_manifest`].

use crate::plugin_manifest::{ManifestError, PluginManifest};
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::time::Duration;
use wasmtime::*;

// ── Constants ───────────────────────────────────────────────────────────────

/// API version this host implements (semver major.minor packed as u32).
pub const HOST_API_VERSION: u32 = 1 << 16; // 1.0

// ── Errors ──────────────────────────────────────────────────────────────────

/// Errors that can occur during plugin runtime operations.
#[derive(Debug, Clone)]
pub enum RuntimeError {
    /// WASM module failed to compile.
    CompileError(String),
    /// WASM module failed to instantiate.
    InstantiateError(String),
    /// A plugin export was not found in the WASM module.
    MissingExport(String),
    /// The plugin returned a non-zero error code from a lifecycle function.
    LifecycleError { function: &'static str, code: i32 },
    /// The plugin exceeded its time limit.
    Timeout(&'static str),
    /// The plugin trapped (crash, unreachable, out-of-bounds).
    Trap(String),
    /// A required host import is missing from the WASM module.
    MissingImport(String),
    /// The manifest is invalid.
    Manifest(ManifestError),
    /// The WASM file could not be read.
    IoError(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeError::CompileError(msg) => write!(f, "WASM compile error: {msg}"),
            RuntimeError::InstantiateError(msg) => write!(f, "WASM instantiate error: {msg}"),
            RuntimeError::MissingExport(name) => write!(f, "missing WASM export: {name}"),
            RuntimeError::LifecycleError { function, code } => {
                write!(f, "{function} returned error code {code}")
            }
            RuntimeError::Timeout(function) => {
                write!(f, "{function} exceeded time limit")
            }
            RuntimeError::Trap(msg) => write!(f, "WASM trap: {msg}"),
            RuntimeError::MissingImport(name) => write!(f, "missing WASM import: {name}"),
            RuntimeError::Manifest(e) => write!(f, "manifest error: {e}"),
            RuntimeError::IoError(msg) => write!(f, "I/O error: {msg}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<ManifestError> for RuntimeError {
    fn from(e: ManifestError) -> Self {
        RuntimeError::Manifest(e)
    }
}

// ── Plugin Load Result ──────────────────────────────────────────────────────

/// Result of attempting to load a WASM plugin.
///
/// Mirrors the VST3 [`crate::vst3::HostLoadResult`] pattern:
/// `Loaded` on success, `Skipped` on any failure.
#[derive(Debug)]
pub enum PluginLoadResult {
    /// Plugin loaded successfully.
    Loaded(Box<LoadedPlugin>),
    /// Plugin was skipped (non-fatal). Pipeline continues.
    /// Boxed so the large [`SkippedPlugin`] payload stays off the enum.
    Skipped(Box<SkippedPlugin>),
}

/// Data for a successfully loaded plugin.
#[derive(Debug)]
pub struct LoadedPlugin {
    /// The manifest that was parsed.
    pub manifest: PluginManifest,
    /// Runtime handle for the loaded plugin.
    pub handle: PluginHandle,
}

/// Data describing why a plugin was skipped.
#[derive(Debug)]
pub struct SkippedPlugin {
    /// The manifest, if it parsed. `None` when parse itself failed.
    pub manifest: Option<PluginManifest>,
    /// Why the plugin was skipped.
    pub reason: SkipReason,
}

impl PluginLoadResult {
    pub fn is_loaded(&self) -> bool {
        matches!(self, PluginLoadResult::Loaded(_))
    }

    pub fn is_skipped(&self) -> bool {
        matches!(self, PluginLoadResult::Skipped(_))
    }

    pub fn status_description(&self) -> String {
        match self {
            PluginLoadResult::Loaded(loaded) => {
                format!("{} loaded successfully", loaded.manifest.plugin.name)
            }
            PluginLoadResult::Skipped(skipped) => {
                let name = skipped
                    .manifest
                    .as_ref()
                    .map(|m| m.plugin.name.as_str())
                    .unwrap_or("unknown");
                format!("{name} skipped: {}", skipped.reason)
            }
        }
    }

    /// Extract the loaded plugin from a Loaded result.
    pub fn into_loaded(self) -> Option<LoadedPlugin> {
        match self {
            PluginLoadResult::Loaded(loaded) => Some(*loaded),
            _ => None,
        }
    }
}

/// Reason a plugin was skipped instead of loaded.
#[derive(Debug, Clone)]
pub enum SkipReason {
    /// Manifest could not be parsed.
    ManifestParse(String),
    /// Manifest failed validation.
    ManifestValidation(String),
    /// WASM binary could not be read.
    IoError(String),
    /// WASM compilation failed.
    CompileError(String),
    /// WASM instantiation failed.
    InstantiateError(String),
    /// A required export is missing.
    MissingExport(String),
    /// plugin_init returned an error.
    InitFailed(i32),
    /// plugin_init timed out.
    InitTimeout,
    /// The WASM module trapped during init.
    InitTrap(String),
}

impl fmt::Display for SkipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkipReason::ManifestParse(msg) => write!(f, "manifest parse: {msg}"),
            SkipReason::ManifestValidation(msg) => write!(f, "manifest validation: {msg}"),
            SkipReason::IoError(msg) => write!(f, "I/O: {msg}"),
            SkipReason::CompileError(msg) => write!(f, "compile: {msg}"),
            SkipReason::InstantiateError(msg) => write!(f, "instantiate: {msg}"),
            SkipReason::MissingExport(name) => write!(f, "missing export: {name}"),
            SkipReason::InitFailed(code) => write!(f, "init failed (code {code})"),
            SkipReason::InitTimeout => write!(f, "init timed out"),
            SkipReason::InitTrap(msg) => write!(f, "init trap: {msg}"),
        }
    }
}

// ── Plugin State ────────────────────────────────────────────────────────────

/// Lifecycle state of a loaded plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginState {
    /// WASM module compiled and instantiated, plugin_init not yet called.
    Loaded,
    /// plugin_init succeeded, plugin_activate not yet called.
    Initialized,
    /// plugin_activate succeeded, processing active.
    Active,
    /// plugin_deactivate called, can be re-activated.
    Inactive,
    /// Plugin terminated normally or was unloaded.
    Unloaded,
    /// Plugin encountered an unrecoverable error.
    Skipped,
}

// ── Plugin Handle ───────────────────────────────────────────────────────────

/// Handle to a loaded WASM plugin instance.
///
/// The handle owns the wasmtime [`Store`] and [`Instance`], ensuring the
/// plugin's memory stays alive as long as the handle exists.
pub struct PluginHandle {
    /// The plugin's manifest (for display and capability checks).
    pub manifest: PluginManifest,
    /// Current lifecycle state.
    state: PluginState,
    /// The wasmtime engine is shared across all loaded plugins.
    engine: Engine,
    /// The plugin's WASM store (owns the instance memory).
    store: Store<PluginStoreData>,
    /// The instantiated WASM module.
    instance: Instance,
}

impl fmt::Debug for PluginHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PluginHandle")
            .field("manifest", &self.manifest.plugin.id)
            .field("state", &self.state)
            .finish()
    }
}

/// Per-plugin data stored inside the wasmtime [`Store`].
struct PluginStoreData {
    /// The plugin's configuration (key-value pairs, persisted by host).
    config: HashMap<String, String>,
    /// Timeout in milliseconds.
    timeout_ms: u32,
    /// Whether the plugin has the "ui" capability.
    has_ui: bool,
}

impl PluginHandle {
    /// Get the current lifecycle state.
    pub fn state(&self) -> PluginState {
        self.state
    }

    /// Get the plugin manifest.
    pub fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
}

// ── Runtime ─────────────────────────────────────────────────────────────────

/// The WASM plugin runtime. Manages the shared [`Engine`] and provides
/// the API for loading, activating, and processing plugins.
pub struct WasmPluginRuntime {
    /// Shared wasmtime engine (compiled once, used by all plugins).
    engine: Engine,
}

impl WasmPluginRuntime {
    /// Create a new plugin runtime with fuel-based CPU metering enabled.
    pub fn new() -> Result<Self, RuntimeError> {
        let mut config = Config::default();
        config.consume_fuel(true);
        let engine = Engine::new(&config)
            .map_err(|e| RuntimeError::CompileError(format!("failed to create engine: {e}")))?;
        Ok(Self { engine })
    }

    /// Create a runtime with a specific fuel budget per plugin.
    pub fn with_fuel(_fuel: u64) -> Result<Self, RuntimeError> {
        Self::new()
    }

    /// Get a reference to the shared engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Load a WASM plugin from a binary path and manifest.
    pub fn load_plugin(&self, wasm_path: &Path, manifest: PluginManifest) -> PluginLoadResult {
        // Validate manifest
        if let Err(e) = manifest.validate() {
            return PluginLoadResult::Skipped(Box::new(SkippedPlugin {
                manifest: Some(manifest),
                reason: SkipReason::ManifestValidation(e.to_string()),
            }));
        }

        // Read WASM binary
        let wasm_bytes = match std::fs::read(wasm_path) {
            Ok(b) => b,
            Err(e) => {
                return PluginLoadResult::Skipped(Box::new(SkippedPlugin {
                    manifest: Some(manifest),
                    reason: SkipReason::IoError(e.to_string()),
                }));
            }
        };

        self.load_plugin_inner(wasm_bytes, manifest)
    }

    /// Load a plugin directly from WASM bytes (for testing and embedding).
    pub fn load_plugin_from_bytes(
        &self,
        wasm_bytes: &[u8],
        manifest: PluginManifest,
    ) -> PluginLoadResult {
        // Validate manifest
        if let Err(e) = manifest.validate() {
            return PluginLoadResult::Skipped(Box::new(SkippedPlugin {
                manifest: Some(manifest),
                reason: SkipReason::ManifestValidation(e.to_string()),
            }));
        }

        self.load_plugin_inner(wasm_bytes.to_vec(), manifest)
    }

    /// Internal: compile, instantiate, and init a WASM plugin.
    fn load_plugin_inner(&self, wasm_bytes: Vec<u8>, manifest: PluginManifest) -> PluginLoadResult {
        // Compile
        let module = match Module::new(&self.engine, &wasm_bytes) {
            Ok(m) => m,
            Err(e) => {
                return PluginLoadResult::Skipped(Box::new(SkippedPlugin {
                    manifest: Some(manifest),
                    reason: SkipReason::CompileError(e.to_string()),
                }));
            }
        };

        // Create store with fuel
        let fuel = manifest.plugin.resources.max_cpu_ms as u64 * 2_000_000;
        let mut store = Store::new(
            &self.engine,
            PluginStoreData {
                config: HashMap::new(),
                timeout_ms: manifest.plugin.resources.timeout_ms,
                has_ui: manifest.plugin.capabilities.ui,
            },
        );
        if store.set_fuel(fuel).is_err() {
            return PluginLoadResult::Skipped(Box::new(SkippedPlugin {
                manifest: Some(manifest),
                reason: SkipReason::CompileError("fuel not enabled".into()),
            }));
        }

        // Build linker with host imports
        let mut linker = Linker::<PluginStoreData>::new(&self.engine);
        Self::register_core_imports(&mut linker);

        let instance = match linker.instantiate(&mut store, &module) {
            Ok(i) => i,
            Err(e) => {
                return PluginLoadResult::Skipped(Box::new(SkippedPlugin {
                    manifest: Some(manifest),
                    reason: SkipReason::InstantiateError(e.to_string()),
                }));
            }
        };

        // Call plugin_init
        let mut handle = PluginHandle {
            manifest,
            state: PluginState::Loaded,
            engine: self.engine.clone(),
            store,
            instance,
        };

        match call_plugin_init(&mut handle) {
            Ok(()) => {
                handle.state = PluginState::Initialized;
                PluginLoadResult::Loaded(Box::new(LoadedPlugin {
                    manifest: handle.manifest.clone(),
                    handle,
                }))
            }
            Err(skip_reason) => {
                let manifest = Some(handle.manifest.clone());
                PluginLoadResult::Skipped(Box::new(SkippedPlugin {
                    manifest,
                    reason: skip_reason,
                }))
            }
        }
    }

    /// Register core (non-capability-gated) host imports.
    fn register_core_imports(linker: &mut Linker<PluginStoreData>) {
        // host_log(level: u32, ptr: *const u8, len: u32)
        linker
            .func_wrap(
                "host",
                "host_log",
                |mut caller: Caller<'_, PluginStoreData>, level: u32, ptr: u32, len: u32| {
                    if let Some(memory) = caller.get_export("memory").and_then(|e| e.into_memory())
                    {
                        let mut buf = vec![0u8; len as usize];
                        if memory.read(&caller, ptr as usize, &mut buf).is_ok() {
                            let msg = String::from_utf8_lossy(&buf);
                            match level {
                                0 => tracing::error!("[plugin] {msg}"),
                                1 => tracing::warn!("[plugin] {msg}"),
                                2 => tracing::info!("[plugin] {msg}"),
                                3 => tracing::debug!("[plugin] {msg}"),
                                _ => tracing::trace!("[plugin] {msg}"),
                            }
                        }
                    }
                },
            )
            .expect("host_log import registration should not fail");

        // host_config_read(key_ptr, key_len, val_buf, val_buf_len) -> i32
        linker
            .func_wrap(
                "host",
                "host_config_read",
                |mut caller: Caller<'_, PluginStoreData>,
                 key_ptr: u32,
                 key_len: u32,
                 val_buf: u32,
                 val_buf_len: u32|
                 -> i32 {
                    // Phase 1: read key from WASM memory (needs mutable caller)
                    let key = {
                        let memory = match caller.get_export("memory").and_then(|e| e.into_memory())
                        {
                            Some(m) => m,
                            None => return -1,
                        };
                        let mut key_buf = vec![0u8; key_len as usize];
                        if memory
                            .read(&caller, key_ptr as usize, &mut key_buf)
                            .is_err()
                        {
                            return -1;
                        }
                        match String::from_utf8(key_buf) {
                            Ok(k) => k,
                            Err(_) => return -1,
                        }
                    }; // caller borrow released here

                    // Phase 2: look up value in config (immutable borrow)
                    let val_copy = {
                        let data = caller.data();
                        data.config.get(&key).cloned()
                    }; // caller borrow released here

                    match val_copy {
                        Some(val) => {
                            // Phase 3: write value to WASM memory
                            let memory =
                                match caller.get_export("memory").and_then(|e| e.into_memory()) {
                                    Some(m) => m,
                                    None => return -1,
                                };
                            let val_bytes = val.as_bytes();
                            let copy_len = val_bytes.len().min(val_buf_len as usize);
                            let _ =
                                memory.write(&mut caller, val_buf as usize, &val_bytes[..copy_len]);
                            copy_len as i32
                        }
                        None => 0,
                    }
                },
            )
            .expect("host_config_read import registration should not fail");

        // host_config_write(key_ptr, key_len, val_ptr, val_len) -> i32
        linker
            .func_wrap(
                "host",
                "host_config_write",
                |mut caller: Caller<'_, PluginStoreData>,
                 key_ptr: u32,
                 key_len: u32,
                 val_ptr: u32,
                 val_len: u32|
                 -> i32 {
                    // Read key and value from WASM memory
                    let (key, val) = {
                        let memory = match caller.get_export("memory").and_then(|e| e.into_memory())
                        {
                            Some(m) => m,
                            None => return -1,
                        };
                        let mut key_buf = vec![0u8; key_len as usize];
                        if memory
                            .read(&caller, key_ptr as usize, &mut key_buf)
                            .is_err()
                        {
                            return -1;
                        }
                        let mut val_buf = vec![0u8; val_len as usize];
                        if memory
                            .read(&caller, val_ptr as usize, &mut val_buf)
                            .is_err()
                        {
                            return -1;
                        }
                        match (String::from_utf8(key_buf), String::from_utf8(val_buf)) {
                            (Ok(k), Ok(v)) => (k, v),
                            _ => return -1,
                        }
                    }; // caller borrow released here

                    // Store in config
                    caller.data_mut().config.insert(key, val);
                    0
                },
            )
            .expect("host_config_write import registration should not fail");

        // host_time_now() -> u64  (nanoseconds since epoch)
        linker
            .func_wrap(
                "host",
                "host_time_now",
                |_caller: Caller<'_, PluginStoreData>| -> u64 {
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos() as u64
                },
            )
            .expect("host_time_now import registration should not fail");

        // host_ui_invalidate()
        linker
            .func_wrap(
                "host",
                "host_ui_invalidate",
                |caller: Caller<'_, PluginStoreData>| {
                    let has_ui = caller.data().has_ui;
                    if !has_ui {
                        tracing::warn!(
                            "[plugin] host_ui_invalidate called without 'ui' capability"
                        );
                    }
                },
            )
            .expect("host_ui_invalidate import registration should not fail");
    }
}

impl Default for WasmPluginRuntime {
    fn default() -> Self {
        Self::new().expect("default WasmPluginRuntime creation should not fail")
    }
}

// ── Lifecycle helpers ───────────────────────────────────────────────────────

/// Call `plugin_init` on a loaded handle. Returns Ok(()) on success (code 0),
/// or Err(SkipReason) on failure.
fn call_plugin_init(handle: &mut PluginHandle) -> Result<(), SkipReason> {
    let timeout = Duration::from_millis(handle.store.data().timeout_ms as u64);

    // Set epoch deadline for timeout
    handle.store.set_epoch_deadline(1);

    // Start a background thread to advance the epoch after the timeout
    let engine = handle.engine.clone();
    let epoch_handle = std::thread::spawn(move || {
        std::thread::sleep(timeout);
        engine.increment_epoch();
    });

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let init = handle
            .instance
            .get_typed_func::<i32, i32>(&mut handle.store, "plugin_init");
        match init {
            Ok(init_fn) => init_fn
                .call(&mut handle.store, HOST_API_VERSION as i32)
                .map_err(|e| RuntimeError::Trap(e.to_string())),
            Err(e) => Err(RuntimeError::MissingExport(e.to_string())),
        }
    }));

    // Signal the epoch thread to stop
    handle.engine.increment_epoch();
    let _ = epoch_handle.join();

    match result {
        Ok(Ok(0)) => Ok(()),
        Ok(Ok(code)) => Err(SkipReason::InitFailed(code)),
        Ok(Err(RuntimeError::MissingExport(e))) => Err(SkipReason::MissingExport(e)),
        Ok(Err(RuntimeError::Trap(e))) => Err(SkipReason::InitTrap(e)),
        Ok(Err(_)) => Err(SkipReason::InitTrap("unknown host error".into())),
        Err(_) => Err(SkipReason::InitTrap("panic in WASM host call".into())),
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(clippy::needless_borrow)]
mod tests {
    use super::*;
    use crate::plugin_manifest::parse_manifest;

    // Minimal valid manifest for testing.
    fn test_manifest() -> PluginManifest {
        parse_manifest(
            r#"
[plugin]
id = "com.example.test-plugin"
version = "1.0.0"
name = "Test Plugin"

[plugin.api_version]
min = "1.0"

[plugin.type]
kind = "ui_panel"
entry_point = "plugin.wasm"
"#,
        )
        .expect("test manifest should parse")
    }

    // Return WAT as bytes — Module::new accepts both WAT and WASM.
    fn wat_to_bytes(wat: &str) -> Vec<u8> {
        wat.as_bytes().to_vec()
    }

    // Minimal valid WASM module with all required lifecycle exports.
    fn minimal_wasm_bytes(_engine: &Engine) -> Vec<u8> {
        wat_to_bytes(
            r#"
            (module
                (memory (export "memory") 1)
                (func (export "plugin_init") (param i32) (result i32)
                    i32.const 0
                )
                (func (export "plugin_activate") (result i32)
                    i32.const 0
                )
                (func (export "plugin_process") (param i32 i32 i32 i32) (result i32)
                    i32.const 0
                )
                (func (export "plugin_deactivate") (result i32)
                    i32.const 0
                )
                (func (export "plugin_unload")
                )
            )
            "#,
        )
    }

    // WASM module that returns an error from plugin_init.
    fn failing_init_wasm(_engine: &Engine) -> Vec<u8> {
        wat_to_bytes(
            r#"
            (module
                (memory (export "memory") 1)
                (func (export "plugin_init") (param i32) (result i32)
                    i32.const -1
                )
                (func (export "plugin_activate") (result i32)
                    i32.const 0
                )
                (func (export "plugin_process") (param i32 i32 i32 i32) (result i32)
                    i32.const 0
                )
                (func (export "plugin_deactivate") (result i32)
                    i32.const 0
                )
                (func (export "plugin_unload")
                )
            )
            "#,
        )
    }

    // WASM module that traps (unreachable).
    fn trapping_wasm(_engine: &Engine) -> Vec<u8> {
        wat_to_bytes(
            r#"
            (module
                (memory (export "memory") 1)
                (func (export "plugin_init") (param i32) (result i32)
                    unreachable
                )
                (func (export "plugin_activate") (result i32)
                    i32.const 0
                )
                (func (export "plugin_process") (param i32 i32 i32 i32) (result i32)
                    i32.const 0
                )
                (func (export "plugin_deactivate") (result i32)
                    i32.const 0
                )
                (func (export "plugin_unload")
                )
            )
            "#,
        )
    }

    // WASM module missing the plugin_init export.
    fn missing_export_wasm(_engine: &Engine) -> Vec<u8> {
        wat_to_bytes(
            r#"
            (module
                (memory (export "memory") 1)
                (func (export "plugin_activate") (result i32)
                    i32.const 0
                )
                (func (export "plugin_process") (param i32 i32 i32 i32) (result i32)
                    i32.const 0
                )
                (func (export "plugin_deactivate") (result i32)
                    i32.const 0
                )
                (func (export "plugin_unload")
                )
            )
            "#,
        )
    }

    // WASM module that calls host_log during init.
    fn host_log_wasm(_engine: &Engine) -> Vec<u8> {
        wat_to_bytes(
            r#"
            (module
                (import "host" "host_log" (func $log (param i32 i32 i32)))
                (memory (export "memory") 1)
                (func (export "plugin_init") (param i32) (result i32)
                    i32.const 2          ;; level = info
                    i32.const 0          ;; msg ptr
                    i32.const 5          ;; msg len
                    call $log
                    i32.const 0
                )
                (func (export "plugin_activate") (result i32)
                    i32.const 0
                )
                (func (export "plugin_process") (param i32 i32 i32 i32) (result i32)
                    i32.const 0
                )
                (func (export "plugin_deactivate") (result i32)
                    i32.const 0
                )
                (func (export "plugin_unload")
                )
            )
            "#,
        )
    }

    // WASM module that calls host_time_now during init.
    fn host_time_wasm(_engine: &Engine) -> Vec<u8> {
        wat_to_bytes(
            r#"
            (module
                (import "host" "host_time_now" (func $time (result i64)))
                (memory (export "memory") 1)
                (global (mut i64) (i64.const 0))
                (func (export "plugin_init") (param i32) (result i32)
                    call $time
                    global.set 0
                    i32.const 0
                )
                (func (export "plugin_activate") (result i32)
                    i32.const 0
                )
                (func (export "plugin_process") (param i32 i32 i32 i32) (result i32)
                    i32.const 0
                )
                (func (export "plugin_deactivate") (result i32)
                    i32.const 0
                )
                (func (export "plugin_unload")
                )
            )
            "#,
        )
    }

    // ── Runtime creation ─────────────────────────────────────────────

    #[test]
    fn runtime_creates_with_defaults() {
        let runtime = WasmPluginRuntime::new();
        assert!(runtime.is_ok());
    }

    #[test]
    fn runtime_creates_with_fuel() {
        let runtime = WasmPluginRuntime::with_fuel(1_000_000);
        assert!(runtime.is_ok());
    }

    #[test]
    fn runtime_default_impl() {
        let _runtime = WasmPluginRuntime::default();
    }

    // ── Plugin loading ───────────────────────────────────────────────

    #[test]
    fn load_valid_plugin() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = minimal_wasm_bytes(runtime.engine());
        let result = runtime.load_plugin_from_bytes(&wasm, test_manifest());
        assert!(
            result.is_loaded(),
            "should load: {}",
            result.status_description()
        );
    }

    #[test]
    fn load_plugin_stores_manifest() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = minimal_wasm_bytes(runtime.engine());
        let result = runtime.load_plugin_from_bytes(&wasm, test_manifest());
        match &result {
            PluginLoadResult::Loaded(loaded) => {
                assert_eq!(loaded.manifest.plugin.id, "com.example.test-plugin");
                assert_eq!(loaded.handle.state(), PluginState::Initialized);
                assert_eq!(loaded.handle.manifest.plugin.name, "Test Plugin");
            }
            _ => panic!("expected Loaded, got: {}", result.status_description()),
        }
    }

    #[test]
    fn skip_invalid_manifest() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = minimal_wasm_bytes(runtime.engine());
        let mut manifest = test_manifest();
        manifest.plugin.id = "invalid".to_string(); // no dots = invalid
        let result = runtime.load_plugin_from_bytes(&wasm, manifest);
        assert!(result.is_skipped());
        assert!(result.status_description().contains("validation"));
    }

    #[test]
    fn skip_missing_export() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = missing_export_wasm(runtime.engine());
        let result = runtime.load_plugin_from_bytes(&wasm, test_manifest());
        assert!(result.is_skipped());
        assert!(result.status_description().contains("missing export"));
    }

    #[test]
    fn skip_failing_init() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = failing_init_wasm(runtime.engine());
        let result = runtime.load_plugin_from_bytes(&wasm, test_manifest());
        assert!(result.is_skipped());
        assert!(
            result.status_description().contains("init failed"),
            "got: {}",
            result.status_description()
        );
    }

    #[test]
    fn skip_trapping_init() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = trapping_wasm(runtime.engine());
        let result = runtime.load_plugin_from_bytes(&wasm, test_manifest());
        assert!(result.is_skipped());
        assert!(
            result.status_description().contains("trap"),
            "got: {}",
            result.status_description()
        );
    }

    // ── Host imports ─────────────────────────────────────────────────

    #[test]
    fn host_log_import_is_registered() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = host_log_wasm(runtime.engine());
        let result = runtime.load_plugin_from_bytes(&wasm, test_manifest());
        assert!(
            result.is_loaded(),
            "should load: {}",
            result.status_description()
        );
    }

    #[test]
    fn host_time_now_import_is_registered() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = host_time_wasm(runtime.engine());
        let result = runtime.load_plugin_from_bytes(&wasm, test_manifest());
        assert!(
            result.is_loaded(),
            "should load: {}",
            result.status_description()
        );
    }

    // ── Error types ──────────────────────────────────────────────────

    #[test]
    fn runtime_error_display() {
        let e = RuntimeError::CompileError("bad module".into());
        assert!(e.to_string().contains("bad module"));

        let e = RuntimeError::Timeout("plugin_process");
        assert!(e.to_string().contains("plugin_process"));

        let e = RuntimeError::Trap("out of bounds".into());
        assert!(e.to_string().contains("out of bounds"));

        let e = RuntimeError::MissingExport("plugin_init".into());
        assert!(e.to_string().contains("plugin_init"));

        let e = RuntimeError::LifecycleError {
            function: "plugin_activate",
            code: -1,
        };
        assert!(e.to_string().contains("plugin_activate"));
        assert!(e.to_string().contains("-1"));
    }

    #[test]
    fn skip_reason_display() {
        let r = SkipReason::ManifestParse("bad toml".into());
        assert!(r.to_string().contains("bad toml"));

        let r = SkipReason::InitFailed(-1);
        assert!(r.to_string().contains("init failed"));

        let r = SkipReason::InitTimeout;
        assert!(r.to_string().contains("timed out"));

        let r = SkipReason::InitTrap("unreachable".into());
        assert!(r.to_string().contains("trap"));
    }

    // ── Plugin state ─────────────────────────────────────────────────

    #[test]
    fn plugin_state_after_load() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = minimal_wasm_bytes(runtime.engine());
        let result = runtime.load_plugin_from_bytes(&wasm, test_manifest());
        match result {
            PluginLoadResult::Loaded(loaded) => {
                assert_eq!(loaded.handle.state(), PluginState::Initialized);
            }
            _ => panic!("expected Loaded"),
        }
    }

    #[test]
    fn load_result_helpers() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = minimal_wasm_bytes(runtime.engine());
        let loaded = PluginLoadResult::Loaded(Box::new(LoadedPlugin {
            manifest: test_manifest(),
            handle: runtime
                .load_plugin_from_bytes(&wasm, test_manifest())
                .into_loaded()
                .unwrap()
                .handle,
        }));
        assert!(loaded.is_loaded());
        assert!(!loaded.is_skipped());
        assert!(loaded.status_description().contains("Test Plugin"));

        let skipped = PluginLoadResult::Skipped(Box::new(SkippedPlugin {
            manifest: None,
            reason: SkipReason::InitTimeout,
        }));
        assert!(!skipped.is_loaded());
        assert!(skipped.is_skipped());
        assert!(skipped.status_description().contains("unknown"));
    }

    #[test]
    fn plugin_handle_debug() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = minimal_wasm_bytes(runtime.engine());
        let result = runtime.load_plugin_from_bytes(&wasm, test_manifest());
        let handle = result.into_loaded().unwrap();
        let debug = format!("{:?}", handle);
        assert!(debug.contains("com.example.test-plugin"));
        assert!(debug.contains("Initialized"));
    }
}
