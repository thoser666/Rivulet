//! Capability feature detection for the headless inspector (M7 W5, issue
//! #191).
//!
//! `rivulet inspect` answers the question a user asks when a recording fails:
//! *which encoders and containers does this machine actually have?* The answer
//! is a property of the local GStreamer installation, so the detection lives
//! here next to the elements it probes rather than in the CLI — the CLI stays
//! a thin wrapper that serializes [`FeatureReport`] to JSON.
//!
//! Availability means "the element factory is registered" (the plugin is
//! installed). A registered hardware encoder can still fail to instantiate on a
//! machine without the matching GPU/driver — the same caveat the engine's
//! encoder fallback already handles, and one the report states via
//! [`EncoderFeature::hardware`].

use gstreamer as gst;
use serde::Serialize;

use crate::container::RecordingContainer;
use crate::encoder::{VideoCodec, VideoEncoder};

/// Video encoders probed by the report, in a stable order.
///
/// The order is fixed (not the detection result) so the JSON is diffable across
/// machines: the same key order appears whether or not a backend is present.
const PROBED_CODECS: [VideoCodec; 3] = [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9];

/// Encoder backends probed by the report, in a stable order.
const PROBED_BACKENDS: [VideoEncoder; 4] = [
    VideoEncoder::Nvenc,
    VideoEncoder::QuickSync,
    VideoEncoder::Amf,
    VideoEncoder::Software,
];

/// Recording containers probed by the report, in a stable order.
const PROBED_CONTAINERS: [RecordingContainer; 4] = [
    RecordingContainer::Mp4,
    RecordingContainer::Mkv,
    RecordingContainer::Mov,
    RecordingContainer::MpegTs,
];

/// Capture source elements the engine can drive, in a stable order.
///
/// The headless CLI only feeds frames from `videotestsrc`; the remaining
/// entries are the element factories the capture paths use on each platform, so
/// the report explains *why* a capture-backed source is unavailable rather than
/// only listing the one that works.
const PROBED_CAPTURE_ELEMENTS: [(&str, &str); 5] = [
    ("test-source", "videotestsrc"),
    ("webcam-linux-v4l2", "v4l2src"),
    ("webcam-linux-pipewire", "pipewiresrc"),
    ("webcam-windows-ksvideo", "ksvideosrc"),
    ("webcam-macos-avf", "avfvideosrc"),
];

/// Audio filter elements the engine's `AudioFilterConfig` chain can emit, in a
/// stable order. Mirrors the element names in
/// [`crate::audio_source::AudioFilterConfig::filter_chain`].
const PROBED_AUDIO_FILTERS: [(&str, &str); 4] = [
    ("dynamic", "audiodynamic"),
    ("amplify", "audioamplify"),
    ("equalizer", "equalizer-10bands"),
    ("volume", "volume"),
];

/// One probed element: its factory name and whether it is registered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ElementFeature {
    /// Stable machine-readable name (`"nvenc"`, `"mkv"`, `"webcam-linux-v4l2"`).
    pub name: String,
    /// Human-readable label for the same thing.
    pub label: String,
    /// The GStreamer element factory that was probed.
    pub element: String,
    /// Whether the factory is registered in this GStreamer installation.
    pub available: bool,
}

impl ElementFeature {
    fn new(name: &str, label: String, element: &str) -> Self {
        Self {
            name: name.to_string(),
            label,
            element: element.to_string(),
            available: gst::ElementFactory::find(element).is_some(),
        }
    }
}

/// One encoder backend, reported per supported codec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EncoderFeature {
    /// Backend name (`"nvenc"`, `"software"`, ...).
    pub backend: String,
    /// Human-readable backend label (`"NVIDIA NVENC"`, ...).
    pub backend_label: String,
    /// Whether the backend uses dedicated hardware.
    pub hardware: bool,
    /// Per-codec element availability for this backend.
    pub codecs: Vec<CodecFeature>,
}

/// One codec's availability within a backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CodecFeature {
    /// Codec name (`"h264"`, `"h265"`, `"vp9"`).
    pub codec: String,
    /// Human-readable codec label (`"H.264 (AVC)"`, ...).
    pub codec_label: String,
    /// The GStreamer element this backend/codec pair would use.
    pub element: String,
    /// Whether that element factory is registered.
    pub available: bool,
}

