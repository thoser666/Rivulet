//! Video encoder abstraction for Rivulet.
//!
//! Rivulet supports hardware-accelerated H.264/H.265 encoding (NVIDIA NVENC,
//! Intel QuickSync, AMD AMF) and VP9 software encoding, with automatic
//! detection of the best available encoder and a software fallback. Encoders
//! are expressed as GStreamer `parse_launch` fragments so the engine can swap
//! the video branch without changing the rest of the pipeline.

use gstreamer as gst;
use serde::{Deserialize, Serialize};

/// Encoder rate-control strategy. Advanced rate control lets users switch
/// between constant bitrate (default, required for live streaming), variable
/// bitrate (better quality for local recordings) and quality-based mode where
/// the encoder targets a fixed quality rather than a bitrate.
///
/// Only the backends that expose clean properties support every mode
/// (software x264, NVENC); the remaining backends (QuickSync, AMF, VP9,
/// software x265) fall back to an average bitrate so a nonzero target is never
/// lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum RateControlMode {
    /// Constant bit rate — the encoder sticks to the target average bitrate.
    /// This is the only mode suitable for live streaming.
    #[default]
    Cbr,
    /// Variable bit rate — allows higher peaks within the configured cap.
    Vbr,
    /// Constant quality (QP/CRF) — fixed quality, file size varies with content.
    Cq,
    /// Constant-quality VBR — quality-driven encoding capped by a max bitrate.
    CqVbr,
}

impl RateControlMode {
    /// Human-readable label for display in the UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::Cbr => "CBR (constant bitrate)",
            Self::Vbr => "VBR (variable bitrate)",
            Self::Cq => "CQ (constant quality)",
            Self::CqVbr => "CQ-VBR (quality + cap)",
        }
    }

    /// Long-form description used as tooltip/hint text.
    pub fn hint(self) -> &'static str {
        match self {
            Self::Cbr => "Predictable size; required for live streaming",
            Self::Vbr => "Better quality per size; use for local recordings",
            Self::Cq => "Fixed quality regardless of size",
            Self::CqVbr => "Target quality but never exceed the max bitrate",
        }
    }
}

/// Advanced rate control configuration applied on top of a target bitrate.
///
/// The target average bitrate is supplied separately (it is also adjusted live
/// by the adaptive-bitrate controller); this struct carries the *mode* plus the
/// quality/cap/options that are backend-specific.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateControl {
    /// The rate-control strategy.
    pub mode: RateControlMode,
    /// Upper bound for VBR/CQ-VBR in kbit/s. `0` means "backend default".
    pub max_bitrate_kbps: u32,
    /// Quality level backend-dependent. For x264/NVENC this is 0..=51 where a
    /// lower value is higher quality (QP / constant-quality index).
    pub quality: i32,
    /// Free-form extra properties appended verbatim to the encoder fragment
    /// (e.g. `key-int-max=250 bframes=3`). Empty by default.
    pub custom_options: String,
}

impl Default for RateControl {
    fn default() -> Self {
        Self {
            mode: RateControlMode::Cbr,
            max_bitrate_kbps: 0,
            quality: 23,
            custom_options: String::new(),
        }
    }
}

/// Supported video codecs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum VideoCodec {
    /// H.264 / AVC — universal compatibility, required for RTMP/FLV streaming.
    #[default]
    H264,
    /// H.265 / HEVC — better compression, supported by MP4 muxer.
    H265,
    /// VP9 — open royalty-free codec, requires WebM muxer.
    VP9,
}

impl VideoCodec {
    /// Human-readable label for display in UI and logs.
    pub fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264 (AVC)",
            Self::H265 => "H.265 (HEVC)",
            Self::VP9 => "VP9",
        }
    }

    /// GStreamer muxer element for this codec's native container format.
    pub fn muxer_element(self) -> &'static str {
        match self {
            Self::H264 | Self::H265 => "mp4mux",
            Self::VP9 => "webmmux",
        }
    }

    /// Default file extension for recordings using this codec.
    pub fn file_extension(self) -> &'static str {
        match self {
            Self::H264 | Self::H265 => "mp4",
            Self::VP9 => "webm",
        }
    }

    /// Whether this codec is compatible with RTMP/FLV streaming.
    ///
    /// RTMP only supports H.264 video; H.265 and VP9 cannot be streamed via
    /// RTMP/RTMPS.
    pub fn is_rtmp_compatible(self) -> bool {
        matches!(self, Self::H264)
    }
}

