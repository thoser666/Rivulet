use anyhow::Result;
use rivulet_core::AudioFrame;

/// Configuration for audio capture.
#[derive(Debug, Clone)]
pub struct AudioConfig {
    /// Capture the system/desktop audio (what you hear).
    pub capture_system: bool,
    /// Capture the microphone input.
    pub capture_mic: bool,
    /// Volume applied to the system audio source in `[0.0, 1.0]`.
    pub system_volume: f32,
    /// Volume applied to the microphone source in `[0.0, 1.0]`.
    pub mic_volume: f32,
    /// Output sample rate in Hz.
    pub sample_rate: u32,
    /// Output channel count.
    pub channels: u16,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            capture_system: true,
            capture_mic: true,
            system_volume: 1.0,
            mic_volume: 1.0,
            sample_rate: 48_000,
            channels: 2,
        }
    }
}

/// Callback invoked with each mixed audio frame.
pub type AudioFrameCallback = Box<dyn FnMut(AudioFrame) + Send + 'static>;

/// Captures and mixes system audio and microphone input.
///
/// The mixing happens inside the GStreamer pipeline (using the `adder`
/// element); the resulting interleaved `f32` PCM is delivered through the
/// callback passed to [`AudioCapture::start`].
pub struct AudioCapture {
    inner: sys_impl::AudioCaptureInner,
}

impl AudioCapture {
    /// Create a new capture session from the given configuration.
    pub fn new(config: AudioConfig) -> Result<Self> {
        Ok(Self {
            inner: sys_impl::AudioCaptureInner::new(&config)?,
        })
    }

    /// Start capturing and deliver mixed frames to `on_frame`.
    pub fn start(&mut self, on_frame: AudioFrameCallback) -> Result<()> {
        self.inner.start(on_frame)
    }

    /// Stop capturing and join the capture thread.
    pub fn stop(&mut self) -> Result<()> {
        self.inner.stop()
    }

    pub fn is_running(&self) -> bool {
        self.inner.is_running()
    }

    /// Change the system audio volume.
    pub fn set_system_volume(&self, volume: f32) {
        self.inner.set_system_volume(volume);
    }

    /// Change the microphone volume.
    pub fn set_mic_volume(&self, volume: f32) {
        self.inner.set_mic_volume(volume);
    }
}

