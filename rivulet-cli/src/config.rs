//! CLI configuration model (TOML file + overrides), mirroring the M7 spec's
//! W1 contract (issue #186): a small, documented surface that maps onto
//! [`rivulet_core::RivuletEngine`] settings.

use rivulet_core::inspect::NondeterminismReport;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Top-level CLI configuration (`rivulet record --config file.toml`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecordConfig {
    /// Recording output settings.
    #[serde(default)]
    pub output: OutputConfig,
    /// Video source and encoding settings.
    #[serde(default)]
    pub video: VideoConfig,
    /// Audio source settings (optional; recording is video-only when absent).
    #[serde(default)]
    pub audio: AudioConfig,
    /// Stop the recording after this many seconds.
    #[serde(default)]
    pub duration_secs: Option<u64>,
}

/// Output file settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OutputConfig {
    /// Destination file path. Required.
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// Container format: `mp4` (default), `mkv`, `mov`, or `mpegts`.
    #[serde(default)]
    pub container: Option<String>,
}

/// Video source and encoding settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoConfig {
    /// Source kind. Currently the only headless-capable value: `test`
    /// (GStreamer `videotestsrc`); capture-backed sources need hardware.
    #[serde(default = "default_source")]
    pub source: String,
    /// Source pattern for `source = "test"` (`smpte`, `ball`, ...).
    #[serde(default)]
    pub test_pattern: Option<String>,
    /// Frame width in pixels.
    #[serde(default = "default_width")]
    pub width: u32,
    /// Frame height in pixels.
    #[serde(default = "default_height")]
    pub height: u32,
    /// Frames per second fed into the engine.
    #[serde(default = "default_fps")]
    pub fps: u32,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            source: default_source(),
            test_pattern: None,
            width: default_width(),
            height: default_height(),
            fps: default_fps(),
        }
    }
}

fn default_source() -> String {
    "test".to_string()
}
fn default_width() -> u32 {
    640
}
fn default_height() -> u32 {
    360
}
fn default_fps() -> u32 {
    30
}

/// Audio settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AudioConfig {
    /// When `true`, a silent stereo test-audio track is recorded alongside
    /// the video (proves the audio branch headless).
    #[serde(default)]
    pub enabled: bool,
}

impl RecordConfig {
    /// Validate the configuration, returning an actionable error naming the
    /// offending key (spec: invalid config exits non-zero with the key).
    pub fn validate(&self) -> Result<(), String> {
        let Some(path) = self.output.path.as_ref() else {
            return Err("output.path: required (no recording output path set)".to_string());
        };
        if path.as_os_str().is_empty() {
            return Err("output.path: must not be empty".to_string());
        }
        if !matches!(
            self.output.container.as_deref(),
            None | Some("mp4") | Some("mkv") | Some("mov") | Some("mpegts")
        ) {
            return Err(format!(
                "output.container: unknown container {:?} (expected mp4, mkv, mov, or mpegts)",
                self.output.container
            ));
        }
        validate_video(&self.video)
    }

    /// Load from a TOML file.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("config file {}: {e}", path.display()))?;
        toml::from_str(&raw)
            .map_err(|e| format!("config file {}: invalid TOML: {e}", path.display()))
    }
}

/// Video validation shared by [`RecordConfig`] and [`StreamConfig`], so a
/// recording and a stream reject the same dimensions and sources with the same
/// message.
fn validate_video(video: &VideoConfig) -> Result<(), String> {
    if !matches!(video.source.as_str(), "test") {
        return Err(format!(
            "video.source: unsupported source {:?} (headless-capable: \"test\"; capture-backed sources need hardware)",
            video.source
        ));
    }
    if video.width == 0 || video.height == 0 {
        return Err("video.width/video.height: must be greater than zero".to_string());
    }
    if video.fps == 0 {
        return Err("video.fps: must be greater than zero".to_string());
    }
    if video.width % 2 != 0 || video.height % 2 != 0 {
        return Err(
            "video.width/video.height: must be even (H.264 requires even dimensions)".to_string(),
        );
    }
    Ok(())
}

