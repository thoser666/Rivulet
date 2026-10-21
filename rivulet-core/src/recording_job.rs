//! The GUI-free in-process recording entry point ([#294](https://github.com/thoser666/Rivulet/issues/294)).
//!
//! A foreign crate embeds recording by building a [`RecordingJobConfig`],
//! starting a [`RecordingJob`], pushing its own frames and samples, and
//! stopping (or dropping) the handle — no `egui`, no `RivuletApp`, no GUI
//! state anywhere in the call chain (`rivulet-core` does not depend on the
//! GUI crate at all).
//!
//! # Gate: invalid configuration fails early, typed and actionable
//!
//! [`RecordingJob::start`] validates **before** it touches GStreamer:
//!
//! 1. the video configuration ([`RecordingConfig::validate`], the typed gate
//!    from recording_config.rs — every field named, every message stating the
//!    fix),
//! 2. the session-level parameters added here (audio rate/channels, at least
//!    one output target, stream settings, H.264 for the FLV stream leg),
//! 3. and finally the GStreamer elements the chosen configuration needs —
//!    a missing encoder fails with the element names instead of an opaque
//!    parse error.
//!
//! Only when all three layers pass is the pipeline built and set to
//! `PLAYING`. Every rejection is a typed [`JobError`] with a machine-readable
//! [`JobError::field`] and a message naming the fix.
//!
//! # Lifecycle
//!
//! | Operation | Effect |
//! | --- | --- |
//! | [`RecordingJob::start`] | validates, builds the pipeline, returns the handle + event stream |
//! | [`push_frame`] / [`push_audio`] | feeds RGBA video / interleaved F32 audio into the session |
//! | [`pause`] | closes the push gate; pushes are rejected with a typed error |
//! | [`resume`] | reopens the push gate |
//! | [`stop`] | consumes the handle: EOS → muxer finalizes → report |
//! | `Drop` | **same clean finalization** (see below), result discarded |
//!
//! **Drop semantics (the documented decision):** dropping the handle stops
//! the recording cleanly and finalizes the output file — exactly like
//! [`stop`]. A recording never keeps running in the background after its
//! handle is gone, and a finalized file (moov atom) is written within the
//! same 10-second EOS budget the engine's own `stop_recording` uses. A
//! session paused at teardown needs no extra steps: the pipeline was never
//! held, so the EOS reaches the muxer directly.
//!
//! **Pause semantics:** time passes while paused but no frames are accepted
//! — `push_frame`/`push_audio` return [`JobError::WrongState`] instead of
//! silently accepting (an accepting API would hide the pause from the
//! embedder and fill the very gap the timeline is supposed to record).
//! The media timeline records the paused interval as a timestamp gap;
//! players freeze or skip across it.
//!
//! # Status stream
//!
//! `start` hands back a [`tokio::sync::broadcast::Receiver<SessionEvent>`]: the
//! lifecycle events ([`SessionEvent::Started`], [`SessionEvent::Paused`],
//! [`SessionEvent::Resumed`], [`SessionEvent::Stopped`]), a
//! [`SessionEvent::Progress`] tick with frames, bytes and elapsed time for
//! driving an embedding UI, and [`SessionEvent::Failed`] when the pipeline
//! reports an asynchronous bus error. Additional listeners can be added with
//! [`RecordingJob::subscribe`]; they see only events sent after subscribing.
//!
//! # Outputs
//!
//! One configuration drives all three shapes: file only, stream only, or
//! **both at once** (video/audio encoded once, `tee`-ed into the container
//! muxer and the FLV/RTMP leg). See [`RecordingJobConfig::pipeline_description`]
//! for the exact pipeline — it embeds the stream key, so redact it with
//! [`crate::redact_pipeline_for_log`] before logging.
//!
//! # Example
//!
//! See `rivulet-core/examples/session_embed.rs` — a foreign-crate style
//! session that pushes frames, pauses, resumes and stops, run by CI on every
//! push.
//!
//! [`push_frame`]: RecordingJob::push_frame
//! [`push_audio`]: RecordingJob::push_audio
//! [`pause`]: RecordingJob::pause
//! [`resume`]: RecordingJob::resume
//! [`stop`]: RecordingJob::stop
//! [`subscribe`]: RecordingJob::subscribe

use crate::encoder::{RateControl, VideoCodec};
use crate::recording_config::{RecordingConfig, RecordingConfigError};
use crate::stream::StreamSettings;
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

