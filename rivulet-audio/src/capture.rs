use anyhow::Result;
use rivulet_core::AudioFrame;

/// Filter chain applied to an input track before it is mixed or recorded.
///
/// The filters are inserted into the GStreamer pipeline between the volume
/// element and the caps filter. They are realised with the `webrtcdsp`
/// (noise suppression) and `audiodynamic` (compressor/limiter) elements.
#[derive(Debug, Clone, Default)]
pub struct AudioFilters {
    /// Noise suppression (WebRTC audio processing library).
    pub noise_suppression: bool,
    /// Compressor (dynamic range compression, ratio ~4:1).
    pub compressor: bool,
    /// Hard limiter (prevents clipping, ratio ~20:1).
    pub limiter: bool,
}

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
    /// Filters applied to the system audio track.
    pub system_filters: AudioFilters,
    /// Filters applied to the microphone track.
    pub mic_filters: AudioFilters,
    /// Send the system track to the default audio output for monitoring.
    pub system_monitor: bool,
    /// Send the microphone track to the default audio output for monitoring.
    pub mic_monitor: bool,
    /// Master volume of the monitoring output in `[0.0, 1.0]`.
    pub monitor_volume: f32,
    /// Output sample rate in Hz.
    pub sample_rate: u32,
    /// Output channel count.
    pub channels: u16,
    /// Deliver system and microphone audio as separate streams instead of a
    /// single mixed stream. Use [`AudioCapture::start_separated`] then.
    pub separate_tracks: bool,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            capture_system: true,
            capture_mic: true,
            system_volume: 1.0,
            mic_volume: 1.0,
            system_filters: AudioFilters::default(),
            mic_filters: AudioFilters::default(),
            system_monitor: false,
            mic_monitor: false,
            monitor_volume: 1.0,
            sample_rate: 48_000,
            channels: 2,
            separate_tracks: false,
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

    /// Start capturing and deliver system and microphone frames separately.
    ///
    /// Only valid when [`AudioConfig::separate_tracks`] is enabled; otherwise
    /// an error is returned.
    pub fn start_separated(
        &mut self,
        system_cb: AudioFrameCallback,
        mic_cb: AudioFrameCallback,
    ) -> Result<()> {
        self.inner.start_separated(system_cb, mic_cb)
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

    /// Change the master monitoring volume.
    pub fn set_monitor_volume(&self, volume: f32) {
        self.inner.set_monitor_volume(volume);
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
        appsink: Option<gst_app::AppSink>,
        appsink_sys: Option<gst_app::AppSink>,
        appsink_mic: Option<gst_app::AppSink>,
        sys_vol: Option<gst::Element>,
        mic_vol: Option<gst::Element>,
        sys_mon_vol: Option<gst::Element>,
        mic_mon_vol: Option<gst::Element>,
        running: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl AudioCaptureInner {
        pub fn new(config: &AudioConfig) -> Result<Self> {
            let _ = gst::init();

            let caps_filter = format!(
                "capsfilter caps=\"audio/x-raw,format=F32LE,layout=interleaved,channels={},rate={}\"",
                config.channels, config.sample_rate
            );

            let system_src = if config.capture_system {
                let device = system_device_name();
                let device_attr = match &device {
                    Some(d) if !d.is_empty() => format!(" device=\"{}\"", d),
                    _ => String::new(),
                };
                Some(format!(
                    "pulsesrc name=sys_in{} do-timestamp=true",
                    device_attr
                ))
            } else {
                None
            };
            let mic_src = if config.capture_mic {
                Some("pulsesrc name=mic_in do-timestamp=true".to_string())
            } else {
                None
            };

            if system_src.is_none() && mic_src.is_none() {
                anyhow::bail!("No audio input sources enabled");
            }

            let mut pipeline_str = String::new();
            if config.separate_tracks {
                if let Some(src) = &system_src {
                    pipeline_str.push_str(&build_source_branch(
                        src,
                        "sys_vol",
                        &config.system_filters,
                        &caps_filter,
                        "appsink name=sys_sink emit-signals=false sync=false",
                        "sys",
                        config.system_monitor,
                    ));
                }
                if let Some(src) = &mic_src {
                    pipeline_str.push_str(&build_source_branch(
                        src,
                        "mic_vol",
                        &config.mic_filters,
                        &caps_filter,
                        "appsink name=mic_sink emit-signals=false sync=false",
                        "mic",
                        config.mic_monitor,
                    ));
                }
            } else {
                for (i, src) in [system_src.as_ref(), mic_src.as_ref()]
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let (vol, name, filters, monitor) = if i == 0 {
                        (
                            "sys_vol",
                            "sys",
                            &config.system_filters,
                            config.system_monitor,
                        )
                    } else {
                        ("mic_vol", "mic", &config.mic_filters, config.mic_monitor)
                    };
                    pipeline_str.push_str(&build_source_branch(
                        src,
                        vol,
                        filters,
                        &caps_filter,
                        "adder.",
                        name,
                        monitor,
                    ));
                }
                pipeline_str.push_str(
                    "adder name=adder ! appsink name=out_sink emit-signals=false sync=false",
                );
            }

            let pipeline = gst::parse::launch(&pipeline_str)
                .map_err(|e| anyhow::anyhow!("Failed to build audio pipeline: {}", e))?
                .downcast::<gst::Pipeline>()
                .expect("parsed element is a pipeline");

            let sys_vol = pipeline.by_name("sys_vol");
            let mic_vol = pipeline.by_name("mic_vol");
            let sys_mon_vol = pipeline.by_name("sys_mon_vol");
            let mic_mon_vol = pipeline.by_name("mic_mon_vol");

            if let Some(v) = &sys_vol {
                v.set_property("volume", config.system_volume as f64);
            }
            if let Some(v) = &mic_vol {
                v.set_property("volume", config.mic_volume as f64);
            }
            if let Some(v) = &sys_mon_vol {
                v.set_property("volume", config.monitor_volume as f64);
            }
            if let Some(v) = &mic_mon_vol {
                v.set_property("volume", config.monitor_volume as f64);
            }

            let (appsink, appsink_sys, appsink_mic) = if config.separate_tracks {
                (
                    None,
                    if config.capture_system {
                        Some(lookup_appsink(&pipeline, "sys_sink")?)
                    } else {
                        None
                    },
                    if config.capture_mic {
                        Some(lookup_appsink(&pipeline, "mic_sink")?)
                    } else {
                        None
                    },
                )
            } else {
                (Some(lookup_appsink(&pipeline, "out_sink")?), None, None)
            };

            Ok(Self {
                config: config.clone(),
                pipeline,
                appsink,
                appsink_sys,
                appsink_mic,
                sys_vol,
                mic_vol,
                sys_mon_vol,
                mic_mon_vol,
                running: Arc::new(AtomicBool::new(false)),
                thread: None,
            })
        }

        pub fn start(&mut self, on_frame: AudioFrameCallback) -> Result<()> {
            if self.config.separate_tracks {
                anyhow::bail!("Separate audio tracks are enabled; use start_separated()");
            }
            if self.running.load(Ordering::SeqCst) {
                return Ok(());
            }
            self.running.store(true, Ordering::SeqCst);

            self.pipeline.set_state(gst::State::Playing)?;

            let appsink = self.appsink.clone().expect("mixed appsink present");
            let running = Arc::clone(&self.running);
            let sample_rate = self.config.sample_rate;
            let channels = self.config.channels;
            let callback = Arc::new(Mutex::new(on_frame));

            let handle = thread::spawn(move || {
                while running.load(Ordering::SeqCst) {
                    if let Some(frame) = pull_frame(&appsink, sample_rate, channels) {
                        if let Ok(mut cb) = callback.lock() {
                            cb(frame);
                        }
                    }
                }
            });

            self.thread = Some(handle);
            Ok(())
        }

        pub fn start_separated(
            &mut self,
            system_cb: AudioFrameCallback,
            mic_cb: AudioFrameCallback,
        ) -> Result<()> {
            if !self.config.separate_tracks {
                anyhow::bail!("Separate audio tracks are disabled; use start()");
            }
            if self.running.load(Ordering::SeqCst) {
                return Ok(());
            }
            self.running.store(true, Ordering::SeqCst);

            self.pipeline.set_state(gst::State::Playing)?;

            let sys_sink = self.appsink_sys.clone();
            let mic_sink = self.appsink_mic.clone();
            if sys_sink.is_none() && mic_sink.is_none() {
                anyhow::bail!("No audio input sources enabled");
            }

            let running = Arc::clone(&self.running);
            let sample_rate = self.config.sample_rate;
            let channels = self.config.channels;
            let sys_callback = Arc::new(Mutex::new(system_cb));
            let mic_callback = Arc::new(Mutex::new(mic_cb));

            let handle = thread::spawn(move || {
                while running.load(Ordering::SeqCst) {
                    if let Some(sink) = &sys_sink {
                        if let Some(frame) = pull_frame(sink, sample_rate, channels) {
                            if let Ok(mut cb) = sys_callback.lock() {
                                cb(frame);
                            }
                        }
                    }
                    if let Some(sink) = &mic_sink {
                        if let Some(frame) = pull_frame(sink, sample_rate, channels) {
                            if let Ok(mut cb) = mic_callback.lock() {
                                cb(frame);
                            }
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

        pub fn set_monitor_volume(&self, volume: f32) {
            let v = f64::from(volume.clamp(0.0, 1.0));
            if let Some(el) = &self.sys_mon_vol {
                el.set_property("volume", v);
            }
            if let Some(el) = &self.mic_mon_vol {
                el.set_property("volume", v);
            }
        }
    }

    /// Resolve the PulseAudio monitor source of the default sink, i.e. the
    /// "what you hear" capture device. Falls back to the default source.
    /// Build the GStreamer element chain for the configured audio filters.
    /// Empty string when no filter is enabled. Elements are joined with `! `
    /// and carry no leading/trailing separator.
    pub(crate) fn filter_chain_str(filters: &AudioFilters) -> String {
        let mut elements = Vec::new();
        if filters.noise_suppression {
            elements.push(
                "webrtcdsp noise-suppression=true echo-cancel=false \
                 gain-control=false high-pass-filter=false",
            );
        }
        if filters.compressor {
            elements.push(
                "audiodynamic mode=compressor characteristics=soft-knee \
                 threshold=0.5 ratio=4.0",
            );
        }
        if filters.limiter {
            elements.push(
                "audiodynamic mode=compressor characteristics=hard-knee \
                 threshold=0.95 ratio=20.0",
            );
        }
        elements.join(" ! ")
    }

    /// Build one complete source branch of the capture pipeline:
    /// `src ! volume ! audioconvert ! audioresample ! <filters> ! <caps>`
    /// followed by `target` (an appsink or the adder). When monitoring is
    /// enabled a `tee` splits the signal so one leg reaches `target` and the
    /// other feeds `autoaudiosink` through a named volume element.
    pub(crate) fn build_source_branch(
        src: &str,
        vol_name: &str,
        filters: &AudioFilters,
        caps_filter: &str,
        target: &str,
        monitor_name: &str,
        monitor: bool,
    ) -> String {
        let filter_chain = filter_chain_str(filters);
        // Nur ein `!` einfügen, wenn überhaupt Filter aktiv sind (sonst
        // entsteht ein leeres Element "!").
        let filter_segment = if filter_chain.is_empty() {
            String::new()
        } else {
            format!("! {filter_chain} ! audioconvert ! audioresample ")
        };
        let mut branch = format!(
            "{src} ! volume name={vol_name} ! audioconvert ! audioresample \
             {filter_segment}! {caps_filter} ",
        );
        if monitor {
            branch.push_str(&format!(
                "! tee name={monitor_name}_tee ! queue ! {target} \
                 {monitor_name}_tee. ! queue ! volume name={monitor_name}_mon_vol ! autoaudiosink ",
            ));
        } else {
            branch.push_str(&format!("! queue ! {target} "));
        }
        branch
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

    /// Look up an appsink element by name in the freshly parsed pipeline.
    fn lookup_appsink(pipeline: &gst::Pipeline, name: &str) -> Result<gst_app::AppSink> {
        pipeline
            .by_name(name)
            .ok_or_else(|| anyhow::anyhow!("Missing appsink '{}' in audio pipeline", name))?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| anyhow::anyhow!("Element '{}' is not an appsink", name))
    }

    /// Pull a single sample from the appsink and convert it into an
    /// [`AudioFrame`]. Returns `None` when no sample is available within the
    /// timeout.
    fn pull_frame(
        appsink: &gst_app::AppSink,
        sample_rate: u32,
        channels: u16,
    ) -> Option<AudioFrame> {
        let sample = appsink.try_pull_sample(Some(gst::ClockTime::from_mseconds(100)))?;
        let buffer = sample.buffer()?;
        let map = buffer.map_readable().ok()?;
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
        Some(AudioFrame::new(data, sample_rate, channels))
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

        pub fn start_separated(
            &mut self,
            _system_cb: AudioFrameCallback,
            _mic_cb: AudioFrameCallback,
        ) -> Result<()> {
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

        pub fn set_monitor_volume(&self, _volume: f32) {}
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

    #[test]
    fn default_config_disables_filters_and_monitoring() {
        let config = AudioConfig::default();
        assert!(!config.system_filters.noise_suppression);
        assert!(!config.system_filters.compressor);
        assert!(!config.system_filters.limiter);
        assert!(!config.mic_filters.noise_suppression);
        assert!(!config.mic_filters.compressor);
        assert!(!config.mic_filters.limiter);
        assert!(!config.system_monitor);
        assert!(!config.mic_monitor);
        assert_eq!(config.monitor_volume.to_bits(), 1.0f32.to_bits());
    }

    #[test]
    fn default_config_uses_mixed_tracks() {
        assert!(!AudioConfig::default().separate_tracks);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn filter_chain_str_returns_empty_by_default() {
        assert_eq!(sys_impl::filter_chain_str(&AudioFilters::default()), "");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn filter_chain_str_lists_enabled_filters() {
        let filters = AudioFilters {
            noise_suppression: true,
            compressor: true,
            limiter: true,
        };
        let chain = sys_impl::filter_chain_str(&filters);
        assert!(chain.contains("webrtcdsp"));
        assert!(chain.contains("audiodynamic"));
        assert_eq!(chain.matches("audiodynamic").count(), 2);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn source_branch_without_monitor_has_no_tee() {
        let branch = sys_impl::build_source_branch(
            "pulsesrc",
            "sys_vol",
            &AudioFilters::default(),
            "capsfilter caps=\"audio/x-raw\"",
            "appsink name=sys_sink",
            "sys",
            false,
        );
        assert!(!branch.contains("tee"));
        assert!(!branch.contains("autoaudiosink"));
        assert!(branch.contains("appsink name=sys_sink"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn source_branch_with_monitor_creates_tee_and_sink() {
        let branch = sys_impl::build_source_branch(
            "pulsesrc",
            "sys_vol",
            &AudioFilters::default(),
            "capsfilter caps=\"audio/x-raw\"",
            "appsink name=sys_sink",
            "sys",
            true,
        );
        assert!(branch.contains("tee name=sys_tee"));
        assert!(branch.contains("volume name=sys_mon_vol"));
        assert!(branch.contains("autoaudiosink"));
        assert!(branch.contains("appsink name=sys_sink"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn all_filters_and_monitoring_builds_pipeline() {
        let config = AudioConfig {
            separate_tracks: true,
            system_filters: AudioFilters {
                noise_suppression: true,
                compressor: true,
                limiter: true,
            },
            mic_filters: AudioFilters {
                noise_suppression: true,
                compressor: true,
                limiter: true,
            },
            system_monitor: true,
            mic_monitor: true,
            ..Default::default()
        };
        assert!(AudioCapture::new(config).is_ok());
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

    #[cfg(target_os = "linux")]
    #[test]
    fn separate_tracks_config_builds_pipeline() {
        let config = AudioConfig {
            separate_tracks: true,
            ..Default::default()
        };
        assert!(AudioCapture::new(config).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn separate_tracks_rejects_single_mixed_start() {
        let config = AudioConfig {
            separate_tracks: true,
            ..Default::default()
        };
        let mut audio = AudioCapture::new(config).expect("pipeline builds");
        assert!(audio.start(Box::new(|_| {})).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mixed_config_rejects_separated_start() {
        let config = AudioConfig::default();
        let mut audio = AudioCapture::new(config).expect("pipeline builds");
        assert!(audio
            .start_separated(Box::new(|_| {}), Box::new(|_| {}))
            .is_err());
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn capture_is_not_supported_off_linux() {
        assert!(AudioCapture::new(AudioConfig::default()).is_err());
    }
}