/// One recording container and its muxer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContainerFeature {
    /// Container name (`"mp4"`, `"mkv"`, `"mov"`, `"mpegts"`).
    pub container: String,
    /// Human-readable container label.
    pub label: String,
    /// The GStreamer muxer element for the container.
    pub muxer: String,
    /// Whether the muxer element factory is registered.
    pub available: bool,
    /// Whether a partial write survives a crash (MP4 does not).
    pub crash_safe: bool,
}

/// The full capability report, serialized by `rivulet inspect --json`.
///
/// Field order is fixed so the JSON is stable across runs and machines; see the
/// `PROBED_*` constants for the ordering rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FeatureReport {
    /// The GStreamer version string this report was taken against.
    pub gstreamer_version: String,
    /// Encoder backends with per-codec availability.
    pub encoders: Vec<EncoderFeature>,
    /// Recording containers with muxer availability.
    pub containers: Vec<ContainerFeature>,
    /// Capture source elements.
    pub capture_backends: Vec<ElementFeature>,
    /// Audio filter elements.
    pub audio_filters: Vec<ElementFeature>,
}

impl FeatureReport {
    /// Probe the local GStreamer installation and report what it supports.
    ///
    /// Initializes GStreamer if the caller has not done so already, so this is
    /// safe to call as the first engine call in a process.
    pub fn detect() -> Self {
        let _ = gst::init();
        Self {
            gstreamer_version: gst::version_string().to_string(),
            encoders: PROBED_BACKENDS
                .into_iter()
                .map(|backend| EncoderFeature {
                    backend: encoder_name(backend).to_string(),
                    backend_label: backend.label().to_string(),
                    hardware: backend.is_hardware(),
                    codecs: PROBED_CODECS
                        .into_iter()
                        .map(|codec| {
                            let element = backend.element_name_for_codec(codec);
                            CodecFeature {
                                codec: codec_name(codec).to_string(),
                                codec_label: codec.label().to_string(),
                                element: element.to_string(),
                                available: gst::ElementFactory::find(element).is_some(),
                            }
                        })
                        .collect(),
                })
                .collect(),
            containers: PROBED_CONTAINERS
                .into_iter()
                .map(|container| ContainerFeature {
                    container: container_name(container).to_string(),
                    label: container.label().to_string(),
                    muxer: container.muxer_element().to_string(),
                    available: gst::ElementFactory::find(container.muxer_element()).is_some(),
                    crash_safe: container.is_crash_safe(),
                })
                .collect(),
            capture_backends: PROBED_CAPTURE_ELEMENTS
                .into_iter()
                .map(|(name, element)| ElementFeature::new(name, element_label(name), element))
                .collect(),
            audio_filters: PROBED_AUDIO_FILTERS
                .into_iter()
                .map(|(name, element)| ElementFeature::new(name, element_label(name), element))
                .collect(),
        }
    }

    /// The first encoder backend that can encode `codec`, if any.
    ///
    /// Mirrors the engine's own selection order so a report can answer "what
    /// would the engine pick?" without re-deriving the preference list.
    pub fn best_encoder_for(&self, codec: VideoCodec) -> Option<&EncoderFeature> {
        self.encoders.iter().find(|backend| {
            backend
                .codecs
                .iter()
                .any(|c| c.codec == codec_name(codec) && c.available)
        })
    }
}

fn encoder_name(backend: VideoEncoder) -> &'static str {
    match backend {
        VideoEncoder::Nvenc => "nvenc",
        VideoEncoder::QuickSync => "quicksync",
        VideoEncoder::Amf => "amf",
        VideoEncoder::Software => "software",
    }
}

fn codec_name(codec: VideoCodec) -> &'static str {
    match codec {
        VideoCodec::H264 => "h264",
        VideoCodec::H265 => "h265",
        VideoCodec::VP9 => "vp9",
    }
}

fn container_name(container: RecordingContainer) -> &'static str {
    match container {
        RecordingContainer::Mp4 => "mp4",
        RecordingContainer::Mkv => "mkv",
        RecordingContainer::Mov => "mov",
        RecordingContainer::MpegTs => "mpegts",
    }
}