/// Top-level streaming configuration (`rivulet stream --config file.toml`,
/// issue #289 / finding F-M7-002).
///
/// Deliberately the same shape as [`RecordConfig`] — video, audio, duration —
/// plus a `[stream]` table, because the job that drives it has the same
/// lifecycle as [`crate::RecordJob`] and differs only in where the frames go.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StreamConfig {
    /// Ingest target settings.
    #[serde(default)]
    pub stream: StreamOutputConfig,
    /// Video source and encoding settings.
    #[serde(default)]
    pub video: VideoConfig,
    /// Audio source settings (the stream is video-only when absent).
    #[serde(default)]
    pub audio: AudioConfig,
    /// Stop the stream after this many seconds.
    #[serde(default)]
    pub duration_secs: Option<u64>,
}

/// Ingest target settings (`[stream]` in the TOML file).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamOutputConfig {
    /// Platform preset: `twitch`, `youtube`, `kick` or `custom` (default).
    ///
    /// Twitch and YouTube supply their own ingest URL; a platform preset
    /// requires TLS (`rtmps://`). `kick` and `custom` take the URL from
    /// `ingest_url`.
    #[serde(default = "default_platform")]
    pub platform: String,
    /// Ingest URL (overrides the platform default when set).
    #[serde(default)]
    pub ingest_url: Option<String>,
    /// The stream key. Required.
    #[serde(default)]
    pub stream_key: Option<String>,
    /// Bitrate preset: `low` (2500 kbit/s), `standard` (4500, default),
    /// `high` (6000) or `custom` (keep the engine default).
    #[serde(default)]
    pub preset: Option<String>,
}

impl Default for StreamOutputConfig {
    fn default() -> Self {
        Self {
            platform: default_platform(),
            ingest_url: None,
            stream_key: None,
            preset: None,
        }
    }
}

fn default_platform() -> String {
    "custom".to_string()
}

impl StreamConfig {
    /// Validate the configuration, naming the offending key — the same
    /// contract [`RecordConfig::validate`] follows.
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(
            self.stream.platform.as_str(),
            "twitch" | "youtube" | "kick" | "custom"
        ) {
            return Err(format!(
                "stream.platform: unknown platform {:?} (expected twitch, youtube, kick, or custom)",
                self.stream.platform
            ));
        }
        if self
            .stream
            .stream_key
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
        {
            return Err("stream.stream_key: required (no stream key set)".to_string());
        }
        let url = self.stream.ingest_url.as_deref().unwrap_or_default();
        if url.is_empty() && matches!(self.stream.platform.as_str(), "kick" | "custom") {
            return Err(format!(
                "stream.ingest_url: required for platform {:?}",
                self.stream.platform
            ));
        }
        if !url.is_empty() && !(url.starts_with("rtmp://") || url.starts_with("rtmps://")) {
            return Err(
                "stream.ingest_url: must use rtmp:// or rtmps:// (the scheme names the protocol)"
                    .to_string(),
            );
        }
        if !url.is_empty() && self.stream.platform != "custom" && !url.starts_with("rtmps://") {
            return Err(format!(
                "stream.ingest_url: platform {:?} requires TLS (rtmps://)",
                self.stream.platform
            ));
        }
        if let Some(preset) = self.stream.preset.as_deref() {
            if !matches!(preset, "low" | "standard" | "high" | "custom") {
                return Err(format!(
                    "stream.preset: unknown preset {preset:?} (expected low, standard, high, or custom)"
                ));
            }
        }
        validate_video(&self.video)
    }

    /// Load from a TOML file.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("config file {}: {e}", path.display()))?;
        toml::from_str(&raw)
            .map_err(|e| format!("config file {}: invalid TOML: {e}", path.display()))
    }

    /// A copy that is safe to leave the process: the stream key is replaced by
    /// its two-character prefix form, and an ingest URL that embeds the key in
    /// its path is replaced wholesale.
    ///
    /// Every surface that serializes the config — the `failed` event and the
    /// `stream --dry-run` plan — uses this instead of the live config. The
    /// masking rule matches [`rivulet_core::StreamSettings::masked_key`].
    pub fn redacted(&self) -> Self {
        let mut copy = self.clone();
        let Some(key) = self.stream.stream_key.as_deref() else {
            return copy;
        };
        copy.stream.stream_key = Some(masked_key(key));
        if !key.is_empty()
            && self
                .stream
                .ingest_url
                .as_deref()
                .is_some_and(|url| url.contains(key))
        {
            copy.stream.ingest_url = Some("<redacted stream URL>".to_string());
        }
        copy
    }
}

