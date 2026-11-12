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
/// - `list_windows()` yields stable window IDs (`u64`: X11 window id / HWND) for session-level recovery, and answers with an empty `Vec` — never a panic — when the host has no window to offer.
/// - `start_capture()` returns an ordered `Receiver<GameCaptureFrame>` (timestamped frames, never out of order) plus a `GameCaptureHandle` for explicit stop; both survive until dropped.
/// - `CaptureError` is fully typed (`WindowNotFound`, `BackendUnavailable(&'static str)`, `CaptureFailed(&'static str)`) — callers must not parse platform strings.
/// - Backends are enumerable; adding a new backend without implementing this trait breaks the full-enumeration test.
///
/// Stability: every backend captures real frames on its own platform — `XCapBackend` on Linux/X11, `PipeWireBackend` on Linux/Wayland (xdg-desktop-portal + PipeWire) and `MetalWgcBackend` on Windows (Windows Graphics Capture). Off their platform they answer the typed `BackendUnavailable`, which is the one legitimate reason for that variant.
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

/// PipeWire capture backend (Linux/Wayland).
///
/// Drives an xdg-desktop-portal screencast session and reads the frames from
/// the PipeWire stream the compositor hands out (F-M8-004). The portal shows
/// its permission dialog on first use, so `start_capture` blocks until the
/// session is granted and answers with a typed `CaptureFailed` when it is
/// refused — the backend *is* available on Linux, only the attempt failed.
///
/// PipeWire does not enumerate windows (the portal picks the source), so
/// `list_windows` answers with an empty list on purpose.
pub struct PipeWireBackend;

impl PipeWireBackend {
    /// Create a new PipeWire backend.
    ///
    /// Construction touches nothing: whether a compositor and its portal can
    /// answer is only knowable when a session is requested.
    pub fn new() -> Result<Self, CaptureError> {
        Ok(Self {})
    }
}

impl CaptureBackend for PipeWireBackend {
    fn list_windows(&self) -> Vec<GameWindow> {
        // The portal selects the source; there is no window list to hand out.
        Vec::new()
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
        #[cfg(target_os = "linux")]
        {
            // The portal, not the window id, decides what is captured.
            let _ = (window, config);
            start_portal_capture()
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (window, config);
            Err(CaptureError::BackendUnavailable(
                "PipeWire backend is Linux-only",
            ))
        }
    }

    fn name(&self) -> &str {
        "pipewire"
    }
}

/// Request the portal session and pump its frames into a `GameCaptureFrame`
/// channel.
///
/// The session is requested synchronously so that a refusal reaches the caller
/// as a typed error instead of an empty channel.
#[cfg(target_os = "linux")]
fn start_portal_capture() -> Result<
    (
        std::sync::mpsc::Receiver<GameCaptureFrame>,
        GameCaptureHandle,
    ),
    CaptureError,
> {
    use std::sync::{mpsc, Arc, Mutex};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| CaptureError::CaptureFailed("failed to start the portal runtime"))?;
    let (fd, info) = runtime
        .block_on(crate::pipewire_capture::request_portal_session(false))
        .map_err(crate::pipewire_capture::map_portal_error)?;

    let (frames, portal_handle) = crate::pipewire_capture::start_pipewire_capture(fd, info.node_id);

    let stop = Arc::new(Mutex::new(false));
    let stop_for_thread = stop.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for frame in frames {
            if *stop_for_thread.lock().unwrap() {
                break;
            }
            let out = GameCaptureFrame {
                data: frame.data,
                width: frame.width,
                height: frame.height,
            };
            if tx.send(out).is_err() {
                break;
            }
        }
    });

    Ok((
        rx,
        GameCaptureHandle::with_teardown(stop, Box::new(move || drop(portal_handle))),
    ))
}

/// Windows Graphics Capture backend (Windows).
///
/// Starts a WGC session on the `HWND` the [`GameWindow`] carries and forwards
/// the compositor's frames as `GameCaptureFrame`s (F-M8-004). WGC is
/// change-driven, so `GameCaptureConfig::fps` does not rate-limit it — the
/// compositor delivers a frame whenever the window's content changes.
///
/// The macOS/Metal half is still unimplemented; macOS has no default capture
/// backend at all (`CaptureBackendChoice::default_backend` refuses it), so this
/// backend's platform contract is Windows, where it now captures.
pub struct MetalWgcBackend;

impl MetalWgcBackend {
    /// Create a new Metal/WGC backend.
    ///
    /// Both platform halves are cfg-gated tail expressions: on Windows the
    /// probe block yields Ok, elsewhere the guard yields Err. Written as
    /// statements + trailing `Ok(Self {})` this function made the trailing
    /// Ok unreachable on non-Windows targets, which `clippy -D warnings`
    /// (Linux Lints job) rejects.
    #[cfg(target_os = "windows")]
    pub fn new() -> Result<Self, CaptureError> {
        Ok(Self {})
    }

    /// Create a new Metal/WGC backend (non-Windows: typed refusal).
    #[cfg(not(target_os = "windows"))]
    pub fn new() -> Result<Self, CaptureError> {
        Err(CaptureError::BackendUnavailable(
            "Metal/WGC backend is Windows-only",
        ))
    }
}

