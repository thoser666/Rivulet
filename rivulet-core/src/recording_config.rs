//! Typed, early configuration validation for the in-process recording entry
//! point ([#294](https://github.com/thoser666/Rivulet/issues/294)).
//!
//! The M8 gate criterion this module answers is *"Invalid configuration fails
//! early with typed, actionable errors."* Historically a bad recording setup
//! surfaced only when GStreamer built the pipeline — an encoder or muxer
//! start failure phrased in GStreamer vocabulary that means nothing to the
//! developer embedding Rivulet. Every parameter here is checked **before** a
//! single frame is requested, and every rejection is a typed
//! [`RecordingConfigError`] that names the offending field and states the fix.
//!
//! This is deliberately hardware-, network- and GStreamer-independent: it is
//! pure data validation, so it runs (and is tested) on any CI runner. What it
//! deliberately does *not* do is answer "is this encoder available on *this*
//! machine" — that is a runtime question and belongs to the feature-detection
//! workstream (#295), not to static config validation.

use crate::container::RecordingContainer;
use crate::encoder::{VideoCodec, VideoEncoder};

/// Smallest video bitrate we accept. Below this the picture is unusable at
/// any of the resolutions we allow, which is almost always a missing zero.
const MIN_VIDEO_BITRATE_KBPS: u32 = 100;
/// Largest video bitrate we accept. Above this the value is almost always a
/// bits-per-second figure entered where kilobits-per-second was expected.
const MAX_VIDEO_BITRATE_KBPS: u32 = 500_000;
/// Largest edge length (width or height) we accept. 8K is the ceiling; larger
/// values are unit or typo errors.
const MAX_DIMENSION: u32 = 8192;
/// Largest frame rate we accept (mirrors the browser-source bound).
const MAX_FPS: u32 = 240;

/// A recording configuration an embedding application builds and hands to the
/// recording entry point.
///
/// Construct it with [`RecordingConfig::default`] (1920×1080 @ 30, 5000 kbps,
/// H.264, MP4, software encoder) and override the fields you care about, then
/// call [`RecordingConfig::validate`] before starting a session. Validation is
/// cheap and side-effect free, so calling it on every config change is fine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingConfig {
    /// Frame width in pixels. Must be even and non-zero.
    pub width: u32,
    /// Frame height in pixels. Must be even and non-zero.
    pub height: u32,
    /// Frames per second. Non-zero and at most 240.
    pub fps: u32,
    /// Target video bitrate in **kilobits** per second.
    pub bitrate_kbps: u32,
    /// Video codec to encode with.
    pub codec: VideoCodec,
    /// Encoder backend to use. Whether it is *available on this machine* is a
    /// runtime question answered by feature detection (#295), not validated
    /// here.
    pub encoder: VideoEncoder,
    /// Container (muxer) to write into. Must be compatible with [`Self::codec`].
    pub container: RecordingContainer,
    /// Output file path. When it carries an extension, that extension must
    /// match [`Self::container`]; a path with no extension is accepted (the
    /// container's own extension can be appended later).
    pub path: Option<String>,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            fps: 30,
            bitrate_kbps: 5000,
            codec: VideoCodec::H264,
            encoder: VideoEncoder::Software,
            container: RecordingContainer::Mp4,
            path: None,
        }
    }
}

impl RecordingConfig {
    /// Validate the configuration, returning the first problem found as a
    /// typed [`RecordingConfigError`].
    ///
    /// Checks run in a fixed order (dimensions, frame rate, bitrate,
    /// codec/container compatibility, path extension) so the reported error is
    /// deterministic for a given config.
    pub fn validate(&self) -> Result<(), RecordingConfigError> {
        self.validate_dimension("width", self.width)?;
        self.validate_dimension("height", self.height)?;

        if self.fps == 0 {
            return Err(RecordingConfigError::FpsZero);
        }
        if self.fps > MAX_FPS {
            return Err(RecordingConfigError::FpsTooHigh {
                fps: self.fps,
                max: MAX_FPS,
            });
        }

        if self.bitrate_kbps == 0 {
            return Err(RecordingConfigError::BitrateZero);
        }
        if self.bitrate_kbps < MIN_VIDEO_BITRATE_KBPS {
            return Err(RecordingConfigError::BitrateTooLow {
                kbps: self.bitrate_kbps,
                min: MIN_VIDEO_BITRATE_KBPS,
            });
        }
        if self.bitrate_kbps > MAX_VIDEO_BITRATE_KBPS {
            return Err(RecordingConfigError::BitrateTooHigh {
                kbps: self.bitrate_kbps,
                max: MAX_VIDEO_BITRATE_KBPS,
            });
        }

        if !container_supports_codec(self.container, self.codec) {
            return Err(RecordingConfigError::CodecContainerIncompatible {
                codec: self.codec,
                container: self.container,
                hint: working_container_for(self.codec),
            });
        }

        if let Some(path) = &self.path {
            self.validate_path_extension(path)?;
        }

        Ok(())
    }

