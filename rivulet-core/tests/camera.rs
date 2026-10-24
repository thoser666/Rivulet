/// Regression tests for the camera source (#296 milestone file that had no
/// test file of its own; ports the behavioural-contract style of
/// `tests/capture_backend.rs`). Extended for issue #318 (multi-platform
/// camera enumeration).
///
/// Guarantees:
/// - `CameraConfig::default()` is the documented "match the source" zero
///   config: width/height/fps 0 add no caps to the pipeline.
/// - `CameraDevice` and `CameraFrame` are plain data with stable equality.
/// - The provider chains are pinned per platform: Linux v4l2 → PipeWire,
///   Windows Media Foundation → Kernel Streaming → DirectShow, macOS
///   AVFoundation — the Windows/macOS chains are what #318 added; before the
///   fix only the two Linux providers were ever queried and Windows/macOS
///   returned an empty list by construction.
/// - `list_cameras_with()` runs a fallback chain (first provider that finds
///   cameras owns the list, never a merge) and deduplicates device paths over
///   the whole result. The injected-lookup seam lets a **stub provider**
///   produce `list_cameras()`-shaped results with no hardware and no
///   GStreamer registry (the #318 acceptance test).
/// - `device_property_for()` names the element property that carries the
///   device path per factory — a wrong name is a `parse_launch` error at
///   capture start.
/// - The real `list_cameras()` never panics, stays inside its platform's
///   consumer-element set and never lists a device path twice.
/// - `start_camera_capture` never wedges: an unusable device answers as a
///   disconnected channel (the thread logs and exits), and dropping the
///   handle afterwards is a no-op, never a panic.
///
/// Everything here runs headless: no camera is opened for real.
use rivulet_core::camera::{
    device_property_for, list_cameras, list_cameras_with, platform_camera_providers,
    start_camera_capture, CameraConfig, CameraDevice, LINUX_CAMERA_PROVIDERS,
    MACOS_CAMERA_PROVIDERS, WINDOWS_CAMERA_PROVIDERS,
};
use std::collections::HashSet;
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

