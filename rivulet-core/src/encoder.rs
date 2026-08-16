//! Video encoder abstraction for Rivulet.
//!
//! Rivulet supports hardware-accelerated H.264 encoding (NVIDIA NVENC, Intel
//! QuickSync, AMD AMF) with automatic detection of the best available encoder
//! and a software fallback (x264). Encoders are expressed as GStreamer
//! `parse_launch` fragments so the engine can swap the video branch without
//! changing the rest of the pipeline.

use gstreamer as gst;

/// Supported H.264 video encoders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoder {
    /// NVIDIA NVENC via `nvh264enc` (gst-plugins-bad, NVCODEC).
    Nvenc,
    /// Intel QuickSync via `qsvh264enc` (gst-plugins-bad, oneVPL).
    QuickSync,
    /// AMD AMF via `amfh264enc` (gst-plugins-bad).
    Amf,
    /// Software H.264 via `x264enc` (gst-plugins-ugly).
    Software,
}

impl VideoEncoder {
    /// The GStreamer element factory name of this encoder.
    pub fn element_name(&self) -> &'static str {
        match self {
            Self::Nvenc => "nvh264enc",
            Self::QuickSync => "qsvh264enc",
            Self::Amf => "amfh264enc",
            Self::Software => "x264enc",
        }
    }

    /// Human-readable label for display in UI and logs.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Nvenc => "NVIDIA NVENC",
            Self::QuickSync => "Intel QuickSync",
            Self::Amf => "AMD AMF",
            Self::Software => "Software (x264)",
        }
    }

    /// Whether this encoder uses dedicated hardware acceleration.
    pub fn is_hardware(&self) -> bool {
        !matches!(self, Self::Software)
    }

    /// Whether the element factory is present in the current GStreamer
    /// installation.
    ///
    /// This only checks availability of the plugin; actually creating the
    /// element can still fail on machines without the matching GPU/driver.
    pub fn is_available(&self) -> bool {
        gst::ElementFactory::find(self.element_name()).is_some()
    }

    /// `parse_launch` caps fragment inserted between `videoconvert` and the
    /// encoder to force a chroma-subsampled input format.
    ///
    /// x264 preserves the 4:4:4 chroma of RGBA input as High 4:4:4, which
    /// `mp4mux` cannot store (the pipeline then never finalizes the file).
    /// Forcing I420 keeps x264 in the 4:2:0 Main/High profile. Hardware
    /// encoders natively expect 4:2:0 NV12, so they are wired the same way.
    pub fn input_caps_fragment(&self) -> &'static str {
        match self {
            Self::Software => "video/x-raw,format=I420",
            Self::Nvenc | Self::QuickSync | Self::Amf => "video/x-raw,format=NV12",
        }
    }

    /// Build the `parse_launch` fragment for this encoder with the given target
    /// bitrate in kbit/s.
    ///
    /// The element is always named `video_enc` so the engine can attach
    /// performance probes to it. Only the `bitrate` property is guaranteed to
    /// exist on every supported element; encoder-specific tuning properties are
    /// added individually so a missing property can never break the whole
    /// pipeline.
    pub fn branch_fragment(&self, bitrate_kbps: u32) -> String {
        let mut props = format!("bitrate={}", bitrate_kbps);
        match self {
            Self::Software => props.push_str(" tune=zerolatency"),
            Self::Nvenc => props.push_str(" zerolatency=true"),
            Self::QuickSync | Self::Amf => {}
        }
        format!("{} name=video_enc {}", self.element_name(), props)
    }
}

/// Detect the hardware encoders available in the current GStreamer
/// installation, ordered by preference. Software x264 is always appended last
/// as a guaranteed fallback, so the result is never empty.
pub fn detect_available_encoders() -> Vec<VideoEncoder> {
    let mut encoders = Vec::new();
    for enc in [
        VideoEncoder::Nvenc,
        VideoEncoder::QuickSync,
        VideoEncoder::Amf,
    ] {
        if enc.is_available() {
            encoders.push(enc);
        }
    }
    encoders.push(VideoEncoder::Software);
    encoders
}

/// Return the best available encoder (hardware first, software as fallback).
pub fn best_encoder() -> VideoEncoder {
    detect_available_encoders()[0]
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
}
