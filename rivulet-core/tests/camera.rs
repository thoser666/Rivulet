/// Regression tests for the camera source (#296 milestone file that had no
/// test file of its own; ports the behavioural-contract style of
/// `tests/capture_backend.rs`).
///
/// Guarantees:
/// - `CameraConfig::default()` is the documented "match the source" zero
///   config: width/height/fps 0 add no caps to the pipeline.
/// - `CameraDevice` and `CameraFrame` are plain data with stable equality.
/// - `list_cameras()` keeps the provider contract: v4l2 first, PipeWire
///   only as fallback (never mixed), `element_factory` from exactly that
///   known set, and no device path listed twice after dedup. Windows has
///   neither provider, so the list must be empty there.
/// - `start_camera_capture` never wedges: an unusable device answers as a
///   disconnected channel (the thread logs and exits), and dropping the
///   handle afterwards is a no-op, never a panic.
///
/// Everything here runs headless: no camera is opened for real.
use rivulet_core::camera::{list_cameras, start_camera_capture, CameraConfig, CameraDevice};
use std::sync::mpsc;
use std::time::Duration;

/// A device whose factory does not exist — the cheapest deterministic
/// failure path of `start_camera_capture` on every platform.
fn ghost_device() -> CameraDevice {
    CameraDevice {
        name: String::from("ghost-camera"),
        element_factory: String::from("rivulet_no_such_element"),
        device_path: String::from("ghost-device"),
    }
}

#[test]
fn config_default_is_the_zero_match_the_source_contract() {
    let config = CameraConfig::default();
    assert_eq!(config.width, 0, "0 must mean: keep the source's width");
    assert_eq!(config.height, 0, "0 must mean: keep the source's height");
    assert_eq!(config.fps, 0, "0 must mean: keep the source's frame rate");
}

#[test]
fn device_and_frame_are_plain_data_with_stable_equality() {
    let device = CameraDevice {
        name: String::from("Integrated Camera"),
        element_factory: String::from("v4l2src"),
        device_path: String::from("/dev/video0"),
    };
    let same = device.clone();
    assert_eq!(device, same, "equality compares all three fields");
    let other = CameraDevice {
        device_path: String::from("/dev/video1"),
        ..device.clone()
    };
    assert_ne!(
        device, other,
        "two cameras differ exactly when their device paths differ"
    );

    let frame = rivulet_core::camera::CameraFrame {
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
fn list_cameras_keeps_the_provider_contract() {
    let cameras = list_cameras();

    #[cfg(target_os = "windows")]
    assert!(
        cameras.is_empty(),
        "neither the v4l2 nor the PipeWire device provider exists on Windows"
    );

    // v4l2 first, PipeWire only as fallback — never both in one list.
    let uses_v4l2 = cameras.iter().any(|c| c.element_factory == "v4l2src");
    let uses_pipewire = cameras.iter().any(|c| c.element_factory == "pipewiresrc");
    assert!(
        !(uses_v4l2 && uses_pipewire),
        "the providers are a fallback chain, not a merge: v4l2 first, PipeWire only when empty"
    );
    for camera in &cameras {
        assert!(
            camera.element_factory == "v4l2src" || camera.element_factory == "pipewiresrc",
            "only the two known providers may appear, got {}",
            camera.element_factory
        );
    }

    // dedup_by contract: adjacent entries never share a device path.
    for pair in cameras.windows(2) {
        assert_ne!(
            pair[0].device_path, pair[1].device_path,
            "the same device must not be listed twice"
        );
    }
}

#[test]
fn a_bad_device_disconnects_instead_of_wedging_the_thread() {
    let config = CameraConfig {
        width: 320,
        height: 240,
        fps: 30,
    };
    let (rx, handle) = start_camera_capture(&ghost_device(), &config);

    // The parse attempt fails for the unknown factory; the thread logs and
    // exits, which drops its sender. Either outcome that is *not* allowed:
    // producing frames from a device that cannot exist, or hanging until
    // the test timeout.
    match rx.recv_timeout(Duration::from_secs(5)) {
        Err(mpsc::RecvTimeoutError::Disconnected) => {}
        Ok(frame) => panic!("a device that cannot exist must not produce frames: {frame:?}"),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            panic!("the capture thread must exit when the device cannot be created")
        }
    }

    drop(handle); // the handle outlives the thread — dropping must be a no-op
}