/// Smallest audio sample rate we accept; below this the value is almost
/// always a unit or typo error.
const MIN_AUDIO_RATE: u32 = 8_000;
/// Largest audio sample rate we accept; above this it is almost always a
/// bits-per-second figure entered where Hertz was expected.
const MAX_AUDIO_RATE: u32 = 384_000;
/// Largest channel count we accept for the embedded session.
const MAX_AUDIO_CHANNELS: u16 = 8;
/// How often the progress tick emits [`SessionEvent::Progress`] and polls the
/// pipeline bus for asynchronous errors.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);
/// Budget for the muxer to finalize the output file after EOS — the same
/// budget `RivuletEngine::stop_recording` uses.
const EOS_BUDGET: Duration = Duration::from_secs(10);

/// Sample format of the audio the embedder pushes: interleaved 32-bit float
/// at the configured rate and channel count.
///
/// Validation of these two bounds happens before the pipeline exists, so a
/// bad rate fails as a typed [`JobError`] rather than a negotiation error
/// after frames already flowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioSettings {
    /// Sample rate in Hertz (`8000..=384_000`).
    pub sample_rate: u32,
    /// Channel count (`1..=8`), interleaved with [`Self::sample_rate`].
    pub channels: u16,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
        }
    }
}

impl AudioSettings {
    /// Validate the audio parameters, returning the first problem as a typed
    /// [`JobError`] that names the field and states the accepted range.
    fn validate(&self) -> Result<(), JobError> {
        if self.sample_rate == 0 {
            return Err(JobError::AudioRateZero);
        }
        if !(MIN_AUDIO_RATE..=MAX_AUDIO_RATE).contains(&self.sample_rate) {
            return Err(JobError::AudioRateOutOfRange {
                rate: self.sample_rate,
                min: MIN_AUDIO_RATE,
                max: MAX_AUDIO_RATE,
            });
        }
        if self.channels == 0 {
            return Err(JobError::AudioChannelsZero);
        }
        if self.channels > MAX_AUDIO_CHANNELS {
            return Err(JobError::AudioChannelsTooHigh {
                channels: self.channels,
                max: MAX_AUDIO_CHANNELS,
            });
        }
        Ok(())
    }

    /// The `parse_launch` caps fragment for the audio appsrc.
    fn caps_fragment(&self) -> String {
        format!(
            "audio/x-raw,format=F32LE,layout=interleaved,channels={},rate={}",
            self.channels, self.sample_rate
        )
    }
}

/// Everything a session needs: the validated video configuration (whose
/// `path` is the file target), an optional audio branch, and an optional
/// stream target. At least one output must be configured.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordingJobConfig {
    /// Video parameters; validated by [`RecordingConfig::validate`].
    /// `path` addresses the file output (its extension is checked against
    /// the container there).
    pub recording: RecordingConfig,
    /// Audio branch; `None` records video only.
    pub audio: Option<AudioSettings>,
    /// RTMP/RTMPS stream target; combined with `recording.path` this is the
    /// dual-output (file **and** stream) shape.
    pub stream: Option<StreamSettings>,
}

impl RecordingJobConfig {
    /// Video-only session recording to `path`.
    pub fn file(recording: RecordingConfig) -> Self {
        Self {
            recording,
            audio: None,
            stream: None,
        }
    }

    /// Enable the audio branch with the default 48 kHz stereo format.
    pub fn with_default_audio(mut self) -> Self {
        self.audio = Some(AudioSettings::default());
        self
    }

    /// Enable the audio branch with explicit parameters.
    pub fn with_audio(mut self, audio: AudioSettings) -> Self {
        self.audio = Some(audio);
        self
    }

    /// Add a stream target; with a file path this yields the dual-output
    /// pipeline.
    pub fn with_stream(mut self, stream: StreamSettings) -> Self {
        self.stream = Some(stream);
        self
    }

    /// Validate every layer without touching GStreamer: the video
    /// configuration, the audio parameters, the output-target combination,
    /// the stream settings, and the H.264 requirement of the FLV stream leg.
    ///
    /// [`RecordingJob::start`] calls this first, so everything below is
    /// reported before any pipeline element is created.
    pub fn validate(&self) -> Result<(), JobError> {
        // Layer 1: the typed video-config gate (field order is deterministic
        // inside RecordingConfig::validate itself).
        self.recording.validate().map_err(JobError::Config)?;

        // Layer 2: session-level parameters added by this API.
        if let Some(audio) = &self.audio {
            audio.validate()?;
        }
        if self.recording.path.is_none() && self.stream.is_none() {
            return Err(JobError::NoOutput);
        }
        if let Some(stream) = &self.stream {
            stream.validate().map_err(JobError::StreamSettings)?;
            // FLV carries H.264 only; the engine silently drops the stream
            // branch for other codecs, this entry point refuses up front.
            if self.recording.codec != VideoCodec::H264 {
                return Err(JobError::StreamRequiresH264 {
                    codec: self.recording.codec,
                });
            }
        }
        Ok(())
    }

