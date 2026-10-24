use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::collections::HashSet;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

// Camera enumeration across platforms (issue #318).
//
// `list_cameras()` used to query only the two Linux device providers
// (`video4linux2deviceprovider`, `pipewiredeviceprovider`), so on Windows and
// macOS — where neither factory exists — it returned an empty list no matter
// how many cameras the system saw. Each platform now has its own provider
// chain, probed in order; the first provider that actually returns devices
// wins (a fallback chain, never a merge — v4l2 and PipeWire describe the same
// physical cameras in different path formats and would double them up).

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraDevice {
    pub name: String,
    pub element_factory: String,
    pub device_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CameraConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

#[derive(Debug, Clone)]
pub struct CameraFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// GStreamer device providers on Linux, in probe order: Video4Linux2 first
/// (it carries the `/dev/videoN` path `v4l2src` wants), PipeWire only when
/// v4l2 found nothing.
pub const LINUX_CAMERA_PROVIDERS: &[&str] =
    &["video4linux2deviceprovider", "pipewiredeviceprovider"];

/// GStreamer device providers on Windows, in probe order: Media Foundation
/// (the modern API — it lists physical *and* virtual cameras, verified against
/// a live system on 2026-10-24), then Kernel Streaming (`ksvideosrc`), then
/// DirectShow (`dshowvideosrc`, rank `none`, last resort for cameras only it
/// can see).
pub const WINDOWS_CAMERA_PROVIDERS: &[&str] = &[
    "mfdeviceprovider",
    "ksdeviceprovider",
    "dshowdeviceprovider",
];

/// GStreamer device providers on macOS: AVFoundation, shipped by
/// gst-plugins-bad's `applemedia` plugin since GStreamer 1.20.
pub const MACOS_CAMERA_PROVIDERS: &[&str] = &["avfdeviceprovider"];

/// The provider chain for the platform this binary was built for. Any OS that
/// is neither Windows nor macOS falls back to the Linux chain (v4l2/PipeWire
/// are the only camera providers there).
pub fn platform_camera_providers() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        WINDOWS_CAMERA_PROVIDERS
    } else if cfg!(target_os = "macos") {
        MACOS_CAMERA_PROVIDERS
    } else {
        LINUX_CAMERA_PROVIDERS
    }
}

/// Which element property of `element_factory` carries the device path from
/// [`CameraDevice::device_path`]. The providers hand out platform paths
/// (`/dev/video0`, a PipeWire node path, a KS/MF symbolic link, an AVFoundation
/// unique id) that only the matching property of the consuming element
/// understands — `start_camera_capture` builds the `parse_launch` string from
/// this table, so it is pinned by `tests/camera.rs`.
pub fn device_property_for(element_factory: &str) -> &'static str {
    match element_factory {
        // `device` is v4l2's path property (the historical default), and it
        // is also what dshowvideosrc calls its "@device:..." moniker path.
        "v4l2src" => "device",
        "dshowvideosrc" => "device",
        // pipewiresrc has no `device` property at all; it is called `path`.
        "pipewiresrc" => "path",
        // Kernel Streaming wants the symbolic link, not an index or name.
        "ksvideosrc" => "device-path",
        // Media Foundation hands out `@device:sw:{...}` paths and its source
        // takes them under the same name (gst-launch's suggestion line prints
        // `mfvideosrc device-path='...'`).
        "mfvideosrc" => "device-path",
        // AVFoundation devices are addressed by AVCaptureDevice unique id
        // (`device-index` is deprecated and non-functional since 1.30).
        "avfvideosrc" => "unique-id",
        // Unknown elements keep the historical behaviour: `device`.
        _ => "device",
    }
}

/// Enumerate cameras on this platform (issue #318).
///
/// Probes [`platform_camera_providers`] in order and returns the devices of
/// the first provider that finds any. Never mixes two providers in one list.
pub fn list_cameras() -> Vec<CameraDevice> {
    let _ = gst::init();
    list_cameras_with(platform_camera_providers(), gst_devices_for_provider)
}

/// Injection seam for [`list_cameras`]: probe an explicit provider chain with
/// an arbitrary lookup instead of GStreamer. This is what lets the regression
/// tests feed a stub provider on a machine without cameras (or without a
/// GStreamer registry), while the fallback-chain contract stays under test.
pub fn list_cameras_with<F>(providers: &[&str], mut lookup: F) -> Vec<CameraDevice>
where
    F: FnMut(&str) -> Vec<CameraDevice>,
{
    for provider in providers {
        let devices = lookup(provider);
        if !devices.is_empty() {
            // Fallback chain: the first provider that sees cameras owns the
            // list. Deduplicate by device path across the whole result, not
            // just adjacent entries (the old `dedup_by` missed a repeat that
            // was separated by another device).
            let mut seen: HashSet<String> = HashSet::new();
            let mut unique = Vec::new();
            for device in devices {
                if seen.insert(device.device_path.clone()) {
                    unique.push(device);
                }
            }
            return unique;
        }
    }
    Vec::new()
}

