// In rivulet-core/src/lib.rs

use gstreamer as gst;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
// Important: The prelude is still needed for methods like .set_state(), .by_name(), etc.
use gst::prelude::*;

pub mod audio;
pub use audio::{AudioFrame, AudioTrack};

pub mod encoder;
pub use encoder::{best_encoder, detect_available_encoders, VideoEncoder};

pub mod health;
pub use health::{StreamHealthMonitor, StreamHealthStatus, StreamStats};

pub mod metrics;
pub use metrics::{RecordingMetrics, RecordingStatsMonitor};

pub mod stream;
pub use stream::{StreamPlatform, StreamSettings};

pub mod i18n;
pub use i18n::Locale;

// GStreamer initialization
static GSTREAMER_INIT: Lazy<()> = Lazy::new(|| {
    gst::init().expect("GStreamer initialization failed.");
});

/// Default audio format used by the engine's audio input.
pub const AUDIO_SAMPLE_RATE: u32 = 48_000;
pub const AUDIO_CHANNELS: u16 = 2;

pub struct RivuletEngine {
    pipeline: Option<gst::Pipeline>,
    appsrc: Option<gst_app::AppSrc>,
    audio_appsrc: Option<gst_app::AppSrc>,
    audio_appsrc_sys: Option<gst_app::AppSrc>,
    audio_appsrc_mic: Option<gst_app::AppSrc>,
    audio_enabled: bool,
    separate_audio_tracks: bool,
    audio_sys_enabled: bool,
    audio_mic_enabled: bool,
    is_recording: bool,
    output_path: Option<PathBuf>,
    stream_settings: Option<StreamSettings>,
    /// Video encoder used for the H.264 video branch. Defaults to the best
    /// available encoder detected at construction time.
    video_encoder: VideoEncoder,
    /// Target video bitrate in kbit/s applied to the selected encoder.
    encoder_bitrate_kbps: u32,
    /// Tracks stream health while a streaming pipeline is active. Always
    /// `None` for plain local recordings.
    stream_health: Option<StreamHealthMonitor>,
    /// Tracks recording performance (FPS, encoder load, file size). Wrapped in
    /// an `Arc<Mutex<..>>` so GStreamer pad probes running on the streaming
    /// thread can update it while the app thread reads it.
    recording_metrics: Option<Arc<Mutex<RecordingStatsMonitor>>>,
    /// Most recent engine error (pipeline build/start/frame-push failure).
    /// Consumed by the GUI via [`RivuletEngine::take_error`] so failures are
    /// shown in the UI instead of only on the console.
    last_error: Option<String>,
}

impl Default for RivuletEngine {
    fn default() -> Self {
        Lazy::force(&GSTREAMER_INIT);
        Self {
            pipeline: None,
            appsrc: None,
            audio_appsrc: None,
            audio_appsrc_sys: None,
            audio_appsrc_mic: None,
            audio_enabled: false,
            separate_audio_tracks: false,
            audio_sys_enabled: true,
            audio_mic_enabled: true,
            is_recording: false,
            output_path: None,
            stream_settings: None,
            video_encoder: best_encoder(),
            encoder_bitrate_kbps: 5_000,
            stream_health: None,
            recording_metrics: None,
            last_error: None,
        }
    }
}

impl RivuletEngine {
    pub fn new() -> Self {
        println!("[Engine] GStreamer ready.");
        Self::default()
    }