/// Supported video encoder backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoder {
    /// NVIDIA NVENC (gst-plugins-bad, NVCODEC).
    Nvenc,
    /// Intel QuickSync (gst-plugins-bad, oneVPL).
    QuickSync,
    /// AMD AMF (gst-plugins-bad).
    Amf,
    /// Software encoding (x264/x265/vp9enc).
    Software,
}

impl VideoEncoder {
    /// The GStreamer element factory name for this backend and codec.
    pub fn element_name_for_codec(self, codec: VideoCodec) -> &'static str {
        match (self, codec) {
            (Self::Nvenc, VideoCodec::H264) => "nvh264enc",
            (Self::Nvenc, VideoCodec::H265) => "nvh265enc",
            (Self::Nvenc, VideoCodec::VP9) => "vp9enc",
            (Self::QuickSync, VideoCodec::H264) => "qsvh264enc",
            (Self::QuickSync, VideoCodec::H265) => "qsvh265enc",
            (Self::QuickSync, VideoCodec::VP9) => "vp9enc",
            (Self::Amf, VideoCodec::H264) => "amfh264enc",
            (Self::Amf, VideoCodec::H265) => "amfh265enc",
            (Self::Amf, VideoCodec::VP9) => "vp9enc",
            (Self::Software, VideoCodec::H264) => "x264enc",
            (Self::Software, VideoCodec::H265) => "x265enc",
            (Self::Software, VideoCodec::VP9) => "vp9enc",
        }
    }

    /// The GStreamer element factory name (backward-compatible, H.264 only).
    pub fn element_name(self) -> &'static str {
        self.element_name_for_codec(VideoCodec::H264)
    }

    /// Human-readable label for display in UI and logs.
    pub fn label(self) -> &'static str {
        match self {
            Self::Nvenc => "NVIDIA NVENC",
            Self::QuickSync => "Intel QuickSync",
            Self::Amf => "AMD AMF",
            Self::Software => "Software",
        }
    }

    /// Combined label including the codec name, e.g. "Software (H.265)".
    pub fn label_with_codec(self, codec: VideoCodec) -> String {
        format!("{} ({})", self.label(), codec.label())
    }

    /// Whether this encoder uses dedicated hardware acceleration.
    pub fn is_hardware(self) -> bool {
        !matches!(self, Self::Software)
    }

    /// Whether the element factory is present in the current GStreamer
    /// installation for the given codec.
    ///
    /// This only checks availability of the plugin; actually creating the
    /// element can still fail on machines without the matching GPU/driver.
    pub fn is_available_for_codec(self, codec: VideoCodec) -> bool {
        gst::ElementFactory::find(self.element_name_for_codec(codec)).is_some()
    }

    /// Whether the element factory is present (backward-compatible, H.264).
    pub fn is_available(self) -> bool {
        self.is_available_for_codec(VideoCodec::H264)
    }

    /// `parse_launch` caps fragment for the given codec.
    ///
    /// Forces a chroma-subsampled input format (I420 for software, NV12 for
    /// hardware) to prevent the encoder from producing 4:4:4 profiles that
    /// muxers cannot store.
    pub fn input_caps_fragment_for_codec(self, codec: VideoCodec) -> &'static str {
        match (self, codec) {
            (Self::Software, _) => "video/x-raw,format=I420",
            (_, VideoCodec::H264) | (_, VideoCodec::H265) => "video/x-raw,format=NV12",
            (_, VideoCodec::VP9) => "video/x-raw,format=I420",
        }
    }

    /// `parse_launch` caps fragment (backward-compatible, H.264).
    pub fn input_caps_fragment(self) -> &'static str {
        self.input_caps_fragment_for_codec(VideoCodec::H264)
    }

    /// Build the `parse_launch` fragment for this encoder with the given codec
    /// and target bitrate in kbit/s using [`RateControl::default`] (CBR).
    ///
    /// The element is always named `video_enc` so the engine can attach
    /// performance probes to it.
    pub fn branch_fragment_for_codec(self, codec: VideoCodec, bitrate_kbps: u32) -> String {
        self.branch_fragment_for_codec_rc(codec, bitrate_kbps, RateControl::default())
    }

    /// Build the `parse_launch` fragment with the given codec, target bitrate,
    /// and advanced [`RateControl`] settings.
    pub fn branch_fragment_for_codec_rc(
        self,
        codec: VideoCodec,
        bitrate_kbps: u32,
        rc: RateControl,
    ) -> String {
        let element = self.element_name_for_codec(codec);
        let custom = rc.custom_options.trim().to_string();
        let mut props = self.rate_control_props_for_codec(codec, bitrate_kbps, rc);
        match (self, codec) {
            (Self::Software, VideoCodec::H264) => props.push("tune=zerolatency".into()),
            (Self::Software, VideoCodec::H265) => props.push("tune=zerolatency".into()),
            (Self::Software, VideoCodec::VP9) => props.push("deadline=realtime".into()),
            (Self::Nvenc, _) => props.push("zerolatency=true".into()),
            (Self::QuickSync | Self::Amf, _) => {}
        }
        if !custom.is_empty() {
            props.push(custom);
        }
        let mut out = format!("{} name=video_enc", element);
        for p in props {
            out.push(' ');
            out.push_str(&p);
        }
        out
    }

    /// Produce the rate-control property fragment for a codec/bitrate/mode.
    ///
    /// Only software x264 and NVENC (H.264/H.265) expose clean rate-control
    /// properties; everything else degrades to an average bitrate so the target
    /// is never silently dropped.
    fn rate_control_props_for_codec(
        self,
        codec: VideoCodec,
        bitrate_kbps: u32,
        rc: RateControl,
    ) -> Vec<String> {
        let cap = |m: u32| {
            if m > 0 {
                format!(" max-bitrate={m}")
            } else {
                String::new()
            }
        };
        match (self, codec, rc.mode) {
            // NVENC exposes rc-mode/max-bitrate/const-quality/qp-const.
            (Self::Nvenc, VideoCodec::H264 | VideoCodec::H265, mode) => match mode {
                RateControlMode::Cbr => vec![format!("rc-mode=cbr bitrate={bitrate_kbps}")],
                RateControlMode::Vbr => vec![format!(
                    "rc-mode=vbr bitrate={bitrate_kbps}{}",
                    cap(rc.max_bitrate_kbps)
                )],
                RateControlMode::Cq => {
                    vec![format!("rc-mode=constqp qp-const={}", rc.quality)]
                }
                RateControlMode::CqVbr => vec![format!(
                    "rc-mode=vbr const-quality={}{}",
                    rc.quality,
                    cap(rc.max_bitrate_kbps)
                )],
            },
            // Software x264 exposes pass/quantizer/vbv-buf-capacity.
            (Self::Software, VideoCodec::H264, _) => match rc.mode {
                RateControlMode::Cbr => vec![format!("bitrate={bitrate_kbps}")],
                RateControlMode::Vbr => vec![format!("bitrate={bitrate_kbps} pass=pass2")],
                RateControlMode::Cq => {
                    vec![format!("pass=quant quantizer={}", rc.quality)]
                }
                RateControlMode::CqVbr => vec![format!(
                    "pass=quant quantizer={} vbv-buf-capacity=2000",
                    rc.quality
                )],
            },
            // Fallback backends: keep an average bitrate regardless of mode.
            _ => vec![format!("bitrate={bitrate_kbps}")],
        }
    }

    /// Build the `parse_launch` fragment (backward-compatible, H.264).
    pub fn branch_fragment(self, bitrate_kbps: u32) -> String {
        self.branch_fragment_for_codec(VideoCodec::H264, bitrate_kbps)
    }
}