    /// The full `gst::parse_launch` description for this configuration —
    /// file only, stream only, or both — built from the same public encoder
    /// and container fragments the engine uses.
    ///
    /// Exposed so embedders (and the regression suite) can inspect what will
    /// run. **The stream leg embeds the stream key**: redact with
    /// [`crate::redact_pipeline_for_log`] before logging.
    pub fn pipeline_description(&self) -> Result<String, JobError> {
        self.validate()?;
        let rec = &self.recording;
        let caps = rec.encoder.input_caps_fragment_for_codec(rec.codec);
        let encoder = rec.encoder.branch_fragment_for_codec_rc(
            rec.codec,
            rec.bitrate_kbps,
            RateControl::default(),
        );
        let file = self.recording.path.as_deref();
        // The FLV leg always needs the AVC parser; for H.265 the container
        // leg needs h265parse (byte-stream H.265 does not link through the
        // muxer, see `video_branch_str`). H.264 into a container muxer needs
        // no parser — the engine records that way in production.
        let container_parse = if rec.codec == VideoCodec::H265 {
            " ! h265parse"
        } else {
            ""
        };

        let mut video = format!(
            "appsrc name=video_src format=time is-live=true do-timestamp=true \
             caps=\"video/x-raw,format=RGBA,width={},height={},framerate={}/1\" \
             ! videoconvert ! {caps} ! {encoder}{container_parse}",
            rec.width, rec.height, rec.fps,
        );

        let mut s = String::new();
        match (file, self.stream.is_some()) {
            (Some(path), true) => {
                // Dual output: encode once, tee into both muxers (the same
                // shape the engine's build_dual_output_pipeline_str builds).
                video.push_str(
                    " ! h264parse ! tee name=video_tee ! queue ! mux_rec. \
                                video_tee. ! queue ! mux_stream.video",
                );
                s.push_str(&video);
                s.push(' ');
                s.push_str(&self.audio_legs(true)?);
                let muxer = rec.container.muxer_element();
                s.push_str(&format!(
                    "{muxer} name=mux_rec ! filesink name=file_sink location=\"{}\"",
                    escape_location(path)
                ));
                s.push(' ');
                s.push_str(&format!(
                    "flvmux name=mux_stream streamable=true ! rtmp2sink location=\"{}\"",
                    escape_location(&self.stream.as_ref().expect("checked").location())
                ));
            }
            (Some(path), false) => {
                video.push_str(" ! queue ! mux_rec.");
                s.push_str(&video);
                s.push(' ');
                s.push_str(&self.audio_legs(false)?);
                let muxer = rec.container.muxer_element();
                s.push_str(&format!(
                    "{muxer} name=mux_rec ! filesink name=file_sink location=\"{}\"",
                    escape_location(path)
                ));
            }
            (None, true) => {
                video.push_str(" ! h264parse ! queue ! mux_stream.video");
                s.push_str(&video);
                s.push(' ');
                s.push_str(&self.audio_legs(false)?.replace("mux_rec.", "mux_stream."));
                s.push_str(&format!(
                    "flvmux name=mux_stream streamable=true ! rtmp2sink location=\"{}\"",
                    escape_location(&self.stream.as_ref().expect("checked").location())
                ));
            }
            (None, false) => unreachable!("validate() rejects a session with no output"),
        }
        Ok(s)
    }

    /// The audio branch feeding the recording muxer (`mux_rec`), the stream
    /// muxer, or both when dual output is active. Empty for video-only
    /// sessions.
    fn audio_legs(&self, dual: bool) -> Result<String, JobError> {
        let Some(audio) = &self.audio else {
            return Ok(String::new());
        };
        audio.validate()?;
        let tail = if dual {
            " ! tee name=audio_tee ! queue ! mux_rec. \
             audio_tee. ! queue ! mux_stream.audio"
        } else {
            " ! queue ! mux_rec."
        };
        Ok(format!(
            "appsrc name=audio_src format=time is-live=true do-timestamp=true \
             caps=\"{}\" ! audioconvert ! audioresample ! avenc_aac{tail}",
            audio.caps_fragment()
        ))
    }

    /// The GStreamer elements this configuration needs, checked before the
    /// pipeline is built so a missing plugin surfaces as element names, not
    /// as a parse error.
    fn required_elements(&self) -> Vec<&'static str> {
        let rec = &self.recording;
        let mut required = vec!["appsrc", "videoconvert", "queue"];
        required.push(rec.encoder.element_name_for_codec(rec.codec));
        if rec.codec == VideoCodec::H265 {
            required.push("h265parse");
        }
        if self.stream.is_some() {
            required.push("tee");
            required.push("h264parse");
            required.push("flvmux");
            required.push("rtmp2sink");
        }
        if self.recording.path.is_some() {
            required.push(rec.container.muxer_element());
            required.push("filesink");
        }
        if self.audio.is_some() {
            required.extend(["audioconvert", "audioresample", "avenc_aac"]);
        }
        required.sort_unstable();
        required.dedup();
        required
    }
}