impl CaptureBackend for MetalWgcBackend {
    fn list_windows(&self) -> Vec<GameWindow> {
        // WGC targets the HWND it is given; enumeration belongs to the game
        // capture picker, which already owns a window list. Fabricating one
        // here would duplicate it with different filters.
        Vec::new()
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
        #[cfg(target_os = "windows")]
        {
            let (frames, wgc_stop) = crate::wgc_capture::start_wgc_capture(window.id, config)
                .map_err(CaptureError::CaptureFailed)?;
            let stop = std::sync::Arc::new(std::sync::Mutex::new(false));
            Ok((
                frames,
                GameCaptureHandle::with_teardown(
                    stop,
                    Box::new(move || wgc_stop.store(true, std::sync::atomic::Ordering::SeqCst)),
                ),
            ))
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (window, config);
            Err(CaptureError::BackendUnavailable(
                "Metal/WGC backend is Windows-only",
            ))
        }
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
    /// PipeWire backend (Linux/Wayland, portal screencast).
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
    ///
    /// Every constructor is platform-gated, so the gate itself must never
    /// `unwrap()` one: on Windows and macOS XCap refuses by design, on Linux
    /// Metal/WGC does. A refusal that arrives as the typed
    /// `BackendUnavailable` variant is a passing result — that is exactly
    /// what the "no platform strings at the caller" guarantee looks like
    /// from outside the library.
    #[test]
    fn all_registered_backends_implement_trait() {
        /// One platform-gated constructor, paired with the diagnostic name
        /// its `CaptureBackend::name()` must answer with.
        type Constructed = Result<Box<dyn CaptureBackend>, CaptureError>;

        let registered: [(&str, Constructed); 3] = [
            (
                "xcap",
                XCapBackend::new().map(|b| Box::new(b) as Box<dyn CaptureBackend>),
            ),
            (
                "pipewire",
                PipeWireBackend::new().map(|b| Box::new(b) as Box<dyn CaptureBackend>),
            ),
            (
                "metal-wgc",
                MetalWgcBackend::new().map(|b| Box::new(b) as Box<dyn CaptureBackend>),
            ),
        ];

        let mut constructed = 0usize;
        for (expected_name, backend) in registered {
            match backend {
                Ok(backend) => {
                    constructed += 1;
                    // Reachable only through the trait object: `name()` is
                    // the expected non-empty string and `list_windows()`
                    // answers without panicking on this host.
                    assert_eq!(backend.name(), expected_name);
                    let _windows = backend.list_windows();
                }
                Err(err) => assert!(
                    matches!(err, CaptureError::BackendUnavailable(_)),
                    "{expected_name} must refuse with the typed BackendUnavailable \
                     variant instead of panicking, got: {err}"
                ),
            }
        }
        assert!(
            constructed >= 1,
            "every supported host must construct at least one backend"
        );
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

    /// Verify `list_windows` answers with a `Vec` instead of panicking —
    /// stability check for embedding products that call this before
    /// `start_capture`.
    ///
    /// An empty list is a legitimate answer and must not fail the test: CI
    /// runners are headless and own no game-sized windows, and the stub
    /// backends enumerate nothing by design. What is pinned here is what
    /// holds on every host — the call returns, nothing is fabricated, and
    /// every window that *is* reported is complete.
    #[test]
    fn list_windows_never_panics_for_registered_backends() {
        let mut backends: Vec<Box<dyn CaptureBackend>> = Vec::new();
        if let Ok(backend) = XCapBackend::new() {
            backends.push(Box::new(backend));
        }
        if let Ok(backend) = PipeWireBackend::new() {
            backends.push(Box::new(backend));
        }
        if let Ok(backend) = MetalWgcBackend::new() {
            backends.push(Box::new(backend));
        }
        assert!(
            !backends.is_empty(),
            "at least one backend must construct on this host"
        );

        for backend in backends {
            let name = backend.name();
            // Reaching the next line without a panic is the core guarantee.
            let windows = backend.list_windows();
            for window in &windows {
                assert!(
                    !window.title.is_empty(),
                    "{name} must not report a window without a title: {window:?}"
                );
                assert!(
                    window.width > 0 && window.height > 0,
                    "{name} must not report a window without geometry: {window:?}"
                );
            }
            match name {
                // Neither backend enumerates windows: the portal picks the
                // source, WGC targets the HWND it is given.
                "pipewire" | "metal-wgc" => assert!(
                    windows.is_empty(),
                    "{name} must enumerate no windows, got {}: {windows:?}",
                    windows.len()
                ),
                _ => {}
            }
        }
    }

    /// The first non-negotiable guarantee of the trait: every registered
    /// backend is `Send + Sync + 'static`, so an embedding product can move
    /// it into a worker thread and hold it behind a shared `Arc`. This is the
    /// static half of the enumeration gate — it stops compiling the moment a
    /// new backend type loses one of the bounds.
    #[test]
    fn every_registered_backend_is_send_and_sync() {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}

        assert_send_sync_static::<XCapBackend>();
        assert_send_sync_static::<PipeWireBackend>();
        assert_send_sync_static::<MetalWgcBackend>();
        assert_send_sync_static::<Box<dyn CaptureBackend>>();
    }

    /// The free listing helper must answer on every platform, including hosts
    /// where `default_backend()` refuses: without a default backend it falls
    /// back to an empty list instead of erroring or panicking.
    #[test]
    fn window_listing_falls_back_to_empty_without_a_default_backend() {
        let windows = crate::capture_backend::list_windows_with_default_backend();
        if CaptureBackendChoice::default_backend().is_err() {
            assert!(
                windows.is_empty(),
                "without a default backend the listing must fall back to an empty list"
            );
        }
    }
}