/// Detect available encoder backends for the given codec, ordered by
/// preference. Software is always appended last as a guaranteed fallback, so
/// the result is never empty.
pub fn detect_available_encoders_for_codec(codec: VideoCodec) -> Vec<VideoEncoder> {
    let mut encoders = Vec::new();
    for enc in [
        VideoEncoder::Nvenc,
        VideoEncoder::QuickSync,
        VideoEncoder::Amf,
    ] {
        if enc.is_available_for_codec(codec) {
            encoders.push(enc);
        }
    }
    encoders.push(VideoEncoder::Software);
    encoders
}

/// Detect available H.264 encoders (backward-compatible).
pub fn detect_available_encoders() -> Vec<VideoEncoder> {
    detect_available_encoders_for_codec(VideoCodec::H264)
}

/// Return the best available encoder for the given codec.
pub fn best_encoder_for_codec(codec: VideoCodec) -> VideoEncoder {
    detect_available_encoders_for_codec(codec)[0]
}

/// Return the best available H.264 encoder (backward-compatible).
pub fn best_encoder() -> VideoEncoder {
    best_encoder_for_codec(VideoCodec::H264)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_names_are_stable() {
        assert_eq!(VideoEncoder::Nvenc.element_name(), "nvh264enc");
        assert_eq!(VideoEncoder::QuickSync.element_name(), "qsvh264enc");
        assert_eq!(VideoEncoder::Amf.element_name(), "amfh264enc");
        assert_eq!(VideoEncoder::Software.element_name(), "x264enc");
    }

    #[test]
    fn labels_are_distinct() {
        let mut labels: Vec<_> = [
            VideoEncoder::Nvenc,
            VideoEncoder::QuickSync,
            VideoEncoder::Amf,
            VideoEncoder::Software,
        ]
        .iter()
        .map(|e| e.label())
        .collect();
        labels.dedup();
        assert_eq!(labels.len(), 4);
    }

    #[test]
    fn hardware_flags_are_correct() {
        assert!(VideoEncoder::Nvenc.is_hardware());
        assert!(VideoEncoder::QuickSync.is_hardware());
        assert!(VideoEncoder::Amf.is_hardware());
        assert!(!VideoEncoder::Software.is_hardware());
    }

    #[test]
    fn software_encoder_is_always_available() {
        let _ = gst::init();
        assert!(VideoEncoder::Software.is_available());
    }

    #[test]
    fn detect_always_ends_with_software_fallback() {
        let _ = gst::init();
        let encoders = detect_available_encoders();
        assert!(!encoders.is_empty());
        assert_eq!(*encoders.last().unwrap(), VideoEncoder::Software);
    }

    #[test]
    fn detect_returns_hardware_before_software() {
        let _ = gst::init();
        let encoders = detect_available_encoders();
        let hw: Vec<_> = encoders
            .iter()
            .filter(|e| e.is_hardware())
            .copied()
            .collect();
        for (i, enc) in hw.iter().enumerate() {
            assert_eq!(encoders[i], *enc, "hardware encoder position mismatch");
        }
        assert_eq!(best_encoder(), encoders[0]);
    }

    #[test]
    fn fragment_contains_element_and_bitrate() {
        let frag = VideoEncoder::Nvenc.branch_fragment(6000);
        assert!(frag.contains("nvh264enc"), "{frag}");
        assert!(frag.contains("bitrate=6000"), "{frag}");
    }

    #[test]
    fn software_fragment_sets_zerolatency() {
        let frag = VideoEncoder::Software.branch_fragment(5000);
        assert!(frag.contains("x264enc"), "{frag}");
        assert!(frag.contains("tune=zerolatency"), "{frag}");
        assert!(frag.contains("bitrate=5000"), "{frag}");
    }

    #[test]
    fn nvenc_fragment_sets_zerolatency() {
        let frag = VideoEncoder::Nvenc.branch_fragment(5000);
        assert!(frag.contains("zerolatency=true"), "{frag}");
    }

    #[test]
    fn qsv_and_amf_fragments_only_set_bitrate() {
        for enc in [VideoEncoder::QuickSync, VideoEncoder::Amf] {
            let frag = enc.branch_fragment(5000);
            assert_eq!(
                frag,
                format!("{} name=video_enc bitrate=5000", enc.element_name()),
                "{frag}"
            );
        }
    }

    #[test]
    fn fragment_names_the_encoder_element() {
        for enc in [
            VideoEncoder::Nvenc,
            VideoEncoder::QuickSync,
            VideoEncoder::Amf,
            VideoEncoder::Software,
        ] {
            assert!(
                enc.branch_fragment(5000).contains("name=video_enc"),
                "encoder must be nameable for performance probes"
            );
        }
    }

    #[test]
    fn software_input_caps_force_420_subsampling() {
        assert_eq!(
            VideoEncoder::Software.input_caps_fragment(),
            "video/x-raw,format=I420"
        );
    }

    #[test]
    fn hardware_input_caps_use_nv12() {
        for enc in [
            VideoEncoder::Nvenc,
            VideoEncoder::QuickSync,
            VideoEncoder::Amf,
        ] {
            assert_eq!(enc.input_caps_fragment(), "video/x-raw,format=NV12");
        }
    }

    // --- VideoCodec tests ---

    #[test]
    fn codec_labels_are_stable() {
        assert_eq!(VideoCodec::H264.label(), "H.264 (AVC)");
        assert_eq!(VideoCodec::H265.label(), "H.265 (HEVC)");
        assert_eq!(VideoCodec::VP9.label(), "VP9");
    }

    #[test]
    fn codec_muxers_are_correct() {
        assert_eq!(VideoCodec::H264.muxer_element(), "mp4mux");
        assert_eq!(VideoCodec::H265.muxer_element(), "mp4mux");
        assert_eq!(VideoCodec::VP9.muxer_element(), "webmmux");
    }

    #[test]
    fn codec_extensions_are_correct() {
        assert_eq!(VideoCodec::H264.file_extension(), "mp4");
        assert_eq!(VideoCodec::H265.file_extension(), "mp4");
        assert_eq!(VideoCodec::VP9.file_extension(), "webm");
    }

    #[test]
    fn only_h264_is_rtmp_compatible() {
        assert!(VideoCodec::H264.is_rtmp_compatible());
        assert!(!VideoCodec::H265.is_rtmp_compatible());
        assert!(!VideoCodec::VP9.is_rtmp_compatible());
    }

    #[test]
    fn codec_default_is_h264() {
        assert_eq!(VideoCodec::default(), VideoCodec::H264);
    }

    #[test]
    fn element_names_per_codec_h264() {
        assert_eq!(
            VideoEncoder::Nvenc.element_name_for_codec(VideoCodec::H264),
            "nvh264enc"
        );
        assert_eq!(
            VideoEncoder::QuickSync.element_name_for_codec(VideoCodec::H264),
            "qsvh264enc"
        );
        assert_eq!(
            VideoEncoder::Amf.element_name_for_codec(VideoCodec::H264),
            "amfh264enc"
        );
        assert_eq!(
            VideoEncoder::Software.element_name_for_codec(VideoCodec::H264),
            "x264enc"
        );
    }

    #[test]
    fn element_names_per_codec_h265() {
        assert_eq!(
            VideoEncoder::Nvenc.element_name_for_codec(VideoCodec::H265),
            "nvh265enc"
        );
        assert_eq!(
            VideoEncoder::QuickSync.element_name_for_codec(VideoCodec::H265),
            "qsvh265enc"
        );
        assert_eq!(
            VideoEncoder::Amf.element_name_for_codec(VideoCodec::H265),
            "amfh265enc"
        );
        assert_eq!(
            VideoEncoder::Software.element_name_for_codec(VideoCodec::H265),
            "x265enc"
        );
    }

    #[test]
    fn element_names_per_codec_vp9() {
        assert_eq!(
            VideoEncoder::Software.element_name_for_codec(VideoCodec::VP9),
            "vp9enc"
        );
    }

    #[test]
    fn label_with_codec_includes_codec_name() {
        let label = VideoEncoder::Software.label_with_codec(VideoCodec::H265);
        assert_eq!(label, "Software (H.265 (HEVC))");
    }

    #[test]
    fn input_caps_per_codec() {
        // Software always I420
        assert_eq!(
            VideoEncoder::Software.input_caps_fragment_for_codec(VideoCodec::H264),
            "video/x-raw,format=I420"
        );
        assert_eq!(
            VideoEncoder::Software.input_caps_fragment_for_codec(VideoCodec::VP9),
            "video/x-raw,format=I420"
        );
        // Hardware NV12 for H.264/H.265
        assert_eq!(
            VideoEncoder::Nvenc.input_caps_fragment_for_codec(VideoCodec::H265),
            "video/x-raw,format=NV12"
        );
        // VP9 hardware uses I420
        assert_eq!(
            VideoEncoder::Nvenc.input_caps_fragment_for_codec(VideoCodec::VP9),
            "video/x-raw,format=I420"
        );
    }

    #[test]
    fn branch_fragment_h265_software_sets_tune() {
        let frag = VideoEncoder::Software.branch_fragment_for_codec(VideoCodec::H265, 8000);
        assert!(frag.contains("x265enc"), "{frag}");
        assert!(frag.contains("tune=zerolatency"), "{frag}");
        assert!(frag.contains("bitrate=8000"), "{frag}");
    }

    #[test]
    fn branch_fragment_vp9_software_sets_deadline() {
        let frag = VideoEncoder::Software.branch_fragment_for_codec(VideoCodec::VP9, 4000);
        assert!(frag.contains("vp9enc"), "{frag}");
        assert!(frag.contains("deadline=realtime"), "{frag}");
        assert!(frag.contains("bitrate=4000"), "{frag}");
    }

    #[test]
    fn branch_fragment_h265_nvenc_sets_zerolatency() {
        let frag = VideoEncoder::Nvenc.branch_fragment_for_codec(VideoCodec::H265, 10000);
        assert!(frag.contains("nvh265enc"), "{frag}");
        assert!(frag.contains("zerolatency=true"), "{frag}");
    }

    #[test]
    fn detect_per_codec_always_ends_with_software() {
        let _ = gst::init();
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            let encoders = detect_available_encoders_for_codec(codec);
            assert!(!encoders.is_empty(), "no encoders for {codec:?}");
            assert_eq!(*encoders.last().unwrap(), VideoEncoder::Software);
        }
    }

    #[test]
    fn best_encoder_for_codec_returns_first() {
        let _ = gst::init();
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            let best = best_encoder_for_codec(codec);
            let all = detect_available_encoders_for_codec(codec);
            assert_eq!(best, all[0]);
        }
    }

    // --- Rate control (advanced) tests ---

    #[test]
    fn rate_control_default_is_cbr() {
        let rc = RateControl::default();
        assert_eq!(rc.mode, RateControlMode::Cbr);
        assert_eq!(rc.quality, 23);
        assert_eq!(rc.max_bitrate_kbps, 0);
        assert!(rc.custom_options.is_empty());
    }

    #[test]
    fn rate_control_modes_have_labels_and_hints() {
        for mode in [
            RateControlMode::Cbr,
            RateControlMode::Vbr,
            RateControlMode::Cq,
            RateControlMode::CqVbr,
        ] {
            assert!(!mode.label().is_empty());
            assert!(!mode.hint().is_empty());
        }
    }

    #[test]
    fn x264_all_four_modes_are_distinct() {
        let bitrate = 5000;
        for mode in [
            RateControlMode::Cbr,
            RateControlMode::Vbr,
            RateControlMode::Cq,
            RateControlMode::CqVbr,
        ] {
            let rc = RateControl {
                mode,
                ..RateControl::default()
            };
            let frag =
                VideoEncoder::Software.branch_fragment_for_codec_rc(VideoCodec::H264, bitrate, rc);
            assert!(frag.contains("x264enc"), "{frag}");
            assert!(frag.contains("tune=zerolatency"), "{frag}");
        }
        // Distinct property for each mode means every strategy is actually applied.
        let cbr = VideoEncoder::Software.branch_fragment(bitrate);
        let vbr = VideoEncoder::Software.branch_fragment_for_codec_rc(
            VideoCodec::H264,
            bitrate,
            RateControl {
                mode: RateControlMode::Vbr,
                ..RateControl::default()
            },
        );
        let cq = VideoEncoder::Software.branch_fragment_for_codec_rc(
            VideoCodec::H264,
            bitrate,
            RateControl {
                mode: RateControlMode::Cq,
                ..RateControl::default()
            },
        );
        let cqvbr = VideoEncoder::Software.branch_fragment_for_codec_rc(
            VideoCodec::H264,
            bitrate,
            RateControl {
                mode: RateControlMode::CqVbr,
                ..RateControl::default()
            },
        );
        assert_ne!(cbr, vbr);
        assert_ne!(cbr, cq);
        assert_ne!(cbr, cqvbr);
        assert_ne!(vbr, cq);
        assert_ne!(cq, cqvbr);
        assert!(cbr.contains("bitrate=5000"), "{cbr}");
        assert!(vbr.contains("pass=pass2"), "{vbr}");
        assert!(cq.contains("pass=quant quantizer=23"), "{cq}");
        assert!(cqvbr.contains("quantizer=23"), "{cqvbr}");
        assert!(cqvbr.contains("vbv-buf-capacity"), "{cqvbr}");
    }

    #[test]
    fn nvenc_modes_emit_rc_mode_properties() {
        let cbr = VideoEncoder::Nvenc.branch_fragment_for_codec_rc(
            VideoCodec::H264,
            6000,
            RateControl::default(),
        );
        assert!(cbr.contains("rc-mode=cbr"), "{cbr}");
        assert!(cbr.contains("bitrate=6000"), "{cbr}");
        assert!(cbr.contains("zerolatency=true"), "{cbr}");

        let vbr = VideoEncoder::Nvenc.branch_fragment_for_codec_rc(
            VideoCodec::H264,
            6000,
            RateControl {
                mode: RateControlMode::Vbr,
                max_bitrate_kbps: 9000,
                ..RateControl::default()
            },
        );
        assert!(vbr.contains("rc-mode=vbr"), "{vbr}");
        assert!(vbr.contains("max-bitrate=9000"), "{vbr}");

        let cq = VideoEncoder::Nvenc.branch_fragment_for_codec_rc(
            VideoCodec::H265,
            6000,
            RateControl {
                mode: RateControlMode::Cq,
                quality: 21,
                ..RateControl::default()
            },
        );
        assert!(cq.contains("rc-mode=constqp"), "{cq}");
        assert!(cq.contains("qp-const=21"), "{cq}");

        let cqvbr = VideoEncoder::Nvenc.branch_fragment_for_codec_rc(
            VideoCodec::H265,
            6000,
            RateControl {
                mode: RateControlMode::CqVbr,
                quality: 20,
                max_bitrate_kbps: 8000,
                ..RateControl::default()
            },
        );
        assert!(cqvbr.contains("rc-mode=vbr const-quality=20"), "{cqvbr}");
        assert!(cqvbr.contains("max-bitrate=8000"), "{cqvbr}");
    }

    #[test]
    fn fallback_backends_keep_average_bitrate_for_all_modes() {
        // QuickSync/AMF/VP9 do not expose clean rate-control properties;
        // every mode must still keep a nonzero average bitrate.
        for mode in [
            RateControlMode::Cbr,
            RateControlMode::Vbr,
            RateControlMode::Cq,
            RateControlMode::CqVbr,
        ] {
            for enc in [VideoEncoder::QuickSync, VideoEncoder::Amf] {
                let frag = enc.branch_fragment_for_codec_rc(
                    VideoCodec::H264,
                    7000,
                    RateControl {
                        mode,
                        ..RateControl::default()
                    },
                );
                assert!(frag.contains("bitrate=7000"), "{enc:?} {mode:?}: {frag}");
            }
            let vp9 = VideoEncoder::Software.branch_fragment_for_codec_rc(
                VideoCodec::VP9,
                4000,
                RateControl {
                    mode,
                    ..RateControl::default()
                },
            );
            assert!(vp9.contains("bitrate=4000"), "{mode:?}: {vp9}");
        }
    }

    #[test]
    fn custom_encoder_options_are_appended() {
        let frag = VideoEncoder::Software.branch_fragment_for_codec_rc(
            VideoCodec::H264,
            5000,
            RateControl {
                mode: RateControlMode::Cbr,
                custom_options: "key-int-max=250 bframes=3".to_string(),
                ..RateControl::default()
            },
        );
        assert!(frag.contains("key-int-max=250 bframes=3"), "{frag}");
        assert!(frag.ends_with("bframes=3"), "{frag}");
    }

    #[test]
    fn default_cbr_fragment_is_backward_compatible() {
        // The rate-control default (CBR) must not change existing output.
        let old = VideoEncoder::QuickSync.branch_fragment(5000);
        let new = VideoEncoder::QuickSync.branch_fragment_for_codec_rc(
            VideoCodec::H264,
            5000,
            RateControl::default(),
        );
        assert_eq!(old, new);
    }
}
