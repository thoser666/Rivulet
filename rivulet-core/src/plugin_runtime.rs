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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
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
    /// The plugin was called from an invalid lifecycle state.
    InvalidState(String),
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
            RuntimeError::InvalidState(msg) => write!(f, "invalid plugin state: {msg}"),
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
    /// Fuel refilled before each lifecycle call (CPU budget per call).
    fuel_budget: u64,
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

    /// Activate the plugin (start processing).
    ///
    /// Transitions `Initialized`/`Inactive` → `Active`. If the plugin
    /// returns an error code it is demoted to `Inactive` (retryable),
    /// mirroring the RFC error-recovery semantics.
    pub fn activate(&mut self) -> Result<(), RuntimeError> {
        if self.state == PluginState::Active {
            return Ok(());
        }
        if !matches!(self.state, PluginState::Initialized | PluginState::Inactive) {
            return Err(RuntimeError::InvalidState(format!(
                "plugin_activate called from {:?}",
                self.state
            )));
        }
        let activate = self
            .instance
            .get_typed_func::<(), i32>(&mut self.store, "plugin_activate")
            .map_err(|_| missing_export("plugin_activate"))?;
        let code = self.invoke_guarded("plugin_activate", |handle| {
            activate.call(&mut handle.store, ())
        })?;
        if code == 0 {
            self.state = PluginState::Active;
            Ok(())
        } else {
            self.state = PluginState::Inactive;
            Err(RuntimeError::LifecycleError {
                function: "plugin_activate",
                code,
            })
        }
    }

    /// Process one unit of work (audio samples, video frame, or event).
    ///
    /// Only valid while `Active`. The input is copied into guest memory at a
    /// host-chosen scratch address, `plugin_process` is invoked with
    /// `(input_ptr, input_len, output_ptr, output_cap)`, and the bytes the
    /// plugin reports having written are read back. A negative return code is
    /// treated as a processing failure and demotes the plugin to `Inactive`.
    pub fn process(&mut self, input: &[u8]) -> Result<Vec<u8>, RuntimeError> {
        if self.state != PluginState::Active {
            return Err(RuntimeError::InvalidState(format!(
                "plugin_process called from {:?}",
                self.state
            )));
        }
        const INPUT_PTR: u32 = 0x1000;
        const OUTPUT_PTR: u32 = 0x2000;
        const OUTPUT_CAP: u32 = 0x1000;
        if input.len() > INPUT_PTR as usize - 1 {
            return Err(RuntimeError::InvalidState(
                "plugin_process input exceeds scratch buffer".into(),
            ));
        }

        let memory = self
            .instance
            .get_memory(&mut self.store, "memory")
            .ok_or_else(|| RuntimeError::MissingExport("memory".into()))?;
        memory
            .write(&mut self.store, INPUT_PTR as usize, input)
            .map_err(|e| RuntimeError::Trap(e.to_string()))?;

        let process_fn = self
            .instance
            .get_typed_func::<(i32, i32, i32, i32), i32>(&mut self.store, "plugin_process")
            .map_err(|_| missing_export("plugin_process"))?;
        let code = self.invoke_guarded("plugin_process", |handle| {
            process_fn.call(
                &mut handle.store,
                (
                    INPUT_PTR as i32,
                    input.len() as i32,
                    OUTPUT_PTR as i32,
                    OUTPUT_CAP as i32,
                ),
            )
        })?;
        if code < 0 {
            self.state = PluginState::Inactive;
            return Err(RuntimeError::LifecycleError {
                function: "plugin_process",
                code,
            });
        }

        let len = (code as usize).min(OUTPUT_CAP as usize);
        let mut output = vec![0u8; len];
        memory
            .read(&mut self.store, OUTPUT_PTR as usize, &mut output)
            .map_err(|e| RuntimeError::Trap(e.to_string()))?;
        Ok(output)
    }

    /// Deactivate the plugin (stop processing).
    ///
    /// Transitions `Active` → `Inactive`. A failing plugin is unloaded
    /// (permanent) per the RFC lifecycle table.
    pub fn deactivate(&mut self) -> Result<(), RuntimeError> {
        if self.state == PluginState::Inactive {
            return Ok(());
        }
        if self.state != PluginState::Active {
            return Err(RuntimeError::InvalidState(format!(
                "plugin_deactivate called from {:?}",
                self.state
            )));
        }
        let deactivate = self
            .instance
            .get_typed_func::<(), i32>(&mut self.store, "plugin_deactivate")
            .map_err(|_| missing_export("plugin_deactivate"))?;
        let code = self.invoke_guarded("plugin_deactivate", |handle| {
            deactivate.call(&mut handle.store, ())
        })?;
        if code == 0 {
            self.state = PluginState::Inactive;
            Ok(())
        } else {
            self.state = PluginState::Unloaded;
            Err(RuntimeError::LifecycleError {
                function: "plugin_deactivate",
                code,
            })
        }
    }

    /// Unload the plugin and free its resources.
    ///
    /// Calls `plugin_unload` (best-effort) and transitions to `Unloaded`.
    /// Idempotent.
    pub fn unload(&mut self) {
        if matches!(self.state, PluginState::Unloaded | PluginState::Skipped) {
            return;
        }
        if let Ok(unload) = self
            .instance
            .get_typed_func::<(), ()>(&mut self.store, "plugin_unload")
        {
            let _ =
                self.invoke_guarded("plugin_unload", |handle| unload.call(&mut handle.store, ()));
        }
        self.state = PluginState::Unloaded;
    }

    /// Refill the fuel budget before a lifecycle call.
    fn refill_fuel(&mut self) {
        if self.store.set_fuel(self.fuel_budget).is_err() {
            tracing::debug!("fuel metering not enabled; ignoring refill");
        }
    }

    /// Run a WASM lifecycle call with a per-call fuel budget, a wall-clock
    /// timeout (via epoch-based interruption), and panic/trap isolation.
    ///
    /// The timeout thread only advances the shared engine epoch while the call
    /// is still in flight (a cancellation flag), so fast calls never block.
    /// Traps, out-of-fuel, and deadline interrupts are never propagated as
    /// panics — they are returned as [`RuntimeError`]s.
    fn invoke_guarded<T, F>(
        &mut self,
        function: &'static str,
        mut call: F,
    ) -> Result<T, RuntimeError>
    where
        T: Copy,
        F: FnMut(&mut PluginHandle) -> Result<T, wasmtime::Error>,
    {
        self.refill_fuel();
        let timeout = Duration::from_millis(self.store.data().timeout_ms as u64);
        self.store.set_epoch_deadline(1);

        let armed = Arc::new(AtomicBool::new(true));
        let stop = armed.clone();
        let engine = self.engine.clone();
        let deadline = Instant::now() + timeout;
        let timeout_thread = std::thread::spawn(move || {
            while stop.load(Ordering::Acquire) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(2));
            }
            if stop.load(Ordering::Acquire) {
                engine.increment_epoch();
            }
        });

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            call(&mut *self).map_err(|e| classify_wasm_error(e, function))
        }));

        armed.store(false, Ordering::Release);
        let _ = timeout_thread.join();

        match result {
            Ok(ok) => ok,
            Err(_) => Err(RuntimeError::Trap("panic in WASM host call".into())),
        }
    }
}