/// Escape a location for `parse_launch`: backslashes (Windows paths) must
/// survive the string parser. Mirrors the engine's escaping rule so both
/// entry points accept the same paths.
fn escape_location(location: &str) -> String {
    location.replace('\\', "\\\\")
}

/// Lifecycle state of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    /// The pipeline is running; frames and samples are accepted.
    Recording,
    /// Pushes are rejected with a typed error until resume; the pipeline
    /// keeps running, so a pause lands as a timestamp gap in the output.
    Paused,
    /// The session has been stopped or dropped; the output is finalized.
    Stopped,
}

impl std::fmt::Display for JobState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Recording => write!(f, "recording"),
            Self::Paused => write!(f, "paused"),
            Self::Stopped => write!(f, "stopped"),
        }
    }
}

/// Outcome of a finished session, returned by [`RecordingJob::stop`] and
/// carried by [`SessionEvent::Stopped`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionReport {
    /// Frames the embedder pushed and the pipeline encoded.
    pub frames_written: u64,
    /// Audio samples the embedder pushed (per channel count summed: total
    /// interleaved sample values pushed ÷ channels = frames of audio).
    pub audio_samples: u64,
    /// Bytes written to the output file; `0` for stream-only sessions
    /// (there is no local file to measure).
    pub bytes_written: u64,
    /// Wall-clock time from `start` to `stop`/`Drop`, pauses included.
    pub elapsed: Duration,
    /// The file target, when configured.
    pub path: Option<PathBuf>,
}

/// Events emitted on the session's status stream.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEvent {
    /// The pipeline is running; carries the configured file target.
    Started {
        /// The output file, when configured.
        path: Option<PathBuf>,
    },
    /// Periodic progress tick for embedding UIs.
    Progress {
        /// Frames pushed so far.
        frames_written: u64,
        /// Audio samples pushed so far.
        audio_samples: u64,
        /// Bytes written to the output file so far (0 without a file).
        bytes_written: u64,
        /// Wall time since the session started, pauses included.
        elapsed: Duration,
    },
    /// The session was paused.
    Paused,
    /// The session was resumed.
    Resumed,
    /// The session ended cleanly; carries the final report.
    Stopped(SessionReport),
    /// The pipeline reported an asynchronous error on its bus.
    Failed(String),
}