/// The two-character prefix form of a stream key (`ab••••`), the deliberate
/// short form the UI shows. Kept identical to
/// [`rivulet_core::StreamSettings::masked_key`].
fn masked_key(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    let end = key.char_indices().nth(2).map(|(i, _)| i).unwrap_or(0);
    format!("{}••••", &key[..end])
}

/// The pipeline stage a run failed in (M7 W5, issue #191).
///
/// This is the machine-readable half of "a failing run names the failing
/// stage": the human stderr line and the `failed` JSON event both identify it.
/// The set is deliberately small and stable so downstream tooling can branch on
/// it without parsing messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Command-line arguments were invalid (exit code 2).
    Usage,
    /// The config file or its values were invalid (exit code 2).
    Config,
    /// The output location could not be prepared (exit code 1).
    ///
    /// For `rivulet stream` this is the ingest endpoint: an unreachable
    /// ingest is an output failure, not a mid-stream surprise.
    Output,
    /// The media engine failed while pushing frames or audio (exit code 1).
    Engine,
    /// The engine failed while stopping/finalizing, or produced nothing.
    Finalize,
}

impl Stage {
    /// The stable snake_case name used in the JSON event and the human line.
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Usage => "usage",
            Stage::Config => "config",
            Stage::Output => "output",
            Stage::Engine => "engine",
            Stage::Finalize => "finalize",
        }
    }
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// JSON status event emitted on stdout (spec: stable documented schema).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum StatusEvent {
    /// Emitted once when the run starts.
    Started {
        width: u32,
        height: u32,
        fps: u32,
        #[serde(rename = "audio")]
        audio_enabled: bool,
    },
    /// Emitted every `progress-interval` seconds with engine metrics.
    Progress {
        seconds: u64,
        frames: u64,
        fps: f64,
        file_size_bytes: u64,
    },
    /// Emitted once when the recording has been finalized.
    Stopped {
        frames: u64,
        seconds: u64,
        file_size_bytes: u64,
        /// Clock mode that drove the run (`system` or `virtual`).
        clock: String,
        /// How video PTS were produced (`do-timestamp` or `clock-driven`).
        pts_source: String,
        /// Encoder backend the run actually used.
        encoder: String,
        /// Whether the active nondeterminism sources permit a reproducible
        /// container timestamp sequence (M7 W2a, spec § Nondeterminism
        /// inventory).
        reproducible: bool,
        /// Whether the encoded bytes are expected to repeat run to run.
        byte_reproducible: bool,
        /// The nondeterminism sources this run actually used, with details.
        ///
        /// Always the full inventory so a consumer can tell a documented limit
        /// from a limit that applied to this run.
        nondeterminism: NondeterminismReport,
    },
    /// Emitted once, as the final stdout object, when the run fails (M7 W5).
    ///
    /// This is the machine-readable failure report: it names the [`Stage`] the
    /// run failed in, the exit code, and (when known) the redacted pipeline and
    /// the resolved config, without leaking secrets. A JSON consumer sees the
    /// same stream shape on success and failure, so a broken run never looks
    /// like an empty successful one.
    Failed {
        /// The stage that failed.
        stage: Stage,
        /// Process exit code the caller should return.
        exit_code: i32,
        /// Actionable, secret-free description of the failure.
        message: String,
        /// The redacted pipeline the engine was running, when one was built.
        #[serde(skip_serializing_if = "Option::is_none")]
        pipeline: Option<String>,
        /// The resolved config the run was driven from, when one was parsed.
        #[serde(skip_serializing_if = "Option::is_none")]
        config: Option<RecordConfig>,
        /// The resolved streaming config, when the failed run was
        /// `rivulet stream`.
        ///
        /// The failure schema is shared by both subcommands so a consumer
        /// parses one `failed` object shape; exactly one of `config` and
        /// `stream_config` is present, depending on which job ran.
        ///
        /// Boxed to keep this event the same size as the other variants (the
        /// enum is cloned per emitted event).
        #[serde(skip_serializing_if = "Option::is_none")]
        stream_config: Option<Box<StreamConfig>>,
    },
}

