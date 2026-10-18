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

/// Trait abstracting over different capture backend implementations.
///
/// Concrete implementations provide platform-specific capture (e.g.
/// xcap on Linux, PipeWire on Linux/Wayland, Metal/WGC on macOS/Windows).
pub trait CaptureBackend: Send + Sync + 'static {
    /// List windows that can be captured.
    fn list_windows(&self) -> Vec<GameWindow>;

    /// Start capturing frames from the given window.
    /// Returns a receiver for frames and a handle to stop the capture.
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
        #[cfg(target_os = "windows")]
        {
            // Probe for Metal or WGC availability - in a full implementation,
            // this would check for the relevant Windows APIs.
        }
        #[cfg(not(target_os = "windows"))]
        {
            return Err(CaptureError::BackendUnavailable(
                "Metal/WGC backend is Windows-only",
            ));
        }
        Ok(Self {})
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