/// Every way a session can refuse to start, refuse an operation, or fail —
/// typed, with a machine-readable field name and an actionable message.
#[derive(Debug, Clone, PartialEq)]
pub enum JobError {
    /// The video configuration failed its typed early gate.
    Config(RecordingConfigError),
    /// `audio.sample_rate` was 0.
    AudioRateZero,
    /// `audio.sample_rate` was outside the accepted range.
    AudioRateOutOfRange {
        /// The rejected rate, in Hertz.
        rate: u32,
        /// The smallest accepted rate.
        min: u32,
        /// The largest accepted rate.
        max: u32,
    },
    /// `audio.channels` was 0.
    AudioChannelsZero,
    /// `audio.channels` exceeded the maximum.
    AudioChannelsTooHigh {
        /// The rejected channel count.
        channels: u16,
        /// The largest accepted channel count.
        max: u16,
    },
    /// The audio buffer length is not a multiple of the channel count.
    AudioBufferMisaligned {
        /// Number of f32 values pushed.
        samples: usize,
        /// The configured channel count.
        channels: u16,
    },
    /// Neither a file path nor a stream target is configured.
    NoOutput,
    /// The stream settings failed validation (empty key, non-RTMP scheme,
    /// platform preset without TLS).
    StreamSettings(&'static str),
    /// The stream leg is FLV and carries H.264 only.
    StreamRequiresH264 {
        /// The configured codec that cannot be streamed.
        codec: VideoCodec,
    },
    /// Required GStreamer elements are missing from this installation.
    MissingElements {
        /// The element factory names that could not be found.
        elements: Vec<&'static str>,
    },
    /// GStreamer itself could not be initialized.
    Gstreamer(String),
    /// The pipeline description could not be parsed or linked.
    Pipeline(String),
    /// A frame's byte length does not match the configured geometry.
    FrameSizeMismatch {
        /// Expected byte count: `width × height × 4` (RGBA).
        expected: usize,
        /// The byte count actually pushed.
        got: usize,
    },
    /// The operation is not valid in the session's current state.
    WrongState {
        /// The operation that was attempted, e.g. `"push_frame"`.
        operation: &'static str,
        /// The state the session is in.
        state: JobState,
    },
    /// Pushing into the pipeline failed.
    PushFailed(String),
}

impl JobError {
    /// The name of the configuration field (or component) at fault. Stable
    /// and machine-readable — an embedding UI can bind it to a control.
    pub fn field(&self) -> &'static str {
        match self {
            Self::Config(inner) => inner.field(),
            Self::AudioRateZero | Self::AudioRateOutOfRange { .. } => "audio.sample_rate",
            Self::AudioChannelsZero | Self::AudioChannelsTooHigh { .. } => "audio.channels",
            Self::AudioBufferMisaligned { .. } => "audio",
            Self::NoOutput => "output",
            Self::StreamSettings(_) => "stream",
            Self::StreamRequiresH264 { .. } => "recording.codec",
            Self::MissingElements { .. } => "pipeline",
            Self::Gstreamer(_) => "gstreamer",
            Self::Pipeline(_) => "pipeline",
            Self::FrameSizeMismatch { .. } => "frame",
            Self::WrongState { .. } => "state",
            Self::PushFailed(_) => "frame",
        }
    }
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(inner) => write!(f, "{inner}"),
            Self::AudioRateZero => write!(f, "audio.sample_rate must be greater than 0"),
            Self::AudioRateOutOfRange { rate, min, max } => write!(
                f,
                "audio.sample_rate must be between {min} and {max} Hz, got {rate}"
            ),
            Self::AudioChannelsZero => write!(f, "audio.channels must be greater than 0"),
            Self::AudioChannelsTooHigh { channels, max } => {
                write!(f, "audio.channels must be at most {max}, got {channels}")
            }
            Self::AudioBufferMisaligned { samples, channels } => write!(
                f,
                "audio buffer holds {samples} samples which is not a multiple \
                 of {channels} channels; push interleaved samples per frame"
            ),
            Self::NoOutput => write!(
                f,
                "no output configured: set RecordingConfig::path for a file \
                 and/or add a stream target with with_stream()"
            ),
            Self::StreamSettings(reason) => write!(f, "stream settings are invalid: {reason}"),
            Self::StreamRequiresH264 { codec } => write!(
                f,
                "streaming requires H.264 (the FLV leg cannot carry {codec:?}); \
                 set RecordingConfig::codec to VideoCodec::H264 or record to \
                 file only"
            ),
            Self::MissingElements { elements } => write!(
                f,
                "missing GStreamer elements: {} — install the plugins that \
                 provide them (e.g. gst-plugins-ugly for x264enc, gst-libav \
                 for avenc_aac)",
                elements.join(", ")
            ),
            Self::Gstreamer(msg) => write!(f, "GStreamer failed to initialize: {msg}"),
            Self::Pipeline(msg) => write!(f, "could not build the recording pipeline: {msg}"),
            Self::FrameSizeMismatch { expected, got } => write!(
                f,
                "frame size mismatch: the configured geometry expects {expected} \
                 bytes per RGBA frame, got {got} — resize before pushing or \
                 change RecordingConfig::width/height"
            ),
            Self::WrongState { operation, state } => write!(
                f,
                "cannot {operation} while the session is {state}{}",
                match state {
                    JobState::Paused => "; call resume() first",
                    JobState::Stopped => "; the handle is finished",
                    JobState::Recording => "",
                }
            ),
            Self::PushFailed(msg) => write!(f, "could not push into the pipeline: {msg}"),
        }
    }
}

impl std::error::Error for JobError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(inner) => Some(inner),
            _ => None,
        }
    }
}

impl From<RecordingConfigError> for JobError {
    fn from(inner: RecordingConfigError) -> Self {
        Self::Config(inner)
    }
}

/// A running in-process recording session.
///
/// Not `Sync`: methods take `&mut self`, so share the handle through a
/// `Mutex` if multiple threads drive it. The event receivers returned by
/// Event receivers are `broadcast` handles: every subscriber sees every
/// event sent after it subscribed, and a receiver can be moved to a UI
/// thread independently of the handle.
pub struct RecordingJob {
    pipeline: Option<gst::Pipeline>,
    video_src: gst_app::AppSrc,
    audio_src: Option<gst_app::AppSrc>,
    state: JobState,
    config: RecordingJobConfig,
    started: Instant,
    frames: Arc<AtomicU64>,
    audio_samples: Arc<AtomicU64>,
    bytes: Arc<AtomicU64>,
    failed: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    events: broadcast::Sender<SessionEvent>,
    ticker: Option<std::thread::JoinHandle<()>>,
}

