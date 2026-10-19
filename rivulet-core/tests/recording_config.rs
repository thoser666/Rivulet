//! Regression tests for the typed early-validation of the in-process
//! recording entry point ([#294](https://github.com/thoser666/Rivulet/issues/294)).
//!
//! The M8 gate criterion is *"Invalid configuration fails early with typed,
//! actionable errors."* These tests pin that promise per parameter: every
//! field an embedding application can get wrong produces a *typed* error that
//! names the offending field and carries a message stating the fix — all
//! before any GStreamer pipeline is touched, so the checks are deterministic
//! on a runner without hardware, network, or codecs.

use rivulet_core::container::RecordingContainer;
use rivulet_core::encoder::{VideoCodec, VideoEncoder};
use rivulet_core::{
    detect_container_availability, detect_encoder_availability, RecordingConfig,
    RecordingConfigError,
};

/// A valid baseline; each failing test mutates exactly one field so the
/// reported error is unambiguous.
fn valid() -> RecordingConfig {
    RecordingConfig::default()
}

#[test]
fn default_config_is_valid() {
    assert_eq!(valid().validate(), Ok(()));
}

#[test]
fn zero_width_is_rejected_and_named() {
    let mut cfg = valid();
    cfg.width = 0;
    let err = cfg.validate().expect_err("width 0 must fail early");
    assert_eq!(err, RecordingConfigError::WidthZero);
    assert_eq!(err.field(), "width");
    assert!(err.to_string().contains("width"));
}

#[test]
fn odd_width_is_rejected_and_names_the_fix() {
    let mut cfg = valid();
    cfg.width = 1919;
    let err = cfg.validate().expect_err("odd width must fail early");
    assert_eq!(err, RecordingConfigError::WidthOdd { width: 1919 });
    assert_eq!(err.field(), "width");
    // The message must be actionable: it names "even" as the requirement.
    assert!(err.to_string().contains("even"));
}

#[test]
fn oversized_width_is_rejected_and_names_the_max() {
    let mut cfg = valid();
    cfg.width = 10_000;
    let err = cfg.validate().expect_err("oversized width must fail early");
    assert_eq!(
        err,
        RecordingConfigError::WidthTooLarge {
            width: 10_000,
            max: 8192
        }
    );
    assert!(err.to_string().contains("8192"));
}

#[test]
fn zero_height_is_rejected_and_named() {
    let mut cfg = valid();
    cfg.height = 0;
    let err = cfg.validate().expect_err("height 0 must fail early");
    assert_eq!(err, RecordingConfigError::HeightZero);
    assert_eq!(err.field(), "height");
}

#[test]
fn odd_height_is_rejected_and_names_the_fix() {
    let mut cfg = valid();
    cfg.height = 1079;
    let err = cfg.validate().expect_err("odd height must fail early");
    assert_eq!(err, RecordingConfigError::HeightOdd { height: 1079 });
    assert_eq!(err.field(), "height");
    assert!(err.to_string().contains("even"));
}

#[test]
fn oversized_height_is_rejected_and_names_the_max() {
    let mut cfg = valid();
    cfg.height = 9000;
    let err = cfg
        .validate()
        .expect_err("oversized height must fail early");
    assert_eq!(
        err,
        RecordingConfigError::HeightTooLarge {
            height: 9000,
            max: 8192
        }
    );
    assert!(err.to_string().contains("8192"));
}

#[test]
fn zero_fps_is_rejected_and_named() {
    let mut cfg = valid();
    cfg.fps = 0;
    let err = cfg.validate().expect_err("fps 0 must fail early");
    assert_eq!(err, RecordingConfigError::FpsZero);
    assert_eq!(err.field(), "fps");
}

#[test]
fn excessive_fps_is_rejected_and_names_the_max() {
    let mut cfg = valid();
    cfg.fps = 1000;
    let err = cfg.validate().expect_err("excessive fps must fail early");
    assert_eq!(
        err,
        RecordingConfigError::FpsTooHigh {
            fps: 1000,
            max: 240
        }
    );
    assert_eq!(err.field(), "fps");
    assert!(err.to_string().contains("240"));
}

#[test]
fn zero_bitrate_is_rejected_and_named() {
    let mut cfg = valid();
    cfg.bitrate_kbps = 0;
    let err = cfg.validate().expect_err("bitrate 0 must fail early");
    assert_eq!(err, RecordingConfigError::BitrateZero);
    assert_eq!(err.field(), "bitrate_kbps");
}

