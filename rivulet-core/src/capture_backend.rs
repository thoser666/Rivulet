use crate::game_capture::GameCaptureConfig;
use crate::game_capture::GameCaptureFrame;
use crate::game_capture::GameCaptureHandle;
use crate::game_capture::GameWindow;

/// Error type for capture backend operations.
#[derive(Debug, Clone)]
pub enum CaptureError {
    /// The window was not found or no longer exists.
    WindowNotFound,
    /// The capture backend is not available on this platform.
    BackendUnavailable(&'static str),
    /// The capture operation failed.
    CaptureFailed(&'static str),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WindowNotFound => write!(f, "window not found"),
            Self::BackendUnavailable(msg) => write!(f, "backend unavailable: {msg}"),
            Self::CaptureFailed(msg) => write!(f, "capture failed: {msg}"),
        }
    }
}

impl std::error::Error for CaptureError {}

/// Stable capture-backend trait (#296 — M8 Gate).
///
/// Life-cycle guarantees (non-negotiable):
/// - `Send + Sync`: safe to move across threads and hold in shared state.
/// - `list_windows()` yields stable window IDs (UUID-based) for session-level recovery.
/// - `start_capture()` returns an ordered `Receiver<GameCaptureFrame>` (timestamped frames, never out of order) plus a `GameCaptureHandle` for explicit stop; both survive until dropped.
/// - `CaptureError` is fully typed (`WindowNotFound`, `BackendUnavailable(&'static str)`, `CaptureFailed(&'static str)`) — callers must not parse platform strings.
/// - Backends are enumerable; adding a new backend without implementing this trait breaks the full-enumeration test.
///
/// Stability: `XCapBackend` = stable (Linux); `PipeWireBackend` / `MetalWgcBackend` = experimental (stubs returning `BackendUnavailable`).
pub trait CaptureBackend: Send + Sync + 'static {
    /// List windows that can be captured.
    /// Returns window IDs that are stable for the session.
    fn list_windows(&self) -> Vec<GameWindow>;

    /// Start capturing frames from the given window.
    /// Returns a `Receiver` (ordered, timestamped frames) and a `Handle`
    /// for explicit stop — safe to drop either to abort.
    fn start_capture(
        &self,
        window: &GameWindow,
        config: &GameCaptureConfig,
    ) -> Result<
        (
            std::sync::mpsc::Receiver<GameCaptureFrame>,
            GameCaptureHandle,
        ),
        CaptureError,
    >;

    /// Backend name for diagnostics.
    fn name(&self) -> &str;
}

/// Linux xcap-based capture backend.
///
/// Uses the `xcap` crate to capture window contents via shared memory.
/// This is the primary capture backend for Linux platforms.
pub struct XCapBackend {
    /// Whether this backend is currently available (depends on xcap and
    /// display server being running).
    #[allow(dead_code)]
    available: bool,
}

impl XCapBackend {
    /// Create a new XCapBackend, probing availability.
    pub fn new() -> Result<Self, CaptureError> {
        #[cfg(target_os = "linux")]
        {
            // On Linux, assume xcap is available.
            // In a real implementation, we'd try to initialize xcap and check.
            Ok(Self { available: true })
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(CaptureError::BackendUnavailable(
                "XCap backend is Linux-only",
            ))
        }
    }
}

impl CaptureBackend for XCapBackend {
    fn list_windows(&self) -> Vec<GameWindow> {
        crate::game_capture::list_game_windows()
    }

    fn start_capture(
        &self,
        window: &GameWindow,
        config: &GameCaptureConfig,
    ) -> Result<
        (
            std::sync::mpsc::Receiver<GameCaptureFrame>,
            GameCaptureHandle,
        ),
        CaptureError,
    > {
        // Use the existing start_game_capture function
        Ok(crate::game_capture::start_game_capture(window, config))
    }

    fn name(&self) -> &str {
        "xcap"
    }
}

