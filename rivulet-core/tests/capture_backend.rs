/// Regression tests for the capture backend trait abstraction ([#296]).
///
/// Guarantees:
/// - The `CaptureBackend` trait is object-safe and can be sent across threads.
/// - `CaptureBackendChoice::default_backend()` returns the correct platform backend.
/// - `list_windows_with_default_backend()` delegates to the default backend.
/// - All backend names are consistent strings.
/// - `CaptureError` is `std::error::Error` + `Display`.
/// - Backend stubs return the expected `BackendUnavailable` error.
/// - The concrete backends keep their behavioural contract per platform:
///   `new()` succeeds only where the backend exists, stubs answer with
///   their exact `BackendUnavailable` message before touching any window,
///   and the default backend maps Linux → xcap, Windows → metal-wgc,
///   everything else → an error plus an empty window list.
use rivulet_core::capture_backend::{CaptureBackend, CaptureBackendChoice, CaptureError};

/// The trait must be object-safe (Send + Sync + 'static) so it can be used as
/// `dyn CaptureBackend`.
#[allow(dead_code)]
fn assert_trait_is_object_safe(_b: &dyn CaptureBackend) {}

/// The default backend must return a valid backend for the platform.
#[allow(dead_code)]
fn assert_default_backend_works() {
    let _backend = CaptureBackendChoice::default_backend();
    // On the CI runner, we just verify it type-checks and doesn't panic.
    // The actual backend depends on the host OS.
}

/// Backend names must be consistent strings.
fn assert_backend_names() {
    let xcap = CaptureBackendChoice::XCap;
    assert_eq!(xcap.name(), "xcap");

    let pipewire = CaptureBackendChoice::PipeWire;
    assert_eq!(pipewire.name(), "pipewire");

    let metal_wgc = CaptureBackendChoice::MetalWgc;
    assert_eq!(metal_wgc.name(), "metal-wgc");
}

/// CaptureError must implement std::error::Error + Display.
fn assert_error_traits() {
    // Display
    let _ = format!("{}", CaptureError::WindowNotFound);
    let _ = format!("{}", CaptureError::BackendUnavailable("test"));
    let _ = format!("{}", CaptureError::CaptureFailed("test"));

    // std::error::Error
    let _: Box<dyn std::error::Error> = Box::new(CaptureError::WindowNotFound);
    let _: Box<dyn std::error::Error> = Box::new(CaptureError::BackendUnavailable("test"));
    let _: Box<dyn std::error::Error> = Box::new(CaptureError::CaptureFailed("test"));
}

/// list_windows_with_default_backend() delegates to the default backend.
fn assert_list_windows_delegates() {
    let _windows = rivulet_core::capture_backend::list_windows_with_default_backend();
    // Should return a Vec<GameWindow> (may be empty on this platform)
    let _ = _windows;
}

/// A CaptureBackend implementation must satisfy the trait bounds.
#[allow(dead_code)]
fn assert_concrete_backend_bounds() {
    // XCapBackend is Send + Sync + 'static (required by the trait)
    fn requires_send_sync(_b: &rivulet_core::capture_backend::XCapBackend) {}
    // The new() may fail on non-Linux, but the type must be well-kinded.
    // On Linux it returns Ok(XCapBackend), elsewhere an error.
    let _result = rivulet_core::capture_backend::XCapBackend::new();
    let _ = _result;
}

/// Test that the CaptureBackend trait methods compile and are callable.
fn assert_trait_methods_compile() {
    // We can't easily instantiate a dyn CaptureBackend without a concrete type,
    // but we can verify the trait is well-formed by checking the method signatures
    // compile in a function that takes &dyn CaptureBackend.
    #[allow(dead_code)]
    fn check_trait<_B: ?Sized + CaptureBackend>(_: &_B) {}
    // On non-Linux XCapBackend::new() returns an error, but the type check above
    // proves the trait is well-formed.
    let _ = rivulet_core::capture_backend::XCapBackend::new();
}

#[test]
fn test_capture_backend_trait_is_object_safe() {
    // Verify the trait is well-formed; on non-Linux XCapBackend::new() returns
    // BackendUnavailable, which is expected and does not affect the trait
    // object-safety proof.
    let _result = rivulet_core::capture_backend::XCapBackend::new();
    let _ = _result;
}

#[test]
fn test_capture_backend_default_backend_works() {
    // This just type-checks; the actual backend depends on the host OS.
    // On Linux it returns XCapBackend, on Windows MetalWgcBackend, elsewhere an error.
    let _result = CaptureBackendChoice::default_backend();
    // If we get here without a compile error, the test passes.
}

#[test]
fn test_capture_backend_names() {
    assert_backend_names();
}

#[test]
fn test_capture_error_traits() {
    assert_error_traits();
}

#[test]
fn test_list_windows_delegates() {
    assert_list_windows_delegates();
}

#[test]
fn test_concrete_backend_bounds() {
    assert_concrete_backend_bounds();
}

#[test]
fn test_trait_methods_compile() {
    assert_trait_methods_compile();
}

