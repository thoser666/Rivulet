// In rivulet-core/src/lib.rs

use gstreamer as gst;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
// Important: The prelude is still needed for methods like .set_state(), .by_name(), etc.
use gst::prelude::*;

pub mod audio;
pub use audio::{AudioFrame, AudioTrack, SkippedFilter};

pub mod camera;
pub use camera::{CameraConfig, CameraDevice, CameraFrame};

pub mod game_capture;
pub use game_capture::{GameCaptureConfig, GameCaptureFrame, GameWindow};

pub mod replay;
pub use replay::{save_replay, ReplayBuffer, ReplaySegment, ReplaySnapshot};

pub mod reconnect;
pub mod stream_runtime;
pub use reconnect::{
    target_index_from_sink_name, ReconnectCommand, ReconnectWorker, RetryPolicy, SinkBusEvent,
    StreamingReconnectSupervisor, TargetReconnectState,
};
pub use stream_runtime::{AdaptiveBitrateController, DelaySupervisor};

pub mod encoder;
pub use encoder::{
    best_encoder, best_encoder_for_codec, detect_available_encoders,
    detect_available_encoders_for_codec, VideoCodec, VideoEncoder,
};

pub mod health;
pub use health::{StreamHealthMonitor, StreamHealthStatus, StreamStats};

pub mod metrics;
pub use metrics::{RecordingMetrics, RecordingStatsMonitor};

pub mod srt;
pub mod stream;
pub use srt::{ContributionProtocol, ContributionSettings};
pub use stream::{
    parse_whip_response, AdaptiveBitrate, MultistreamSettings, MultitrackVideo, StreamDelay,
    StreamPlatform, StreamPreset, StreamSettings, StreamTarget, StreamTargetState,
    TargetPluginStatus, WhipSession, WhipSettings,
};

pub mod i18n;
pub use i18n::Locale;

pub mod preset;
pub use preset::RecordingPreset;

pub mod region;
pub use region::CaptureRegion;

pub mod scene;
pub use scene::{Scene, SceneManager};

pub mod scene_snapshot;
pub use scene_snapshot::{SceneSnapshot, SnapshotLayer};

pub mod transition;
pub use transition::{SceneTransition, TransitionKind};

pub mod studio;
pub use studio::StudioMode;

pub mod source;
pub use source::{ChromaKey, Crop, SceneSource, Source, SourceKind, SourceManager, Transform};

pub mod image_source;
pub use image_source::{is_image_file, supported_image_extensions, ImageSource};

pub mod text_source;
pub use text_source::{FontWeight, Rgba, ScrollDirection, TextAlign, TextSource};

pub mod webcam_source;
pub use webcam_source::{PixelFormat, Resolution, WebcamSource};

pub mod color_source;
pub use color_source::ColorSource;

pub mod media_source;
pub use media_source::{MediaSource, MediaType, PlaybackMode};

pub mod audio_source;
pub use audio_source::{AudioSource, AudioSourceKind};

pub mod browser_source;
pub use browser_source::{
    BrowserFrame, BrowserInput, BrowserMouseButton, BrowserSource, BrowserSourceBackend,
    BrowserSourceError,
};

pub mod benchmark;
pub mod capture_channel;
pub mod opengl_hook;
pub mod vulkan_hook;
pub mod vulkan_layer;
pub use benchmark::{BenchmarkReport, FramePercentiles, OverheadBudget, OverheadResult, SampleSet};

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
    stream_targets: Option<MultistreamSettings>,
    stream_target_states: Vec<StreamTargetState>,
    reconnect_supervisor: Option<StreamingReconnectSupervisor>,
    reconnect_worker: Option<ReconnectWorker>,
    bitrate_controller: AdaptiveBitrateController,
    delay_supervisors: Vec<DelaySupervisor>,
    reconnect_started: Option<Instant>,
    /// Video encoder used for the H.264 video branch. Defaults to the best
    /// available encoder detected at construction time.
    video_encoder: VideoEncoder,
    /// Video codec selected for recording (H.264, H.265, or VP9).
    video_codec: VideoCodec,
    /// Recording preset controlling resolution, FPS, and bitrate.
    preset: RecordingPreset,
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
    /// When `true`, a `textoverlay` element is included in the video branch
    /// that burns a timer and FPS counter into the recorded video.
    show_overlay: bool,
    /// RAM ring buffer keeping the last N seconds of encoded video/audio so
    /// the user can save an "instant replay" clip. `None` disables the
    /// replay buffer (and the tee/appsink branches it adds to the pipeline).
    /// The buffer itself is written from the GStreamer streaming thread, so
    /// it lives behind an `Arc<Mutex<..>>`.
    replay: Option<Arc<Mutex<ReplayBuffer>>>,
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
            stream_targets: None,
            stream_target_states: Vec::new(),
            reconnect_supervisor: None,
            reconnect_worker: None,
            reconnect_started: None,
            bitrate_controller: AdaptiveBitrateController::default(),
            delay_supervisors: Vec::new(),
            video_encoder: best_encoder(),
            video_codec: VideoCodec::default(),
            preset: RecordingPreset::default(),
            encoder_bitrate_kbps: 5_000,
            stream_health: None,
            recording_metrics: None,
            last_error: None,
            show_overlay: false,
            replay: None,
        }
    }
}

impl RivuletEngine {
    pub fn new() -> Self {
        tracing::info!("GStreamer ready.");
        Self::default()
    }