#[cfg(target_os = "linux")]
mod sys_impl {
    use super::*;
    use gst::prelude::*;
    use gstreamer as gst;
    use gstreamer_app as gst_app;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};

    pub struct AudioCaptureInner {
        config: AudioConfig,
        pipeline: gst::Pipeline,
        appsink: gst_app::AppSink,
        sys_vol: Option<gst::Element>,
        mic_vol: Option<gst::Element>,
        running: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl AudioCaptureInner {
        pub fn new(config: &AudioConfig) -> Result<Self> {
            let _ = gst::init();

            let mut pipeline_str = String::new();
            let mut inputs = 0usize;

            if config.capture_system {
                let device = system_device_name();
                let device_attr = match &device {
                    Some(d) if !d.is_empty() => format!(" device=\"{}\"", d),
                    _ => String::new(),
                };
                pipeline_str.push_str(&format!(
                    "pulsesrc name=sys_in{} do-timestamp=true \
                     ! volume name=sys_vol \
                     ! audioconvert ! audioresample \
                     ! capsfilter caps=\"audio/x-raw,format=F32LE,layout=interleaved,channels={},rate={}\" \
                     ! queue ! adder. ",
                    device_attr, config.channels, config.sample_rate
                ));
                inputs += 1;
            }

            if config.capture_mic {
                pipeline_str.push_str(&format!(
                    "pulsesrc name=mic_in do-timestamp=true \
                     ! volume name=mic_vol \
                     ! audioconvert ! audioresample \
                     ! capsfilter caps=\"audio/x-raw,format=F32LE,layout=interleaved,channels={},rate={}\" \
                     ! queue ! adder. ",
                    config.channels, config.sample_rate
                ));
                inputs += 1;
            }

            if inputs == 0 {
                anyhow::bail!("No audio input sources enabled");
            }

            pipeline_str
                .push_str("adder name=adder ! appsink name=out_sink emit-signals=false sync=false");

            let pipeline = gst::parse::launch(&pipeline_str)
                .map_err(|e| anyhow::anyhow!("Failed to build audio pipeline: {}", e))?
                .downcast::<gst::Pipeline>()
                .expect("parsed element is a pipeline");

            let sys_vol = pipeline.by_name("sys_vol");
            let mic_vol = pipeline.by_name("mic_vol");

            if let Some(v) = &sys_vol {
                v.set_property("volume", config.system_volume as f64);
            }
            if let Some(v) = &mic_vol {
                v.set_property("volume", config.mic_volume as f64);
            }

            let appsink = pipeline
                .by_name("out_sink")
                .ok_or_else(|| anyhow::anyhow!("Missing appsink in audio pipeline"))?
                .downcast::<gst_app::AppSink>()
                .expect("element is an appsink");

            Ok(Self {
                config: config.clone(),
                pipeline,
                appsink,
                sys_vol,
                mic_vol,
                running: Arc::new(AtomicBool::new(false)),
                thread: None,
            })
        }

        pub fn start(&mut self, on_frame: AudioFrameCallback) -> Result<()> {
            if self.running.load(Ordering::SeqCst) {
                return Ok(());
            }
            self.running.store(true, Ordering::SeqCst);

            self.pipeline.set_state(gst::State::Playing)?;

            let appsink = self.appsink.clone();
            let running = Arc::clone(&self.running);
            let sample_rate = self.config.sample_rate;
            let channels = self.config.channels;
            let callback = Arc::new(Mutex::new(on_frame));

            let handle = thread::spawn(move || {
                while running.load(Ordering::SeqCst) {
                    let sample = appsink.try_pull_sample(Some(gst::ClockTime::from_mseconds(200)));
                    if let Some(sample) = sample {
                        let Some(buffer) = sample.buffer() else {
                            continue;
                        };
                        let Ok(map) = buffer.map_readable() else {
                            continue;
                        };
                        let bytes = map.as_slice();
                        let count = bytes.len() / 4;
                        let mut data = Vec::with_capacity(count);
                        for i in 0..count {
                            let off = i * 4;
                            data.push(f32::from_ne_bytes([
                                bytes[off],
                                bytes[off + 1],
                                bytes[off + 2],
                                bytes[off + 3],
                            ]));
                        }
                        let frame = AudioFrame::new(data, sample_rate, channels);
                        if let Ok(mut cb) = callback.lock() {
                            cb(frame);
                        }
                    }
                }
            });

            self.thread = Some(handle);
            Ok(())
        }

        pub fn stop(&mut self) -> Result<()> {
            if !self.running.load(Ordering::SeqCst) {
                return Ok(());
            }
            self.running.store(false, Ordering::SeqCst);
            self.pipeline.set_state(gst::State::Null)?;
            if let Some(handle) = self.thread.take() {
                let _ = handle.join();
            }
            Ok(())
        }

        pub fn is_running(&self) -> bool {
            self.running.load(Ordering::SeqCst)
        }

        pub fn set_system_volume(&self, volume: f32) {
            if let Some(v) = &self.sys_vol {
                v.set_property("volume", f64::from(volume.clamp(0.0, 1.0)));
            }
        }

        pub fn set_mic_volume(&self, volume: f32) {
            if let Some(v) = &self.mic_vol {
                v.set_property("volume", f64::from(volume.clamp(0.0, 1.0)));
            }
        }
    }

    /// Resolve the PulseAudio monitor source of the default sink, i.e. the
    /// "what you hear" capture device. Falls back to the default source.
    fn system_device_name() -> Option<String> {
        let output = std::process::Command::new("pactl")
            .arg("get-default-sink")
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let sink = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if sink.is_empty() {
            return None;
        }
        Some(format!("{}.monitor", sink))
    }
}

#[cfg(not(target_os = "linux"))]
mod sys_impl {
    use super::*;

    pub struct AudioCaptureInner {
        _private: (),
    }

    impl AudioCaptureInner {
        pub fn new(_config: &AudioConfig) -> Result<Self> {
            anyhow::bail!("Audio capture is currently only supported on Linux")
        }

        pub fn start(&mut self, _on_frame: AudioFrameCallback) -> Result<()> {
            anyhow::bail!("Audio capture is currently only supported on Linux")
        }

        pub fn stop(&mut self) -> Result<()> {
            Ok(())
        }

        pub fn is_running(&self) -> bool {
            false
        }

        pub fn set_system_volume(&self, _volume: f32) {}

        pub fn set_mic_volume(&self, _volume: f32) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_captures_system_and_mic() {
        let config = AudioConfig::default();
        assert!(config.capture_system);
        assert!(config.capture_mic);
        assert_eq!(config.sample_rate, 48_000);
        assert_eq!(config.channels, 2);
        assert_eq!(config.system_volume.to_bits(), 1.0f32.to_bits());
        assert_eq!(config.mic_volume.to_bits(), 1.0f32.to_bits());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn capture_rejects_config_without_sources() {
        let config = AudioConfig {
            capture_system: false,
            capture_mic: false,
            ..Default::default()
        };
        assert!(AudioCapture::new(config).is_err());
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn capture_is_not_supported_off_linux() {
        assert!(AudioCapture::new(AudioConfig::default()).is_err());
    }
}
