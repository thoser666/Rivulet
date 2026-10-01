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

use crate::clock::ClockMode;
use crate::container::RecordingContainer;
use crate::encoder::{VideoCodec, VideoEncoder};
use crate::source::SourceKind;

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

/// One source of run-to-run nondeterminism, and whether a run actually hit it.
///
/// The variant list mirrors the table in `docs/m7-automation.md`
/// § Nondeterminism inventory one-to-one, so a run report can be compared
/// against the documented contract without a mapping table in code. Every
/// variant is reported on *every* run — the `active` flag says whether this
/// particular run touched it, so a consumer can tell "documented limit" from
/// "limit that applied to you".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NondeterminismSource {
    /// PTS and session duration follow the injected clock. Deterministic when
    /// the virtual clock drives the run.
    EngineClock,
    /// Encoder rate-control state (lookahead/VBV). Deterministic for identical
    /// input frames, but the first frames depend on the encoder settings.
    EncoderRateControl,
    /// A hardware encoder (NVENC/QSV/AMF) is in use. Not bit-deterministic
    /// across runs or driver versions.
    HardwareEncoder,
    /// Wall-clock-derived container metadata (creation time). Nondeterministic
    /// by nature.
    WallClockMetadata,
    /// A live capture source (screen/camera/mic). Live input cannot repeat.
    CaptureSource,
    /// Element-internal threads (for example queue leaks) may reorder
    /// independent branches even under a virtual clock.
    ElementThreading,
}

impl NondeterminismSource {
    /// Stable machine-readable name used in the run report.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EngineClock => "engine_clock",
            Self::EncoderRateControl => "encoder_rate_control",
            Self::HardwareEncoder => "hardware_encoder",
            Self::WallClockMetadata => "wall_clock_metadata",
            Self::CaptureSource => "capture_source",
            Self::ElementThreading => "element_threading",
        }
    }

    /// Every source in fixed order, so reports are diffable across runs.
    pub const ALL: [NondeterminismSource; 6] = [
        Self::EngineClock,
        Self::EncoderRateControl,
        Self::HardwareEncoder,
        Self::WallClockMetadata,
        Self::CaptureSource,
        Self::ElementThreading,
    ];

    /// Whether this source can make the container *timestamp sequence*
    /// differ between two otherwise identical runs.
    ///
    /// This is the contract the reproducible-run test asserts (spec § W2a:
    /// "identical inputs + virtual clock → identical container timestamps").
    pub fn affects_timestamps(self) -> bool {
        matches!(self, Self::EngineClock | Self::ElementThreading)
    }

    /// Whether this source is a *risk* to the timestamp sequence rather than a
    /// guaranteed divergence.
    ///
    /// Wall-clock PTS always diverge, and element-internal threads *may*
    /// reorder independent branches. The spec lists the latter as "validated by
    /// the reproducibility test", so the report has to keep the two apart: a
    /// validated risk still gets reported, but it does not by itself mean the
    /// run is outside the contract.
    pub fn is_timestamp_risk(self) -> bool {
        matches!(self, Self::ElementThreading)
    }

    /// Whether this source can make the encoded *bytes* differ between two
    /// otherwise identical runs, leaving the timeline intact.
    ///
    /// `capture_source` counts: live input is the reason a desktop run cannot
    /// be byte-compared, and excluding it would let a run with a webcam layer
    /// report `is_byte_reproducible() == true`.
    pub fn affects_bytes(self) -> bool {
        matches!(
            self,
            Self::EncoderRateControl
                | Self::HardwareEncoder
                | Self::WallClockMetadata
                | Self::CaptureSource
        )
    }
}

/// One row of the run report's nondeterminism inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NondeterminismEntry {
    /// Stable machine-readable source name.
    pub source: String,
    /// Human-readable explanation of the behavior.
    pub detail: String,
    /// Whether this run actually used the source.
    pub active: bool,
    /// Whether the source can perturb the container timestamp sequence.
    pub affects_timestamps: bool,
    /// Whether the source is only a *risk* to the timestamps, i.e. it needs the
    /// reproducibility test to establish that the sequence actually repeats.
    pub timestamp_risk: bool,
    /// Whether the source can perturb the encoded bytes only.
    pub affects_bytes: bool,
}