impl RecordingJob {
    /// Validate the configuration, build the pipeline, and start the
    /// session.
    ///
    /// Returns the handle plus the initial event receiver (already connected
    /// for [`SessionEvent::Started`]; later subscribers via
    /// [`Self::subscribe`] see events from their subscription onward).
    ///
    /// # Errors
    ///
    /// Every [`JobError`] variant is reachable here: the typed configuration
    /// gate runs first (no GStreamer involved), then element preflight, then
    /// pipeline construction.
    pub fn start(
        config: RecordingJobConfig,
    ) -> Result<(Self, broadcast::Receiver<SessionEvent>), JobError> {
        // Layer 1+2: typed validation, no side effects.
        config.validate()?;

        // Layer 3: environment preflight — typed before the parse attempt.
        // `make`, not `find`: a stale registry can still hand out factories
        // whose plugin DLL no longer loads (e.g. a broken libav build). Such
        // a factory survives `find` but cannot produce an element, and the
        // parse attempt would then fail with something untyped like
        // "link has no sink" instead of naming the element. Creating (and
        // immediately dropping) the element is the only honest availability
        // check.
        gst::init().map_err(|e| JobError::Gstreamer(e.to_string()))?;
        let missing: Vec<&'static str> = config
            .required_elements()
            .into_iter()
            .filter(|name| gst::ElementFactory::make(name).build().is_err())
            .collect();
        if !missing.is_empty() {
            return Err(JobError::MissingElements { elements: missing });
        }

        let description = config.pipeline_description()?;
        let pipeline = gst::parse::launch(&description)
            .map_err(|e| JobError::Pipeline(e.to_string()))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| JobError::Pipeline("description did not create a pipeline".into()))?;

        let video_src = pipeline
            .by_name("video_src")
            .and_then(|el| el.downcast::<gst_app::AppSrc>().ok())
            .ok_or_else(|| JobError::Pipeline("video_src appsrc missing".into()))?;
        let audio_src = if config.audio.is_some() {
            Some(
                pipeline
                    .by_name("audio_src")
                    .and_then(|el| el.downcast::<gst_app::AppSrc>().ok())
                    .ok_or_else(|| JobError::Pipeline("audio_src appsrc missing".into()))?,
            )
        } else {
            None
        };

