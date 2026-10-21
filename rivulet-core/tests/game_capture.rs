/// Regression tests for the game capture module (#296 milestone file that
/// had no test file of its own; ports the behavioural-contract style of
/// `tests/capture_backend.rs`).
///
/// Guarantees:
/// - `GameCaptureConfig::default()` keeps the documented fallback contract
///   (fps 0 = match the game's FPS with 60 as fallback, cursor on).
/// - `start_game_capture` answers with a live channel + handle; dropping
///   the handle stops the capture thread and disconnects the channel — no
///   leaked thread, no wedge — on every platform.
/// - `start_vulkan_layer_capture` keeps its platform contract: Windows
///   answers `Some` (stub thread), every other platform `None`.
/// - `list_game_windows` keeps the heuristic of its filter (non-empty
///   title, strictly larger than 640x480) and stays empty on Windows,
///   where the GUI supplies its own window list.
/// - `list_windows_with_default_backend` is the platform-selected
///   delegation (empty on Windows while the Metal/WGC backend does not
///   enumerate the xcap way).
///
/// Everything here runs headless: no window is ever opened for real, the
/// capture threads are stopped through their handles immediately.
use rivulet_core::game_capture::{
    list_game_windows, list_windows_with_default_backend, start_game_capture,
    start_vulkan_layer_capture, GameCaptureConfig, GameCaptureFrame, GameWindow,
};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// A window for stub calls; `start_game_capture` matches on `id` only and
/// never reads the geometry, so a 1x1 placeholder is enough.
fn dummy_window() -> GameWindow {
    GameWindow {
        id: 0,
        title: String::from("game-capture-test"),
        width: 1,
        height: 1,
    }
}

/// Drain a capture channel until the thread has exited (channel
/// disconnected). Frames may still arrive between the stop request and the
/// thread's next loop check — that is fine; the contract is that the
/// channel *ends*, promptly, once the handle is gone.
fn drain_until_disconnected(rx: &mpsc::Receiver<GameCaptureFrame>, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(_frame) => continue, // live frame before the stop landed — fine
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                assert!(
                    Instant::now() < deadline,
                    "{what}: the capture thread must exit and drop its sender after the handle is dropped"
                );
            }
        }
    }
}

#[test]
fn config_defaults_pin_the_fallback_contract() {
    let config = GameCaptureConfig::default();
    assert_eq!(
        config.fps, 0,
        "0 must mean: match the game's FPS (60 is the fallback inside start_game_capture)"
    );
    assert!(
        config.capture_cursor,
        "the cursor is captured by default — streamers expect it on"
    );
}

#[test]
fn window_and_frame_are_plain_data_structures() {
    let window = dummy_window();
    let cloned = window.clone();
    assert_eq!(window.id, cloned.id);
    assert_eq!(window.title, cloned.title);
    assert_eq!(window.width, cloned.width);
    assert_eq!(window.height, cloned.height);

    // A frame carries its own geometry next to the raw bytes.
    let frame = GameCaptureFrame {
        data: vec![0u8; 2 * 2 * 4],
        width: 2,
        height: 2,
    };
    assert_eq!(
        frame.data.len(),
        (frame.width * frame.height * 4) as usize,
        "RGBA frames are width*height*4 bytes"
    );
}

#[test]
fn dropping_the_handle_stops_the_capture_thread() {
    let (rx, handle) = start_game_capture(&dummy_window(), &GameCaptureConfig::default());
    drop(handle);
    drain_until_disconnected(&rx, "game capture");
}

#[test]
fn vulkan_layer_capture_keeps_its_platform_contract() {
    #[cfg(target_os = "windows")]
    {
        let (rx, handle) =
            start_vulkan_layer_capture(30).expect("Windows answers Some (stub layer thread)");
        drop(handle);
        drain_until_disconnected(&rx, "vulkan layer capture");
    }
    #[cfg(not(target_os = "windows"))]
    {
        assert!(
            start_vulkan_layer_capture(30).is_none(),
            "the Vulkan layer is Windows-only; other platforms must get None, \
             not a silently streaming stub"
        );
    }
}

#[test]
fn list_game_windows_keeps_the_filter_heuristic() {
    let windows = list_game_windows();
    #[cfg(target_os = "windows")]
    assert!(
        windows.is_empty(),
        "the in-process Windows enumerator returns nothing — the GUI supplies \
         its own window list (same pin as the metal-wgc backend)"
    );
    for window in &windows {
        assert!(
            !window.title.is_empty(),
            "untitled windows are not capture candidates"
        );
        assert!(
            window.width > 640 && window.height > 480,
            "the filter admits only windows strictly larger than 640x480, got {}x{}",
            window.width,
            window.height
        );
    }
}

#[test]
fn list_windows_with_default_backend_delegates_to_capture_backend() {
    let windows = list_windows_with_default_backend();
    #[cfg(target_os = "windows")]
    assert!(
        windows.is_empty(),
        "delegates to the Metal/WGC backend, which does not enumerate the xcap way"
    );
    #[cfg(not(target_os = "windows"))]
    let _ = windows; // real enumeration depends on the host session; never panics
}