    /// A zero width or height is rejected outright; a non-zero edge must be
    /// even (H.264/H.265 codecs work on macroblocks) and within [`MAX_DIMENSION`].
    fn validate_dimension(
        &self,
        field: &'static str,
        value: u32,
    ) -> Result<(), RecordingConfigError> {
        let too_large = match field {
            "width" => RecordingConfigError::WidthTooLarge {
                width: value,
                max: MAX_DIMENSION,
            },
            _ => RecordingConfigError::HeightTooLarge {
                height: value,
                max: MAX_DIMENSION,
            },
        };
        if value == 0 {
            return Err(match field {
                "width" => RecordingConfigError::WidthZero,
                _ => RecordingConfigError::HeightZero,
            });
        }
        if !value.is_multiple_of(2) {
            return Err(match field {
                "width" => RecordingConfigError::WidthOdd { width: value },
                _ => RecordingConfigError::HeightOdd { height: value },
            });
        }
        if value > MAX_DIMENSION {
            return Err(too_large);
        }
        Ok(())
    }

    /// When a path carries an extension, it must name the configured
    /// container. An unrecognised extension is reported as a mismatch naming
    /// the expected one; a path with no extension is accepted.
    fn validate_path_extension(&self, path: &str) -> Result<(), RecordingConfigError> {
        let ext = std::path::Path::new(path)
            .extension()
            .map(|e| e.to_string_lossy().into_owned());
        let Some(ext) = ext else {
            return Ok(());
        };
        match RecordingContainer::from_extension(&ext) {
            Some(c) if c == self.container => Ok(()),
            _ => Err(RecordingConfigError::PathExtensionMismatch {
                extension: ext,
                container: self.container,
                expected: self.container.file_extension().to_string(),
            }),
        }
    }
}

/// Which containers can carry which codecs, matching what the muxers in
/// [`crate::container`] actually support. VP9 has no MP4 muxer path here (no
/// WebM muxer exists in this build), and H.265 is kept out of MPEG-TS because
/// only H.264 Annex-B is wired.
fn container_supports_codec(container: RecordingContainer, codec: VideoCodec) -> bool {
    match codec {
        VideoCodec::H264 => true,
        VideoCodec::H265 => !matches!(container, RecordingContainer::MpegTs),
        VideoCodec::VP9 => matches!(container, RecordingContainer::Mkv),
    }
}

/// The container a codec *can* be written into, used to make the
/// incompatibility error actionable rather than just a refusal.
fn working_container_for(codec: VideoCodec) -> RecordingContainer {
    match codec {
        VideoCodec::H264 => RecordingContainer::Mp4,
        VideoCodec::H265 => RecordingContainer::Mp4,
        VideoCodec::VP9 => RecordingContainer::Mkv,
    }
}

