//! Video encoder abstraction for Rivulet.
//!
//! Rivulet supports hardware-accelerated H.264/H.265 encoding (NVIDIA NVENC,
//! Intel QuickSync, AMD AMF) and VP9 software encoding, with automatic
//! detection of the best available encoder and a software fallback. Encoders
//! are expressed as GStreamer `parse_launch` fragments so the engine can swap
//! the video branch without changing the rest of the pipeline.

use gstreamer as gst;
use serde::{Deserialize, Serialize};

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
    /// and target bitrate in kbit/s.
    ///
    /// The element is always named `video_enc` so the engine can attach
    /// performance probes to it.
    pub fn branch_fragment_for_codec(self, codec: VideoCodec, bitrate_kbps: u32) -> String {
        let element = self.element_name_for_codec(codec);
        let mut props = format!("bitrate={}", bitrate_kbps);
        match (self, codec) {
            (Self::Software, VideoCodec::H264) => props.push_str(" tune=zerolatency"),
            (Self::Software, VideoCodec::H265) => props.push_str(" tune=zerolatency"),
            (Self::Software, VideoCodec::VP9) => props.push_str(" deadline=realtime"),
            (Self::Nvenc, _) => props.push_str(" zerolatency=true"),
            (Self::QuickSync | Self::Amf, _) => {}
        }
        format!("{} name=video_enc {}", element, props)
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
}