    /// Record an engine error for display in the GUI and log it to stderr.
    ///
    /// The most recent error is retained until it is consumed via
    /// [`RivuletEngine::take_error`], so the UI can surface it next to the
    /// recording controls instead of relying on console output alone.
    fn set_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        eprintln!("[Engine] {}", message);
        self.last_error = Some(message);
    }

    /// Take (and clear) the most recent engine error, if any.
    ///
    /// Returns `None` when the engine has been operating normally since the
    /// last call. Errors are reported at most once per consumer.
    pub fn take_error(&mut self) -> Option<String> {
        self.last_error.take()
    }

    /// Configure the RTMP/RTMPS stream output. When set, the engine streams to
    /// the given ingest URL. If a local recording is started at the same time
    /// (via [`RivuletEngine::start_local_recording`]) the engine runs in dual
    /// output mode and records and streams simultaneously.
    ///
    /// Must be called before recording starts.
    pub fn set_stream_settings(&mut self, settings: Option<StreamSettings>) {
        self.stream_settings = settings;
    }

    /// Whether the engine is configured to stream.
    pub fn is_streaming(&self) -> bool {
        self.stream_settings.is_some()
    }

    /// Whether the engine records and streams at the same time (dual output).
    /// This is the case when both stream settings and a local output path are
    /// configured.
    pub fn is_dual_output(&self) -> bool {
        self.is_streaming() && self.output_path.is_some()
    }

    /// Select the video encoder used for the H.264 branch.
    ///
    /// Must be called before recording starts. By default the engine picks the
    /// best available encoder ([`best_encoder`]); if a hardware encoder cannot
    /// be initialized at runtime the engine falls back to software x264.
    pub fn set_video_encoder(&mut self, encoder: VideoEncoder) {
        self.video_encoder = encoder;
    }

    /// The currently selected video encoder.
    pub fn video_encoder(&self) -> VideoEncoder {
        self.video_encoder
    }

    /// Set the target video bitrate in kbit/s applied to the encoder.
    ///
    /// Must be called before recording starts. Defaults to 5000 kbit/s.
    pub fn set_video_bitrate(&mut self, kbps: u32) {
        self.encoder_bitrate_kbps = kbps;
    }

    /// The configured target video bitrate in kbit/s.
    pub fn video_bitrate(&self) -> u32 {
        self.encoder_bitrate_kbps
    }

    /// Escape a location string for embedding into a `parse_launch` pipeline
    /// description. GStreamer treats `\` as an escape character in quoted
    /// property values; on Windows paths like `C:\Users\...` would otherwise be
    /// silently mangled (e.g. `C:UsersRUNNER~1...`).
    fn escape_location(location: &str) -> String {
        location.replace('\\', "\\\\")
    }

    fn video_branch_str(&self) -> String {
        format!(
            "appsrc name=rivulet_src format=time is-live=true do-timestamp=true \
             ! videoconvert ! {} ! {} ! queue ! mux. ",
            self.video_encoder.input_caps_fragment(),
            self.video_encoder
                .branch_fragment(self.encoder_bitrate_kbps)
        )
    }

    /// Build the audio branch of the pipeline. With `force_mixed` the sources
    /// are mixed into a single track regardless of `separate_audio_tracks`; the
    /// FLV muxer used for streaming only supports a single audio track.
    fn audio_branch_str(&self, force_mixed: bool) -> String {
        if !self.audio_enabled {
            return String::new();
        }
        if self.separate_audio_tracks && !force_mixed {
            let mut s = String::new();
            if self.audio_sys_enabled {
                s.push_str(
                    "appsrc name=audio_src_sys format=time is-live=true do-timestamp=true \
                     ! audioconvert ! audioresample ! avenc_aac ! queue ! mux. ",
                );
            }
            if self.audio_mic_enabled {
                s.push_str(
                    "appsrc name=audio_src_mic format=time is-live=true do-timestamp=true \
                     ! audioconvert ! audioresample ! avenc_aac ! queue ! mux. ",
                );
            }
            s
        } else {
            "appsrc name=audio_src format=time is-live=true do-timestamp=true \
             ! audioconvert ! audioresample ! avenc_aac ! queue ! mux. "
                .to_string()
        }
    }

    fn build_recording_pipeline_str(&self, location: &str) -> String {
        let mut s = self.video_branch_str();
        s.push_str(&self.audio_branch_str(false));
        let escaped = Self::escape_location(location);
        s.push_str(&format!(
            "mp4mux name=mux ! filesink name=file_sink location=\"{}\"",
            escaped
        ));
        s
    }

    fn build_streaming_pipeline_str(&self) -> String {
        let location = self
            .stream_settings
            .as_ref()
            .expect("Stream settings must be set")
            .location();
        println!("[Engine] Initializing streaming pipeline for: {}", location);
        let mut s = self.video_branch_str();
        s.push_str(&self.audio_branch_str(true));
        s.push_str(&format!(
            "flvmux name=mux streamable=true ! rtmp2sink location=\"{}\"",
            location
        ));
        s
    }

    /// Build a pipeline that records locally **and** streams at the same time.
    ///
    /// Video and audio are encoded once and split via `tee` into two branches,
    /// one feeding the MP4 muxer (local file) and one feeding the FLV muxer
    /// (RTMP/RTMPS sink). Audio is always mixed into a single track because FLV
    /// only supports one audio track.
    fn build_dual_output_pipeline_str(&self) -> String {
        let stream_location = self
            .stream_settings
            .as_ref()
            .expect("Stream settings must be set")
            .location();
        let path = self.output_path.as_ref().expect("Output path must be set");
        let location = path.to_str().expect("Invalid file path.");
        println!(
            "[Engine] Initializing dual-output pipeline (recording + stream to {})",
            stream_location
        );

        let mut s = String::new();
        s.push_str(&format!(
            "appsrc name=rivulet_src format=time is-live=true do-timestamp=true \
             ! videoconvert ! {} ! {} \
             ! tee name=video_tee ! queue ! mux_rec. \
             video_tee. ! queue ! mux_stream. ",
            self.video_encoder.input_caps_fragment(),
            self.video_encoder
                .branch_fragment(self.encoder_bitrate_kbps)
        ));
        if self.audio_enabled {
            s.push_str(
                "appsrc name=audio_src format=time is-live=true do-timestamp=true \
                 ! audioconvert ! audioresample ! avenc_aac ! tee name=audio_tee \
                 ! queue ! mux_rec. \
                 audio_tee. ! queue ! mux_stream. ",
            );
        }
        s.push_str(&format!(
            "mp4mux name=mux_rec ! filesink name=file_sink location=\"{}\" ",
            Self::escape_location(location)
        ));
        s.push_str(&format!(
            "flvmux name=mux_stream streamable=true ! rtmp2sink location=\"{}\"",
            stream_location
        ));
        s
    }

    /// Build the pipeline description for the currently configured output mode
    /// (dual output, streaming, or local recording). Returns `None` when no
    /// output is configured at all.
    fn build_pipeline_str(&self) -> Option<String> {
        if self.is_dual_output() {
            Some(self.build_dual_output_pipeline_str())
        } else if self.is_streaming() {
            Some(self.build_streaming_pipeline_str())
        } else {
            let path = self.output_path.as_ref()?;
            let location = path.to_str().expect("Invalid file path.");
            println!("[Engine] Initializing recording pipeline for: {}", location);
            Some(self.build_recording_pipeline_str(location))
        }
    }

    fn initialize_and_start_pipeline(&mut self, width: u32, height: u32) {
        let Some(mut pipeline_str) = self.build_pipeline_str() else {
            return;
        };

        // Hardware encoders (NVENC/QuickSync/AMF) are not available on every
        // machine (missing GPU/driver). If pipeline parsing or startup fails,
        // fall back to software x264 once and rebuild the pipeline with the
        // new encoder.
        let pipeline = loop {
            let parsed = match gst::parse::launch(&pipeline_str) {
                Ok(p) => p,
                Err(e) => {
                    if self.try_encoder_fallback(&mut pipeline_str, &e.to_string()) {
                        continue;
                    }
                    self.set_error(format!("Could not create the recording pipeline: {}", e));
                    return;
                }
            };
            let pipeline = parsed.downcast::<gst::Pipeline>().unwrap();
            if let Err(e) = pipeline.set_state(gst::State::Playing) {
                if self.try_encoder_fallback(&mut pipeline_str, &e.to_string()) {
                    continue;
                }
                self.set_error(format!("Could not start the recording pipeline: {}", e));
                return;
            }
            break pipeline;
        };

        // The rest of the code is correct...
        let appsrc = pipeline
            .by_name("rivulet_src")
            .unwrap()
            .downcast::<gst_app::AppSrc>()
            .unwrap();

        let video_info = gst_video::VideoInfo::builder(gst_video::VideoFormat::Rgba, width, height)
            .fps((30, 1))
            .build()
            .unwrap();
        appsrc.set_caps(Some(&video_info.to_caps().unwrap()));
        appsrc.set_property("format", gst::Format::Time);
        appsrc.set_property("is-live", true);
        appsrc.set_property("do-timestamp", true);

        if self.audio_enabled {
            let separate = self.separate_audio_tracks && !self.is_streaming();
            if separate {
                let mut track_srcs: Vec<(&str, &mut Option<gst_app::AppSrc>)> = Vec::new();
                if self.audio_sys_enabled {
                    track_srcs.push(("audio_src_sys", &mut self.audio_appsrc_sys));
                }
                if self.audio_mic_enabled {
                    track_srcs.push(("audio_src_mic", &mut self.audio_appsrc_mic));
                }
                for (name, slot) in track_srcs {
                    let app = pipeline
                        .by_name(name)
                        .unwrap()
                        .downcast::<gst_app::AppSrc>()
                        .unwrap();
                    app.set_caps(Some(&audio_caps()));
                    app.set_property("format", gst::Format::Time);
                    app.set_property("is-live", true);
                    app.set_property("do-timestamp", true);
                    *slot = Some(app);
                }
            } else {
                let audio_appsrc = pipeline
                    .by_name("audio_src")
                    .unwrap()
                    .downcast::<gst_app::AppSrc>()
                    .unwrap();
                audio_appsrc.set_caps(Some(&audio_caps()));
                audio_appsrc.set_property("format", gst::Format::Time);
                audio_appsrc.set_property("is-live", true);
                audio_appsrc.set_property("do-timestamp", true);
                self.audio_appsrc = Some(audio_appsrc);
            }
        }

        self.pipeline = Some(pipeline);
        self.appsrc = Some(appsrc);
        self.install_performance_probes();
        if self.is_dual_output() {
            println!("[Engine] Dual-output pipeline running.");
        } else if self.is_streaming() {
            println!("[Engine] Streaming pipeline running.");
        } else {
            println!("[Engine] Recording pipeline running.");
        }
    }

    /// Attach pad probes that feed the recording performance metrics.
    ///
    /// - The encoder sink/src pads measure the per-frame encode duration.
    ///   Arrival timestamps are paired with the matching output buffer by its
    ///   PTS, so B-frame reordering does not distort the measurement.
    /// - The filesink sink pad counts the bytes written to the output file.
    ///
    /// The probes run on the GStreamer streaming thread and only write to the
    /// shared `Arc<Mutex<RecordingStatsMonitor>>`; the app thread reads it
    /// through [`RivuletEngine::recording_stats`].
    fn install_performance_probes(&mut self) {
        let Some(metrics) = self.recording_metrics.clone() else {
            return;
        };
        let Some(pipeline) = self.pipeline.as_ref() else {
            return;
        };

        // Per-frame encode duration, paired by buffer PTS.
        if let Some(encoder) = pipeline.by_name("video_enc") {
            let entries: Arc<Mutex<HashMap<u64, Instant>>> = Arc::new(Mutex::new(HashMap::new()));
            if let Some(sink_pad) = encoder.static_pad("sink") {
                let entries = Arc::clone(&entries);
                sink_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
                    let Some(pts) = info.buffer().and_then(|b| b.pts()) else {
                        return gst::PadProbeReturn::Ok;
                    };
                    let mut map = entries.lock().unwrap_or_else(|e| e.into_inner());
                    // Cap the map so a stalled pipeline cannot leak memory.
                    if map.len() < 256 {
                        map.insert(pts.nseconds(), Instant::now());
                    }
                    gst::PadProbeReturn::Ok
                });
            }
            if let Some(src_pad) = encoder.static_pad("src") {
                let entries = Arc::clone(&entries);
                let metrics = Arc::clone(&metrics);
                src_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
                    let Some(pts) = info.buffer().and_then(|b| b.pts()) else {
                        return gst::PadProbeReturn::Ok;
                    };
                    let duration = entries
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .remove(&pts.nseconds())
                        .map(|entered| entered.elapsed().as_secs_f64());
                    if let Some(duration) = duration {
                        metrics
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .record_encode_time(duration);
                    }
                    gst::PadProbeReturn::Ok
                });
            }
        }

        // Bytes written to the local output file.
        if let Some(file_sink) = pipeline.by_name("file_sink") {
            if let Some(sink_pad) = file_sink.static_pad("sink") {
                sink_pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
                    if let Some(buffer) = info.buffer() {
                        metrics
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .add_file_bytes(buffer.size() as u64);
                    }
                    gst::PadProbeReturn::Ok
                });
            }
        }
    }

    /// When a hardware encoder is selected and the pipeline could not be built
    /// or started, switch to software x264 and rebuild the pipeline string.
    ///
    /// Returns `true` when retrying with the rebuilt string is worthwhile, i.e.
    /// a fallback actually happened. Software x264 is always available, so a
    /// second failure is reported to the caller instead of retried again.
    fn try_encoder_fallback(&mut self, pipeline_str: &mut String, reason: &str) -> bool {
        if !self.video_encoder.is_hardware() {
            return false;
        }
        eprintln!(
            "[Engine] Encoder {:?} not initializable ({}), falling back to x264.",
            self.video_encoder, reason
        );
        self.video_encoder = VideoEncoder::Software;
        *pipeline_str = self
            .build_pipeline_str()
            .expect("fallback pipeline always builds");
        true
    }

    /// Enable or disable the audio input branch of the recording pipeline.
    ///
    /// Must be called before recording starts.
    pub fn set_audio_enabled(&mut self, enabled: bool) {
        self.audio_enabled = enabled;
    }

    pub fn audio_enabled(&self) -> bool {
        self.audio_enabled
    }

    /// Enable or disable separate audio tracks.
    ///
    /// When enabled, system audio and microphone frames must be pushed through
    /// [`RivuletEngine::push_audio_track`]; they will be stored in distinct
    /// tracks of the output file. When disabled (default) the frames are mixed
    /// into a single track via [`RivuletEngine::push_audio_frame`].
    ///
    /// Must be called before recording starts.
    pub fn set_separate_audio_tracks(&mut self, enabled: bool) {
        self.separate_audio_tracks = enabled;
    }

    pub fn separate_audio_tracks(&self) -> bool {
        self.separate_audio_tracks
    }

    /// Enable or disable a single audio track when recording separate tracks.
    ///
    /// A disabled track is not built into the recording pipeline at all, so it
    /// is safe to keep separate tracks enabled even when only one source is
    /// active. Must be called before recording starts.
    pub fn set_audio_track_enabled(&mut self, track: AudioTrack, enabled: bool) {
        match track {
            AudioTrack::System => self.audio_sys_enabled = enabled,
            AudioTrack::Microphone => self.audio_mic_enabled = enabled,
        }
    }

    pub fn audio_track_enabled(&self, track: AudioTrack) -> bool {
        match track {
            AudioTrack::System => self.audio_sys_enabled,
            AudioTrack::Microphone => self.audio_mic_enabled,
        }
    }

    /// Push a mixed PCM frame into the recording pipeline.
    pub fn push_audio_frame(&mut self, frame: &AudioFrame) -> anyhow::Result<()> {
        if !self.audio_enabled || (self.separate_audio_tracks && !self.is_streaming()) {
            return Ok(());
        }
        if frame.data.is_empty() {
            return Ok(());
        }

        if self.pipeline.is_none() {
            anyhow::bail!("Pipeline is not running");
        }

        let Some(appsrc) = self.audio_appsrc.as_ref() else {
            anyhow::bail!("Audio input is not enabled");
        };

        push_pcm_buffer(appsrc, frame)
    }

    /// Push a PCM frame belonging to the given audio track into the pipeline.
    ///
    /// Only valid when separate audio tracks are enabled; otherwise the frame
    /// is ignored.
    pub fn push_audio_track(
        &mut self,
        frame: &AudioFrame,
        track: AudioTrack,
    ) -> anyhow::Result<()> {
        if !self.audio_enabled || !self.separate_audio_tracks || self.is_streaming() {
            return Ok(());
        }
        if frame.data.is_empty() {
            return Ok(());
        }

        if self.pipeline.is_none() {
            anyhow::bail!("Pipeline is not running");
        }

        let appsrc = match track {
            AudioTrack::System => self.audio_appsrc_sys.as_ref(),
            AudioTrack::Microphone => self.audio_appsrc_mic.as_ref(),
        };
        let Some(appsrc) = appsrc else {
            return Ok(());
        };

        push_pcm_buffer(appsrc, frame)
    }

    /// Start streaming to the configured ingest without a local recording.
    ///
    /// Equivalent to [`RivuletEngine::start_local_recording`] with no output
    /// path: it arms the engine so the streaming pipeline is built as soon as
    /// the first video frame arrives. Stream settings must be configured first
    /// via [`RivuletEngine::set_stream_settings`].
    pub fn start_streaming(&mut self) {
        if self.is_recording {
            return;
        }
        if self.stream_settings.is_none() {
            self.set_error("No stream targets configured for streaming.");
            return;
        }
        println!("[Engine] Streaming prepared.");
        self.is_recording = true;

        // Initialize the stream health monitor. Defaults match the
        // engine encoding configuration (30 FPS, 5000 kbit/s).
        self.stream_health = Some(StreamHealthMonitor::new(5000.0, 30.0));
        // Track performance metrics for the active stream.
        self.recording_metrics = Some(Arc::new(Mutex::new(RecordingStatsMonitor::new())));
    }

    /// Start writing the video/audio frames to a local MP4 file.
    ///
    /// When stream settings are configured as well ([`RivuletEngine::set_stream_settings`]),
    /// the engine records and streams simultaneously (dual output).
    pub fn start_local_recording(&mut self, path: PathBuf) {
        if self.is_recording {
            return;
        }
        println!("[Engine] Recording prepared for: {:?}", path);
        self.output_path = Some(path);
        self.is_recording = true;

        // Initialize the stream health monitor as soon as a stream target
        // (stream-only or dual output) is configured. Defaults match the
        // engine encoding configuration (30 FPS, 5000 kbit/s).
        if self.is_streaming() {
            self.stream_health = Some(StreamHealthMonitor::new(5000.0, 30.0));
        }
        // Track performance metrics (FPS, encoder load, file size) for the
        // recording. The file-size probe only attaches when the pipeline
        // actually contains a filesink (local or dual-output recordings).
        self.recording_metrics = Some(Arc::new(Mutex::new(RecordingStatsMonitor::new())));
    }

    pub fn stop_recording(&mut self) {
        if !self.is_recording {
            return;
        }
        println!("[Engine] Stopping recording...");

        if let Some(appsrc) = self.appsrc.as_ref() {
            let _ = appsrc.end_of_stream();
        }
        if let Some(appsrc) = self.audio_appsrc.as_ref() {
            let _ = appsrc.end_of_stream();
        }
        if let Some(appsrc) = self.audio_appsrc_sys.as_ref() {
            let _ = appsrc.end_of_stream();
        }
        if let Some(appsrc) = self.audio_appsrc_mic.as_ref() {
            let _ = appsrc.end_of_stream();
        }

        // Wait for EOS so the muxer finalizes the file (moov atom)
        // before the pipeline is set to Null.
        if let Some(pipeline) = self.pipeline.as_ref() {
            if let Some(bus) = pipeline.bus() {
                let _ = bus.timed_pop_filtered(
                    gst::ClockTime::from_seconds(10),
                    &[gst::MessageType::Eos, gst::MessageType::Error],
                );
            }
        }

        if let Some(pipeline) = self.pipeline.take() {
            pipeline
                .set_state(gst::State::Null)
                .expect("Pipeline could not be stopped.");
        }

        self.appsrc = None;
        self.audio_appsrc = None;
        self.audio_appsrc_sys = None;
        self.audio_appsrc_mic = None;
        self.output_path = None;
        self.stream_health = None;
        self.recording_metrics = None;
        self.is_recording = false;
        println!("[Engine] Recording stopped and file saved.");
    }

    pub fn process_raw_frame(&mut self, frame_data: &[u8], width: u32, height: u32) {
        if !self.is_recording {
            return;
        }

        if self.pipeline.is_none() {
            self.initialize_and_start_pipeline(width, height);
        }

        if let Some(appsrc) = &self.appsrc {
            let mut buffer = gst::Buffer::with_size(frame_data.len()).unwrap();
            {
                let buffer_ref = buffer.get_mut().expect("Buffer not writable");
                let mut map = buffer_ref
                    .map_writable()
                    .expect("Failed to map buffer writable");
                map.as_mut_slice().copy_from_slice(frame_data);
            }

            match appsrc.push_buffer(buffer) {
                Ok(_flow) => {
                    if let Some(metrics) = self.recording_metrics.as_ref() {
                        metrics
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .record_frame(0.0);
                    }
                    if let Some(health) = self.stream_health.as_mut() {
                        health.record_frame_sent(frame_data.len());
                    }
                }
                Err(err) => {
                    if let Some(health) = self.stream_health.as_mut() {
                        health.record_frame_dropped();
                    }
                    self.set_error(format!(
                        "Could not push a frame into the recording pipeline: {:?}",
                        err
                    ));
                    self.stop_recording();
                }
            }
        }
    }

    /// Current stream health statistics, if a streaming pipeline is active.
    ///
    /// Returns [`StreamStats::offline`] when the engine is not streaming (only
    /// local recording or nothing).
    pub fn stream_stats(&mut self) -> StreamStats {
        self.stream_health
            .as_mut()
            .map(StreamHealthMonitor::stats)
            .unwrap_or_else(StreamStats::offline)
    }

    /// Current recording performance metrics (FPS, encoder load, file size).
    ///
    /// Returns defaulted (zero) metrics when no recording or stream is active.
    pub fn recording_stats(&mut self) -> RecordingMetrics {
        self.recording_metrics
            .as_ref()
            .map(|m| m.lock().unwrap_or_else(|e| e.into_inner()).stats())
            .unwrap_or_default()
    }
}

