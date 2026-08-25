use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

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

pub fn list_cameras() -> Vec<CameraDevice> {
    let _ = gst::init();
    let mut cameras = Vec::new();

    if let Some(provider) = gst::DeviceProviderFactory::by_name("video4linux2deviceprovider") {
        let devices = provider.devices();
        for device in &devices {
            let display_name = device.display_name().to_string();
            let device_path = device.property::<String>("device-path");
            let class = device.device_class().to_string();
            if class.contains("Video") || class.contains("Source") {
                cameras.push(CameraDevice {
                    name: display_name,
                    element_factory: "v4l2src".to_string(),
                    device_path,
                });
            }
        }
    }

    if cameras.is_empty() {
        if let Some(provider) = gst::DeviceProviderFactory::by_name("pipewiredeviceprovider") {
            let devices = provider.devices();
            for device in &devices {
                let display_name = device.display_name().to_string();
                let device_path = device.property::<String>("device-path");
                let class = device.device_class().to_string();
                if class.contains("Video") || class.contains("Source") {
                    cameras.push(CameraDevice {
                        name: display_name,
                        element_factory: "pipewiresrc".to_string(),
                        device_path,
                    });
                }
            }
        }
    }

    cameras.dedup_by(|a, b| a.device_path == b.device_path);
    cameras
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

        let pipeline_str = format!(
            "{} device=\"{}\" ! videoconvert ! capsfilter caps=\"{}\" ! appsink name=camera_sink",
            factory_name, device_path, caps_str
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