/// GStreamer lookup for one device provider: `None` (unknown factory) yields
/// an empty list so the chain moves on to the next provider.
fn gst_devices_for_provider(provider: &str) -> Vec<CameraDevice> {
    let Some(factory) = gst::DeviceProviderFactory::by_name(provider) else {
        return Vec::new();
    };
    let mut devices = Vec::new();
    for device in factory.devices() {
        let class = device.device_class().to_string();
        if !(class.contains("Video") || class.contains("Source")) {
            continue;
        }
        devices.push(CameraDevice {
            name: device.display_name().to_string(),
            element_factory: element_factory_for(&device, provider),
            device_path: device.property::<String>("device-path"),
        });
    }
    devices
}

/// The element that consumes a device: whatever the provider itself would
/// instantiate (`GstDevice` knows its own factory), falling back to the
/// provider's documented element when that factory cannot be created here
/// (plugin missing from a minimal install).
fn element_factory_for(device: &gst::Device, provider: &str) -> String {
    if let Ok(element) = device.create_element(None::<&str>) {
        if let Some(factory) = element.factory() {
            return factory.name().to_string();
        }
    }
    default_factory_for_provider(provider).to_string()
}

fn default_factory_for_provider(provider: &str) -> &'static str {
    match provider {
        "video4linux2deviceprovider" => "v4l2src",
        "pipewiredeviceprovider" => "pipewiresrc",
        "mfdeviceprovider" => "mfvideosrc",
        "ksdeviceprovider" => "ksvideosrc",
        "dshowdeviceprovider" => "dshowvideosrc",
        "avfdeviceprovider" => "avfvideosrc",
        _ => "v4l2src",
    }
}

pub fn start_camera_capture(
    device: &CameraDevice,
    config: &CameraConfig,
) -> (mpsc::Receiver<CameraFrame>, CameraCaptureHandle) {
    let _ = gst::init();
    let (tx, rx) = mpsc::channel::<CameraFrame>();
    let stop = Arc::new(Mutex::new(false));
    let stop_clone = stop.clone();

    let device_path = device.device_path.clone();
    let factory_name = device.element_factory.clone();
    let width = config.width;
    let height = config.height;
    let fps = config.fps;

    let handle = CameraCaptureHandle { stop };

    thread::spawn(move || {
        let mut caps_str = "video/x-raw,format=RGBA".to_string();
        if width > 0 {
            caps_str.push_str(&format!(",width={}", width));
        }
        if height > 0 {
            caps_str.push_str(&format!(",height={}", height));
        }
        if fps > 0 {
            caps_str.push_str(&format!(",framerate={}/1", fps));
        }

        // Each element names its device property differently (v4l2src `device`,
        // pipewiresrc `path`, ksvideosrc `device-path`, avfvideosrc `unique-id`);
        // a wrong name is a parse error, so the property comes from the table
        // instead of being hard-coded (issue #318).
        let device_property = device_property_for(&factory_name);
        let pipeline_str = format!(
            "{} {}=\"{}\" ! videoconvert ! capsfilter caps=\"{}\" ! appsink name=camera_sink",
            factory_name, device_property, device_path, caps_str
        );

        let pipeline = match gst::parse::launch(&pipeline_str) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(error = ?e, "Camera pipeline creation failed");
                return;
            }
        };

        let pipeline = pipeline.dynamic_cast::<gst::Pipeline>().unwrap();
        let appsink = pipeline
            .by_name("camera_sink")
            .unwrap()
            .dynamic_cast::<gst_app::AppSink>()
            .unwrap();

        appsink.set_property("drop", true);
        appsink.set_property("max-buffers", 1u32);

        if pipeline.set_state(gst::State::Playing).is_err() {
            tracing::error!("Camera pipeline failed to enter Playing state");
            return;
        }

        loop {
            if *stop_clone.lock().unwrap() {
                break;
            }

            match appsink.try_pull_sample(gst::ClockTime::from_mseconds(33)) {
                Some(sample) => {
                    if let Some(buffer) = sample.buffer() {
                        if let Ok(map) = buffer.map_readable() {
                            let data = map.as_slice().to_vec();
                            if let Some(caps) = sample.caps() {
                                if let Some(structure) = caps.structure(0) {
                                    let w = structure.get::<i32>("width").unwrap_or(0) as u32;
                                    let h = structure.get::<i32>("height").unwrap_or(0) as u32;
                                    if w > 0 && h > 0 && !data.is_empty() {
                                        let _ = tx.send(CameraFrame {
                                            data,
                                            width: w,
                                            height: h,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                None => {
                    thread::sleep(std::time::Duration::from_millis(5));
                }
            }
        }

        let _ = pipeline.set_state(gst::State::Null);
    });

    (rx, handle)
}

pub struct CameraCaptureHandle {
    stop: Arc<Mutex<bool>>,
}

impl Drop for CameraCaptureHandle {
    fn drop(&mut self) {
        *self.stop.lock().unwrap() = true;
    }
}