    /// Record an engine error for display in the GUI and log it to stderr.
    ///
    /// The most recent error is retained until it is consumed via
    /// [`RivuletEngine::take_error`], so the UI can surface it next to the
    /// recording controls instead of relying on console output alone.
    fn set_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        tracing::error!("Engine error: {message}");
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
        self.stream_targets = None;
        self.stream_target_states.clear();
    }

    /// Configure independent streaming targets. The primary settings remain
    /// the compatibility fallback when no target list is supplied.
    pub fn set_multistream_settings(&mut self, settings: Option<MultistreamSettings>) {
        self.stream_targets = settings;
        self.stream_target_states = self
            .stream_targets
            .as_ref()
            .map(|value| vec![StreamTargetState::Connecting; value.targets.len()])
            .unwrap_or_default();
        self.reconnect_supervisor = self.stream_targets.as_ref().map(|value| {
            StreamingReconnectSupervisor::new(
                value.targets.iter().map(|target| target.name.clone()),
                RetryPolicy::default(),
            )
        });
        self.reconnect_started = self.reconnect_supervisor.as_ref().map(|_| Instant::now());
        self.delay_supervisors = self
            .stream_targets
            .as_ref()
            .map(|targets| {
                targets
                    .targets
                    .iter()
                    .map(|target| {
                        DelaySupervisor::new(
                            target.settings.delay.duration,
                            Duration::from_secs(300),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    pub fn stream_target_states(&self) -> &[StreamTargetState] {
        &self.stream_target_states
    }

    /// Update one target from a GStreamer bus event and return whether the
    /// target should be retried. This keeps bus handling target-local.
    pub fn handle_stream_target_failure(&mut self, target: &str) -> bool {
        let retry = self
            .reconnect_supervisor
            .as_mut()
            .and_then(|supervisor| supervisor.mark_failed_once(target))
            .unwrap_or(false);
        if let Some(index) = self
            .stream_targets
            .as_ref()
            .and_then(|targets| targets.targets.iter().position(|item| item.name == target))
        {
            self.stream_target_states[index] = StreamTargetState::Failed;
        }
        tracing::warn!(target, retry, "Streaming target failed");
        if retry {
            if let Some(supervisor) = self.reconnect_supervisor.as_ref() {
                if let Some(state) = supervisor
                    .targets()
                    .iter()
                    .find(|state| state.name == target)
                {
                    self.reconnect_worker =
                        Some(ReconnectWorker::start(target, state.next_backoff));
                }
            }
        }
        retry
    }

    /// Mark a target live after its sink has connected or reconnected.
    pub fn handle_stream_target_live(&mut self, target: &str) -> bool {
        let found = self
            .reconnect_supervisor
            .as_mut()
            .map(|supervisor| supervisor.mark_live(target))
            .unwrap_or(false);
        if let Some(index) = self
            .stream_targets
            .as_ref()
            .and_then(|targets| targets.targets.iter().position(|item| item.name == target))
        {
            self.stream_target_states[index] = StreamTargetState::Live;
        }
        found
    }

    /// Poll non-blocking retry timers and claim targets whose backoff elapsed.
    pub fn poll_stream_reconnects(&mut self) -> Vec<String> {
        let Some(started) = self.reconnect_started else {
            return Vec::new();
        };
        self.reconnect_supervisor
            .as_mut()
            .map(|supervisor| supervisor.due_retries(started.elapsed()))
            .unwrap_or_default()
    }

    /// Consume queued branch-rebuild commands on the pipeline owner thread.
    /// The command is only a target name; this method deliberately keeps all
    /// GStreamer mutation on the caller's context.
    pub fn drain_reconnect_commands(&mut self) -> Vec<ReconnectCommand> {
        self.reconnect_worker
            .as_ref()
            .map(ReconnectWorker::drain)
            .unwrap_or_default()
    }

    /// Poll the bus, consume due worker commands, and rebuild only affected
    /// targets. This is intended for the pipeline-owner tick; it never blocks.
    pub fn service_streaming(&mut self) -> anyhow::Result<Vec<String>> {
        self.poll_stream_bus();
        self.service_adaptive_bitrate();
        let commands = self.drain_reconnect_commands();
        let mut rebuilt = Vec::new();
        for command in commands {
            if let ReconnectCommand::RebuildBranch(target) = command {
                if self.rebuild_stream_target(&target)? {
                    rebuilt.push(target);
                }
            }
        }
        Ok(rebuilt)
    }

    /// Apply the latest stream-health sample to the bounded live bitrate controller.
    /// This is called from the non-blocking streaming-owner tick.
    pub fn service_adaptive_bitrate(&mut self) -> bool {
        let Some(health) = self.stream_health.as_mut() else {
            return false;
        };
        let status = health.stats().status;
        self.reconfigure_stream_bitrate(status)
    }

    /// Process pending GStreamer bus messages without blocking the caller.
    /// Applications should call this from their periodic pipeline-owner tick.
    pub fn poll_stream_bus(&mut self) -> usize {
        let Some(pipeline) = self.pipeline.as_ref() else {
            return 0;
        };
        let Some(bus) = pipeline.bus() else {
            return 0;
        };
        let mut processed = 0;
        while let Some(message) = bus.pop() {
            processed += 1;
            let name = message
                .src()
                .and_then(|src| src.name().strip_prefix("stream_sink_").map(str::to_owned));
            let Some(name) = name else {
                continue;
            };
            let event = match message.view() {
                gst::MessageView::Error(_) | gst::MessageView::Eos(..) => Some(SinkBusEvent::Error),
                gst::MessageView::Warning(_) => Some(SinkBusEvent::Warning),
                gst::MessageView::StateChanged(_) => Some(SinkBusEvent::StateChanged),
                _ => None,
            };
            if let Some(event) = event {
                if event == SinkBusEvent::Error {
                    self.handle_stream_target_failure(&name);
                } else if event == SinkBusEvent::StateChanged {
                    self.handle_stream_target_live(&name);
                }
            }
        }
        processed
    }

    /// Resolve and rebuild one target sink branch while the caller owns the
    /// pipeline context. The operation is intentionally conservative: it
    /// recreates the complete streaming pipeline only when no local output is
    /// active, preserving all configured targets and target isolation.
    pub fn rebuild_stream_target(&mut self, target: &str) -> anyhow::Result<bool> {
        let Some(index) = self
            .stream_targets
            .as_ref()
            .and_then(|targets| targets.targets.iter().position(|item| item.name == target))
        else {
            return Ok(false);
        };
        let Some(pipeline) = self.pipeline.take() else {
            return Ok(false);
        };
        let sink_name = format!("stream_sink_{index}");
        let Some(sink) = pipeline.by_name(&sink_name) else {
            self.pipeline = Some(pipeline);
            anyhow::bail!("stream target sink is missing: {sink_name}");
        };
        sink.set_state(gst::State::Null)?;
        pipeline.remove(&sink)?;
        // Recreate the branch from the current target settings. The target
        // remains independent; failure is returned without stopping peers.
        let target = self
            .stream_targets
            .as_ref()
            .and_then(|targets| targets.targets.get(index))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("stream target disappeared during rebuild"))?;
        let sink_fragment = target
            .contribution
            .as_ref()
            .and_then(|contribution| contribution.validated_pipeline_sink_fragment().ok())
            .unwrap_or_else(|| {
                format!("rtmp2sink location=\\\"{}\\\"", target.settings.location())
            });
        let description = format!("{} name={sink_name}", sink_fragment);
        let rebuilt = gst::parse::launch(&description)?
            .downcast::<gst::Element>()
            .map_err(|_| anyhow::anyhow!("rebuilt stream sink is not a GStreamer element"))?;
        pipeline.add(&rebuilt)?;
        rebuilt.sync_state_with_parent()?;
        self.pipeline = Some(pipeline);
        self.handle_stream_target_live(&target.name);
        Ok(true)
    }

    /// Resolve a rebuild command against the current pipeline. The engine
    /// reports the target index; actual element removal/relinking remains
    /// deliberately explicit because a GStreamer branch must be mutated on its
    /// owning pipeline context.
    pub fn resolve_reconnect_command(&self, command: &ReconnectCommand) -> Option<usize> {
        match command {
            ReconnectCommand::RebuildBranch(name) => {
                self.stream_targets.as_ref().and_then(|targets| {
                    targets
                        .targets
                        .iter()
                        .position(|target| target.name == *name)
                })
            }
            ReconnectCommand::Shutdown => None,
        }
    }

    pub fn retryable_stream_targets(&self) -> Vec<&str> {
        self.reconnect_supervisor
            .as_ref()
            .map(StreamingReconnectSupervisor::retryable_targets)
            .unwrap_or_default()
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

    /// Select the video codec for recording.
    ///
    /// Must be called before recording starts. Defaults to H.264. Changing the
    /// codec also selects the best available encoder backend for that codec.
    pub fn set_video_codec(&mut self, codec: VideoCodec) {
        self.video_codec = codec;
        self.video_encoder = best_encoder_for_codec(codec);
    }

    /// The currently selected video codec.
    pub fn video_codec(&self) -> VideoCodec {
        self.video_codec
    }

    /// Select a recording preset that controls resolution, FPS, and bitrate.
    ///
    /// Must be called before recording starts. Defaults to
    /// [`RecordingPreset::ORIGINAL`] (no transformation). If the preset
    /// specifies a bitrate, it overrides the value set via
    /// [`set_video_bitrate`](Self::set_video_bitrate).
    pub fn set_preset(&mut self, preset: RecordingPreset) {
        if let Some(bitrate) = preset.bitrate_kbps {
            self.encoder_bitrate_kbps = bitrate;
        }
        self.preset = preset;
    }

    /// The currently selected recording preset.
    pub fn preset(&self) -> RecordingPreset {
        self.preset
    }

    /// Set the target video bitrate in kbit/s applied to the encoder.
    ///
    /// Must be called before recording starts. Defaults to 5000 kbit/s.
    pub fn set_video_bitrate(&mut self, kbps: u32) {
        self.encoder_bitrate_kbps = kbps;
    }

    /// The configured target video bitrate in kbit/s.
    /// Update the active encoder bitrate. Returns whether the encoder property
    /// was changed; callers can invoke this from their health polling loop.
    pub fn update_stream_bitrate_from_health(
        &mut self,
        status: StreamHealthStatus,
        elapsed: Duration,
    ) -> bool {
        let Some(settings) = self.stream_settings.as_ref() else {
            return false;
        };
        let changed = self.bitrate_controller.update(
            settings.adaptive_bitrate,
            &mut self.encoder_bitrate_kbps,
            status,
            elapsed,
        );
        if changed {
            if let Some(encoder) = self
                .pipeline
                .as_ref()
                .and_then(|pipeline| pipeline.by_name("video_enc"))
            {
                encoder.set_property("bitrate", self.encoder_bitrate_kbps);
            }
        }
        changed
    }

    pub fn reconfigure_stream_bitrate(&mut self, status: StreamHealthStatus) -> bool {
        let Some(settings) = self.stream_settings.as_ref() else {
            return false;
        };
        let changed = settings
            .adaptive_bitrate
            .reconfigure_live_encoder(&mut self.encoder_bitrate_kbps, status);
        if changed {
            if let Some(pipeline) = self.pipeline.as_ref() {
                if let Some(encoder) = pipeline.by_name("video_enc") {
                    encoder.set_property("bitrate", self.encoder_bitrate_kbps);
                }
            }
        }
        changed
    }

    pub fn video_bitrate(&self) -> u32 {
        self.encoder_bitrate_kbps
    }

    /// Enable or disable the recording overlay (timer + FPS counter burned
    /// into the video). Must be called before recording starts.
    pub fn set_overlay_enabled(&mut self, enabled: bool) {
        self.show_overlay = enabled;
    }

    /// Whether the recording overlay is enabled.
    pub fn overlay_enabled(&self) -> bool {
        self.show_overlay
    }

    /// Update the text shown by the `textoverlay` element in a running
    /// pipeline. The `text` is rendered directly into the recorded video.
    ///
    /// Does nothing when the overlay is disabled or no pipeline is active.
    pub fn update_overlay_text(&self, text: &str) {
        let Some(pipeline) = self.pipeline.as_ref() else {
            return;
        };
        if let Some(overlay) = pipeline.by_name("overlay") {
            overlay.set_property("text", text);
        }
    }

    /// Escape a location string for embedding into a `parse_launch` pipeline
    /// description. GStreamer treats `\` as an escape character in quoted
    /// property values; on Windows paths like `C:\Users\...` would otherwise be
    /// silently mangled (e.g. `C:UsersRUNNER~1...`).
    fn escape_location(location: &str) -> String {
        location.replace('\\', "\\\\")
    }

    /// Fragment appended after the video encoder: either a plain queue into
    /// the muxer, or (when the replay buffer is enabled) a tee that also
    /// feeds a parsing appsink so encoded packets can be captured into the
    /// ring. The `h264parse` in the capture branch yields AVC caps carrying
    /// `codec_data`, which lets a replay clip start mid-stream.
    fn replay_video_branch(&self) -> &'static str {
        if self.replay.is_some() {
            " ! tee name=replay_vtee ! queue ! mux. \
             replay_vtee. ! queue ! h264parse ! appsink name=replay_video_sink"
        } else {
            " ! queue ! mux."
        }
    }

    fn video_branch_str(&self) -> String {
        let transform = self.preset.transform_fragment();
        let caps = self
            .video_encoder
            .input_caps_fragment_for_codec(self.video_codec);
        let encoder = self
            .video_encoder
            .branch_fragment_for_codec(self.video_codec, self.encoder_bitrate_kbps);
        let overlay = if self.show_overlay {
            " ! textoverlay name=overlay text=\"\" valignment=top halignment=right \
             font-desc=\"Sans, 24\" background-color=0x000000AA "
        } else {
            ""
        };
        if transform.is_empty() {
            format!(
                "appsrc name=rivulet_src format=time is-live=true do-timestamp=true \
                 ! videoconvert ! {}{} ! {}{} ",
                caps,
                overlay,
                encoder,
                self.replay_video_branch()
            )
        } else {
            // Transform includes videoscale/videorate and target resolution/FPS.
            // We need to append the format (I420/NV12) so the encoder gets the
            // right input format.
            let transform_caps = format!("{}!{}", transform.trim_end(), caps.trim_start());
            format!(
                "appsrc name=rivulet_src format=time is-live=true do-timestamp=true \
                 ! videoconvert ! {}{} ! {}{} ",
                transform_caps,
                overlay,
                encoder,
                self.replay_video_branch()
            )
        }
    }

    /// Build the audio branch of the pipeline. With `force_mixed` the sources
    /// are mixed into a single track regardless of `separate_audio_tracks`; the
    /// FLV muxer used for streaming only supports a single audio track.
    ///
    /// When the replay buffer is enabled, every audio branch additionally
    /// feeds a tee into a dedicated appsink (`replay_audio_sink_<n>`) so the
    /// encoded AAC frames can be captured into the ring.
    fn audio_branch_str(&self, force_mixed: bool) -> String {
        if !self.audio_enabled {
            return String::new();
        }
        let replay_enabled = self.replay.is_some();
        let mut branch_idx = 0usize;
        let mut push_branch = |out: &mut String, appsrc_name: &str| {
            let idx = branch_idx;
            branch_idx += 1;
            let replay = if replay_enabled {
                format!(
                    " ! tee name=replay_atee{idx} ! queue ! mux. \
                     replay_atee{idx}. ! queue ! appsink name=replay_audio_sink_{idx}"
                )
            } else {
                " ! queue ! mux.".to_string()
            };
            out.push_str(&format!(
                "appsrc name={appsrc_name} format=time is-live=true do-timestamp=true \
                 ! audioconvert ! audioresample ! avenc_aac{replay} "
            ));
        };
        let mut s = String::new();
        if self.separate_audio_tracks && !force_mixed {
            if self.audio_sys_enabled {
                push_branch(&mut s, "audio_src_sys");
            }
            if self.audio_mic_enabled {
                push_branch(&mut s, "audio_src_mic");
            }
        } else {
            push_branch(&mut s, "audio_src");
        }
        s
    }

    fn build_recording_pipeline_str(&self, location: &str) -> String {
        let mut s = self.video_branch_str();
        s.push_str(&self.audio_branch_str(false));
        let escaped = Self::escape_location(location);
        let muxer = self.video_codec.muxer_element();
        s.push_str(&format!(
            "{} name=mux ! filesink name=file_sink location=\"{}\"",
            muxer, escaped
        ));
        s
    }

    fn build_streaming_pipeline_str(&self) -> String {
        let settings = self
            .stream_settings
            .as_ref()
            .expect("Stream settings must be set");
        tracing::info!(
            delay_ms = settings.delay.latency_ms(),
            tracks = settings.multitrack_video.effective_tracks(),
            "Initializing streaming pipeline"
        );
        let mut s = self.video_branch_str();
        s.push_str(&self.audio_branch_str(true));
        let delay = if settings.delay.enabled {
            format!(
                " ! identity name=stream_delay single-segment=true sleep-time={} ",
                settings.delay.duration.as_nanos() as u64
            )
        } else {
            String::new()
        };
        let targets = self
            .stream_targets
            .as_ref()
            .map(|targets| {
                targets
                    .targets
                    .iter()
                    .cloned()
                    .map(|mut target| {
                        target.refresh_plugin_status();
                        target
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| vec![StreamTarget::new("primary", settings.clone())]);
        let mut fanout = String::from("flvmux name=mux streamable=true");
        for (index, target) in targets.iter().enumerate() {
            let sink_fragment = target
                .contribution
                .as_ref()
                .and_then(|contribution| contribution.validated_pipeline_sink_fragment().ok())
                .unwrap_or_else(|| {
                    format!("rtmp2sink location=\\\"{}\\\"", target.settings.location())
                });
            let branch =
                format!(" ! queue name=stream_queue_{index} ! tee name=stream_branch_{index}");
            let sink = format!("{} name=stream_sink_{index}", sink_fragment);
            fanout.push_str(&format!(
                "{branch} stream_branch_{index}. ! queue name=stream_output_queue_{index}{delay} ! {sink}"
            ));
        }
        s.push_str(&fanout);
        s
    }

    /// Build a pipeline that records locally **and** streams at the same time.
    ///
    /// Video and audio are encoded once and split via `tee` into two branches,
    /// one feeding the local file muxer and one feeding the FLV muxer
    /// (RTMP/RTMPS sink). Audio is always mixed into a single track because FLV
    /// only supports one audio track. Streaming requires H.264; for non-H.264
    /// codecs only the local recording branch is built.
    fn build_dual_output_pipeline_str(&self) -> String {
        let stream_location = self
            .stream_settings
            .as_ref()
            .expect("Stream settings must be set")
            .location();
        let path = self.output_path.as_ref().expect("Output path must be set");
        let location = path.to_str().expect("Invalid file path.");
        tracing::info!(stream_location = %stream_location, "Initializing dual-output pipeline");

        let muxer = self.video_codec.muxer_element();
        let transform = self.preset.transform_fragment();
        let caps = self
            .video_encoder
            .input_caps_fragment_for_codec(self.video_codec);
        let encoder = self
            .video_encoder
            .branch_fragment_for_codec(self.video_codec, self.encoder_bitrate_kbps);
        let overlay = if self.show_overlay {
            " ! textoverlay name=overlay text=\"\" valignment=top halignment=right \
             font-desc=\"Sans, 24\" background-color=0x000000AA "
        } else {
            ""
        };
        let video_part = if transform.is_empty() {
            format!(
                "appsrc name=rivulet_src format=time is-live=true do-timestamp=true \
                 ! videoconvert ! {}{} ! {} ",
                caps, overlay, encoder
            )
        } else {
            let transform_caps = format!("{}!{}", transform.trim_end(), caps.trim_start());
            format!(
                "appsrc name=rivulet_src format=time is-live=true do-timestamp=true \
                 ! videoconvert ! {}{} ! {} ",
                transform_caps, overlay, encoder
            )
        };
        let mut s = String::new();
        s.push_str(&format!(
            "{} ! tee name=video_tee ! queue ! mux_rec. \
             video_tee. ! queue ! mux_stream. ",
            video_part
        ));
        // Replay capture branch (third output of the video tee).
        if self.replay.is_some() {
            s.push_str("video_tee. ! queue ! h264parse ! appsink name=replay_video_sink ");
        }
        if self.audio_enabled {
            s.push_str(
                "appsrc name=audio_src format=time is-live=true do-timestamp=true \
                 ! audioconvert ! audioresample ! avenc_aac ! tee name=audio_tee \
                 ! queue ! mux_rec. \
                 audio_tee. ! queue ! mux_stream. ",
            );
            if self.replay.is_some() {
                s.push_str("audio_tee. ! queue ! appsink name=replay_audio_sink_0 ");
            }
        }
        s.push_str(&format!(
            "{} name=mux_rec ! filesink name=file_sink location=\"{}\" ",
            muxer,
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
            tracing::info!(location = %location, "Initializing recording pipeline");
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

        let target_fps = self.preset.effective_fps(30) as i32;
        let video_info = gst_video::VideoInfo::builder(gst_video::VideoFormat::Rgba, width, height)
            .fps((target_fps, 1))
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

        self.install_replay_capture(&pipeline);
        self.pipeline = Some(pipeline);
        self.appsrc = Some(appsrc);
        self.install_performance_probes();
        if self.is_dual_output() {
            tracing::info!("Dual-output pipeline running.");
        } else if self.is_streaming() {
            tracing::info!("Streaming pipeline running.");
        } else {
            tracing::info!("Recording pipeline running.");
        }
    }

    /// Attach appsink callbacks that capture the encoded video/audio packets
    /// into the replay ring buffer.
    ///
    /// The capture branches only exist when the replay buffer is enabled (see
    /// [`RivuletEngine::replay_video_branch`] and [`RivuletEngine::audio_branch_str`]);
    /// the callbacks run on the GStreamer streaming thread and write into the
    /// shared `Arc<Mutex<ReplayBuffer>>`. They are installed before the first
    /// frame is pushed, so no encoded packet can slip past the ring.
    fn install_replay_capture(&mut self, pipeline: &gst::Pipeline) {
        let Some(replay) = self.replay.clone() else {
            return;
        };

        // Video capture. The branch includes `h264parse`, so the recorded
        // caps carry `codec_data` and the ring can start mid-stream.
        if let Some(sink) = pipeline.by_name("replay_video_sink") {
            if let Ok(sink) = sink.downcast::<gst_app::AppSink>() {
                let replay = replay.clone();
                let callbacks = gst_app::AppSinkCallbacks::builder()
                    .new_sample(move |sink| {
                        if let Ok(sample) = sink.pull_sample() {
                            if let Some(buf) = sample.buffer() {
                                let pts = buf.pts().map(|t| t.nseconds()).unwrap_or(0);
                                let dts =
                                    buf.dts().or(buf.pts()).map(|t| t.nseconds()).unwrap_or(pts);
                                let keyframe = !buf.flags().contains(gst::BufferFlags::DELTA_UNIT);
                                let data = buf
                                    .map_readable()
                                    .map(|m| m.as_slice().to_vec())
                                    .unwrap_or_default();
                                let mut rb = replay.lock().unwrap_or_else(|e| e.into_inner());
                                if rb.video_caps().is_none() {
                                    if let Some(caps) = sample.caps() {
                                        rb.set_video_caps(caps.to_string());
                                    }
                                }
                                rb.push_video(pts, dts, data, keyframe);
                            }
                        }
                        Ok(gst::FlowSuccess::Ok)
                    })
                    .build();
                sink.set_callbacks(callbacks);
            }
        }

        // Audio capture: one appsink per audio branch (mixed track, or
        // system + microphone when separate tracks are enabled).
        for i in 0..2usize {
            let name = format!("replay_audio_sink_{i}");
            let Some(sink) = pipeline.by_name(&name) else {
                continue;
            };
            let Ok(sink) = sink.downcast::<gst_app::AppSink>() else {
                continue;
            };
            let replay = replay.clone();
            let callbacks = gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    if let Ok(sample) = sink.pull_sample() {
                        if let Some(buf) = sample.buffer() {
                            let pts = buf.pts().map(|t| t.nseconds()).unwrap_or(0);
                            let dts = buf.dts().or(buf.pts()).map(|t| t.nseconds()).unwrap_or(pts);
                            let data = buf
                                .map_readable()
                                .map(|m| m.as_slice().to_vec())
                                .unwrap_or_default();
                            let mut rb = replay.lock().unwrap_or_else(|e| e.into_inner());
                            if rb.audio_caps(i).is_none() {
                                if let Some(caps) = sample.caps() {
                                    rb.set_audio_caps(i, caps.to_string());
                                }
                            }
                            rb.push_audio(i, pts, dts, data);
                        }
                    }
                    Ok(gst::FlowSuccess::Ok)
                })
                .build();
            sink.set_callbacks(callbacks);
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
        tracing::warn!(encoder = ?self.video_encoder, codec = ?self.video_codec, reason, "Hardware encoder unavailable; falling back to software");
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
        tracing::info!("Streaming prepared.");
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
        tracing::info!(path = ?path, "Recording prepared");
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
        tracing::info!("Stopping recording");

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
        self.reconnect_supervisor = None;
        self.reconnect_worker = None;
        self.reconnect_started = None;
        self.delay_supervisors.clear();
        self.is_recording = false;
        // Drop the buffered replay data: a clip is only meaningful while the
        // session that produced it is running. The replay setting itself
        // (duration/enabled) is kept for the next recording.
        if let Some(replay) = &self.replay {
            replay.lock().unwrap_or_else(|e| e.into_inner()).clear();
        }
        tracing::info!("Recording stopped and file saved");
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

    /// Enable the replay buffer with the given retention window. The encoded
    /// video/audio of the next recording (or stream) is kept in RAM for the
    /// last `duration`, so [`RivuletEngine::save_replay`] can write a clip
    /// without recording the whole session.
    ///
    /// Safe to call while a recording is running: the window shrinks/grows
    /// immediately and old data is evicted.
    pub fn set_replay_duration(&mut self, duration: Duration) {
        if let Some(replay) = &self.replay {
            replay
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .set_duration(duration);
        } else {
            self.replay = Some(Arc::new(Mutex::new(ReplayBuffer::new(duration))));
        }
    }

    /// Disable the replay buffer. The in-memory ring (and its pipeline
    /// branches) are dropped; the setting must be re-applied before the next
    /// recording to capture clips again.
    pub fn disable_replay(&mut self) {
        self.replay = None;
    }

    /// Whether the replay buffer is enabled.
    pub fn replay_enabled(&self) -> bool {
        self.replay.is_some()
    }

    /// The configured replay retention window, if the buffer is enabled.
    pub fn replay_duration(&self) -> Option<Duration> {
        self.replay
            .as_ref()
            .map(|r| r.lock().unwrap_or_else(|e| e.into_inner()).duration())
    }

    /// Approximate seconds of video currently retained in the replay buffer
    /// (0 when empty or disabled). Useful for a "~30s buffered" hint in the
    /// GUI.
    pub fn replay_retained_secs(&self) -> Option<u64> {
        self.replay
            .as_ref()
            .map(|r| r.lock().unwrap_or_else(|e| e.into_inner()).retained_ns() / 1_000_000_000)
    }

    /// Save the currently buffered replay clip to `path` as an MP4 file.
    ///
    /// The buffered packets are remuxed (never re-encoded), so saving is
    /// fast. Fails cleanly when the replay buffer is disabled or empty, or
    /// when the remux reports an error.
    pub fn save_replay(&mut self, path: PathBuf) -> anyhow::Result<PathBuf> {
        let Some(replay) = &self.replay else {
            anyhow::bail!("replay buffer is disabled");
        };
        let snapshot = replay.lock().unwrap_or_else(|e| e.into_inner()).snapshot();
        replay::save_replay(&snapshot, &path)?;
        Ok(path)
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

    /// End-to-end test with the replay buffer enabled: the same synthetic
    /// recording must both write a non-empty MP4 AND fill the replay ring
    /// with encoded packets (including caps), so a clip can be saved.
    #[test]
    fn records_synthetic_frames_and_fills_replay_buffer() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_replay_duration(std::time::Duration::from_secs(30));

        let path = std::env::temp_dir().join(format!(
            "rivulet_engine_replay_test_{}.mp4",
            std::process::id()
        ));
        engine.start_local_recording(path.clone());

        let (width, height) = (320u32, 240u32);
        let video = vec![0u8; (width * height * 4) as usize];
        for _ in 0..24 {
            engine.process_raw_frame(&video, width, height);
        }
        let audio = AudioFrame::new(vec![0.0f32; 4800], AUDIO_SAMPLE_RATE, AUDIO_CHANNELS);
        for _ in 0..24 {
            let _ = engine.push_audio_frame(&audio);
        }

        std::thread::sleep(std::time::Duration::from_millis(200));

        // The ring must have captured encoded packets with caps while the
        // recording was live.
        if let Some(replay) = &engine.replay {
            let rb = replay.lock().unwrap();
            assert!(!rb.is_empty(), "replay ring should hold encoded packets");
            assert!(
                rb.video().len() > 1,
                "replay ring should hold multiple encoded video packets"
            );
            assert!(
                rb.video_caps().is_some(),
                "video caps must be captured from the pipeline"
            );
            assert!(
                rb.video().iter().any(|s| s.keyframe),
                "ring must contain at least one keyframe"
            );
            assert!(
                rb.retained_ns() > 0,
                "ring must retain a positive span of video"
            );
        } else {
            panic!("replay buffer must be enabled");
        }

        engine.stop_recording();
        assert!(path.exists(), "output file should exist");
        assert!(engine.take_error().is_none());
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

    #[test]
    fn overlay_enabled_is_false_by_default() {
        let engine = RivuletEngine::default();
        assert!(!engine.overlay_enabled());
    }

    #[test]
    fn set_overlay_enabled_toggles_flag() {
        let mut engine = RivuletEngine::default();
        engine.set_overlay_enabled(true);
        assert!(engine.overlay_enabled());
        engine.set_overlay_enabled(false);
        assert!(!engine.overlay_enabled());
    }

    #[test]
    fn overlay_in_pipeline_string_when_enabled() {
        let mut engine = RivuletEngine::default();
        engine.set_overlay_enabled(true);
        let path = std::env::temp_dir().join("rivulet_test_overlay.mp4");
        engine.start_local_recording(path.clone());
        let pipeline_str = engine.build_pipeline_str().unwrap();
        assert!(
            pipeline_str.contains("textoverlay name=overlay"),
            "pipeline should contain textoverlay when overlay is enabled: {}",
            pipeline_str
        );
        engine.stop_recording();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn no_overlay_in_pipeline_string_when_disabled() {
        let mut engine = RivuletEngine::default();
        let path = std::env::temp_dir().join("rivulet_test_no_overlay.mp4");
        engine.start_local_recording(path.clone());
        let pipeline_str = engine.build_pipeline_str().unwrap();
        assert!(
            !pipeline_str.contains("textoverlay"),
            "pipeline should not contain textoverlay when overlay is disabled: {}",
            pipeline_str
        );
        engine.stop_recording();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn overlay_in_dual_output_pipeline_when_enabled() {
        let mut engine = RivuletEngine::default();
        engine.set_overlay_enabled(true);
        engine.set_stream_settings(Some(StreamSettings {
            platform: StreamPlatform::Twitch,
            ingest_url: "rtmps://live.twitch.tv/app/test_key".into(),
            stream_key: String::new(),
            preset: StreamPreset::Standard,
            adaptive_bitrate: AdaptiveBitrate::default(),
            delay: StreamDelay::default(),
            multitrack_video: MultitrackVideo::default(),
        }));
        let path = std::env::temp_dir().join("rivulet_test_overlay_dual.mp4");
        engine.start_local_recording(path.clone());
        let pipeline_str = engine.build_pipeline_str().unwrap();
        assert!(
            pipeline_str.contains("textoverlay name=overlay"),
            "dual output pipeline should contain textoverlay: {}",
            pipeline_str
        );
        engine.stop_recording();
        let _ = std::fs::remove_file(&path);
    }

    // ── Replay buffer ────────────────────────────────────────────────

    #[test]
    fn replay_buffer_is_disabled_by_default() {
        let engine = RivuletEngine::default();
        assert!(!engine.replay_enabled());
        assert_eq!(engine.replay_duration(), None);
        assert_eq!(engine.replay_retained_secs(), None);
    }

    #[test]
    fn replay_duration_can_be_enabled_updated_and_disabled() {
        let mut engine = RivuletEngine::default();
        engine.set_replay_duration(std::time::Duration::from_secs(30));
        assert!(engine.replay_enabled());
        assert_eq!(
            engine.replay_duration(),
            Some(std::time::Duration::from_secs(30))
        );

        engine.set_replay_duration(std::time::Duration::from_secs(60));
        assert_eq!(
            engine.replay_duration(),
            Some(std::time::Duration::from_secs(60))
        );

        engine.disable_replay();
        assert!(!engine.replay_enabled());
        assert_eq!(engine.replay_duration(), None);
    }

    #[test]
    fn saving_replay_while_disabled_fails_cleanly() {
        let mut engine = RivuletEngine::default();
        let path = std::env::temp_dir().join("rivulet_replay_disabled.mp4");
        let err = engine.save_replay(path.clone()).unwrap_err();
        assert!(err.to_string().contains("disabled"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn saving_replay_with_empty_buffer_fails_cleanly() {
        let mut engine = RivuletEngine::default();
        engine.set_replay_duration(std::time::Duration::from_secs(30));
        let path = std::env::temp_dir().join("rivulet_replay_empty.mp4");
        let err = engine.save_replay(path.clone()).unwrap_err();
        assert!(err.to_string().contains("empty"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn recording_pipeline_contains_replay_branches_when_enabled() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_replay_duration(std::time::Duration::from_secs(30));
        let path = std::env::temp_dir().join("rivulet_replay_pipeline.mp4");
        engine.start_local_recording(path.clone());
        let pipeline_str = engine.build_pipeline_str().unwrap();
        assert!(
            pipeline_str.contains("tee name=replay_vtee"),
            "video branch must tee into the replay capture: {}",
            pipeline_str
        );
        assert!(
            pipeline_str.contains("appsink name=replay_video_sink"),
            "video branch must contain the replay appsink: {}",
            pipeline_str
        );
        assert!(
            pipeline_str.contains("h264parse"),
            "replay branch must parse H.264 for codec_data caps: {}",
            pipeline_str
        );
        assert!(
            pipeline_str.contains("appsink name=replay_audio_sink_0"),
            "audio branch must contain the replay appsink: {}",
            pipeline_str
        );
        engine.stop_recording();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn recording_pipeline_has_no_replay_branches_when_disabled() {
        let mut engine = RivuletEngine::default();
        let path = std::env::temp_dir().join("rivulet_no_replay_pipeline.mp4");
        engine.start_local_recording(path.clone());
        let pipeline_str = engine.build_pipeline_str().unwrap();
        assert!(
            !pipeline_str.contains("replay_vtee"),
            "pipeline must not contain replay tee when disabled: {}",
            pipeline_str
        );
        assert!(
            !pipeline_str.contains("replay_video_sink"),
            "pipeline must not contain replay appsink when disabled: {}",
            pipeline_str
        );
        engine.stop_recording();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn streaming_pipeline_uses_contribution_sink_fragment() {
        let mut engine = RivuletEngine::default();
        let mut targets = MultistreamSettings::default();
        targets
            .add_target(
                StreamTarget::new(
                    "srt-target",
                    StreamSettings::custom("rtmp://localhost/live", "unused"),
                )
                .with_contribution(ContributionSettings::new(
                    ContributionProtocol::Srt,
                    "srt://127.0.0.1:9000",
                )),
            )
            .unwrap();
        engine.set_stream_settings(Some(StreamSettings::custom("rtmp://localhost/live", "key")));
        engine.set_multistream_settings(Some(targets));
        engine.start_streaming();
        let pipeline = engine.build_pipeline_str().unwrap();
        assert!(pipeline.contains("srtsink"));
        assert!(pipeline.contains("stream_sink_0"));
        engine.stop_recording();
    }

    #[test]
    fn streaming_pipeline_contains_replay_branches_when_enabled() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_replay_duration(std::time::Duration::from_secs(30));
        engine.set_stream_settings(Some(StreamSettings {
            platform: StreamPlatform::Twitch,
            ingest_url: "rtmps://live.twitch.tv/app/test_key".into(),
            stream_key: String::new(),
            preset: StreamPreset::Standard,
            adaptive_bitrate: AdaptiveBitrate::default(),
            delay: StreamDelay::default(),
            multitrack_video: MultitrackVideo::default(),
        }));
        engine.start_streaming();
        let pipeline_str = engine.build_pipeline_str().unwrap();
        assert!(
            pipeline_str.contains("appsink name=replay_video_sink"),
            "streaming pipeline must contain replay capture: {}",
            pipeline_str
        );
        assert!(
            pipeline_str.contains("appsink name=replay_audio_sink_0"),
            "streaming pipeline must contain replay audio capture: {}",
            pipeline_str
        );
        engine.stop_recording();
    }

    #[test]
    fn dual_output_pipeline_contains_replay_branches_when_enabled() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_replay_duration(std::time::Duration::from_secs(30));
        engine.set_stream_settings(Some(StreamSettings {
            platform: StreamPlatform::Twitch,
            ingest_url: "rtmps://live.twitch.tv/app/test_key".into(),
            stream_key: String::new(),
            preset: StreamPreset::Standard,
            adaptive_bitrate: AdaptiveBitrate::default(),
            delay: StreamDelay::default(),
            multitrack_video: MultitrackVideo::default(),
        }));
        let path = std::env::temp_dir().join("rivulet_replay_dual.mp4");
        engine.start_local_recording(path.clone());
        let pipeline_str = engine.build_pipeline_str().unwrap();
        assert!(
            pipeline_str.contains("appsink name=replay_video_sink"),
            "dual output pipeline must contain replay capture: {}",
            pipeline_str
        );
        assert!(
            pipeline_str.contains("appsink name=replay_audio_sink_0"),
            "dual output pipeline must contain replay audio capture: {}",
            pipeline_str
        );
        engine.stop_recording();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn separate_audio_tracks_get_individual_replay_sinks() {
        let mut engine = RivuletEngine::default();
        engine.set_audio_enabled(true);
        engine.set_separate_audio_tracks(true);
        engine.set_replay_duration(std::time::Duration::from_secs(30));
        let path = std::env::temp_dir().join("rivulet_replay_tracks.mp4");
        engine.start_local_recording(path.clone());
        let pipeline_str = engine.build_pipeline_str().unwrap();
        assert!(
            pipeline_str.contains("appsink name=replay_audio_sink_0"),
            "first audio track must feed replay_audio_sink_0: {}",
            pipeline_str
        );
        assert!(
            pipeline_str.contains("appsink name=replay_audio_sink_1"),
            "second audio track must feed replay_audio_sink_1: {}",
            pipeline_str
        );
        engine.stop_recording();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn stop_recording_clears_the_replay_ring() {
        let mut engine = RivuletEngine::default();
        engine.set_replay_duration(std::time::Duration::from_secs(30));
        let path = std::env::temp_dir().join("rivulet_replay_clear.mp4");
        engine.start_local_recording(path.clone());
        // Simulate the streaming thread filling the ring.
        if let Some(replay) = &engine.replay {
            let mut rb = replay.lock().unwrap();
            rb.push_video(0, 0, vec![0], true);
            rb.push_audio(0, 0, 0, vec![0]);
            assert!(!rb.is_empty());
        }
        engine.stop_recording();
        // The setting persists, but the buffered data is gone.
        assert!(engine.replay_enabled());
        assert_eq!(engine.replay_retained_secs(), Some(0));
        if let Some(replay) = &engine.replay {
            assert!(replay.lock().unwrap().is_empty());
        }
        let _ = std::fs::remove_file(&path);
    }
}