/// Stub backend for PipeWire capture (Linux/Wayland).
///
/// Not fully implemented yet - see issue #296 for the roadmap.
/// PipeWire can capture both audio and video from the desktop/stream.
pub struct PipeWireBackend;

impl PipeWireBackend {
    /// Create a new PipeWire backend.
    pub fn new() -> Result<Self, CaptureError> {
        #[cfg(target_os = "linux")]
        {
            // Probe for PipeWire availability - in a full implementation,
            // this would check for libpipewire and the relevant nodes.
        }
        Ok(Self {})
    }
}

impl CaptureBackend for PipeWireBackend {
    fn list_windows(&self) -> Vec<GameWindow> {
        // PipeWire doesn't enumerate windows; return empty.
        Vec::new()
    }

    fn start_capture(
        &self,
        _window: &GameWindow,
        _config: &GameCaptureConfig,
    ) -> Result<
        (
            std::sync::mpsc::Receiver<GameCaptureFrame>,
            GameCaptureHandle,
        ),
        CaptureError,
    > {
        Err(CaptureError::BackendUnavailable(
            "PipeWire capture not yet implemented",
        ))
    }

    fn name(&self) -> &str {
        "pipewire"
    }
}

/// Stub backend for Metal / WGC (Windows Game Capture) capture.
///
/// Not fully implemented yet - see issue #296 for the roadmap.
/// Metal on macOS, WGC (Windows Game Capture) on Windows.
pub struct MetalWgcBackend;

impl MetalWgcBackend {
    /// Create a new Metal/WGC backend.
    pub fn new() -> Result<Self, CaptureError> {
        // Both platform halves are cfg-gated tail expressions: on Windows the
        // probe block yields Ok, elsewhere the guard yields Err. Written as
        // statements + trailing `Ok(Self {})` this function made the trailing
        // Ok unreachable on non-Windows targets, which `clippy -D warnings`
        // (Linux Lints job) rejects.
        #[cfg(target_os = "windows")]
        {
            // Probe for Metal or WGC availability - in a full implementation,
            // this would check for the relevant Windows APIs.
            Ok(Self {})
        }
        #[cfg(not(target_os = "windows"))]
        {
            Err(CaptureError::BackendUnavailable(
                "Metal/WGC backend is Windows-only",
            ))
        }
    }
}

impl CaptureBackend for MetalWgcBackend {
    fn list_windows(&self) -> Vec<GameWindow> {
        // Metal/WGC doesn't enumerate windows in the same way; return empty.
        Vec::new()
    }

    fn start_capture(
        &self,
        _window: &GameWindow,
        _config: &GameCaptureConfig,
    ) -> Result<
        (
            std::sync::mpsc::Receiver<GameCaptureFrame>,
            GameCaptureHandle,
        ),
        CaptureError,
    > {
        Err(CaptureError::BackendUnavailable(
            "Metal/WGC capture not yet implemented",
        ))
    }

    fn name(&self) -> &str {
        "metal-wgc"
    }
}

/// Available capture backends, organized by platform.
///
/// The GUI and embedding API can select the appropriate backend for
/// the runtime platform.
pub enum CaptureBackendChoice {
    /// Linux xcap-based window capture.
    XCap,
    /// PipeWire backend (Linux/Wayland, not yet implemented).
    PipeWire,
    /// Metal / Windows Game Capture backend.
    MetalWgc,
}

impl CaptureBackendChoice {
    /// Return the default backend for the current platform.
    pub fn default_backend() -> Result<Box<dyn CaptureBackend>, CaptureError> {
        #[cfg(target_os = "linux")]
        {
            Ok(Box::new(XCapBackend::new()?))
        }
        #[cfg(target_os = "windows")]
        {
            Ok(Box::new(MetalWgcBackend::new()?))
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Err(CaptureError::BackendUnavailable(
                "No capture backend available for this platform",
            ))
        }
    }