        // Byte counter on the filesink, mirroring the engine's metrics probe.
        let bytes = Arc::new(AtomicU64::new(0));
        if let Some(sink) = pipeline.by_name("file_sink") {
            if let Some(pad) = sink.static_pad("sink") {
                let bytes = Arc::clone(&bytes);
                pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
                    if let Some(buffer) = info.buffer() {
                        bytes.fetch_add(buffer.size() as u64, Ordering::Relaxed);
                    }
                    gst::PadProbeReturn::Ok
                });
            }
        }

        pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| JobError::Pipeline(format!("could not start: {e}")))?;

        let (events, receiver) = broadcast::channel(64);
        let _ = events.send(SessionEvent::Started {
            path: config.recording.path.as_ref().map(PathBuf::from),
        });

        let frames = Arc::new(AtomicU64::new(0));
        let audio_samples = Arc::new(AtomicU64::new(0));
        let failed = Arc::new(AtomicBool::new(false));
        let running = Arc::new(AtomicBool::new(true));

        // Progress/error ticker: reads the shared counters and drains bus
        // errors so the embedder's UI gets async failures without polling
        // GStreamer itself.
        let ticker = {
            let events = events.clone();
            let frames = Arc::clone(&frames);
            let audio_samples = Arc::clone(&audio_samples);
            let bytes = Arc::clone(&bytes);
            let failed = Arc::clone(&failed);
            let running = Arc::clone(&running);
            let bus = pipeline.bus();
            let started = Instant::now();
            std::thread::spawn(move || {
                while running.load(Ordering::Relaxed) {
                    // One bounded bus wait per tick: a pipeline error
                    // returns early, an idle bus times out after
                    // PROGRESS_INTERVAL. The bound is what makes both
                    // promises hold — a healthy pipeline still gets its
                    // Progress tick, and `running = false` is observed
                    // within one tick so `finalize`'s ticker join cannot
                    // deadlock. The previous unbounded error-only pop
                    // returned on neither count: no Progress events on a
                    // healthy pipeline and a join that never finished.
                    let msg = match bus.as_ref() {
                        Some(bus) => bus.timed_pop_filtered(
                            Some(gst::ClockTime::from_nseconds(
                                PROGRESS_INTERVAL.as_nanos() as u64
                            )),
                            &[gst::MessageType::Error],
                        ),
                        None => {
                            std::thread::sleep(PROGRESS_INTERVAL);
                            None
                        }
                    };
                    if !running.load(Ordering::Relaxed) {
                        break;
                    }
                    if let Some(msg) = msg {
                        let err = match msg.view() {
                            gst::MessageView::Error(e) => e.error().to_string(),
                            _ => "unknown pipeline error".to_string(),
                        };
                        failed.store(true, Ordering::Relaxed);
                        let _ = events.send(SessionEvent::Failed(err));
                    }
                    let _ = events.send(SessionEvent::Progress {
                        frames_written: frames.load(Ordering::Relaxed),
                        audio_samples: audio_samples.load(Ordering::Relaxed),
                        bytes_written: bytes.load(Ordering::Relaxed),
                        elapsed: started.elapsed(),
                    });
                }
            })
        };

        Ok((
            Self {
                pipeline: Some(pipeline),
                video_src,
                audio_src,
                state: JobState::Recording,
                config,
                started: Instant::now(),
                frames,
                audio_samples,
                bytes,
                failed,
                running,
                events,
                ticker: Some(ticker),
            },
            receiver,
        ))
    }

    /// Register an additional event listener. It receives events sent from
    /// this point onward (the receiver from [`Self::start`] already saw
    /// [`SessionEvent::Started`]).
    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.events.subscribe()
    }

    /// The current lifecycle state.
    pub fn state(&self) -> JobState {
        self.state
    }

    /// Push one RGBA frame (`width × height × 4` bytes, matching
    /// [`RecordingConfig::width`]/[`RecordingConfig::height`]).
    ///
    /// Blocks only against pipeline backpressure; while paused or stopped it
    /// returns [`JobError::WrongState`] immediately instead of buffering.
    pub fn push_frame(&mut self, rgba: &[u8]) -> Result<(), JobError> {
        if self.state != JobState::Recording {
            return Err(JobError::WrongState {
                operation: "push_frame",
                state: self.state,
            });
        }
        let expected =
            self.config.recording.width as usize * self.config.recording.height as usize * 4;
        if rgba.len() != expected {
            return Err(JobError::FrameSizeMismatch {
                expected,
                got: rgba.len(),
            });
        }

        let mut buffer =
            gst::Buffer::with_size(rgba.len()).map_err(|e| JobError::PushFailed(e.to_string()))?;
        {
            let buffer_ref = buffer
                .get_mut()
                .ok_or_else(|| JobError::PushFailed("buffer not writable".into()))?;
            let mut map = buffer_ref
                .map_writable()
                .map_err(|e| JobError::PushFailed(e.to_string()))?;
            map.as_mut_slice().copy_from_slice(rgba);
        }

        // First push can race the pipeline's transition to PLAYING under
        // load; retry briefly instead of failing on a transient flow error
        // (the engine's `process_raw_frame` retries the same way).
        let mut result = self.video_src.push_buffer(buffer);
        for _ in 0..3 {
            if result.is_ok() || self.pipeline.is_none() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
            let mut retry = gst::Buffer::with_size(expected)
                .map_err(|e| JobError::PushFailed(e.to_string()))?;
            if let Some(buffer_ref) = retry.get_mut() {
                if let Ok(mut map) = buffer_ref.map_writable() {
                    map.as_mut_slice().copy_from_slice(rgba);
                }
            }
            result = self.video_src.push_buffer(retry);
        }
        result
            .map(|_| ())
            .map_err(|e| JobError::PushFailed(e.to_string()))?;
        self.frames.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Push interleaved F32 samples matching [`AudioSettings`]. The length
    /// must be a multiple of the channel count.
    ///
    /// Returns [`JobError::WrongState`] while paused or stopped, for the
    /// same no-silent-buffering reason as [`Self::push_frame`].
    pub fn push_audio(&mut self, samples: &[f32]) -> Result<(), JobError> {
        if self.state != JobState::Recording {
            return Err(JobError::WrongState {
                operation: "push_audio",
                state: self.state,
            });
        }
        let audio = self.config.audio.as_ref().ok_or(JobError::WrongState {
            operation: "push_audio",
            state: self.state,
        })?;
        if !samples.len().is_multiple_of(audio.channels as usize) {
            return Err(JobError::AudioBufferMisaligned {
                samples: samples.len(),
                channels: audio.channels,
            });
        }
        let Some(audio_src) = self.audio_src.as_ref() else {
            return Err(JobError::WrongState {
                operation: "push_audio",
                state: self.state,
            });
        };

        let mut bytes = Vec::with_capacity(samples.len() * 4);
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        let mut buffer =
            gst::Buffer::with_size(bytes.len()).map_err(|e| JobError::PushFailed(e.to_string()))?;
        if let Some(buffer_ref) = buffer.get_mut() {
            if let Ok(mut map) = buffer_ref.map_writable() {
                map.as_mut_slice().copy_from_slice(&bytes);
            }
        }
        audio_src
            .push_buffer(buffer)
            .map(|_| ())
            .map_err(|e| JobError::PushFailed(e.to_string()))?;
        self.audio_samples
            .fetch_add(samples.len() as u64, Ordering::Relaxed);
        Ok(())
    }

    /// Stop accepting frames and samples until [`Self::resume`].
    ///
    /// The gate is the lifecycle state, not the pipeline: setting a pipeline
    /// that carries a live appsrc into `PAUSED` parks its streaming thread
    /// mid-transition, and the appsrc's src pad is still flushing after a
    /// later `PLAYING` (`push_buffer` then fails with "Pad is flushing").
    /// The pipeline therefore keeps running and only the pushes stop — the
    /// paused time still shows up as a timestamp gap in the output, because
    /// appsrc stamps buffers at push time (`do-timestamp=true`).
    pub fn pause(&mut self) -> Result<(), JobError> {
        if self.state != JobState::Recording {
            return Err(JobError::WrongState {
                operation: "pause",
                state: self.state,
            });
        }
        self.state = JobState::Paused;
        let _ = self.events.send(SessionEvent::Paused);
        Ok(())
    }

    /// Accept frames and samples again after [`Self::pause`].
    pub fn resume(&mut self) -> Result<(), JobError> {
        if self.state != JobState::Paused {
            return Err(JobError::WrongState {
                operation: "resume",
                state: self.state,
            });
        }
        self.state = JobState::Recording;
        let _ = self.events.send(SessionEvent::Resumed);
        Ok(())
    }

    /// Stop the session, finalize the output, and return the report.
    ///
    /// Consumes the handle; dropping it afterwards is a no-op. Works from
    /// `Paused` as well — the pipeline was never held, so the EOS reaches
    /// the muxer directly.
    pub fn stop(mut self) -> Result<SessionReport, JobError> {
        self.finalize()
    }

    /// Shared teardown for `stop` and `Drop`: EOS on every appsrc, wait for
    /// the muxer to finalize (up to [`EOS_BUDGET`], skipped when the pipeline
    /// already failed), set the pipeline to `NULL`, stop the ticker, emit
    /// [`SessionEvent::Stopped`].
    fn finalize(&mut self) -> Result<SessionReport, JobError> {
        self.state = JobState::Stopped;
        let report = self.report();

        // Stop the ticker before waiting on the bus: it polls the same
        // bus, and its filtered pop consumes messages that are not errors —
        // including the EOS the wait below is looking for. The ticker's pop
        // is bounded by PROGRESS_INTERVAL, so this join cannot block.
        self.running.store(false, Ordering::Relaxed);
        if let Some(ticker) = self.ticker.take() {
            let _ = ticker.join();
        }

        let Some(pipeline) = self.pipeline.take() else {
            return Ok(report); // already finalized (defensive; stop() consumes)
        };

        if !self.failed.load(Ordering::Relaxed) {
            let _ = self.video_src.end_of_stream();
            if let Some(audio) = &self.audio_src {
                let _ = audio.end_of_stream();
            }
            if let Some(bus) = pipeline.bus() {
                let _ = bus.timed_pop_filtered(
                    gst::ClockTime::from_nseconds(EOS_BUDGET.as_nanos() as u64),
                    &[gst::MessageType::Eos, gst::MessageType::Error],
                );
            }
        }

        let _ = pipeline.set_state(gst::State::Null);
        let _ = self.events.send(SessionEvent::Stopped(report.clone()));
        Ok(report)
    }

    /// Snapshot the counters into a report (works for `stop` and `Drop`).
    fn report(&self) -> SessionReport {
        SessionReport {
            frames_written: self.frames.load(Ordering::Relaxed),
            audio_samples: self.audio_samples.load(Ordering::Relaxed),
            bytes_written: self.bytes.load(Ordering::Relaxed),
            elapsed: self.started.elapsed(),
            path: self.config.recording.path.as_ref().map(PathBuf::from),
        }
    }
}

impl Drop for RecordingJob {
    /// The documented decision: dropping the handle **stops and finalizes**
    /// the recording, exactly like [`RecordingJob::stop`] — an embedder that
    /// forgets to stop still gets a valid output file, and no pipeline
    /// outlives its handle. Failures during teardown are recorded on the
    /// event stream rather than panicking in `Drop`.
    fn drop(&mut self) {
        if self.pipeline.is_some() {
            let _ = self.finalize();
        } else {
            // Stopped normally but the ticker may still be running if
            // `stop` returned early; make sure it exits.
            self.running.store(false, Ordering::Relaxed);
            if let Some(ticker) = self.ticker.take() {
                let _ = ticker.join();
            }
        }
    }
}