/// Human-readable one-line description of an event (stderr diagnostics).
pub fn describe(event: &StatusEvent) -> String {
    match event {
        StatusEvent::Started {
            width,
            height,
            fps,
            audio_enabled,
        } => format!("recording started: {width}x{height}@{fps} audio={audio_enabled}"),
        StatusEvent::Progress {
            seconds,
            frames,
            fps,
            file_size_bytes,
        } => format!("progress: {seconds}s frames={frames} fps={fps:.1} bytes={file_size_bytes}"),
        StatusEvent::Stopped {
            frames,
            seconds,
            file_size_bytes,
            clock,
            pts_source,
            encoder,
            reproducible,
            byte_reproducible,
            nondeterminism,
        } => {
            let limits: Vec<&str> = nondeterminism
                .active_sources()
                .iter()
                .filter(|entry| entry.affects_timestamps || entry.affects_bytes)
                .map(|entry| entry.source.as_str())
                .collect();
            let mut line = format!(
                "recording stopped: {seconds}s frames={frames} bytes={file_size_bytes} \
                 clock={clock} pts={pts_source} encoder={encoder} \
                 reproducible={reproducible} byte_reproducible={byte_reproducible}"
            );
            if !limits.is_empty() {
                line.push_str(&format!(" limits={}", limits.join(",")));
            }
            line
        }
        StatusEvent::Failed {
            stage,
            exit_code: _,
            message,
            ..
        } => format!("error: {stage}: {message}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_minimal_config_passes() {
        let cfg = RecordConfig {
            output: OutputConfig {
                path: Some(PathBuf::from("/tmp/out.mp4")),
                container: None,
            },
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn missing_path_names_the_key() {
        let err = RecordConfig::default().validate().unwrap_err();
        assert!(err.contains("output.path"), "error = {err}");
    }

    #[test]
    fn bad_container_names_the_key() {
        let cfg = RecordConfig {
            output: OutputConfig {
                path: Some(PathBuf::from("x.mp4")),
                container: Some("avi".to_string()),
            },
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("output.container"), "error = {err}");
    }

    #[test]
    fn bad_source_names_the_key() {
        let cfg = RecordConfig {
            output: OutputConfig {
                path: Some(PathBuf::from("x.mp4")),
                container: None,
            },
            video: VideoConfig {
                source: "window".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("video.source"), "error = {err}");
    }

    #[test]
    fn odd_dimensions_rejected() {
        let cfg = RecordConfig {
            output: OutputConfig {
                path: Some(PathBuf::from("x.mp4")),
                container: None,
            },
            video: VideoConfig {
                width: 641,
                ..Default::default()
            },
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("video.width"), "error = {err}");
    }

    #[test]
    fn toml_roundtrip_preserves_fields() {
        let cfg = RecordConfig {
            output: OutputConfig {
                path: Some(PathBuf::from("/tmp/a.mkv")),
                container: Some("mkv".to_string()),
            },
            video: VideoConfig {
                source: "test".to_string(),
                test_pattern: Some("ball".to_string()),
                width: 1280,
                height: 720,
                fps: 60,
            },
            audio: AudioConfig { enabled: true },
            duration_secs: Some(5),
        };
        let text = toml::to_string(&cfg).unwrap();
        let parsed: RecordConfig = toml::from_str(&text).unwrap();
        assert_eq!(cfg, parsed);
    }

    #[test]
    fn status_event_json_shape_is_stable() {
        let started = StatusEvent::Started {
            width: 640,
            height: 360,
            fps: 30,
            audio_enabled: false,
        };
        assert_eq!(
            serde_json::to_string(&started).unwrap(),
            r#"{"event":"started","width":640,"height":360,"fps":30,"audio":false}"#
        );
        let progress = StatusEvent::Progress {
            seconds: 2,
            frames: 60,
            fps: 30.0,
            file_size_bytes: 1024,
        };
        assert_eq!(
            serde_json::to_string(&progress).unwrap(),
            r#"{"event":"progress","seconds":2,"frames":60,"fps":30.0,"file_size_bytes":1024}"#
        );
        let stopped = StatusEvent::Stopped {
            frames: 120,
            seconds: 4,
            file_size_bytes: 4096,
            clock: "system".to_string(),
            pts_source: "do-timestamp".to_string(),
            encoder: "Software".to_string(),
            reproducible: false,
            byte_reproducible: false,
            nondeterminism: NondeterminismReport::for_run(
                rivulet_core::clock::ClockMode::System,
                rivulet_core::encoder::VideoEncoder::Software,
                rivulet_core::source::SourceKind::Color,
            ),
        };
        // The schema is a documented machine-readable contract (spec
        // § CLI surface reference), so the key set is pinned rather than
        // string-compared: the inventory alone makes the payload large and
        // every source is covered by its own tests.
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&stopped).unwrap()).unwrap();
        assert_eq!(value["event"], "stopped");
        assert_eq!(value["clock"], "system");
        assert_eq!(value["pts_source"], "do-timestamp");
        assert_eq!(value["encoder"], "Software");
        assert_eq!(value["reproducible"], false);
        assert_eq!(value["byte_reproducible"], false);
        assert_eq!(value["frames"], 120);
        assert_eq!(value["seconds"], 4);
        assert_eq!(value["file_size_bytes"], 4096);
        assert!(
            value["nondeterminism"]["sources"].is_array(),
            "the run report must carry the nondeterminism inventory"
        );

        // The human line names the limit instead of only printing a number.
        let line = describe(&stopped);
        assert!(line.contains("clock=system"), "got {line}");
        assert!(line.contains("reproducible=false"), "got {line}");
        assert!(line.contains("limits="), "the limit must be named: {line}");
    }

    #[test]
    fn failed_event_json_shape_is_stable() {
        // M7 W5: the machine-readable failure report. The event name comes from
        // the variant under `rename_all = "snake_case"`, so assert the wire
        // shape here rather than in the source pin.
        let failed = StatusEvent::Failed {
            stage: Stage::Engine,
            exit_code: 1,
            message: "engine error: bus error".to_string(),
            pipeline: Some("videotestsrc ! mp4mux ! filesink".to_string()),
            config: None,
            stream_config: None,
        };
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&failed).unwrap()).unwrap();
        assert_eq!(value["event"], "failed");
        assert_eq!(value["stage"], "engine");
        assert_eq!(value["exit_code"], 1);
        assert_eq!(value["message"], "engine error: bus error");
        assert_eq!(value["pipeline"], "videotestsrc ! mp4mux ! filesink");
        assert!(
            value.get("config").is_none(),
            "an unset config must be omitted, not serialized as null"
        );

        // Every stage maps to its documented snake_case name and exit code.
        for (stage, name, code) in [
            (Stage::Usage, "usage", 2),
            (Stage::Config, "config", 2),
            (Stage::Output, "output", 1),
            (Stage::Engine, "engine", 1),
            (Stage::Finalize, "finalize", 1),
        ] {
            assert_eq!(stage.as_str(), name);
            let event = StatusEvent::Failed {
                stage,
                exit_code: code,
                message: "m".to_string(),
                pipeline: None,
                config: None,
                stream_config: None,
            };
            let value: serde_json::Value =
                serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
            assert_eq!(value["stage"], name);
            assert_eq!(value["exit_code"], code);
        }

        let line = describe(&failed);
        assert!(line.starts_with("error: "), "{line}");
        assert!(
            line.contains("engine"),
            "the human line names the stage: {line}"
        );
    }

    // --- `rivulet stream` config (M7 closeout, issue #289) ---

    fn stream_config() -> StreamConfig {
        StreamConfig {
            stream: StreamOutputConfig {
                platform: "custom".to_string(),
                ingest_url: Some("rtmp://127.0.0.1:1935/live".to_string()),
                stream_key: Some("fixture_key".to_string()),
                preset: None,
            },
            ..Default::default()
        }
    }

    #[test]
    fn a_minimal_stream_config_passes() {
        assert!(stream_config().validate().is_ok());
    }

    #[test]
    fn a_missing_stream_key_names_the_key() {
        let mut cfg = stream_config();
        cfg.stream.stream_key = None;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("stream.stream_key"), "error = {err}");
    }

    #[test]
    fn an_unknown_platform_names_the_key() {
        let mut cfg = stream_config();
        cfg.stream.platform = "owned".to_string();
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("stream.platform"), "error = {err}");
        assert!(err.contains("owned"), "error names the value: {err}");
    }

    #[test]
    fn a_custom_target_without_an_ingest_url_names_the_key() {
        let mut cfg = stream_config();
        cfg.stream.ingest_url = None;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("stream.ingest_url"), "error = {err}");
        assert!(err.contains("custom"), "error names the platform: {err}");
    }

    #[test]
    fn a_platform_preset_requires_tls() {
        let mut cfg = stream_config();
        cfg.stream.platform = "twitch".to_string();
        cfg.stream.ingest_url = Some("rtmp://live.twitch.tv/app".to_string());
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("stream.ingest_url"), "error = {err}");
        assert!(err.contains("rtmps://"), "error names the fix: {err}");
    }

    #[test]
    fn a_platform_preset_may_use_its_default_ingest() {
        let mut cfg = stream_config();
        cfg.stream.platform = "youtube".to_string();
        cfg.stream.ingest_url = None;
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn an_unknown_preset_names_the_key() {
        let mut cfg = stream_config();
        cfg.stream.preset = Some("ultra".to_string());
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("stream.preset"), "error = {err}");
        assert!(err.contains("ultra"), "error names the value: {err}");
    }

    #[test]
    fn the_stream_config_reuses_the_video_validation() {
        let mut cfg = stream_config();
        cfg.video.width = 641;
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("video.width"), "error = {err}");
    }

    #[test]
    fn stream_config_toml_roundtrip_preserves_fields() {
        let mut cfg = stream_config();
        cfg.stream.preset = Some("high".to_string());
        cfg.duration_secs = Some(30);
        cfg.audio.enabled = true;
        let text = toml::to_string(&cfg).unwrap();
        let parsed: StreamConfig = toml::from_str(&text).unwrap();
        assert_eq!(cfg, parsed);
    }

    #[test]
    fn a_stream_failure_carries_the_stream_config_not_the_record_config() {
        let cfg = stream_config();
        let event = StatusEvent::Failed {
            stage: Stage::Output,
            exit_code: 1,
            message: "ingest unreachable".to_string(),
            pipeline: None,
            config: None,
            stream_config: Some(Box::new(cfg.clone())),
        };
        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&event).unwrap()).unwrap();
        assert_eq!(value["event"], "failed");
        assert_eq!(value["stage"], "output");
        assert!(
            value.get("config").is_none(),
            "the record config must be omitted for a stream run"
        );
        assert_eq!(value["stream_config"]["stream"]["platform"], "custom");
        assert!(
            value["stream_config"].get("output").is_none(),
            "a stream config has no recording output: {value}"
        );
    }
}