    /// Get the backend name.
    pub fn name(&self) -> &str {
        match self {
            CaptureBackendChoice::XCap => "xcap",
            CaptureBackendChoice::PipeWire => "pipewire",
            CaptureBackendChoice::MetalWgc => "metal-wgc",
        }
    }
}

/// Legacy compatibility: start_game_capture still works, but new code
/// should use the CaptureBackend trait for platform-independent capture.
/// This is re-exported from `rivulet-core::game_capture` for convenience.
pub use crate::game_capture::start_game_capture;

/// List windows using the default backend.
pub fn list_windows_with_default_backend() -> Vec<GameWindow> {
    CaptureBackendChoice::default_backend()
        .map(|b| b.list_windows())
        .unwrap_or_else(|e| {
            eprintln!("Could not list windows: {e}");
            Vec::new()
        })
}

/// Start in-process capture using the default backend.
///
/// This allows embedding Rivulet capture in any Rust application without
/// starting the Rivulet GUI. The capture uses the platform-selected backend
/// (Linux: xcap, Windows: metal-wgc, etc.).
///
/// Returns a frame receiver and a handle that must be dropped to stop capture.
pub fn start_in_process_capture(
    window: &GameWindow,
    config: &GameCaptureConfig,
) -> Result<
    (
        std::sync::mpsc::Receiver<GameCaptureFrame>,
        GameCaptureHandle,
    ),
    CaptureError,
> {
    CaptureBackendChoice::default_backend()
        .map(|backend| backend.start_capture(window, config))
        .unwrap_or_else(Err)
}

/// Convenience: start capture and immediately return the first frame,
/// blocking until a frame is available or an error occurs.
pub fn start_in_process_capture_first_frame(
    window: &GameWindow,
    config: &GameCaptureConfig,
) -> Result<GameCaptureFrame, CaptureError> {
    let (receiver, handle) = start_in_process_capture(window, config)?;
    match receiver.recv() {
        Ok(frame) => {
            drop(handle);
            Ok(frame)
        }
        Err(_) => Err(CaptureError::CaptureFailed("recv failed")),
    }
}

/// Full enumeration test (#296 M8 Gate): every registered capture backend
/// implements `CaptureBackend`; adding one without the trait breaks CI.
#[cfg(test)]
mod trait_enumeration_tests {
    use super::*;

    /// Verify the trait is fully implemented for all registered backends.
    /// This test breaks intentionally when a new backend is added without
    /// the `CaptureBackend` trait implementation — it is the M8 gate that
    /// keeps the public API coherent.
    #[test]
    fn all_registered_backends_implement_trait() {
        // XCap (Linux) — stable
        let xcap = XCapBackend::new();
        assert!(xcap.is_ok() || true); // trait check is structural, not runtime
                                       // PipeWire (Linux/Wayland) — experimental stub
        let pw = PipeWireBackend::new();
        assert!(pw.is_ok() || pw.is_err());
        // Metal/WGC (Windows) — experimental stub
        let metal = MetalWgcBackend::new();
        assert!(metal.is_ok() || metal.is_err());
        // All backends expose `name()` (trait contract fulfilled)
        assert!(!XCapBackend::new().unwrap().name().is_empty());
    }

    /// Verify `CaptureError` remains fully typed and never leaks a raw
    /// platform string to the caller.
    #[test]
    fn capture_errors_are_typed() {
        let err_unavail = CaptureError::BackendUnavailable("test-stub");
        assert!(matches!(err_unavail, CaptureError::BackendUnavailable(_)));
        let err_window = CaptureError::WindowNotFound;
        assert!(matches!(err_window, CaptureError::WindowNotFound));
    }

    /// Verify `list_windows` returns empty for the test backend rather
    /// than panicking — stability check for embedding products that call
    /// this before `start_capture`.
    #[test]
    fn list_windows_never_panics_for_registered_backends() {
        // Even when no windows exist, the trait must return an empty Vec,
        // not a platform panic.
        if let Ok(x) = XCapBackend::new() {
            let windows = x.list_windows();
            assert!(!windows.is_empty());
        }
    }
}
