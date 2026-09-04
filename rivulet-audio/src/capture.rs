use anyhow::Result;
use rivulet_core::{AudioFrame, SkippedFilter};

use crate::messages;

/// Filter chain applied to an input track before it is mixed or recorded.
///
/// The filters are inserted into the GStreamer pipeline between the volume
/// element and the caps filter. They are realised with the `webrtcdsp`
/// (noise suppression), `audiodynamic` (compressor/limiter/expander/gate),
/// `audioamplify` (gain) and `equalizer-10bands` elements.
///
/// Elements whose GStreamer factory is not installed (e.g. `webrtcdsp` on
/// distros that do not ship it) are skipped with a warning when the pipeline
/// is built, so a missing optional filter never fails the capture.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AudioFilters {
    /// Noise suppression (WebRTC audio processing library).
    pub noise_suppression: bool,
    /// Compressor (dynamic range compression, ratio ~4:1).
    pub compressor: bool,
    /// Hard limiter (prevents clipping, ratio ~20:1).
    pub limiter: bool,
    /// Noise gate (downward expansion below a low threshold, ratio ~10:1).
    pub noise_gate: bool,
    /// Expander (downward expansion below a mid threshold, gentle ratio).
    pub expander: bool,
    /// Makeup gain in decibels (`-30..=+30`); `0.0` disables the gain stage.
    /// Realised with `audioamplify` (linear factor `10^(dB/20)`).
    pub gain_db: f32,
    /// 10-band equalizer gains in dB (`-24..=+12`, element nominal range).
    /// An all-zero array disables the equalizer stage.
    pub eq_bands: [f32; 10],
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
    /// Master volume applied to the whole mix in `[0.0, 1.0]`. Scales the
    /// combined system + microphone output after they are summed by the
    /// `adder`, i.e. a single output level that sits on top of the per-source
    /// volumes.
    pub master_volume: f32,
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
            master_volume: 1.0,
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

    /// Filters that were requested but are not installed on this system and
    /// were therefore skipped when the pipeline was built (e.g. `webrtcdsp`
    /// on distros that do not ship it). Empty when every requested filter
    /// could be built.
    pub fn skipped_filters(&self) -> &[SkippedFilter] {
        self.inner.skipped_filters()
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

    /// Change the master volume of the whole mix.
    pub fn set_master_volume(&self, volume: f32) {
        self.inner.set_master_volume(volume);
    }

    /// Human-readable note about the system-audio source that was resolved
    /// (macOS only; empty on Linux where the PulseAudio monitor is used).
    ///
    /// The GUI uses this to explain why "what you hear" is or is not being
    /// captured, e.g. that a loopback driver (BlackHole/Soundflower) must be
    /// installed for system audio on macOS.
    pub fn system_source_note(&self) -> &str {
        self.inner.system_source_note()
    }
}

