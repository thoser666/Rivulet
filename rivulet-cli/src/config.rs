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
        if !matches!(self.video.source.as_str(), "test") {
            return Err(format!(
                "video.source: unsupported source {:?} (headless-capable: \"test\"; capture-backed sources need hardware)",
                self.video.source
            ));
        }
        if self.video.width == 0 || self.video.height == 0 {
            return Err("video.width/video.height: must be greater than zero".to_string());
        }
        if self.video.fps == 0 {
            return Err("video.fps: must be greater than zero".to_string());
        }
        if self.video.width % 2 != 0 || self.video.height % 2 != 0 {
            return Err(
                "video.width/video.height: must be even (H.264 requires even dimensions)"
                    .to_string(),
            );
        }
        Ok(())
    }

    /// Load from a TOML file.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("config file {}: {e}", path.display()))?;
        toml::from_str(&raw)
            .map_err(|e| format!("config file {}: invalid TOML: {e}", path.display()))
    }
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
}