/// A plausible stub device as a provider would report it.
fn stub_device(name: &str, path: &str) -> CameraDevice {
    CameraDevice {
        name: name.to_string(),
        element_factory: String::from("v4l2src"),
        device_path: path.to_string(),
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
fn platform_provider_chains_are_pinned_per_platform() {
    // Issue #318: all three chains are compiled on every platform so this
    // test pins the Windows and macOS chains even when it runs on Linux.
    assert_eq!(
        LINUX_CAMERA_PROVIDERS,
        ["video4linux2deviceprovider", "pipewiredeviceprovider"],
        "Linux keeps v4l2 first, PipeWire as fallback"
    );
    assert_eq!(
        WINDOWS_CAMERA_PROVIDERS,
        [
            "mfdeviceprovider",
            "ksdeviceprovider",
            "dshowdeviceprovider"
        ],
        "Windows must query Media Foundation, Kernel Streaming and DirectShow — \
         before #318 none of them were queried, so the list was always empty"
    );
    assert_eq!(
        MACOS_CAMERA_PROVIDERS,
        ["avfdeviceprovider"],
        "macOS must query the AVFoundation provider from gst-plugins-bad"
    );

    let active = platform_camera_providers();
    let expected: &[&str] = if cfg!(target_os = "windows") {
        WINDOWS_CAMERA_PROVIDERS
    } else if cfg!(target_os = "macos") {
        MACOS_CAMERA_PROVIDERS
    } else {
        LINUX_CAMERA_PROVIDERS
    };
    assert_eq!(
        active, expected,
        "platform_camera_providers() must select the chain of the compiled OS"
    );
}

#[test]
fn a_stub_provider_feeds_the_camera_list_without_hardware() {
    // #318 acceptance test: a fake provider must be able to produce
    // list_cameras()-shaped results with no camera and no GStreamer registry.
    // Without the injection seam this test cannot exist; before the fix,
    // `list_cameras()` on Windows/macOS returned `Vec::new()` unconditionally.
    let cameras = list_cameras_with(&["stub_provider"], |provider| {
        assert_eq!(provider, "stub_provider");
        vec![
            stub_device("Stub Cam A", "/dev/stub0"),
            stub_device("Stub Cam B", "/dev/stub1"),
        ]
    });
    assert_eq!(cameras.len(), 2, "both stub devices reach the caller");
    assert_eq!(cameras[0].name, "Stub Cam A", "provider order is preserved");
    assert_eq!(cameras[1].device_path, "/dev/stub1");

    // An empty provider falls through to the next one in the chain.
    let cameras = list_cameras_with(&["nothing_here", "stub_provider"], |provider| {
        if provider == "nothing_here" {
            Vec::new()
        } else {
            vec![stub_device("Stub Cam C", "/dev/stub2")]
        }
    });
    assert_eq!(cameras.len(), 1, "the first non-empty provider wins");
    assert_eq!(cameras[0].name, "Stub Cam C");
}

#[test]
fn the_fallback_chain_never_mixes_two_providers_and_dedups_paths() {
    // Fallback, not merge: if two providers describe the same physical
    // cameras (v4l2 vs PipeWire), mixing them would double every entry.
    let cameras = list_cameras_with(&["first", "second"], |provider| {
        if provider == "first" {
            vec![stub_device("Seen by first", "/dev/video0")]
        } else {
            vec![stub_device("Seen by second", "/dev/video0")]
        }
    });
    assert_eq!(cameras.len(), 1, "only the first provider's list is used");
    assert_eq!(cameras[0].name, "Seen by first");

    // Path dedup covers the whole result, not just adjacent pairs — the old
    // `dedup_by` kept a repeat that another device separated.
    let cameras = list_cameras_with(&["only"], |_| {
        vec![
            stub_device("A", "/dev/video0"),
            stub_device("B", "/dev/video1"),
            stub_device("A again", "/dev/video0"),
        ]
    });
    let paths: Vec<&str> = cameras.iter().map(|c| c.device_path.as_str()).collect();
    assert_eq!(paths, ["/dev/video0", "/dev/video1"], "duplicates collapse");
    let mut unique: HashSet<&str> = HashSet::new();
    assert!(
        paths.iter().all(|p| unique.insert(p)),
        "no device path may appear twice"
    );

    // An all-empty chain returns an empty list, never a panic.
    assert!(list_cameras_with(&["a", "b"], |_| Vec::new()).is_empty());
}

#[test]
fn device_property_mapping_is_pinned() {
    // A wrong property name is a `parse_launch` parse error the moment a
    // camera is opened, so each consumer element's device property is pinned.
    assert_eq!(device_property_for("v4l2src"), "device");
    assert_eq!(device_property_for("pipewiresrc"), "path");
    assert_eq!(device_property_for("ksvideosrc"), "device-path");
    assert_eq!(device_property_for("mfvideosrc"), "device-path");
    assert_eq!(device_property_for("dshowvideosrc"), "device");
    assert_eq!(device_property_for("avfvideosrc"), "unique-id");
    assert_eq!(
        device_property_for("some_future_element"),
        "device",
        "unknown factories keep the historical `device` property"
    );
}

#[test]
fn list_cameras_stays_inside_the_platform_consumer_set() {
    let cameras = list_cameras();

    let allowed: &[&str] = if cfg!(target_os = "windows") {
        &["mfvideosrc", "ksvideosrc", "dshowvideosrc"]
    } else if cfg!(target_os = "macos") {
        &["avfvideosrc"]
    } else {
        &["v4l2src", "pipewiresrc"]
    };
    for camera in &cameras {
        assert!(
            allowed.contains(&camera.element_factory.as_str()),
            "on this platform only {:?} may appear, got {:?}",
            allowed,
            camera.element_factory
        );
        assert!(
            !camera.name.is_empty(),
            "a listed camera must carry a display name"
        );
    }

    // On Linux the two providers describe the same physical cameras: they
    // must still appear as a fallback chain, never mixed in one list.
    #[cfg(target_os = "linux")]
    {
        let uses_v4l2 = cameras.iter().any(|c| c.element_factory == "v4l2src");
        let uses_pipewire = cameras.iter().any(|c| c.element_factory == "pipewiresrc");
        assert!(
            !(uses_v4l2 && uses_pipewire),
            "the providers are a fallback chain, not a merge"
        );
    }

    // Device paths are unique over the whole list (the dedup contract).
    let mut seen: HashSet<&str> = HashSet::new();
    for camera in &cameras {
        assert!(
            seen.insert(camera.device_path.as_str()),
            "the same device must not be listed twice: {}",
            camera.device_path
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