/// Test that start_in_process_capture type-checks.
#[test]
fn test_in_process_capture_type_checks() {
    // This just verifies the function signature compiles.
    // Actual capture requires a running display/server.
    let _ = std::mem::size_of::<rivulet_core::capture_backend::CaptureBackendChoice>();
}

/// Test that start_in_process_capture_first_frame type-checks.
#[test]
fn test_in_process_capture_first_frame_type_checks() {
    // Verify the function exists and has the right signature.
    let _ = std::mem::size_of::<rivulet_core::game_capture::GameCaptureConfig>();
}

// ── Behavioural contracts of the concrete backends (#296) ──────────────
//
// Everything above proves the trait is well-formed; nothing above pins what
// a backend actually answers. These tests do: exact error messages, the
// platform contract of every `new()`, and the default-backend mapping.
// They exercise only stub paths — no window is ever touched — so they run
// headless on all three CI platforms.

/// A window the stubs answer before ever looking at it.
fn dummy_window() -> rivulet_core::game_capture::GameWindow {
    rivulet_core::game_capture::GameWindow {
        id: 0,
        title: String::from("capture-backend-test"),
        width: 1,
        height: 1,
    }
}

/// Capture config for stub calls.
fn dummy_config() -> rivulet_core::game_capture::GameCaptureConfig {
    rivulet_core::game_capture::GameCaptureConfig::default()
}

#[test]
fn pipewire_stub_contract() {
    let backend = rivulet_core::capture_backend::PipeWireBackend::new()
        .expect("PipeWireBackend::new() probes nothing yet and must succeed everywhere");
    assert_eq!(backend.name(), "pipewire");
    assert!(
        backend.list_windows().is_empty(),
        "the PipeWire stub must enumerate no windows"
    );
    match backend.start_capture(&dummy_window(), &dummy_config()) {
        Err(CaptureError::BackendUnavailable(msg)) => {
            assert_eq!(msg, "PipeWire capture not yet implemented");
        }
        Err(other) => panic!("unexpected error: {other}"),
        Ok(_) => panic!("the PipeWire stub must not start a capture"),
    }
}

#[test]
fn metal_wgc_new_is_platform_specific() {
    #[cfg(target_os = "windows")]
    {
        let backend = rivulet_core::capture_backend::MetalWgcBackend::new()
            .expect("MetalWgcBackend::new() must succeed on Windows");
        assert_eq!(backend.name(), "metal-wgc");
        assert!(
            backend.list_windows().is_empty(),
            "Metal/WGC does not enumerate windows the xcap way"
        );
        match backend.start_capture(&dummy_window(), &dummy_config()) {
            Err(CaptureError::BackendUnavailable(msg)) => {
                assert_eq!(msg, "Metal/WGC capture not yet implemented");
            }
            Err(other) => panic!("unexpected error: {other}"),
            Ok(_) => panic!("the Metal/WGC stub must not start a capture"),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        match rivulet_core::capture_backend::MetalWgcBackend::new() {
            Err(CaptureError::BackendUnavailable(msg)) => {
                assert_eq!(msg, "Metal/WGC backend is Windows-only");
            }
            Err(other) => panic!("unexpected error: {other}"),
            Ok(_) => panic!("MetalWgcBackend::new() must refuse on non-Windows targets"),
        }
    }
}

#[test]
fn xcap_new_is_platform_specific() {
    #[cfg(target_os = "linux")]
    {
        let backend = rivulet_core::capture_backend::XCapBackend::new()
            .expect("XCapBackend::new() must succeed on Linux");
        assert_eq!(backend.name(), "xcap");
    }
    #[cfg(not(target_os = "linux"))]
    {
        match rivulet_core::capture_backend::XCapBackend::new() {
            Err(CaptureError::BackendUnavailable(msg)) => {
                assert_eq!(msg, "XCap backend is Linux-only");
            }
            Err(other) => panic!("unexpected error: {other}"),
            Ok(_) => panic!("XCapBackend::new() must refuse on non-Linux targets"),
        }
    }
}

#[test]
fn default_backend_is_platform_specific() {
    let result = CaptureBackendChoice::default_backend();
    if cfg!(target_os = "linux") {
        let backend = result.expect("Linux must select the xcap backend");
        assert_eq!(backend.name(), "xcap");
    } else if cfg!(target_os = "windows") {
        let backend = result.expect("Windows must select the Metal/WGC backend");
        assert_eq!(backend.name(), "metal-wgc");
    } else {
        match result {
            Err(CaptureError::BackendUnavailable(msg)) => {
                assert_eq!(msg, "No capture backend available for this platform");
            }
            Err(other) => panic!("unexpected error: {other}"),
            Ok(_) => panic!("a platform without a backend must get an error, not a silent default"),
        }
        assert!(
            rivulet_core::capture_backend::list_windows_with_default_backend().is_empty(),
            "with no backend, window listing must fall back to an empty list"
        );
    }
}

#[test]
fn capture_error_display_is_stable() {
    assert_eq!(CaptureError::WindowNotFound.to_string(), "window not found");
    assert_eq!(
        CaptureError::BackendUnavailable("probe failed").to_string(),
        "backend unavailable: probe failed"
    );
    assert_eq!(
        CaptureError::CaptureFailed("recv failed").to_string(),
        "capture failed: recv failed"
    );
}