#[test]
fn tiny_bitrate_is_rejected_and_names_the_min() {
    let mut cfg = valid();
    cfg.bitrate_kbps = 5;
    let err = cfg.validate().expect_err("tiny bitrate must fail early");
    assert_eq!(
        err,
        RecordingConfigError::BitrateTooLow { kbps: 5, min: 100 }
    );
    assert!(err.to_string().contains("100"));
}

#[test]
fn huge_bitrate_is_rejected_and_hints_at_bits_vs_kilobits() {
    let mut cfg = valid();
    // 50 Mbps entered as if it were bits-per-second: a classic unit mistake.
    cfg.bitrate_kbps = 50_000_000;
    let err = cfg.validate().expect_err("huge bitrate must fail early");
    assert_eq!(
        err,
        RecordingConfigError::BitrateTooHigh {
            kbps: 50_000_000,
            max: 500_000
        }
    );
    assert_eq!(err.field(), "bitrate_kbps");
    // Actionable: the message points at the bits-vs-kilobits mistake.
    assert!(err.to_string().contains("divide by 1000"));
}

#[test]
fn vp9_in_mp4_is_rejected_and_names_a_working_container() {
    let mut cfg = valid();
    cfg.codec = VideoCodec::VP9;
    cfg.container = RecordingContainer::Mp4;
    let err = cfg
        .validate()
        .expect_err("VP9 in MP4 must fail before the muxer start");
    match &err {
        RecordingConfigError::CodecContainerIncompatible {
            codec,
            container,
            hint,
        } => {
            assert_eq!(*codec, VideoCodec::VP9);
            assert_eq!(*container, RecordingContainer::Mp4);
            // The error names the fix: MKV carries VP9.
            assert_eq!(*hint, RecordingContainer::Mkv);
        }
        other => panic!("expected CodecContainerIncompatible, got {other:?}"),
    }
    assert_eq!(err.field(), "container");
}

#[test]
fn vp9_in_mkv_is_accepted() {
    let mut cfg = valid();
    cfg.codec = VideoCodec::VP9;
    cfg.container = RecordingContainer::Mkv;
    assert_eq!(cfg.validate(), Ok(()));
}

#[test]
fn h265_in_mpegts_is_rejected() {
    let mut cfg = valid();
    cfg.codec = VideoCodec::H265;
    cfg.container = RecordingContainer::MpegTs;
    let err = cfg
        .validate()
        .expect_err("H.265 in MPEG-TS is not wired in this build");
    assert_eq!(err.field(), "container");
}

#[test]
fn h264_is_accepted_in_every_container() {
    for container in [
        RecordingContainer::Mp4,
        RecordingContainer::Mkv,
        RecordingContainer::Mov,
        RecordingContainer::MpegTs,
    ] {
        let mut cfg = valid();
        cfg.container = container;
        assert_eq!(cfg.validate(), Ok(()), "H.264 must work in {container:?}");
    }
}

#[test]
fn mismatched_path_extension_is_rejected_and_names_the_expected_one() {
    let mut cfg = valid();
    cfg.container = RecordingContainer::Mkv;
    cfg.path = Some("capture.mp4".to_string());
    let err = cfg
        .validate()
        .expect_err("path extension must match the container");
    assert_eq!(
        err,
        RecordingConfigError::PathExtensionMismatch {
            extension: "mp4".to_string(),
            container: RecordingContainer::Mkv,
            expected: "mkv".to_string(),
        }
    );
    assert_eq!(err.field(), "path");
    assert!(err.to_string().contains("mkv"));
}

#[test]
fn matching_path_extension_is_accepted() {
    let mut cfg = valid();
    cfg.container = RecordingContainer::Mkv;
    cfg.path = Some("capture.mkv".to_string());
    assert_eq!(cfg.validate(), Ok(()));
}

#[test]
fn path_without_extension_is_accepted() {
    let mut cfg = valid();
    cfg.path = Some("capture".to_string());
    assert_eq!(cfg.validate(), Ok(()));
}

#[test]
fn unknown_path_extension_is_rejected_as_mismatch() {
    let mut cfg = valid();
    cfg.path = Some("capture.avi".to_string());
    let err = cfg
        .validate()
        .expect_err("an unsupported extension must fail early");
    assert_eq!(err.field(), "path");
}