/// Which nondeterminism sources a recording run actually used.
///
/// This is the machine-readable half of the reproducible-run contract
/// (spec § W2a / § Nondeterminism inventory): rather than promising
/// reproducibility and staying silent when a run falls outside it, the run
/// report states which limits applied. A fully deterministic headless run
/// (virtual clock, synthetic source, software encoder) reports no active
/// limit; a desktop run using NVENC from a window capture reports those three.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NondeterminismReport {
    /// All sources in fixed order, with `active` set per run.
    pub sources: Vec<NondeterminismEntry>,
}

impl NondeterminismReport {
    /// Evaluate the inventory for one run.
    ///
    /// `clock` decides whether PTS are reproducible, `encoder` whether a
    /// hardware path is in use, and `source` whether the input is live. Audio
    /// PTS are derived from sample counts at the fixed engine rate, so they
    /// are clock-independent by construction and carry no limit of their own.
    pub fn for_run(clock: ClockMode, encoder: VideoEncoder, source: SourceKind) -> Self {
        let virtual_clock = clock == ClockMode::Virtual;
        // Wall-clock container metadata is written from the system clock
        // regardless of how PTS are produced, so it stays a limit under the
        // virtual clock too — that is exactly the kind of surprise this report
        // exists to surface.
        let entries = [
            (
                NondeterminismSource::EngineClock,
                format!("PTS/session duration from the {} clock", clock.as_str()),
                !virtual_clock,
            ),
            (
                NondeterminismSource::EncoderRateControl,
                format!(
                    "encoder {} rate control; first-frame effects depend on settings",
                    encoder.label()
                ),
                true,
            ),
            (
                NondeterminismSource::HardwareEncoder,
                format!(
                    "the {} encoder is not bit-deterministic across \
                     runs/drivers (this run: {})",
                    encoder.label(),
                    if encoder.is_hardware() {
                        "hardware, so the limit applies"
                    } else {
                        "software, so this limit does not apply"
                    }
                ),
                encoder.is_hardware(),
            ),
            (
                NondeterminismSource::WallClockMetadata,
                "container creation-time metadata comes from the system clock".to_string(),
                true,
            ),
            (
                NondeterminismSource::CaptureSource,
                format!("{} is live input and cannot repeat", source.label()),
                source.is_live(),
            ),
            (
                NondeterminismSource::ElementThreading,
                "element-internal threads (e.g. queue leaks) may reorder branches".to_string(),
                true,
            ),
        ];

        Self {
            sources: entries
                .into_iter()
                .map(|(source, detail, active)| NondeterminismEntry {
                    source: source.as_str().to_string(),
                    detail,
                    active,
                    affects_timestamps: source.affects_timestamps(),
                    timestamp_risk: source.is_timestamp_risk(),
                    affects_bytes: source.affects_bytes(),
                })
                .collect(),
        }
    }

    /// The sources this run actually used, in fixed order.
    pub fn active_sources(&self) -> Vec<&NondeterminismEntry> {
        self.sources.iter().filter(|entry| entry.active).collect()
    }

    /// Whether the container *timestamp sequence* is reproducible, i.e. the
    /// run falls inside the documented reproducible-run contract.
    ///
    /// A source that is an unconditional divergence (wall-clock PTS) rules the
    /// run out. A source that is only a *risk* — element-internal threads
    /// reordering independent branches — is reported but does not decide this:
    /// establishing that the sequence actually repeats is the job of the
    /// reproducibility integration test, not of this static analysis.
    ///
    /// Byte-identity is a stronger, separate claim: the encoded payload also
    /// depends on encoder settings, a possible hardware encoder, and
    /// wall-clock container metadata. Use [`Self::is_byte_reproducible`] for
    /// that; the distinction is why the report carries both flags.
    pub fn is_reproducible(&self) -> bool {
        !self
            .sources
            .iter()
            .any(|entry| entry.active && entry.affects_timestamps && !entry.timestamp_risk)
    }

    /// The timestamp risks that still need empirical validation for this run.
    ///
    /// Non-empty means the reproducibility test has not yet proven the
    /// timestamp sequence repeats for this configuration.
    pub fn open_timestamp_risks(&self) -> Vec<&NondeterminismEntry> {
        self.sources
            .iter()
            .filter(|entry| entry.active && entry.timestamp_risk)
            .collect()
    }