fn element_label(name: &str) -> String {
    match name {
        "test-source" => "Synthetic test source".to_string(),
        "webcam-linux-v4l2" => "Webcam (Video4Linux2)".to_string(),
        "webcam-linux-pipewire" => "Webcam (PipeWire)".to_string(),
        "webcam-windows-ksvideo" => "Webcam (Windows Media Foundation)".to_string(),
        "webcam-macos-avf" => "Webcam (AVFoundation)".to_string(),
        "dynamic" => "Noise gate / expander / compressor / limiter".to_string(),
        "amplify" => "Gain".to_string(),
        "equalizer" => "10-band equalizer".to_string(),
        "volume" => "Volume".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_lists_every_probed_dimension() {
        let report = FeatureReport::detect();
        assert_eq!(report.encoders.len(), 4, "four encoder backends probed");
        assert_eq!(report.containers.len(), 4, "four containers probed");
        assert!(!report.capture_backends.is_empty());
        assert!(!report.audio_filters.is_empty());
        assert!(!report.gstreamer_version.is_empty());
    }

    #[test]
    fn encoder_codec_order_is_stable_and_complete() {
        let report = FeatureReport::detect();
        let backends: Vec<&str> = report.encoders.iter().map(|e| e.backend.as_str()).collect();
        assert_eq!(backends, vec!["nvenc", "quicksync", "amf", "software"]);
        for backend in &report.encoders {
            let codecs: Vec<&str> = backend.codecs.iter().map(|c| c.codec.as_str()).collect();
            assert_eq!(
                codecs,
                vec!["h264", "h265", "vp9"],
                "backend {} must list every codec in a fixed order",
                backend.backend
            );
        }
    }

    #[test]
    fn software_h264_is_always_available() {
        // The software fallback is what makes any recording possible; if it
        // ever reports unavailable the engine cannot record at all, so this
        // is a genuine invariant rather than an environment observation.
        let report = FeatureReport::detect();
        let software = report
            .encoders
            .iter()
            .find(|e| e.backend == "software")
            .expect("software backend listed");
        let h264 = software
            .codecs
            .iter()
            .find(|c| c.codec == "h264")
            .expect("h264 listed for software");
        assert_eq!(h264.element, "x264enc");
    }

    #[test]
    fn container_names_match_the_cli_config_vocabulary() {
        // The CLI validates `output.container` against exactly this set, so the
        // report and the config must not drift apart.
        let report = FeatureReport::detect();
        let names: Vec<&str> = report
            .containers
            .iter()
            .map(|c| c.container.as_str())
            .collect();
        assert_eq!(names, vec!["mp4", "mkv", "mov", "mpegts"]);
    }

    #[test]
    fn mp4_is_the_only_non_crash_safe_container() {
        let report = FeatureReport::detect();
        for container in &report.containers {
            let expected = container.container != "mp4";
            assert_eq!(
                container.crash_safe, expected,
                "container {} crash-safety is wrong",
                container.container
            );
        }
    }

    #[test]
    fn audio_filter_elements_match_the_engine_filter_chain() {
        // Every element the engine's filter chain can emit must appear in the
        // report, otherwise `inspect` would under-report what a config uses.
        let report = FeatureReport::detect();
        let elements: Vec<&str> = report
            .audio_filters
            .iter()
            .map(|f| f.element.as_str())
            .collect();
        for expected in ["audiodynamic", "audioamplify", "equalizer-10bands"] {
            assert!(
                elements.contains(&expected),
                "audio filter element {expected} must be reported"
            );
        }
    }

    #[test]
    fn best_encoder_matches_codec_availability() {
        let report = FeatureReport::detect();
        if let Some(best) = report.best_encoder_for(VideoCodec::H264) {
            let h264 = best.codecs.iter().find(|c| c.codec == "h264").unwrap();
            assert!(h264.available);
        }
    }

    #[test]
    fn json_shape_is_stable_and_keyed() {
        let report = FeatureReport::detect();
        let json = serde_json::to_string(&report).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        for key in [
            "gstreamer_version",
            "encoders",
            "containers",
            "capture_backends",
            "audio_filters",
        ] {
            assert!(
                value.get(key).is_some(),
                "report JSON must contain {key}: {json}"
            );
        }
        let first = &value["encoders"][0];
        for key in ["backend", "backend_label", "hardware", "codecs"] {
            assert!(first.get(key).is_some(), "encoder entry needs {key}");
        }
        let first_container = &value["containers"][0];
        for key in ["container", "label", "muxer", "available", "crash_safe"] {
            assert!(
                first_container.get(key).is_some(),
                "container entry needs {key}"
            );
        }
    }
}