// Shared DSP helpers used by the macOS backend (cpal). They are compiled on
// every platform so their unit tests run everywhere, but only reachable from
// `cfg(target_os = "macos")` code.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod dsp {
    /// Map interleaved PCM samples to `f32` in `[-1.0, 1.0]`.
    ///
    /// Supported input formats are `f32` (identity), `i16` (÷ 32768) and `u16`
    /// (mapped so `32768` -> `0.0`). The `stride` is the native interleaved
    /// channel count from the capture device.
    pub(crate) fn to_f32(sample_format: &str, samples: &[u8], stride: u16) -> Vec<f32> {
        let mut out = Vec::with_capacity(samples.len() / 2);
        match sample_format {
            "f32" => {
                for c in samples.as_chunks::<4>().0 {
                    out.push(f32::from_ne_bytes([c[0], c[1], c[2], c[3]]));
                }
            }
            "i16" => {
                for c in samples.as_chunks::<2>().0 {
                    out.push(f32::from(i16::from_ne_bytes([c[0], c[1]])) / 32_768.0);
                }
            }
            "u16" => {
                for c in samples.as_chunks::<2>().0 {
                    let v = u16::from_ne_bytes([c[0], c[1]]);
                    out.push((f32::from(v) - 32_768.0) / 32_768.0);
                }
            }
            _ => {
                let frames = samples.len() / (stride as usize * 2);
                out.resize(frames * stride as usize, 0.0);
            }
        }
        out
    }

    /// Normalise the native interleaved samples to the configured channel
    /// count (stereo for recordings). Mono is duplicated to both channels;
    /// more than two channels keep the first channel pair.
    pub(crate) fn to_channels(samples: &[f32], channels: u16, out_channels: u16) -> Vec<f32> {
        if channels == out_channels {
            return samples.to_vec();
        }
        let frames = samples.len() / channels.max(1) as usize;
        let mut out = Vec::with_capacity(frames * out_channels as usize);
        for f in 0..frames {
            let base = f * channels as usize;
            let l = samples[base];
            let r = if channels > 1 { samples[base + 1] } else { l };
            for ch in 0..out_channels as usize {
                if ch == 0 {
                    out.push(l);
                } else if ch == 1 {
                    out.push(r);
                } else {
                    out.push(0.0);
                }
            }
        }
        out
    }

    /// Resample interleaved audio linearly from `input_rate` to `output_rate`.
    ///
    /// Linear interpolation is a pragmatic choice for a voice/game-audio
    /// capture path (no filter state, deterministic, trivially testable).
    /// Every output frame picks the two surrounding input frames and blends
    /// them by the fractional position.
    pub(crate) fn resample(
        samples: &[f32],
        channels: u16,
        input_rate: u32,
        output_rate: u32,
    ) -> Vec<f32> {
        if input_rate == 0 || output_rate == 0 || input_rate == output_rate {
            return samples.to_vec();
        }
        let ch = channels.max(1) as usize;
        let frames_in = samples.len() / ch;
        if frames_in == 0 {
            return Vec::new();
        }
        let ratio = output_rate as f64 / input_rate as f64;
        let frames_out = (frames_in as f64 * ratio).floor() as usize;
        let mut out = Vec::with_capacity(frames_out * ch);
        for o in 0..frames_out {
            let pos = o as f64 / ratio;
            let i = (pos.floor() as usize).min(frames_in - 1);
            let frac = (pos - i as f64) as f32;
            for c in 0..ch {
                let a = samples[i * ch + c];
                // Linear continuation: the implied next frame extends the
                // last segment (hold a constant slope) instead of clamping to
                // the final sample, so up-sampled tails stay smooth.
                let a2 = if i + 1 < frames_in {
                    samples[(i + 1) * ch + c]
                } else {
                    let prev = if i > 0 { samples[(i - 1) * ch + c] } else { a };
                    a + (a - prev)
                };
                out.push(a + (a2 - a) * frac);
            }
        }
        out
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
        master_vol: Option<gst::Element>,
        running: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
        skipped: Vec<SkippedFilter>,
    }

    impl AudioCaptureInner {
        /// Filters that were requested but not installed, in the order they
        /// were first encountered while building the pipeline.
        pub fn skipped_filters(&self) -> &[SkippedFilter] {
            &self.skipped
        }
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
                anyhow::bail!("{}", messages::no_audio_input_sources());
            }

            let mut pipeline_str = String::new();
            let mut skipped = Vec::new();
            if config.separate_tracks {
                if let Some(src) = &system_src {
                    let (branch, missing) = build_source_branch(
                        src,
                        "sys_vol",
                        &config.system_filters,
                        &caps_filter,
                        "appsink name=sys_sink emit-signals=false sync=false",
                        "sys",
                        config.system_monitor,
                    );
                    record_skipped(&mut skipped, &missing);
                    pipeline_str.push_str(&branch);
                }
                if let Some(src) = &mic_src {
                    let (branch, missing) = build_source_branch(
                        src,
                        "mic_vol",
                        &config.mic_filters,
                        &caps_filter,
                        "appsink name=mic_sink emit-signals=false sync=false",
                        "mic",
                        config.mic_monitor,
                    );
                    record_skipped(&mut skipped, &missing);
                    pipeline_str.push_str(&branch);
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
                    let (branch, missing) = build_source_branch(
                        src,
                        vol,
                        filters,
                        &caps_filter,
                        "adder.",
                        name,
                        monitor,
                    );
                    record_skipped(&mut skipped, &missing);
                    pipeline_str.push_str(&branch);
                }
                pipeline_str.push_str(
                    "adder name=adder ! volume name=master_vol ! \
                     appsink name=out_sink emit-signals=false sync=false",
                );
            }

            let pipeline = gst::parse::launch(&pipeline_str)
                .map_err(|e| anyhow::anyhow!("{}", messages::audio_pipeline_build_failed(&e)))?
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
            let master_vol = pipeline.by_name("master_vol");
            if let Some(v) = &master_vol {
                v.set_property("volume", config.master_volume as f64);
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
                master_vol,
                running: Arc::new(AtomicBool::new(false)),
                thread: None,
                skipped,
            })
        }

        pub fn start(&mut self, on_frame: AudioFrameCallback) -> Result<()> {
            if self.config.separate_tracks {
                anyhow::bail!("{}", messages::separate_tracks_enabled_misuse());
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
                anyhow::bail!("{}", messages::separate_tracks_disabled_misuse());
            }
            if self.running.load(Ordering::SeqCst) {
                return Ok(());
            }
            self.running.store(true, Ordering::SeqCst);

            self.pipeline.set_state(gst::State::Playing)?;

            let sys_sink = self.appsink_sys.clone();
            let mic_sink = self.appsink_mic.clone();
            if sys_sink.is_none() && mic_sink.is_none() {
                anyhow::bail!("{}", messages::no_audio_input_sources());
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

        pub fn set_master_volume(&self, volume: f32) {
            if let Some(el) = &self.master_vol {
                el.set_property("volume", f64::from(volume.clamp(0.0, 1.0)));
            }
        }

        pub fn system_source_note(&self) -> &str {
            ""
        }
    }

    /// Resolve the PulseAudio monitor source of the default sink, i.e. the
    /// "what you hear" capture device. Falls back to the default source.
    /// Map an element factory name to the human-readable filter feature it
    /// provides, for user-facing messages. Delegates to the shared mapping in
    /// [`SkippedFilter::feature_name`] so the capture log and the GUI warning
    /// report the same feature name.
    pub(crate) fn filter_feature_name(element: &'static str) -> &'static str {
        SkippedFilter::feature_name(element)
    }

    /// Record skipped filter elements into `skipped` (deduplicated), logging a
    /// warning the first time each feature is reported.
    pub(crate) fn record_skipped(skipped: &mut Vec<SkippedFilter>, names: &[&'static str]) {
        record_skipped_with(skipped, names, &mut |filter| {
            tracing::warn!("{}", filter.log_message());
        });
    }

    /// Core dedup/record logic with an injectable log sink.
    ///
    /// Splitting the sink out of [`record_skipped`] keeps the dedup and
    /// one-warning-per-feature behaviour testable without depending on the
    /// global `tracing` dispatcher (whose per-callsite interest cache can be
    /// poisoned by a concurrent test that logs without a subscriber, making
    /// any `tracing`-capture test order-dependent and flaky).
    pub(crate) fn record_skipped_with(
        skipped: &mut Vec<SkippedFilter>,
        names: &[&'static str],
        log: &mut dyn FnMut(&SkippedFilter),
    ) {
        for name in names {
            let filter = SkippedFilter {
                element: name,
                feature: filter_feature_name(name),
            };
            if skipped.contains(&filter) {
                continue;
            }
            log(&filter);
            skipped.push(filter);
        }
    }

    /// Availability-aware variant used by [`build_source_branch`] and the
    /// tests: the `available` predicate decides which element factories exist.
    /// Returns the joined chain and the unique factory names that were
    /// requested but not available.
    pub(crate) fn filter_chain_str_with(
        filters: &AudioFilters,
        available: impl Fn(&str) -> bool,
    ) -> (String, Vec<&'static str>) {
        let mut elements: Vec<String> = Vec::new();
        let mut skipped: Vec<&'static str> = Vec::new();

        let mut push = |factory: &'static str, fragment: String| {
            if available(factory) {
                elements.push(fragment);
            } else if !skipped.contains(&factory) {
                skipped.push(factory);
            }
        };

        if filters.noise_suppression {
            push(
                "webrtcdsp",
                "webrtcdsp noise-suppression=true echo-cancel=false \
                 gain-control=false high-pass-filter=false"
                    .to_string(),
            );
        }
        if filters.noise_gate {
            // A gate is a strong expander that closes fully below the threshold.
            push(
                "audiodynamic",
                "audiodynamic mode=expander characteristics=hard-knee \
                 threshold=0.03 ratio=10.0"
                    .to_string(),
            );
        }
        if filters.expander {
            push(
                "audiodynamic",
                "audiodynamic mode=expander characteristics=soft-knee \
                 threshold=0.3 ratio=1.5"
                    .to_string(),
            );
        }
        if filters.compressor {
            push(
                "audiodynamic",
                "audiodynamic mode=compressor characteristics=soft-knee \
                 threshold=0.5 ratio=4.0"
                    .to_string(),
            );
        }
        if filters.limiter {
            push(
                "audiodynamic",
                "audiodynamic mode=compressor characteristics=hard-knee \
                 threshold=0.95 ratio=20.0"
                    .to_string(),
            );
        }
        if filters.gain_db.abs() >= 0.1 {
            let factor = 10f32.powf(filters.gain_db / 20.0);
            push(
                "audioamplify",
                format!("audioamplify amplification={factor:.4}"),
            );
        }
        if filters.eq_bands.iter().any(|db| db.abs() >= 0.5) {
            let mut frag = String::from("equalizer-10bands ");
            for (i, db) in filters.eq_bands.iter().enumerate() {
                frag.push_str(&format!("band{i}={db:.1} "));
            }
            push("equalizer-10bands", frag.trim_end().to_string());
        }

        (elements.join(" ! "), skipped)
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
    ) -> (String, Vec<&'static str>) {
        let _ = gst::init();
        let (filter_chain, missing) =
            filter_chain_str_with(filters, |name| gst::ElementFactory::find(name).is_some());
        // Only insert a `!` when at least one filter is active; otherwise a
        // dangling empty element "!" would be produced.
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
        (branch, missing)
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
            .ok_or_else(|| anyhow::anyhow!("{}", messages::missing_appsink(name)))?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| anyhow::anyhow!("{}", messages::not_an_appsink(name)))
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

// ─── macOS backend (cpal) ────────────────────────────────────────────────
//
// Captures the microphone through cpal's default input device and the system
// audio through a virtual loopback device (BlackHole / Soundflower / VB-Cable)
// when one is installed. Without a loopback device macOS offers no public API
// to capture "what you hear", so system capture is disabled with a note the
// GUI surfaces instead of silently recording silence. Filters and live
// monitoring are not implemented on macOS (the DSP chain stays a documented
// follow-up); volumes are applied on the raw samples before delivery.
#[cfg(target_os = "macos")]
mod sys_impl {
    use super::*;
    use anyhow::Context;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};

    /// Capture device kind: which physical source the samples come from.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum SourceKind {
        System,
        Mic,
    }

    /// Raw samples + native format pushed from the cpal callback.
    struct RawSamples {
        kind: SourceKind,
        sample_format: &'static str,
        bytes: Vec<u8>,
        stride: u16,
        input_rate: u32,
    }

    pub struct AudioCaptureInner {
        config: AudioConfig,
        running: Arc<AtomicBool>,
        sys_stream: Option<cpal::Stream>,
        mic_stream: Option<cpal::Stream>,
        sys_thread: Option<JoinHandle<()>>,
        mic_thread: Option<JoinHandle<()>>,
        system_note: String,
        sys_volume: f32,
        mic_volume: f32,
        master_volume: f32,
    }

    impl AudioCaptureInner {
        pub fn new(config: &AudioConfig) -> Result<Self> {
            let system_note = if config.capture_system {
                match find_loopback_device() {
                    Some(device) => {
                        format!("System audio: {}", device.name().unwrap_or_default())
                    }
                    None => messages::macos_system_audio_unavailable().to_string(),
                }
            } else {
                String::new()
            };
            Ok(Self {
                config: config.clone(),
                running: Arc::new(AtomicBool::new(false)),
                sys_stream: None,
                mic_stream: None,
                sys_thread: None,
                mic_thread: None,
                system_note,
                sys_volume: config.system_volume,
                mic_volume: config.mic_volume,
                master_volume: config.master_volume,
            })
        }

        /// Whether system audio can actually be captured (a loopback device
        /// was found and its stream could be opened).
        fn loopback_available(&self) -> bool {
            self.system_note.starts_with("System audio:")
        }

        /// Deliver the raw samples from both enabled sources into a single
        /// mixed frame stream (system and mic volumes applied per source).
        pub fn start(&mut self, on_frame: AudioFrameCallback) -> Result<()> {
            if self.config.separate_tracks {
                anyhow::bail!("{}", messages::separate_tracks_enabled_misuse());
            }
            if self.running.load(Ordering::SeqCst) {
                return Ok(());
            }
            let (raw_tx, raw_rx) = mpsc::channel::<RawSamples>();
            let sys_volume = self.sys_volume * self.master_volume;
            let mic_volume = self.mic_volume * self.master_volume;
            let sample_rate = self.config.sample_rate;
            let channels = self.config.channels;
            let sys_enabled = self.config.capture_system && self.loopback_available();
            if sys_enabled {
                self.spawn_source(SourceKind::System, &raw_tx)?;
            }
            if self.config.capture_mic {
                self.spawn_source(SourceKind::Mic, &raw_tx)?;
            }
            if !sys_enabled && !self.config.capture_mic {
                anyhow::bail!("{}", messages::no_audio_input_sources());
            }

            let running = Arc::clone(&self.running);
            let callback = Arc::new(Mutex::new(on_frame));
            let handle = thread::spawn(move || {
                while running.load(Ordering::SeqCst) {
                    let mut any = false;
                    while let Ok(raw) = raw_rx.try_recv() {
                        let kind = raw.kind;
                        let mut frame = to_audio_frame(raw, channels, sample_rate);
                        apply_volume(
                            &mut frame,
                            if kind == SourceKind::System {
                                sys_volume
                            } else {
                                mic_volume
                            },
                        );
                        if let Ok(mut cb) = callback.lock() {
                            cb(frame);
                        }
                        any = true;
                    }
                    if !any {
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                }
            });
            self.sys_thread = Some(handle);

            self.running.store(true, Ordering::SeqCst);
            Ok(())
        }

        /// Deliver system and microphone frames on separate channels
        /// (volumes applied per source).
        pub fn start_separated(
            &mut self,
            system_cb: AudioFrameCallback,
            mic_cb: AudioFrameCallback,
        ) -> Result<()> {
            if !self.config.separate_tracks {
                anyhow::bail!("{}", messages::separate_tracks_disabled_misuse());
            }
            if self.running.load(Ordering::SeqCst) {
                return Ok(());
            }
            let (sys_tx, sys_rx) = mpsc::channel::<RawSamples>();
            let (mic_tx, mic_rx) = mpsc::channel::<RawSamples>();
            let sys_volume = self.sys_volume * self.master_volume;
            let mic_volume = self.mic_volume * self.master_volume;
            let sample_rate = self.config.sample_rate;
            let channels = self.config.channels;
            let sys_enabled = self.config.capture_system && self.loopback_available();
            if self.config.capture_system && !sys_enabled {
                tracing::warn!("{}", messages::macos_system_audio_unavailable());
            }
            if sys_enabled {
                self.spawn_source(SourceKind::System, &sys_tx)?;
            }
            if self.config.capture_mic {
                self.spawn_source(SourceKind::Mic, &mic_tx)?;
            }
            if !sys_enabled && !self.config.capture_mic {
                anyhow::bail!("{}", messages::no_audio_input_sources());
            }

            let running = Arc::clone(&self.running);
            let sys_callback = Arc::new(Mutex::new(system_cb));
            let mic_callback = Arc::new(Mutex::new(mic_cb));
            let handle = thread::spawn(move || {
                while running.load(Ordering::SeqCst) {
                    let mut any = false;
                    while let Ok(raw) = sys_rx.try_recv() {
                        let mut frame = to_audio_frame(raw, channels, sample_rate);
                        apply_volume(&mut frame, sys_volume);
                        if let Ok(mut cb) = sys_callback.lock() {
                            cb(frame);
                        }
                        any = true;
                    }
                    while let Ok(raw) = mic_rx.try_recv() {
                        let mut frame = to_audio_frame(raw, channels, sample_rate);
                        apply_volume(&mut frame, mic_volume);
                        if let Ok(mut cb) = mic_callback.lock() {
                            cb(frame);
                        }
                        any = true;
                    }
                    if !any {
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                }
            });
            self.sys_thread = Some(handle);

            self.running.store(true, Ordering::SeqCst);
            Ok(())
        }

        pub fn stop(&mut self) -> Result<()> {
            if !self.running.load(Ordering::SeqCst) {
                return Ok(());
            }
            self.running.store(false, Ordering::SeqCst);
            if let Some(stream) = self.sys_stream.take() {
                stream.pause().ok();
            }
            if let Some(stream) = self.mic_stream.take() {
                stream.pause().ok();
            }
            if let Some(handle) = self.sys_thread.take() {
                let _ = handle.join();
            }
            if let Some(handle) = self.mic_thread.take() {
                let _ = handle.join();
            }
            Ok(())
        }

        pub fn is_running(&self) -> bool {
            self.running.load(Ordering::SeqCst)
        }

        pub fn skipped_filters(&self) -> &[SkippedFilter] {
            &[]
        }

        pub fn set_system_volume(&self, volume: f32) {
            let _ = volume;
        }

        pub fn set_mic_volume(&self, volume: f32) {
            let _ = volume;
        }

        pub fn set_monitor_volume(&self, _volume: f32) {}

        pub fn set_master_volume(&self, _volume: f32) {}

        pub fn system_source_note(&self) -> &str {
            &self.system_note
        }

        /// Open one cpal input stream for the given kind and wire its samples
        /// into `raw_tx`. The stream object is stored on `self` so it stays
        /// alive (and keeps producing callbacks) until [`Self::stop`].
        fn spawn_source(
            &mut self,
            kind: SourceKind,
            raw_tx: &mpsc::Sender<RawSamples>,
        ) -> Result<()> {
            let host = cpal::default_host();
            let device = match kind {
                SourceKind::System => match find_loopback_device() {
                    Some(d) => d,
                    None => return Ok(()),
                },
                SourceKind::Mic => host
                    .default_input_device()
                    .context(messages::macos_no_input_device())?,
            };
            // cpal 0.15: `default_input_config` yields a `SupportedStreamConfig`
            // whose fields are private — read them through the accessors and
            // derive the plain `StreamConfig` for the stream builder.
            let supported = device
                .default_input_config()
                .context(messages::macos_no_input_format())?;
            // Only the sample formats our byte pipeline can decode are
            // accepted; anything else is reported instead of silently
            // recording silence. The typed stream is built with the very
            // sample type the device advertises, so the byte tag always
            // matches the samples the callback receives.
            let channels = supported.channels().max(1);
            let input_rate = supported.sample_rate().0;
            let config = supported.config();
            let kind_for_cb = kind;
            let raw_tx = raw_tx.clone();

            let stream = match supported.sample_format() {
                cpal::SampleFormat::F32 => open_typed_stream::<f32>(
                    &device,
                    &config,
                    "f32",
                    kind_for_cb,
                    raw_tx,
                    channels,
                    input_rate,
                ),
                cpal::SampleFormat::I16 => open_typed_stream::<i16>(
                    &device,
                    &config,
                    "i16",
                    kind_for_cb,
                    raw_tx,
                    channels,
                    input_rate,
                ),
                cpal::SampleFormat::U16 => open_typed_stream::<u16>(
                    &device,
                    &config,
                    "u16",
                    kind_for_cb,
                    raw_tx,
                    channels,
                    input_rate,
                ),
                other => Err(anyhow::anyhow!(
                    "{}",
                    messages::macos_unsupported_sample_format(&format!("{other:?}"))
                )),
            }?;

            match kind {
                SourceKind::System => self.sys_stream = Some(stream),
                SourceKind::Mic => self.mic_stream = Some(stream),
            }
            Ok(())
        }
    }

    /// Open a typed cpal input stream on `device` whose samples are forwarded
    /// as raw bytes tagged with `sample_format` (matching `T`) into `raw_tx`.
    /// cpal 0.15 requires an explicit [`cpal::traits::StreamTrait::play`] after
    /// creation, otherwise no callback ever fires.
    fn open_typed_stream<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        sample_format: &'static str,
        kind: SourceKind,
        raw_tx: mpsc::Sender<RawSamples>,
        channels: u16,
        input_rate: u32,
    ) -> Result<cpal::Stream>
    where
        T: cpal::SizedSample,
    {
        let kind_for_cb = kind;
        let stream = device
            .build_input_stream::<T, _, _>(
                config,
                move |data: &[T], _: &cpal::InputCallbackInfo| {
                    // cpal delivers `T` samples; reinterpret the buffer as raw
                    // bytes so `to_f32` can decode it according to the captured
                    // sample format.
                    // SAFETY: `T` is a plain numeric sample type with no
                    // padding, so the `&[T]` buffer's underlying bytes form a
                    // valid `&[u8]` of `len * size_of::<T>()` bytes.
                    let bytes = unsafe {
                        std::slice::from_raw_parts(
                            data.as_ptr() as *const u8,
                            data.len() * std::mem::size_of::<T>(),
                        )
                    }
                    .to_vec();
                    let _ = raw_tx.send(RawSamples {
                        kind: kind_for_cb,
                        sample_format,
                        bytes,
                        stride: channels,
                        input_rate,
                    });
                },
                move |_| {},
                None,
            )
            .map_err(|e| anyhow::anyhow!("{}", messages::macos_stream_failed(&format!("{e}"))))?;
        stream
            .play()
            .map_err(|e| anyhow::anyhow!("{}", messages::macos_stream_failed(&format!("{e}"))))?;
        Ok(stream)
    }

    /// Convert raw device samples into a configured [`AudioFrame`]: native
    /// format -> f32, channel mapping to the configured count, resampling to
    /// the configured rate.
    fn to_audio_frame(raw: RawSamples, channels: u16, sample_rate: u32) -> AudioFrame {
        let f32_samples = dsp::to_f32(raw.sample_format, &raw.bytes, raw.stride);
        let mapped = dsp::to_channels(&f32_samples, raw.stride, channels);
        let resampled = dsp::resample(&mapped, channels, raw.input_rate, sample_rate);
        AudioFrame::new(resampled, sample_rate, channels)
    }

    /// Scale a frame in place by a linear volume factor.
    fn apply_volume(frame: &mut AudioFrame, volume: f32) {
        let v = volume.clamp(0.0, 1.0);
        if (v - 1.0).abs() < f32::EPSILON {
            return;
        }
        for s in &mut frame.data {
            *s *= v;
        }
    }

    /// Find an installed loopback capture device (BlackHole, Soundflower,
    /// VB-Cable) among the input devices. Returns the device itself so the
    /// caller can both name it and open a stream on it.
    fn find_loopback_device() -> Option<cpal::Device> {
        let host = cpal::default_host();
        let mut devices = host.input_devices().ok()?;
        while let Some(device) = devices.next() {
            if let Ok(name) = device.name() {
                let lower = name.to_lowercase();
                if ["blackhole", "soundflower", "loopback", "vbcable"]
                    .iter()
                    .any(|k| lower.contains(k))
                {
                    return Some(device);
                }
            }
        }
        None
    }
}

// ─── fallback for other platforms (no audio capture yet) ─────────────────
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod sys_impl {
    use super::*;

    pub struct AudioCaptureInner {
        _private: (),
    }

    impl AudioCaptureInner {
        pub fn new(_config: &AudioConfig) -> Result<Self> {
            anyhow::bail!("{}", messages::capture_linux_only())
        }

        pub fn start(&mut self, _on_frame: AudioFrameCallback) -> Result<()> {
            anyhow::bail!("{}", messages::capture_linux_only())
        }

        pub fn start_separated(
            &mut self,
            _system_cb: AudioFrameCallback,
            _mic_cb: AudioFrameCallback,
        ) -> Result<()> {
            anyhow::bail!("{}", messages::capture_linux_only())
        }

        pub fn stop(&mut self) -> Result<()> {
            Ok(())
        }

        pub fn is_running(&self) -> bool {
            false
        }

        pub fn skipped_filters(&self) -> &[SkippedFilter] {
            &[]
        }

        pub fn set_system_volume(&self, _volume: f32) {}

        pub fn set_mic_volume(&self, _volume: f32) {}

        pub fn set_monitor_volume(&self, _volume: f32) {}

        pub fn set_master_volume(&self, _volume: f32) {}

        pub fn system_source_note(&self) -> &str {
            ""
        }
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
        assert_eq!(config.master_volume.to_bits(), 1.0f32.to_bits());
    }

    #[test]
    fn default_config_disables_filters_and_monitoring() {
        let config = AudioConfig::default();
        assert!(!config.system_filters.noise_suppression);
        assert!(!config.system_filters.compressor);
        assert!(!config.system_filters.limiter);
        assert!(!config.system_filters.noise_gate);
        assert!(!config.system_filters.expander);
        assert_eq!(config.system_filters.gain_db, 0.0);
        assert!(config.system_filters.eq_bands.iter().all(|db| *db == 0.0));
        assert!(!config.mic_filters.noise_suppression);
        assert!(!config.mic_filters.compressor);
        assert!(!config.mic_filters.limiter);
        assert!(!config.mic_filters.noise_gate);
        assert!(!config.mic_filters.expander);
        assert!(!config.system_monitor);
        assert!(!config.mic_monitor);
        assert_eq!(config.monitor_volume.to_bits(), 1.0f32.to_bits());
        assert_eq!(config.master_volume.to_bits(), 1.0f32.to_bits());
    }

    #[test]
    fn default_config_uses_mixed_tracks() {
        assert!(!AudioConfig::default().separate_tracks);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn filter_chain_str_returns_empty_by_default() {
        let (chain, skipped) = sys_impl::filter_chain_str_with(&AudioFilters::default(), |_| true);
        assert_eq!(chain, "");
        assert!(skipped.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn filter_chain_str_lists_enabled_filters() {
        let filters = AudioFilters {
            noise_suppression: true,
            compressor: true,
            limiter: true,
            ..Default::default()
        };
        // All elements available -> full chain, nothing skipped.
        let (chain, skipped) = sys_impl::filter_chain_str_with(&filters, |_| true);
        assert!(chain.contains("webrtcdsp"));
        assert!(chain.contains("audiodynamic"));
        assert_eq!(chain.matches("audiodynamic").count(), 2);
        assert!(skipped.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn filter_chain_str_skips_unavailable_elements() {
        let filters = AudioFilters {
            noise_suppression: true,
            compressor: true,
            limiter: true,
            ..Default::default()
        };
        // Simulate a distro whose gst-plugins-bad does not ship webrtcdsp
        // (e.g. Ubuntu): the filter is dropped, the rest of the chain stays.
        let (chain, skipped) =
            sys_impl::filter_chain_str_with(&filters, |name| name != "webrtcdsp");
        assert!(!chain.contains("webrtcdsp"));
        assert!(chain.contains("audiodynamic"));
        assert_eq!(chain.matches("audiodynamic").count(), 2);
        assert_eq!(skipped, vec!["webrtcdsp"]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn filter_chain_emits_gate_expander_gain_and_eq() {
        let filters = AudioFilters {
            noise_gate: true,
            expander: true,
            gain_db: 20.0,
            eq_bands: [3.0, -6.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            ..Default::default()
        };
        let (chain, skipped) = sys_impl::filter_chain_str_with(&filters, |_| true);
        assert!(
            chain.contains("mode=expander characteristics=hard-knee"),
            "gate: {chain}"
        );
        assert!(
            chain.contains("mode=expander characteristics=soft-knee"),
            "expander: {chain}"
        );
        // Gain is emitted as a linear amplitude factor (10^(20/20) == 10.0).
        assert!(
            chain.contains("audioamplify amplification=10.0000"),
            "gain: {chain}"
        );
        assert!(chain.contains("equalizer-10bands"), "eq: {chain}");
        assert!(chain.contains("band0=3.0"), "band0: {chain}");
        assert!(chain.contains("band1=-6.0"), "band1: {chain}");
        assert!(skipped.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn filter_chain_emits_no_gain_or_eq_when_neutral() {
        // A 0 dB gain and an all-zero EQ are no-ops and must not emit elements.
        let filters = AudioFilters {
            noise_gate: true,
            ..Default::default()
        };
        let (chain, skipped) = sys_impl::filter_chain_str_with(&filters, |_| true);
        assert!(!chain.contains("audioamplify"), "{chain}");
        assert!(!chain.contains("equalizer-10bands"), "{chain}");
        assert!(chain.contains("audiodynamic"), "{chain}");
        assert!(skipped.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn filter_chain_str_skips_all_unavailable_elements() {
        let filters = AudioFilters {
            noise_suppression: true,
            compressor: true,
            limiter: true,
            ..Default::default()
        };
        // Nothing available -> empty chain, each factory reported once.
        let (chain, skipped) = sys_impl::filter_chain_str_with(&filters, |_| false);
        assert_eq!(chain, "");
        assert_eq!(skipped, vec!["webrtcdsp", "audiodynamic"]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn filter_feature_name_uses_the_shared_core_mapping() {
        // The capture log must use the same feature name as the shared core
        // mapping, so log and GUI warnings can never drift apart.
        for element in ["webrtcdsp", "audiodynamic", "unknown"] {
            assert_eq!(
                sys_impl::filter_feature_name(element),
                SkippedFilter::feature_name(element),
                "capture log feature name for `{element}` must match the shared mapping"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn record_skipped_dedupes_and_maps() {
        let mut skipped = Vec::new();
        sys_impl::record_skipped(&mut skipped, &["webrtcdsp", "audiodynamic", "webrtcdsp"]);
        assert_eq!(
            skipped,
            vec![
                SkippedFilter {
                    element: "webrtcdsp",
                    feature: "noise suppression",
                },
                SkippedFilter {
                    element: "audiodynamic",
                    feature: "compressor/limiter/expander/gate",
                },
            ]
        );
        // The recorded filters must produce the exact log line that
        // `record_skipped` emits via `tracing::warn!`.
        assert_eq!(
            skipped[0].log_message(),
            "noise suppression skipped: GStreamer element `webrtcdsp` is not installed"
        );
        assert_eq!(
            skipped[1].log_message(),
            "compressor/limiter/expander/gate skipped: GStreamer element `audiodynamic` is not installed"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn record_skipped_logs_each_new_feature_exactly_once() {
        let mut skipped = Vec::new();
        let mut logged = Vec::new();
        sys_impl::record_skipped_with(
            &mut skipped,
            &["webrtcdsp", "audiodynamic", "webrtcdsp"],
            &mut |filter| logged.push(filter.log_message()),
        );

        assert_eq!(
            logged,
            vec![
                "noise suppression skipped: GStreamer element `webrtcdsp` is not installed",
                "compressor/limiter/expander/gate skipped: GStreamer element `audiodynamic` is not installed",
            ],
            "each new feature must be logged exactly once, in first-seen order"
        );
        assert_eq!(skipped.len(), 2, "the duplicate entry must be deduplicated");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn skipped_filters_reports_unavailable_elements() {
        let _ = gstreamer::init();
        let config = AudioConfig {
            separate_tracks: true,
            system_filters: AudioFilters {
                noise_suppression: true,
                compressor: true,
                limiter: true,
                ..Default::default()
            },
            mic_filters: AudioFilters {
                noise_suppression: true,
                compressor: true,
                limiter: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let cap = AudioCapture::new(config).expect("pipeline builds even with missing filters");
        let webrtc_missing = gstreamer::ElementFactory::find("webrtcdsp").is_none();
        let reports_webrtc = cap
            .skipped_filters()
            .iter()
            .any(|s| s.element == "webrtcdsp");
        assert_eq!(
            webrtc_missing, reports_webrtc,
            "skipped_filters() must report webrtcdsp exactly when it is not installed"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn source_branch_without_monitor_has_no_tee() {
        let (branch, missing) = sys_impl::build_source_branch(
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
        assert!(missing.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn source_branch_with_monitor_creates_tee_and_sink() {
        let (branch, missing) = sys_impl::build_source_branch(
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
        assert!(missing.is_empty());
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
                ..Default::default()
            },
            mic_filters: AudioFilters {
                noise_suppression: true,
                compressor: true,
                limiter: true,
                ..Default::default()
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
    fn mixed_config_builds_pipeline_with_master_volume() {
        // The default (non-separated) path sums both sources in an adder and
        // applies a master volume element on the combined output. Building the
        // pipeline validates that the `master_vol` fragment parses.
        let config = AudioConfig {
            capture_system: true,
            capture_mic: true,
            master_volume: 0.5,
            ..Default::default()
        };
        let capture = AudioCapture::new(config).expect("mixed pipeline builds");
        capture.set_master_volume(0.25); // no panic, element exists on Linux
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

    #[cfg(target_os = "macos")]
    #[test]
    fn capture_is_supported_on_macos() {
        // Construction must not fail on macOS even when no device is present
        // (the note is set and start() later reports the real error); the
        // point is that the API is no longer the Linux-only stub.
        let cap = AudioCapture::new(AudioConfig::default()).expect("macOS capture constructs");
        // With the default config system audio is requested: either a loopback
        // device note or the "unavailable" note is present.
        let note = cap.system_source_note();
        assert!(
            note.is_empty() || note.starts_with("System audio:") || note.contains("loopback"),
            "unexpected system note: {note}"
        );
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    #[test]
    fn capture_is_not_supported_off_linux() {
        assert!(AudioCapture::new(AudioConfig::default()).is_err());
    }

    // ── shared DSP helpers (compiled and tested on every platform) ──

    #[test]
    fn to_f32_converts_i16_samples() {
        // [0x0000, 0x8000] little-endian = 0 and -32768 -> 0.0 and -1.0.
        let bytes = [0x00, 0x00, 0x00, 0x80];
        let out = dsp::to_f32("i16", &bytes, 2);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[1], -1.0);
    }

    #[test]
    fn to_f32_passes_f32_through() {
        let bytes = 0.5f32.to_ne_bytes();
        let out = dsp::to_f32("f32", &bytes, 1);
        assert_eq!(out, vec![0.5]);
    }

    #[test]
    fn to_channels_duplicates_mono_to_stereo() {
        let out = dsp::to_channels(&[0.1, 0.2, 0.3], 1, 2);
        assert_eq!(out, vec![0.1, 0.1, 0.2, 0.2, 0.3, 0.3]);
    }

    #[test]
    fn to_channels_keeps_first_pair_when_downmixing() {
        // 4-channel input (quad) -> stereo keeps L/R of each frame.
        let out = dsp::to_channels(&[1.0, 2.0, 9.0, 9.0, 3.0, 4.0, 9.0, 9.0], 4, 2);
        assert_eq!(out, vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn to_channels_identity_when_counts_match() {
        assert_eq!(dsp::to_channels(&[1.0, 2.0], 2, 2), vec![1.0, 2.0]);
    }

    #[test]
    fn resample_is_identity_at_same_rate() {
        let samples = vec![0.1, 0.2, 0.3];
        assert_eq!(dsp::resample(&samples, 1, 48_000, 48_000), samples);
    }

    #[test]
    fn resample_down_samples_length_and_interpolates() {
        // 4 frames at 48 kHz -> 2 frames at 24 kHz (ratio 0.5): frames 0 and 2.
        let out = dsp::resample(&[0.0, 1.0, 2.0, 3.0], 1, 48_000, 24_000);
        assert_eq!(out, vec![0.0, 2.0]);
    }

    #[test]
    fn resample_up_samples_to_the_target_length() {
        // 2 frames at 24 kHz -> 4 frames at 48 kHz (ratio 2.0): 0, 0.5, 1, 1.5.
        let out = dsp::resample(&[0.0, 1.0], 1, 24_000, 48_000);
        assert_eq!(out.len(), 4);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[1], 0.5);
        assert_eq!(out[2], 1.0);
        assert_eq!(out[3], 1.5);
    }

    #[test]
    fn resample_keeps_stereo_interleaving() {
        // Stereo frames [L,R] resampled 2:1 keep the interleave intact:
        // 2 frames in (4 samples) -> 1 frame out (2 samples).
        let out = dsp::resample(&[0.0, 0.1, 1.0, 1.1], 2, 48_000, 24_000);
        assert_eq!(out, vec![0.0, 0.1]);
    }
}