// ── Runtime ─────────────────────────────────────────────────────────────────

/// The WASM plugin runtime. Manages the shared [`Engine`] and provides
/// the API for loading, activating, and processing plugins.
pub struct WasmPluginRuntime {
    /// Shared wasmtime engine (compiled once, used by all plugins).
    engine: Engine,
    /// Optional fixed fuel budget per lifecycle call. When `None`, the budget
    /// is derived from the manifest's `resources.max_cpu_ms`.
    fuel_budget: Option<u64>,
}

impl WasmPluginRuntime {
    /// Create a new plugin runtime with fuel-based CPU metering and
    /// epoch-based interruption (wall-clock timeouts) enabled.
    pub fn new() -> Result<Self, RuntimeError> {
        let mut config = Config::default();
        config.consume_fuel(true);
        config.epoch_interruption(true);
        let engine = Engine::new(&config)
            .map_err(|e| RuntimeError::CompileError(format!("failed to create engine: {e}")))?;
        Ok(Self {
            engine,
            fuel_budget: None,
        })
    }

    /// Create a runtime with a specific fuel budget per plugin call.
    pub fn with_fuel(fuel: u64) -> Result<Self, RuntimeError> {
        let mut config = Config::default();
        config.consume_fuel(true);
        config.epoch_interruption(true);
        let engine = Engine::new(&config)
            .map_err(|e| RuntimeError::CompileError(format!("failed to create engine: {e}")))?;
        Ok(Self {
            engine,
            fuel_budget: Some(fuel),
        })
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
        let fuel = self
            .fuel_budget
            .unwrap_or(manifest.plugin.resources.max_cpu_ms as u64 * 2_000_000);
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
        // With epoch interruption enabled (wasmtime >= 32), a fresh store's
        // deadline is the current epoch: any instantiation would trap with
        // "interrupt" before we ever get to arm the real per-call deadline.
        // Arm a far-future deadline here; `invoke_guarded` sets the actual
        // wall-clock deadline before every lifecycle call.
        store.set_epoch_deadline(u64::MAX);

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
            fuel_budget: fuel,
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

/// Build a [`RuntimeError::MissingExport`] for a lifecycle function.
fn missing_export(name: &str) -> RuntimeError {
    RuntimeError::MissingExport(format!("{name} export not found in WASM module"))
}

/// Map a wasmtime call error onto [`RuntimeError`].
///
/// Epoch-interrupt traps (wall-clock timeout) and out-of-fuel traps (CPU
/// budget) both surface as [`RuntimeError::Timeout`]; everything else is a
/// regular trap.
fn classify_wasm_error(e: wasmtime::Error, function: &'static str) -> RuntimeError {
    match e.downcast_ref::<wasmtime::Trap>() {
        Some(wasmtime::Trap::Interrupt) | Some(wasmtime::Trap::OutOfFuel) => {
            RuntimeError::Timeout(function)
        }
        Some(trap) => RuntimeError::Trap(trap.to_string()),
        None => RuntimeError::Trap(e.to_string()),
    }
}

/// Call `plugin_init` on a loaded handle. Returns Ok(()) on success (code 0),
/// or Err(SkipReason) on failure.
fn call_plugin_init(handle: &mut PluginHandle) -> Result<(), SkipReason> {
    let init = match handle
        .instance
        .get_typed_func::<i32, i32>(&mut handle.store, "plugin_init")
    {
        Ok(f) => f,
        Err(_) => {
            return Err(SkipReason::MissingExport(
                missing_export("plugin_init").to_string(),
            ))
        }
    };

    let result = handle.invoke_guarded("plugin_init", |handle| {
        init.call(&mut handle.store, HOST_API_VERSION as i32)
    });

    match result {
        Ok(0) => Ok(()),
        Ok(code) => Err(SkipReason::InitFailed(code)),
        Err(RuntimeError::Timeout(_)) => Err(SkipReason::InitTimeout),
        Err(RuntimeError::MissingExport(e)) => Err(SkipReason::MissingExport(e)),
        Err(e) => Err(SkipReason::InitTrap(e.to_string())),
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

    // WASM module that writes then reads back a config key via the host
    // imports during init, returning 0 only if the round trip succeeded.
    fn config_roundtrip_wasm(_engine: &Engine) -> Vec<u8> {
        wat_to_bytes(
            r#"
            (module
                (import "host" "host_config_write"
                    (func $write (param i32 i32 i32 i32) (result i32)))
                (import "host" "host_config_read"
                    (func $read (param i32 i32 i32 i32) (result i32)))
                (memory (export "memory") 1)
                (data (i32.const 0) "theme")
                (data (i32.const 16) "dark")
                (func (export "plugin_init") (param i32) (result i32)
                    i32.const 0
                    i32.const 5
                    i32.const 16
                    i32.const 4
                    call $write
                    drop
                    i32.const 0
                    i32.const 5
                    i32.const 32
                    i32.const 16
                    call $read
                    i32.const 4
                    i32.eq
                    if (result i32)
                        i32.const 0
                    else
                        i32.const -2
                    end
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

    // WASM module that calls host_ui_invalidate during init.
    fn ui_invalidate_wasm(_engine: &Engine) -> Vec<u8> {
        wat_to_bytes(
            r#"
            (module
                (import "host" "host_ui_invalidate" (func $invalidate))
                (memory (export "memory") 1)
                (func (export "plugin_init") (param i32) (result i32)
                    call $invalidate
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

    // WASM module whose plugin_process reads one input byte, writes it +1
    // to the output buffer, and returns 1 (bytes written).
    fn echo_process_wasm(_engine: &Engine) -> Vec<u8> {
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
                (func (export "plugin_process")
                    (param $inp i32) (param $inlen i32)
                    (param $outp i32) (param $outcap i32)
                    (result i32)
                    (local $in_byte i32)
                    local.get $outcap
                    i32.const 1
                    i32.lt_s
                    if
                        i32.const -3
                        return
                    end
                    local.get $inlen
                    i32.const 1
                    i32.lt_s
                    if
                        i32.const -4
                        return
                    end
                    local.get $inp
                    i32.load8_u
                    local.set $in_byte
                    local.get $outp
                    local.get $in_byte
                    i32.const 1
                    i32.add
                    i32.store8
                    i32.const 1
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

    // WASM module whose plugin_process fails with a negative code.
    fn failing_process_wasm(_engine: &Engine) -> Vec<u8> {
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
                    i32.const -7
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

    // WASM module whose plugin_activate fails with a negative code.
    fn failing_activate_wasm(_engine: &Engine) -> Vec<u8> {
        wat_to_bytes(
            r#"
            (module
                (memory (export "memory") 1)
                (func (export "plugin_init") (param i32) (result i32)
                    i32.const 0
                )
                (func (export "plugin_activate") (result i32)
                    i32.const -5
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

    // WASM module whose plugin_deactivate fails with a negative code.
    fn failing_deactivate_wasm(_engine: &Engine) -> Vec<u8> {
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
                    i32.const -8
                )
                (func (export "plugin_unload")
                )
            )
            "#,
        )
    }

    // WASM module whose plugin_init spins forever (used for timeout/fuel).
    fn infinite_loop_wasm(_engine: &Engine) -> Vec<u8> {
        wat_to_bytes(
            r#"
            (module
                (memory (export "memory") 1)
                (func $spin (result i32)
                    (loop (result i32)
                        br 0
                    )
                )
                (func (export "plugin_init") (param i32) (result i32)
                    call $spin
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

    // Same manifest as test_manifest but with explicit resource limits.
    fn manifest_with_resources(timeout_ms: u32, max_cpu_ms: u32) -> PluginManifest {
        parse_manifest(&format!(
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

[plugin.resources]
timeout_ms = {timeout_ms}
max_cpu_ms = {max_cpu_ms}
"#
        ))
        .expect("test manifest should parse")
    }

    // Same manifest as test_manifest but with the ui capability enabled.
    fn manifest_with_ui() -> PluginManifest {
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

[plugin.capabilities]
ui = true
"#,
        )
        .expect("test manifest should parse")
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

    #[test]
    fn host_config_write_read_roundtrip() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = config_roundtrip_wasm(runtime.engine());
        let result = runtime.load_plugin_from_bytes(&wasm, test_manifest());
        assert!(
            result.is_loaded(),
            "config roundtrip should succeed: {}",
            result.status_description()
        );
    }

    #[test]
    fn ui_invalidate_without_ui_capability_is_safe() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = ui_invalidate_wasm(runtime.engine());
        let result = runtime.load_plugin_from_bytes(&wasm, test_manifest());
        assert!(
            result.is_loaded(),
            "ui_invalidate without 'ui' capability must remain a no-op: {}",
            result.status_description()
        );
    }

    #[test]
    fn ui_invalidate_with_ui_capability_is_safe() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = ui_invalidate_wasm(runtime.engine());
        let result = runtime.load_plugin_from_bytes(&wasm, manifest_with_ui());
        assert!(
            result.is_loaded(),
            "ui_invalidate with 'ui' capability should succeed: {}",
            result.status_description()
        );
    }

    // ── Lifecycle ─────────────────────────────────────────────────────

    #[test]
    fn lifecycle_activate_process_deactivate_unload() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = echo_process_wasm(runtime.engine());
        let mut handle = runtime
            .load_plugin_from_bytes(&wasm, test_manifest())
            .into_loaded()
            .unwrap()
            .handle;

        assert_eq!(handle.state(), PluginState::Initialized);

        handle.activate().unwrap();
        assert_eq!(handle.state(), PluginState::Active);

        let output = handle.process(&[7]).unwrap();
        assert_eq!(output, vec![8]);

        handle.deactivate().unwrap();
        assert_eq!(handle.state(), PluginState::Inactive);

        handle.unload();
        assert_eq!(handle.state(), PluginState::Unloaded);

        // unload is idempotent
        handle.unload();
        assert_eq!(handle.state(), PluginState::Unloaded);
    }

    #[test]
    fn reactivation_after_deactivate() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = echo_process_wasm(runtime.engine());
        let mut handle = runtime
            .load_plugin_from_bytes(&wasm, test_manifest())
            .into_loaded()
            .unwrap()
            .handle;

        handle.activate().unwrap();
        handle.deactivate().unwrap();
        assert_eq!(handle.state(), PluginState::Inactive);

        handle.activate().unwrap();
        assert_eq!(handle.state(), PluginState::Active);
        assert_eq!(handle.process(&[41]).unwrap(), vec![42]);
    }

    #[test]
    fn activate_is_idempotent() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = echo_process_wasm(runtime.engine());
        let loaded = runtime.load_plugin_from_bytes(&wasm, test_manifest());
        assert!(loaded.is_loaded(), "echo module should load: {:?}", loaded);
        let mut handle = loaded.into_loaded().unwrap().handle;

        handle.activate().unwrap();
        handle.activate().unwrap();
        assert_eq!(handle.state(), PluginState::Active);
    }

    #[test]
    fn process_requires_active_state() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = echo_process_wasm(runtime.engine());
        let mut handle = runtime
            .load_plugin_from_bytes(&wasm, test_manifest())
            .into_loaded()
            .unwrap()
            .handle;

        let err = handle.process(&[1]).unwrap_err();
        assert!(matches!(err, RuntimeError::InvalidState(_)));
        assert_eq!(handle.state(), PluginState::Initialized);
    }

    #[test]
    fn process_error_demotes_to_inactive() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = failing_process_wasm(runtime.engine());
        let mut handle = runtime
            .load_plugin_from_bytes(&wasm, test_manifest())
            .into_loaded()
            .unwrap()
            .handle;

        handle.activate().unwrap();
        let err = handle.process(&[1]).unwrap_err();
        assert!(matches!(
            err,
            RuntimeError::LifecycleError {
                function: "plugin_process",
                ..
            }
        ));
        assert_eq!(handle.state(), PluginState::Inactive);
    }

    #[test]
    fn activate_failure_demotes_to_inactive() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = failing_activate_wasm(runtime.engine());
        let mut handle = runtime
            .load_plugin_from_bytes(&wasm, test_manifest())
            .into_loaded()
            .unwrap()
            .handle;

        let err = handle.activate().unwrap_err();
        assert!(matches!(
            err,
            RuntimeError::LifecycleError {
                function: "plugin_activate",
                ..
            }
        ));
        assert_eq!(handle.state(), PluginState::Inactive);
    }

    #[test]
    fn deactivate_failure_unloads_plugin() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = failing_deactivate_wasm(runtime.engine());
        let mut handle = runtime
            .load_plugin_from_bytes(&wasm, test_manifest())
            .into_loaded()
            .unwrap()
            .handle;

        handle.activate().unwrap();
        let err = handle.deactivate().unwrap_err();
        assert!(matches!(
            err,
            RuntimeError::LifecycleError {
                function: "plugin_deactivate",
                ..
            }
        ));
        assert_eq!(handle.state(), PluginState::Unloaded);
    }

    #[test]
    fn deactivate_requires_active_state() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = echo_process_wasm(runtime.engine());
        let mut handle = runtime
            .load_plugin_from_bytes(&wasm, test_manifest())
            .into_loaded()
            .unwrap()
            .handle;

        let err = handle.deactivate().unwrap_err();
        assert!(matches!(err, RuntimeError::InvalidState(_)));
    }

    #[test]
    fn activate_requires_initialized_or_inactive() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = echo_process_wasm(runtime.engine());
        let mut handle = runtime
            .load_plugin_from_bytes(&wasm, test_manifest())
            .into_loaded()
            .unwrap()
            .handle;

        handle.unload();
        let err = handle.activate().unwrap_err();
        assert!(matches!(err, RuntimeError::InvalidState(_)));
    }

    // ── Timeout / fuel ────────────────────────────────────────────────

    #[test]
    fn init_timeout_is_enforced() {
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = infinite_loop_wasm(runtime.engine());
        let manifest = manifest_with_resources(100, 200); // 100ms wall-clock
        let result = runtime.load_plugin_from_bytes(&wasm, manifest);
        assert!(
            result.is_skipped(),
            "infinite-loop plugin must not hang the host: {}",
            result.status_description()
        );
        assert!(
            result.status_description().contains("timed out"),
            "expected a timeout skip, got: {}",
            result.status_description()
        );
    }

    #[test]
    fn with_fuel_budget_is_honored() {
        let runtime = WasmPluginRuntime::with_fuel(1_000).unwrap();
        let wasm = infinite_loop_wasm(runtime.engine());
        let manifest = manifest_with_resources(60_000, 200); // generous wall-clock
        let result = runtime.load_plugin_from_bytes(&wasm, manifest);
        assert!(
            result.is_skipped(),
            "tiny fuel budget must bound the infinite loop: {}",
            result.status_description()
        );
        assert!(
            result.status_description().contains("timed out"),
            "expected a timeout skip, got: {}",
            result.status_description()
        );
    }

    #[test]
    fn load_valid_plugin_is_fast() {
        // Regression test: the previous join-based timeout arrested every load
        // for the full timeout value (default 5000ms).
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = minimal_wasm_bytes(runtime.engine());
        let start = Instant::now();
        for _ in 0..5 {
            let result = runtime.load_plugin_from_bytes(&wasm, test_manifest());
            assert!(result.is_loaded());
        }
        assert!(
            start.elapsed() < Duration::from_millis(2000),
            "load_plugin should not block for the full timeout"
        );
    }

    #[test]
    fn instantiate_with_epoch_interruption_does_not_trap_immediately() {
        // Regression (wasmtime >= 32): a fresh store with epoch interruption
        // enabled starts with its deadline at the current epoch, so
        // instantiation trapped with `interrupt` before any lifecycle call
        // could arm a deadline. The runtime must arm a far-future deadline
        // after store creation; every host-import test module (config
        // roundtrip, ui_invalidate, host_log, host_time) covers the same
        // path, but this asserts the contract explicitly on a bare module.
        let runtime = WasmPluginRuntime::new().unwrap();
        let wasm = minimal_wasm_bytes(runtime.engine());
        let result = runtime.load_plugin_from_bytes(&wasm, test_manifest());
        let description = result.status_description();
        assert!(
            result.is_loaded(),
            "instantiation must not trap with `interrupt`: {description}"
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