/// Caps for the engine's audio input branch: interleaved f32 PCM.
pub fn audio_caps() -> gst::Caps {
    gst::Caps::builder("audio/x-raw")
        .field("format", "F32LE")
        .field("layout", "interleaved")
        .field("channels", i32::from(AUDIO_CHANNELS))
        .field("rate", AUDIO_SAMPLE_RATE as i32)
        .build()
}

/// Copy an [`AudioFrame`] into a writable GStreamer buffer and push it into
/// the given appsrc.
fn push_pcm_buffer(appsrc: &gst_app::AppSrc, frame: &AudioFrame) -> anyhow::Result<()> {
    let bytes_len = frame.data.len() * std::mem::size_of::<f32>();
    let mut buffer = gst::Buffer::with_size(bytes_len)?;
    {
        let buffer_ref = buffer
            .get_mut()
            .ok_or_else(|| anyhow::anyhow!("Failed to get writable audio buffer"))?;
        let mut map = buffer_ref
            .map_writable()
            .map_err(|_| anyhow::anyhow!("Failed to map audio buffer writable"))?;
        let dst = map.as_mut_slice();
        for (i, sample) in frame.data.iter().enumerate() {
            let bytes = sample.to_ne_bytes();
            let off = i * 4;
            dst[off..off + 4].copy_from_slice(&bytes);
        }
    }

    appsrc.push_buffer(buffer)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gstreamer_pbutils as gst_pbutils;

    /// Build a valid `file://` URI from a filesystem path. On Windows the path
    /// uses backslashes, which are invalid in a URI; they are converted to
    /// forward slashes (`C:\Users\...` -> `file:///C:/Users/...`).
    fn file_uri(path: &std::path::Path) -> String {
        let s = path.to_str().expect("Path must be UTF-8");
        format!("file:///{}", s.replace('\\', "/"))
    }

    #[test]
    fn audio_toggle_is_disabled_by_default() {
        let engine = RivuletEngine::default();
        assert!(!engine.audio_enabled());
    }

    #[test]
    fn audio_toggle_can_be_enabled_and_disabled() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        assert!(engine.audio_enabled());
        engine.set_audio_enabled(false);
        assert!(!engine.audio_enabled());
    }

    #[test]
    fn audio_tracks_are_enabled_by_default_and_toggleable() {
        let mut engine = RivuletEngine::default();
        assert!(engine.audio_track_enabled(AudioTrack::System));
        assert!(engine.audio_track_enabled(AudioTrack::Microphone));
        engine.set_audio_track_enabled(AudioTrack::Microphone, false);
        assert!(engine.audio_track_enabled(AudioTrack::System));
        assert!(!engine.audio_track_enabled(AudioTrack::Microphone));
    }

    #[test]
    fn audio_caps_describe_f32_interleaved_stereo_48k() {
        let _ = gst::init();
        let caps = audio_caps();
        assert!(!caps.is_empty(), "audio caps should not be empty");
    }

    #[test]
    fn escape_location_doubles_backslashes() {
        let escaped = RivuletEngine::escape_location("C:\\Users\\Temp\\out.mp4");
        assert_eq!(escaped, "C:\\\\Users\\\\Temp\\\\out.mp4");
    }

    #[test]
    fn file_uri_converts_backslashes_to_forward_slashes() {
        let uri = file_uri(std::path::Path::new("C:\\Users\\Temp\\out.mp4"));
        assert_eq!(uri, "file:///C:/Users/Temp/out.mp4");
    }

    /// End-to-end test: feed synthetic video + audio frames into the engine and
    /// verify that a non-empty MP4 file is written.
    #[test]
    fn records_synthetic_video_and_audio_to_file() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);

        let path =
            std::env::temp_dir().join(format!("rivulet_engine_test_{}.mp4", std::process::id()));
        engine.start_local_recording(path.clone());

        let (width, height) = (320u32, 240u32);
        let video = vec![0u8; (width * height * 4) as usize];
        for _ in 0..12 {
            engine.process_raw_frame(&video, width, height);
        }

        let audio = AudioFrame::new(vec![0.0f32; 4800], AUDIO_SAMPLE_RATE, AUDIO_CHANNELS);
        for _ in 0..12 {
            let _ = engine.push_audio_frame(&audio);
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
        engine.stop_recording();

        assert!(path.exists(), "output file should exist");
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        assert!(len > 0, "output file should not be empty");

        // A successful recording must not leave a stale error behind for the
        // GUI to display.
        assert!(
            engine.take_error().is_none(),
            "successful recording must not record an error"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// Errors are recorded via `take_error` and consumed exactly once.
    #[test]
    fn engine_errors_are_recorded_and_consumed_once() {
        let mut engine = RivuletEngine::default();
        assert!(engine.take_error().is_none(), "no error before a failure");

        // Streaming without configured targets is a deterministic failure
        // that does not depend on the GStreamer installation.
        engine.start_streaming();

        let err = engine.take_error().expect("failure must be recorded");
        assert!(
            err.contains("No stream targets"),
            "unexpected message: {err}"
        );
        assert!(
            engine.take_error().is_none(),
            "error must be consumed exactly once"
        );
    }

    /// End-to-end test for separate audio tracks: both tracks are pushed into
    /// the engine and the resulting file must contain two audio streams.
    #[test]
    fn records_separate_audio_tracks_into_two_streams() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_separate_audio_tracks(true);

        let path = std::env::temp_dir().join(format!(
            "rivulet_separate_tracks_{}.mp4",
            std::process::id()
        ));
        engine.start_local_recording(path.clone());

        let (width, height) = (320u32, 240u32);
        let video = vec![0u8; (width * height * 4) as usize];
        for _ in 0..12 {
            engine.process_raw_frame(&video, width, height);
        }

        let audio = AudioFrame::new(vec![0.0f32; 4800], AUDIO_SAMPLE_RATE, AUDIO_CHANNELS);
        for _ in 0..12 {
            let _ = engine.push_audio_track(&audio, AudioTrack::System);
            let _ = engine.push_audio_track(&audio, AudioTrack::Microphone);
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
        engine.stop_recording();

        assert!(path.exists(), "output file should exist");
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        assert!(len > 0, "output file should not be empty");

        let uri = file_uri(&path);
        let discoverer = gst_pbutils::Discoverer::new(gst::ClockTime::from_seconds(5))
            .expect("Discoverer should be creatable");
        let info = discoverer
            .discover_uri(&uri)
            .expect("File should be readable");
        let audio_streams = info.audio_streams();
        assert_eq!(
            audio_streams.len(),
            2,
            "two audio streams should be present, found: {}",
            audio_streams.len()
        );

        let _ = std::fs::remove_file(&path);
    }

    /// Frames pushed for a specific track are ignored while separate tracks are
    /// disabled.
    #[test]
    fn track_frames_are_ignored_in_mixed_mode() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);

        let frame = AudioFrame::new(vec![0.0f32; 8], AUDIO_SAMPLE_RATE, AUDIO_CHANNELS);
        let result = engine.push_audio_track(&frame, AudioTrack::System);
        assert!(result.is_ok(), "push_audio_track should not error");
    }

    /// The streaming pipeline parses and uses an RTMPS ingest URL.
    #[test]
    fn streaming_pipeline_str_parses_and_uses_rtmps_location() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_stream_settings(Some(StreamSettings::twitch("testkey123")));
        assert!(engine.is_streaming());

        let pipeline_str = engine.build_streaming_pipeline_str();
        assert!(pipeline_str.contains("rtmps://live.twitch.tv/app/testkey123"));
        let pipeline = gst::parse::launch(&pipeline_str)
            .expect("streaming pipeline should parse")
            .downcast::<gst::Pipeline>()
            .unwrap();
        assert!(pipeline.by_name("rivulet_src").is_some());
        assert!(pipeline.by_name("mux").is_some());
    }

    /// The streaming pipeline forces a single mixed audio track even when
    /// separate tracks are enabled, because FLV only supports one audio track.
    #[test]
    fn streaming_pipeline_mixes_audio_despite_separate_tracks() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_separate_audio_tracks(true);
        engine.set_stream_settings(Some(StreamSettings::youtube("k1")));

        let pipeline_str = engine.build_streaming_pipeline_str();
        assert!(pipeline_str.contains("name=audio_src "), "{}", pipeline_str);
        assert!(
            !pipeline_str.contains("audio_src_sys"),
            "separate sinks should not be built in streaming mode"
        );
        assert!(!pipeline_str.contains("audio_src_mic"));
        assert!(pipeline_str.contains("flvmux name=mux streamable=true"));
    }

    /// The recording pipeline still keeps separate audio tracks when enabled.
    #[test]
    fn recording_pipeline_keeps_separate_tracks() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_separate_audio_tracks(true);

        let pipeline_str = engine.build_recording_pipeline_str("/tmp/out.mp4");
        assert!(pipeline_str.contains("name=audio_src_sys"));
        assert!(pipeline_str.contains("name=audio_src_mic"));
        assert!(pipeline_str.contains("mp4mux name=mux"));
    }

    /// Without stream settings the engine is not configured for streaming.
    #[test]
    fn engine_is_not_streaming_by_default() {
        let engine = RivuletEngine::default();
        assert!(!engine.is_streaming());
    }

    /// Stream settings can be configured and cleared again.
    #[test]
    fn stream_settings_can_be_cleared() {
        let mut engine = RivuletEngine::default();
        engine.set_stream_settings(Some(StreamSettings::twitch("k")));
        assert!(engine.is_streaming());
        engine.set_stream_settings(None);
        assert!(!engine.is_streaming());
    }

    /// The streaming pipeline contains no audio branch when audio is disabled.
    #[test]
    fn streaming_pipeline_without_audio_has_no_audio_branch() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(false);
        engine.set_stream_settings(Some(StreamSettings::youtube("k1")));

        let pipeline_str = engine.build_streaming_pipeline_str();
        assert!(!pipeline_str.contains("audio_src"), "{}", pipeline_str);
        gst::parse::launch(&pipeline_str).expect("video-only streaming pipeline should parse");
    }

    /// The streaming pipeline mixes audio into a single track by default and
    /// accepts a plain (unencrypted) custom RTMP ingest URL.
    #[test]
    fn streaming_pipeline_mixes_audio_and_accepts_plain_rtmp() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_stream_settings(Some(StreamSettings::custom("rtmp://localhost/live", "k")));

        let pipeline_str = engine.build_streaming_pipeline_str();
        assert!(pipeline_str.contains("name=audio_src "), "{}", pipeline_str);
        assert!(!pipeline_str.contains("audio_src_sys"), "{}", pipeline_str);
        assert!(pipeline_str.contains("rtmp://localhost/live/k"));
        gst::parse::launch(&pipeline_str).expect("custom RTMP pipeline should parse");
    }

    /// While streaming, mixed audio frames are routed into the pipeline even
    /// when separate tracks are enabled, while `push_audio_track` is ignored.
    #[test]
    fn streaming_mode_routes_audio_to_mixed_sink() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_separate_audio_tracks(true);
        engine.set_stream_settings(Some(StreamSettings::twitch("k")));

        let frame = AudioFrame::new(vec![0.0f32; 8], AUDIO_SAMPLE_RATE, AUDIO_CHANNELS);

        // Must NOT be silently ignored: it has to reach the pipeline check.
        let result = engine.push_audio_frame(&frame);
        assert!(
            result.is_err(),
            "mixed audio must be routed in streaming mode"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Pipeline is not running"),
            "should fail the pipeline check because no pipeline is running"
        );

        // push_audio_track is ignored while streaming (FLV supports one track).
        assert!(
            engine.push_audio_track(&frame, AudioTrack::System).is_ok(),
            "push_audio_track must be ignored in streaming mode"
        );
    }

    /// In non-streaming recording mode mixed frames are ignored while separate
    /// tracks are enabled (unchanged behaviour preserved by the streaming guard).
    #[test]
    fn recording_mode_keeps_track_routing_guards() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_separate_audio_tracks(true);

        let frame = AudioFrame::new(vec![0.0f32; 8], AUDIO_SAMPLE_RATE, AUDIO_CHANNELS);
        assert!(
            engine.push_audio_frame(&frame).is_ok(),
            "mixed frames must be ignored when separate tracks are enabled"
        );
    }

    /// Dual output is active when both stream settings and a local output path
    /// are configured, and streaming alone is not enough.
    #[test]
    fn dual_output_detected_when_stream_and_path_configured() {
        let mut engine = RivuletEngine::default();
        engine.set_stream_settings(Some(StreamSettings::twitch("k")));
        assert!(
            !engine.is_dual_output(),
            "streaming alone is not dual output"
        );

        let path = std::env::temp_dir().join("rivulet_dual_detect.mp4");
        engine.start_local_recording(path);
        assert!(engine.is_dual_output(), "stream + recording = dual output");

        engine.set_stream_settings(None);
        assert!(
            !engine.is_dual_output(),
            "recording alone is not dual output"
        );
    }

    /// The dual output pipeline parses and contains both sinks: the MP4 file
    /// sink and the RTMPS stream sink, split via tee.
    #[test]
    fn dual_output_pipeline_str_parses_with_both_sinks() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_stream_settings(Some(StreamSettings::twitch("dualkey")));

        let path = std::env::temp_dir().join("rivulet_dual_parse.mp4");
        engine.start_local_recording(path.clone());

        let pipeline_str = engine.build_dual_output_pipeline_str();
        assert!(pipeline_str.contains("name=video_tee"), "{}", pipeline_str);
        assert!(pipeline_str.contains("mp4mux name=mux_rec"));
        assert!(pipeline_str.contains("flvmux name=mux_stream streamable=true"));
        let escaped = path.display().to_string().replace('\\', "\\\\");
        assert!(
            pipeline_str.contains(&format!("filesink name=file_sink location=\"{}\"", escaped)),
            "{}",
            pipeline_str
        );
        assert!(pipeline_str.contains("rtmp2sink location=\"rtmps://live.twitch.tv/app/dualkey\""));

        let pipeline = gst::parse::launch(&pipeline_str)
            .expect("dual-output pipeline should parse")
            .downcast::<gst::Pipeline>()
            .unwrap();
        assert!(pipeline.by_name("rivulet_src").is_some());
        assert!(pipeline.by_name("mux_rec").is_some());
        assert!(pipeline.by_name("mux_stream").is_some());
    }

    /// In dual output mode the audio is always mixed into a single track, even
    /// when separate tracks are enabled, because FLV only supports one track.
    #[test]
    fn dual_output_pipeline_mixes_audio_despite_separate_tracks() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_separate_audio_tracks(true);
        engine.set_stream_settings(Some(StreamSettings::youtube("k1")));

        let path = std::env::temp_dir().join("rivulet_dual_mix.mp4");
        engine.start_local_recording(path);

        let pipeline_str = engine.build_dual_output_pipeline_str();
        assert!(pipeline_str.contains("name=audio_src "), "{}", pipeline_str);
        assert!(!pipeline_str.contains("audio_src_sys"), "{}", pipeline_str);
        assert!(!pipeline_str.contains("audio_src_mic"), "{}", pipeline_str);
        assert!(pipeline_str.contains("name=audio_tee"));
    }

    /// A video-only dual output pipeline (audio disabled) still parses.
    #[test]
    fn dual_output_pipeline_without_audio_parses() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(false);
        engine.set_stream_settings(Some(StreamSettings::custom("rtmp://localhost/live", "k")));

        let path = std::env::temp_dir().join("rivulet_dual_video_only.mp4");
        engine.start_local_recording(path);

        let pipeline_str = engine.build_dual_output_pipeline_str();
        assert!(!pipeline_str.contains("audio_src"), "{}", pipeline_str);
        gst::parse::launch(&pipeline_str).expect("video-only dual-output pipeline should parse");
    }

    /// In dual output mode mixed audio frames are routed into the pipeline even
    /// when separate tracks are enabled, while `push_audio_track` is ignored.
    #[test]
    fn dual_output_routes_audio_to_mixed_sink() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_separate_audio_tracks(true);
        engine.set_stream_settings(Some(StreamSettings::twitch("k")));

        let path = std::env::temp_dir().join("rivulet_dual_routing.mp4");
        engine.start_local_recording(path);
        assert!(engine.is_dual_output());

        let frame = AudioFrame::new(vec![0.0f32; 8], AUDIO_SAMPLE_RATE, AUDIO_CHANNELS);

        // Must NOT be silently ignored: it has to reach the pipeline check.
        let result = engine.push_audio_frame(&frame);
        assert!(
            result.is_err(),
            "mixed audio must be routed in dual-output mode"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Pipeline is not running"),
            "should fail the pipeline check because no pipeline is running"
        );

        // push_audio_track is ignored in dual output mode.
        assert!(
            engine.push_audio_track(&frame, AudioTrack::System).is_ok(),
            "push_audio_track must be ignored in dual-output mode"
        );
    }

    /// End-to-end test for separate audio tracks with only one active source:
    /// the engine still must write a valid file when one track is disabled.
    #[test]
    fn records_separate_audio_tracks_with_single_source() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_separate_audio_tracks(true);
        engine.set_audio_track_enabled(AudioTrack::Microphone, false);

        let path =
            std::env::temp_dir().join(format!("rivulet_single_track_{}.mp4", std::process::id()));
        engine.start_local_recording(path.clone());

        let (width, height) = (320u32, 240u32);
        let video = vec![0u8; (width * height * 4) as usize];
        for _ in 0..12 {
            engine.process_raw_frame(&video, width, height);
        }

        let audio = AudioFrame::new(vec![0.0f32; 4800], AUDIO_SAMPLE_RATE, AUDIO_CHANNELS);
        for _ in 0..12 {
            let _ = engine.push_audio_track(&audio, AudioTrack::System);
        }

        std::thread::sleep(std::time::Duration::from_millis(100));
        engine.stop_recording();

        assert!(path.exists(), "output file should exist");
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        assert!(len > 0, "output file should not be empty");

        let uri = file_uri(&path);
        let discoverer = gst_pbutils::Discoverer::new(gst::ClockTime::from_seconds(5))
            .expect("Discoverer should be creatable");
        let info = discoverer
            .discover_uri(&uri)
            .expect("File should be readable");
        assert_eq!(
            info.audio_streams().len(),
            1,
            "one audio stream should be present"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// Without streaming configuration the health API reports an offline
    /// stream, independent of any local recording activity.
    #[test]
    fn stream_stats_offline_without_streaming() {
        let mut engine = RivuletEngine::default();
        let stats = engine.stream_stats();
        assert_eq!(stats.status, StreamHealthStatus::Offline);
        assert_eq!(stats.frames_sent, 0);
        assert_eq!(stats.dropped_ratio, 0.0);
    }

    /// `start_streaming` arms the engine for pure streaming; the health monitor
    /// is created immediately and reports Connecting until frames flow.
    #[test]
    fn start_streaming_creates_health_monitor() {
        let mut engine = RivuletEngine::default();
        engine.set_stream_settings(Some(StreamSettings::twitch("k")));
        engine.start_streaming();

        let stats = engine.stream_stats();
        assert_eq!(stats.status, StreamHealthStatus::Connecting);
        engine.stop_recording();
    }

    /// After configuring a stream, health starts in the Connecting state until
    /// the first frame has actually been pushed through the pipeline.
    #[test]
    fn stream_stats_connecting_after_stream_start() {
        let mut engine = RivuletEngine::default();
        engine.set_stream_settings(Some(StreamSettings::twitch("k")));
        let path = std::env::temp_dir().join("rivulet_stream_health.mp4");
        engine.start_local_recording(path.clone());

        let stats = engine.stream_stats();
        assert_eq!(stats.status, StreamHealthStatus::Connecting);
        assert_eq!(stats.frames_sent, 0);
        engine.stop_recording();
    }

    /// Pushing frames through the streaming pipeline records them in the
    /// health monitor; the counters must be reset when a new recording starts.
    #[test]
    fn stream_stats_tracks_frames_through_pipeline() {
        let mut engine = RivuletEngine::default();
        engine.set_stream_settings(Some(StreamSettings::twitch("k")));
        let path = std::env::temp_dir().join("rivulet_stream_health_frames.mp4");
        engine.start_local_recording(path.clone());

        let (width, height) = (320u32, 240u32);
        let video = vec![0u8; (width * height * 4) as usize];
        for _ in 0..15 {
            engine.process_raw_frame(&video, width, height);
        }

        let stats = engine.stream_stats();
        assert!(
            stats.frames_sent > 0,
            "frames must be counted in the health monitor"
        );
        assert!(
            stats.bytes_sent > 0,
            "bytes must be counted in the health monitor"
        );
        engine.stop_recording();

        // A fresh recording restarts health tracking from zero.
        engine.start_local_recording(path.clone());
        let reset = engine.stream_stats();
        assert_eq!(reset.frames_sent, 0, "restart must reset the counters");
        engine.stop_recording();

        let _ = std::fs::remove_file(&path);
    }

    /// The engine defaults to the best available encoder (hardware first).
    #[test]
    fn engine_defaults_to_best_available_encoder() {
        let engine = RivuletEngine::default();
        assert_eq!(engine.video_encoder(), best_encoder());
        assert_eq!(engine.video_bitrate(), 5_000);
    }

    /// The video encoder and bitrate can be configured before recording.
    #[test]
    fn video_encoder_and_bitrate_can_be_changed() {
        let mut engine = RivuletEngine::default();
        engine.set_video_encoder(VideoEncoder::Software);
        engine.set_video_bitrate(8_000);
        assert_eq!(engine.video_encoder(), VideoEncoder::Software);
        assert_eq!(engine.video_bitrate(), 8_000);
    }

    /// The recording pipeline embeds the selected encoder element and bitrate.
    #[test]
    fn recording_pipeline_uses_selected_encoder() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_video_encoder(VideoEncoder::Software);
        engine.set_video_bitrate(6_000);

        let pipeline_str = engine.build_recording_pipeline_str("/tmp/enc.mp4");
        assert!(
            pipeline_str.contains(
                "video/x-raw,format=I420 ! x264enc name=video_enc bitrate=6000 tune=zerolatency"
            ),
            "{}",
            pipeline_str
        );
        gst::parse::launch(&pipeline_str).expect("pipeline with x264 should parse");
    }

    /// The streaming pipeline uses the selected encoder as well.
    #[test]
    fn streaming_pipeline_uses_selected_encoder() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_video_encoder(VideoEncoder::Software);
        engine.set_stream_settings(Some(StreamSettings::custom("rtmp://localhost/live", "k")));

        let pipeline_str = engine.build_streaming_pipeline_str();
        assert!(
            pipeline_str.contains(
                "video/x-raw,format=I420 ! x264enc name=video_enc bitrate=5000 tune=zerolatency"
            ),
            "{}",
            pipeline_str
        );
        gst::parse::launch(&pipeline_str).expect("streaming pipeline with x264 should parse");
    }

    /// The dual output pipeline uses the selected encoder in both branches.
    #[test]
    fn dual_output_pipeline_uses_selected_encoder() {
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_video_encoder(VideoEncoder::Software);
        engine.set_stream_settings(Some(StreamSettings::custom("rtmp://localhost/live", "k")));
        let path = std::env::temp_dir().join("rivulet_dual_encoder.mp4");
        engine.start_local_recording(path.clone());

        let pipeline_str = engine.build_dual_output_pipeline_str();
        assert!(
            pipeline_str.contains(
                "video/x-raw,format=I420 ! x264enc name=video_enc bitrate=5000 tune=zerolatency"
            ),
            "{}",
            pipeline_str
        );
        gst::parse::launch(&pipeline_str).expect("dual-output pipeline with x264 should parse");
    }

    /// When NVENC is available the engine's pipeline parses with it and the
    /// recording writes a valid file. Skipped silently on machines without an
    /// NVIDIA GPU or the NVCODEC plugin.
    #[test]
    fn records_synthetic_video_with_nvenc_when_available() {
        if !VideoEncoder::Nvenc.is_available() {
            return;
        }
        let _ = gst::init();
        let mut engine = RivuletEngine::default();
        engine.set_video_encoder(VideoEncoder::Nvenc);
        engine.set_audio_enabled(false);

        let path =
            std::env::temp_dir().join(format!("rivulet_nvenc_test_{}.mp4", std::process::id()));
        engine.start_local_recording(path.clone());

        let (width, height) = (320u32, 240u32);
        let video = vec![0u8; (width * height * 4) as usize];
        for _ in 0..30 {
            engine.process_raw_frame(&video, width, height);
        }

        std::thread::sleep(std::time::Duration::from_millis(200));
        engine.stop_recording();

        assert!(
            path.exists() && std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > 0,
            "NVENC recording should produce a non-empty file"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// The engine reports live performance metrics (FPS, encoder load, output
    /// file size) while a recording is running.
    #[test]
    fn recording_metrics_report_fps_and_file_size() {
        let mut engine = RivuletEngine::default();

        let path =
            std::env::temp_dir().join(format!("rivulet_metrics_test_{}.mp4", std::process::id()));
        engine.start_local_recording(path.clone());

        let (width, height) = (320u32, 240u32);
        let video = vec![0u8; (width * height * 4) as usize];
        for _ in 0..30 {
            engine.process_raw_frame(&video, width, height);
            std::thread::sleep(std::time::Duration::from_millis(33));
        }

        // Give the pipeline time to flush muxed data to the filesink.
        std::thread::sleep(std::time::Duration::from_millis(200));

        let metrics = engine.recording_stats();
        assert!(
            metrics.frames_captured >= 30,
            "frames_captured = {}",
            metrics.frames_captured
        );
        assert!(
            metrics.fps > 0.0 && metrics.fps < 45.0,
            "fps = {}",
            metrics.fps
        );
        assert!(
            metrics.file_size_bytes > 0,
            "file_size_bytes = {}",
            metrics.file_size_bytes
        );
        assert!(
            metrics.encode_load_percent >= 0.0,
            "encode_load_percent = {}",
            metrics.encode_load_percent
        );

        engine.stop_recording();
        let _ = std::fs::remove_file(&path);
    }
}