/// A typed configuration error for the recording entry point.
///
/// Every variant names the field at fault ([`RecordingConfigError::field`])
/// and carries a message that states the fix, so an embedding application can
/// both log it and surface it next to the offending control.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordingConfigError {
    /// `width` was 0.
    WidthZero,
    /// `width` was odd; H.264/H.265 encode in even-sized macroblocks.
    WidthOdd {
        /// The rejected width.
        width: u32,
    },
    /// `width` exceeded the supported maximum.
    WidthTooLarge {
        /// The rejected width.
        width: u32,
        /// The largest accepted width.
        max: u32,
    },
    /// `height` was 0.
    HeightZero,
    /// `height` was odd.
    HeightOdd {
        /// The rejected height.
        height: u32,
    },
    /// `height` exceeded the supported maximum.
    HeightTooLarge {
        /// The rejected height.
        height: u32,
        /// The largest accepted height.
        max: u32,
    },
    /// `fps` was 0.
    FpsZero,
    /// `fps` exceeded the supported maximum.
    FpsTooHigh {
        /// The rejected frame rate.
        fps: u32,
        /// The largest accepted frame rate.
        max: u32,
    },
    /// `bitrate_kbps` was 0.
    BitrateZero,
    /// `bitrate_kbps` was below the usable minimum.
    BitrateTooLow {
        /// The rejected bitrate, in kbps.
        kbps: u32,
        /// The smallest accepted bitrate, in kbps.
        min: u32,
    },
    /// `bitrate_kbps` exceeded the supported maximum.
    BitrateTooHigh {
        /// The rejected bitrate, in kbps.
        kbps: u32,
        /// The largest accepted bitrate, in kbps.
        max: u32,
    },
    /// The chosen container cannot carry the chosen codec.
    CodecContainerIncompatible {
        /// The requested codec.
        codec: VideoCodec,
        /// The requested container.
        container: RecordingContainer,
        /// A container that *does* carry the codec, so the error names the fix.
        hint: RecordingContainer,
    },
    /// The output path's extension does not match the configured container.
    PathExtensionMismatch {
        /// The extension found on the path.
        extension: String,
        /// The configured container.
        container: RecordingContainer,
        /// The extension the configured container expects.
        expected: String,
    },
}

impl RecordingConfigError {
    /// The name of the configuration field at fault. Stable and machine
    /// readable — an embedding UI can bind it to a control.
    pub fn field(&self) -> &'static str {
        match self {
            Self::WidthZero | Self::WidthOdd { .. } | Self::WidthTooLarge { .. } => "width",
            Self::HeightZero | Self::HeightOdd { .. } | Self::HeightTooLarge { .. } => "height",
            Self::FpsZero | Self::FpsTooHigh { .. } => "fps",
            Self::BitrateZero | Self::BitrateTooLow { .. } | Self::BitrateTooHigh { .. } => {
                "bitrate_kbps"
            }
            Self::CodecContainerIncompatible { .. } => "container",
            Self::PathExtensionMismatch { .. } => "path",
        }
    }
}

impl std::fmt::Display for RecordingConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WidthZero => write!(f, "width must be greater than 0"),
            Self::WidthOdd { width } => write!(
                f,
                "width must be even (H.264/H.265 encode in even macroblocks), got {width}"
            ),
            Self::WidthTooLarge { width, max } => {
                write!(f, "width must be at most {max}, got {width}")
            }
            Self::HeightZero => write!(f, "height must be greater than 0"),
            Self::HeightOdd { height } => write!(
                f,
                "height must be even (H.264/H.265 encode in even macroblocks), got {height}"
            ),
            Self::HeightTooLarge { height, max } => {
                write!(f, "height must be at most {max}, got {height}")
            }
            Self::FpsZero => write!(f, "fps must be greater than 0"),
            Self::FpsTooHigh { fps, max } => {
                write!(f, "fps must be at most {max}, got {fps}")
            }
            Self::BitrateZero => write!(f, "bitrate_kbps must be greater than 0"),
            Self::BitrateTooLow { kbps, min } => write!(
                f,
                "bitrate_kbps must be at least {min} (below that the picture is unusable), got {kbps}"
            ),
            Self::BitrateTooHigh { kbps, max } => write!(
                f,
                "bitrate_kbps must be at most {max} — if you meant bits per second, divide by 1000 — got {kbps}"
            ),
            Self::CodecContainerIncompatible {
                codec,
                container,
                hint,
            } => write!(
                f,
                "{codec:?} cannot be muxed into {container:?}; use {} for {codec:?}",
                hint.label()
            ),
            Self::PathExtensionMismatch {
                extension,
                container,
                expected,
            } => write!(
                f,
                "path extension \".{extension}\" does not match {container:?}; use \".{expected}\" or remove the extension"
            ),
        }
    }
}

impl std::error::Error for RecordingConfigError {}