    /// Whether the encoded bytes are expected to be identical run to run.
    ///
    /// False for every hardware-encoder run and for runs that carry
    /// wall-clock container metadata, which is why the headless CI path
    /// asserts timestamp reproducibility rather than byte equality.
    pub fn is_byte_reproducible(&self) -> bool {
        !self
            .sources
            .iter()
            .any(|entry| entry.active && entry.affects_bytes)
    }

    /// Whether `source` was active (convenience lookup for callers/tests).
    pub fn is_active(&self, source: NondeterminismSource) -> bool {
        self.sources
            .iter()
            .any(|entry| entry.active && entry.source == source.as_str())
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

    // --- W2a: nondeterminism inventory (issue #187) ---

    #[test]
    fn nondeterminism_report_covers_every_source_in_a_fixed_order() {
        let report = NondeterminismReport::for_run(
            ClockMode::Virtual,
            VideoEncoder::Software,
            SourceKind::Color,
        );
        let names: Vec<&str> = report.sources.iter().map(|e| e.source.as_str()).collect();
        let expected: Vec<&str> = NondeterminismSource::ALL
            .iter()
            .map(|source| source.as_str())
            .collect();
        assert_eq!(
            names, expected,
            "every documented source must appear, in the same order, on every run"
        );
    }

    #[test]
    fn virtual_clock_run_reports_the_clock_as_deterministic() {
        let report = NondeterminismReport::for_run(
            ClockMode::Virtual,
            VideoEncoder::Software,
            SourceKind::Color,
        );
        assert!(
            !report.is_active(NondeterminismSource::EngineClock),
            "a virtual clock must not be reported as an active nondeterminism source"
        );
        assert!(report.is_active(NondeterminismSource::EncoderRateControl));
    }

    #[test]
    fn system_clock_run_reports_the_clock_as_active() {
        let report = NondeterminismReport::for_run(
            ClockMode::System,
            VideoEncoder::Software,
            SourceKind::Color,
        );
        assert!(report.is_active(NondeterminismSource::EngineClock));
    }

    #[test]
    fn the_software_encoder_detail_does_not_claim_the_hardware_limit() {
        // The detail string interpolates the encoder, so a software run used to
        // read "Software encoder is not bit-deterministic" - the one limit that
        // explicitly does not apply to it.
        let sw = NondeterminismReport::for_run(
            ClockMode::Virtual,
            VideoEncoder::Software,
            SourceKind::Color,
        );
        let detail = &sw
            .sources
            .iter()
            .find(|entry| entry.source == "hardware_encoder")
            .expect("entry")
            .detail;
        assert!(
            detail.contains("does not apply"),
            "a software run must not be told it has a hardware limit: {detail}"
        );
        assert!(
            !sw.is_active(NondeterminismSource::HardwareEncoder),
            "and the entry must stay inactive for software encoding"
        );
    }

    #[test]
    fn live_capture_is_reported_as_a_byte_limit() {
        // A webcam/screen layer is the reason a desktop run cannot be compared
        // byte for byte, so it must not be reported as timeline-only.
        let live = NondeterminismReport::for_run(
            ClockMode::Virtual,
            VideoEncoder::Software,
            SourceKind::ScreenCapture,
        );
        assert!(NondeterminismSource::CaptureSource.affects_bytes());
        let entry = live
            .sources
            .iter()
            .find(|entry| entry.source == "capture_source")
            .expect("entry");
        assert!(entry.affects_bytes, "{entry:?}");
        assert!(
            entry.active,
            "a screen-capture source must be active: {entry:?}"
        );
        assert!(
            !live.is_byte_reproducible(),
            "a live source rules byte-reproducibility out"
        );
        // It is not a *timestamp* divergence: the virtual clock still holds.
        assert!(live.is_reproducible());
    }

    #[test]
    fn hardware_encoder_is_reported_as_a_byte_limit() {
        let hw = NondeterminismReport::for_run(
            ClockMode::Virtual,
            VideoEncoder::Nvenc,
            SourceKind::Color,
        );
        assert!(hw.is_active(NondeterminismSource::HardwareEncoder));
        assert!(
            !hw.is_byte_reproducible(),
            "a hardware encoder means the bytes are not identical run to run"
        );
        assert!(
            hw.is_reproducible(),
            "the timestamp contract does not depend on the encoder: {}",
            serde_json::to_string(&hw).unwrap()
        );

        let sw = NondeterminismReport::for_run(
            ClockMode::Virtual,
            VideoEncoder::Software,
            SourceKind::Color,
        );
        assert!(!sw.is_active(NondeterminismSource::HardwareEncoder));
    }

    #[test]
    fn live_capture_source_is_reported_as_an_active_limit() {
        for live in [
            SourceKind::Webcam,
            SourceKind::ScreenCapture,
            SourceKind::GameCapture,
            SourceKind::Audio,
        ] {
            assert!(
                live.is_live(),
                "{} must be classified as live",
                live.label()
            );
            let report = NondeterminismReport::for_run(
                ClockMode::Virtual,
                VideoEncoder::Software,
                live.clone(),
            );
            assert!(
                report.is_active(NondeterminismSource::CaptureSource),
                "{} is live input and must be reported",
                live.label()
            );
        }

        for recorded in [
            SourceKind::Image,
            SourceKind::Text,
            SourceKind::Browser,
            SourceKind::Media,
            SourceKind::Color,
        ] {
            assert!(
                !recorded.is_live(),
                "{} is not live input",
                recorded.label()
            );
        }
    }

    #[test]
    fn wall_clock_metadata_survives_the_virtual_clock() {
        // Creation-time metadata comes from the system clock even when PTS are
        // scripted. It perturbs the bytes, not the timeline, and the report has
        // to distinguish those two instead of claiming full reproducibility.
        let report = NondeterminismReport::for_run(
            ClockMode::Virtual,
            VideoEncoder::Software,
            SourceKind::Color,
        );
        assert!(report.is_active(NondeterminismSource::WallClockMetadata));
        assert!(
            !report.is_byte_reproducible(),
            "wall-clock metadata keeps the bytes from being identical"
        );
        assert!(
            report.is_reproducible(),
            "but the timestamp contract still holds"
        );
    }

    #[test]
    fn a_virtual_clock_run_holds_the_timestamp_contract_and_lists_its_limits() {
        let headless = NondeterminismReport::for_run(
            ClockMode::Virtual,
            VideoEncoder::Software,
            SourceKind::Color,
        );
        assert!(
            headless.is_reproducible(),
            "no active source perturbs timestamps: {:?}",
            headless.active_sources()
        );
        assert!(
            !headless.active_sources().is_empty(),
            "the headless run still documents its active sources"
        );
    }

    #[test]
    fn a_wall_clock_run_is_outside_the_timestamp_contract() {
        let desktop = NondeterminismReport::for_run(
            ClockMode::System,
            VideoEncoder::Nvenc,
            SourceKind::ScreenCapture,
        );
        assert!(!desktop.is_reproducible(), "wall-clock PTS cannot repeat");
        assert!(desktop.is_active(NondeterminismSource::EngineClock));
        assert!(desktop.is_active(NondeterminismSource::HardwareEncoder));
        assert!(desktop.is_active(NondeterminismSource::CaptureSource));
    }

    #[test]
    fn nondeterminism_source_names_are_stable() {
        // Machine-readable surface: renaming a source breaks run-report
        // consumers, so the strings are pinned here.
        let names: Vec<&str> = NondeterminismSource::ALL
            .iter()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(
            names,
            vec![
                "engine_clock",
                "encoder_rate_control",
                "hardware_encoder",
                "wall_clock_metadata",
                "capture_source",
                "element_threading",
            ]
        );
    }

    #[test]
    fn nondeterminism_json_shape_is_stable() {
        let report = NondeterminismReport::for_run(
            ClockMode::System,
            VideoEncoder::Software,
            SourceKind::Color,
        );
        let json = serde_json::to_string(&report).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let sources = value["sources"].as_array().expect("sources array");
        assert_eq!(sources.len(), NondeterminismSource::ALL.len());
        for key in [
            "source",
            "detail",
            "active",
            "affects_timestamps",
            "affects_bytes",
        ] {
            assert!(sources[0].get(key).is_some(), "entry needs {key}: {json}");
        }
    }
}