#[test]
fn field_names_are_stable_for_binding_to_ui_controls() {
    // The field() strings are a machine-readable contract an embedding UI
    // binds controls to; renaming one silently would break that binding.
    let cases: Vec<(RecordingConfigError, &str)> = vec![
        (RecordingConfigError::WidthZero, "width"),
        (RecordingConfigError::HeightZero, "height"),
        (RecordingConfigError::FpsZero, "fps"),
        (RecordingConfigError::BitrateZero, "bitrate_kbps"),
        (
            RecordingConfigError::CodecContainerIncompatible {
                codec: VideoCodec::VP9,
                container: RecordingContainer::Mp4,
                hint: RecordingContainer::Mkv,
            },
            "container",
        ),
        (
            RecordingConfigError::PathExtensionMismatch {
                extension: "mp4".to_string(),
                container: RecordingContainer::Mkv,
                expected: "mkv".to_string(),
            },
            "path",
        ),
    ];
    for (err, field) in cases {
        assert_eq!(err.field(), field, "field name for {err:?}");
    }
}

#[test]
fn detect_encoder_availability_is_available_for_software() {
    let result = detect_encoder_availability(VideoEncoder::Software);
    assert!(result.is_available());
}

#[test]
fn detect_encoder_availability_is_available_for_all_backends() {
    for encoder in [
        VideoEncoder::Software,
        VideoEncoder::Nvenc,
        VideoEncoder::QuickSync,
        VideoEncoder::Amf,
    ] {
        let result = detect_encoder_availability(encoder);
        assert!(
            result.is_available(),
            "encoder {encoder:?} should be available"
        );
    }
}

#[test]
fn detect_container_availability_is_compatible_h264_mp4() {
    let result = detect_container_availability(RecordingContainer::Mp4, VideoCodec::H264);
    assert!(result.is_available());
}

#[test]
fn detect_container_availability_is_compatible_h264_mkv() {
    let result = detect_container_availability(RecordingContainer::Mkv, VideoCodec::H264);
    assert!(result.is_available());
}

#[test]
fn detect_container_availability_is_compatible_h264_mpegts() {
    let result = detect_container_availability(RecordingContainer::MpegTs, VideoCodec::H264);
    assert!(result.is_available());
}

#[test]
fn detect_container_availability_is_compatible_h265_mp4() {
    let result = detect_container_availability(RecordingContainer::Mp4, VideoCodec::H265);
    assert!(result.is_available());
}

#[test]
fn detect_container_availability_is_not_compatible_h265_mpegts() {
    let result = detect_container_availability(RecordingContainer::MpegTs, VideoCodec::H265);
    assert!(!result.is_available());
    assert_eq!(
        result.reason(),
        "H.265 is not supported in MPEG-TS; use MP4 or MKV instead"
    );
}

#[test]
fn detect_container_availability_is_compatible_vp9_mkv() {
    let result = detect_container_availability(RecordingContainer::Mkv, VideoCodec::VP9);
    assert!(result.is_available());
}

#[test]
fn detect_container_availability_is_not_compatible_vp9_mp4() {
    let result = detect_container_availability(RecordingContainer::Mp4, VideoCodec::VP9);
    assert!(!result.is_available());
    assert_eq!(
        result.reason(),
        "VP9 requires MKV container; MP4 muxer not available"
    );
}

#[test]
fn detect_container_availability_is_not_compatible_vp9_mpegts() {
    let result = detect_container_availability(RecordingContainer::MpegTs, VideoCodec::VP9);
    assert!(!result.is_available());
    assert_eq!(
        result.reason(),
        "VP9 requires MKV container; MP4 muxer not available"
    );
}

#[test]
fn every_variant_is_typed_and_implements_error() {
    // A compile-time-ish guarantee that the typed error is a real std Error,
    // so `?` and error-reporting crates work for embedders.
    let err: Box<dyn std::error::Error> = Box::new(RecordingConfigError::FpsZero);
    assert!(!err.to_string().is_empty());
    // Every encoder backend is accepted by validation (availability is #295's
    // runtime concern, not static validation).
    for encoder in [
        VideoEncoder::Software,
        VideoEncoder::Nvenc,
        VideoEncoder::QuickSync,
        VideoEncoder::Amf,
    ] {
        let mut cfg = valid();
        cfg.encoder = encoder;
        assert_eq!(cfg.validate(), Ok(()));
    }
}
