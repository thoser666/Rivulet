//! CLI configuration model (TOML file + overrides), mirroring the M7 spec's
//! W1 contract (issue #186): a small, documented surface that maps onto
//! [`rivulet_core::RivuletEngine`] settings.

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
        } => format!("recording stopped: {seconds}s frames={frames} bytes={file_size_bytes}"),
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
        };
        assert_eq!(
            serde_json::to_string(&stopped).unwrap(),
            r#"{"event":"stopped","frames":120,"seconds":4,"file_size_bytes":4096}"#
        );
    }
}
