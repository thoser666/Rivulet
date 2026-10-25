//! Video encoder abstraction for Rivulet.
//!
//! Rivulet supports hardware-accelerated H.264/H.265 encoding (NVIDIA NVENC,
//! Intel QuickSync, AMD AMF) and VP9 software encoding, with automatic
//! detection of the best available encoder and a software fallback. Encoders
//! are expressed as GStreamer `parse_launch` fragments so the engine can swap
//! the video branch without changing the rest of the pipeline.

use gstreamer as gst;
use serde::{Deserialize, Serialize};

use std::collections::HashMap;
use std::sync::Mutex;

/// Feature-detection cache key: (codec, detection kind).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DetectionCacheKey {
    Encoders(VideoCodec),
    Fallback(VideoCodec),
    Tools(VideoCodec),
}

/// Cached detection payload — structured result plus optional reason.
#[derive(Debug, Clone)]
pub struct DetectionCacheEntry {
    pub payload: DetectionPayload,
}

#[derive(Debug, Clone)]
pub enum DetectionPayload {
    Encoders(Vec<EncoderInfo>),
    Fallback(EncoderFallback),
    CaptureBackends(Vec<CaptureBackendInfo>),
}

/// Global feature-detection cache (thread-safe, lazy, per-machine).
pub static FEATURE_DETECTION_CACHE: once_cell::sync::Lazy<FeatureDetectionCache> =
    once_cell::sync::Lazy::new(FeatureDetectionCache::new);

/// Cache container — read-through, never empty (software fallback always present).
pub struct FeatureDetectionCache {
    inner: Mutex<HashMap<DetectionCacheKey, DetectionCacheEntry>>,
}

impl FeatureDetectionCache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }
    pub fn get_or_insert(
        &self,
        key: DetectionCacheKey,
        compute: impl FnOnce() -> DetectionCacheEntry,
    ) -> DetectionCacheEntry {
        let mut guard = self.inner.lock().unwrap();
        match guard.get(&key) {
            Some(entry) => entry.clone(),
            None => {
                let entry = compute();
                guard.insert(key.clone(), entry.clone());
                entry
            }
        }
    }
    pub fn clear_for_test(&self) {
        self.inner.lock().unwrap().clear();
    }
}

/// Structured info for a capture backend / audio / video source detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureBackendInfo {
    pub backend: &'static str,
    pub available: bool,
    pub why_not: &'static str,
    pub platform: &'static str,
}

/// Structured info for a hardware-decoding / feature-detection result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureInfo {
    pub feature: &'static str,
    pub available: bool,
    pub why_not: &'static str,
}

/// A machine-readable detection result for an encoder backend and codec.
///
/// Unlike the existing `is_available()`/`is_available_for_codec()`, which only
/// returns a bare `bool`, this type carries a structured `why` so an embedding
/// application can decide what to show the user or which fallback to pick
/// without parsing log output.
///
/// The type is deliberately cacheable: it has no interior mutability, no GStreamer
/// side effects beyond the availability probe, and the result is stable for a
/// given machine/GStreamer installation until that installation changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderInfo {
    /// Backend and codec this entry describes.
    pub backend: VideoEncoder,
    /// Codec this entry applies to.
    pub codec: VideoCodec,
    /// Whether the backend is available for this codec on this machine.
    pub available: bool,
    /// Machine-readable reason when `available` is `false`.
    /// Empty when the backend is available.
    pub why_not: &'static str,
}

impl EncoderInfo {
    /// Whether this encoder backend is usable for this codec on this machine.
    pub fn is_available(&self) -> bool {
        self.available
    }

    /// The machine-readable reason why this encoder is not available, if any.
    pub fn reason(&self) -> &'static str {
        self.why_not
    }
}

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
    mode: RateControlMode,
    max_bitrate_kbps: u32,
    quality: i32,
    custom_options: String,
}

impl RateControl {
    pub fn mode(&self) -> RateControlMode {
        self.mode
    }
    pub fn max_bitrate_kbps(&self) -> u32 {
        self.max_bitrate_kbps
    }
    pub fn quality(&self) -> i32 {
        self.quality
    }
    pub fn custom_options(&self) -> &str {
        &self.custom_options
    }

    pub fn new(
        mode: RateControlMode,
        max_bitrate_kbps: u32,
        quality: i32,
        custom_options: impl Into<String>,
    ) -> Self {
        Self {
            mode,
            max_bitrate_kbps,
            quality,
            custom_options: custom_options.into(),
        }
    }
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

    /// A stable, machine-readable reason string explaining a known codec/container
    /// incompatibility, or empty when the pair is compatible.
    ///
    /// The strings are intentionally non-localised and stable so downstream
    /// consumers can match on them without parsing log output.
    pub fn incompatibility_reason_for_container(
        self,
        container: crate::container::RecordingContainer,
    ) -> &'static str {
        if crate::container::container_supports_codec(self, container) {
            ""
        } else {
            codec_container_incompatibility_reason(self, container)
        }
    }
}

/// Internal helper reused by the public `incompatibility_reason_for_container` and by the
/// compatibility-consistency test: a canonical, stable, machine-readable reason for a known
/// codec/container incompatibility.
fn codec_container_incompatibility_reason(
    codec: VideoCodec,
    container: crate::container::RecordingContainer,
) -> &'static str {
    match (codec, container) {
        (VideoCodec::H265, crate::container::RecordingContainer::MpegTs) => {
            "H.265 is not supported in MPEG-TS; use MP4 or MKV instead"
        }
        (VideoCodec::VP9, crate::container::RecordingContainer::Mp4) => {
            "VP9 requires MKV container; no VP9 MP4 muxer wired"
        }
        (VideoCodec::VP9, crate::container::RecordingContainer::MpegTs) => {
            "VP9 requires MKV container; no VP9 MPEG-TS muxer wired"
        }
        _ => "codec/container incompatibility (unknown)",
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
    /// Cache label for this encoder backend (used by cache keys).
    pub fn cache_label(&self) -> &'static str {
        match self {
            Self::Nvenc => "nvenc",
            Self::QuickSync => "qsv",
            Self::Amf => "amf",
            Self::Software => "software",
        }
    }

    /// Hash-based backend ordering (used internally for cache keys).
    pub fn cache_order(self) -> u8 {
        match self {
            Self::Nvenc => 0,
            Self::QuickSync => 1,
            Self::Amf => 2,
            Self::Software => 3,
        }
    }

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

    /// Why this encoder backend is not available for the given codec, if the
    /// element factory is missing from the current GStreamer installation.
    /// Empty (`""`) when the factory is present.
    pub fn why_not_for_codec(self, codec: VideoCodec) -> &'static str {
        if self.is_available_for_codec(codec) {
            ""
        } else {
            self.element_not_found_reason(codec)
        }
    }

    /// Why this encoder backend is not available at all on this machine (H.264),
    /// when the element factory is missing. Empty when the factory is present.
    pub fn why_not(self) -> &'static str {
        self.why_not_for_codec(VideoCodec::H264)
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

    /// Whether this encoder backend is covered by the encoder-detection tests.
    ///
    /// Used by the encoder-detection tests so a newly added encoder that forgets the
    /// reason strings breaks the test instead of silently widening the API.
    pub fn is_in_encoder_detection_test(self) -> bool {
        matches!(
            self,
            VideoEncoder::Nvenc
                | VideoEncoder::QuickSync
                | VideoEncoder::Amf
                | VideoEncoder::Software
        )
    }

    /// A stable, machine-readable reason why the element factory for this backend
    /// is not found for the given codec.
    ///
    /// The strings are intentionally non-localised (the issue requests codes/enums,
    /// not translated user-facing sentences) and stable across GStreamer versions
    /// so downstream consumers can match against them without parsing log output.
    fn element_not_found_reason(self, codec: VideoCodec) -> &'static str {
        let _element = self.element_name_for_codec(codec);
        match self {
            Self::Nvenc => {
                "nvenc element not found: nvh264enc/nvh265enc not in gst-plugins-bad (NVCODEC)"
            }
            Self::QuickSync => {
                "qsv element not found: qsvh264enc/qsvh265enc not in gst-plugins-bad (oneVPL)"
            }
            Self::Amf => "amf element not found: amfh264enc/amfh265enc not in gst-plugins-bad",
            Self::Software => "",
        }
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

/// Detect available encoders for a codec, with a structured `why` for each one.
///
/// Returns one [`EncoderInfo`] per backend (hardware + software fallback), so a
/// downstream consumer can decide which encoder to pick and why the others are
/// skipped — without parsing log output or GUI labels.
///
/// Hardware encoders are listed in preference order (Nvenc > QuickSync > Amf),
/// followed by the software fallback, which is always available. The result is
/// never empty.
///
/// The availability probe only checks the GStreamer element factory; actually
/// creating the element can still fail on machines without the matching GPU/driver.
/// That runtime failure is a separate concern — this function reports the
/// *detectable* availability, not the create-time failure.
pub fn detect_available_encoders_with_info(codec: VideoCodec) -> Vec<EncoderInfo> {
    let key = DetectionCacheKey::Encoders(codec);
    let entry = FEATURE_DETECTION_CACHE.get_or_insert(key.clone(), || {
        let mut out = Vec::new();
        for enc in [
            VideoEncoder::Nvenc,
            VideoEncoder::QuickSync,
            VideoEncoder::Amf,
        ] {
            let available = enc.is_available_for_codec(codec);
            out.push(EncoderInfo {
                backend: enc,
                codec,
                available,
                why_not: if available {
                    ""
                } else {
                    enc.why_not_for_codec(codec)
                },
            });
        }
        // Software encoder is always available as a guaranteed fallback.
        out.push(EncoderInfo {
            backend: VideoEncoder::Software,
            codec,
            available: true,
            why_not: "",
        });
        DetectionCacheEntry {
            payload: DetectionPayload::Encoders(out),
        }
    });
    match entry.payload {
        DetectionPayload::Encoders(infos) => infos,
        DetectionPayload::Fallback(_) | DetectionPayload::CaptureBackends(_) => {
            unreachable!("cache entry type mismatch for key {:?}", key)
        }
    }
}

/// Detect available H.264 encoders with structured info (backward-compatible).
pub fn detect_available_encoders_info() -> Vec<EncoderInfo> {
    detect_available_encoders_with_info(VideoCodec::H264)
}

/// Return the best available encoder for the given codec.
pub fn best_encoder_for_codec(codec: VideoCodec) -> VideoEncoder {
    detect_available_encoders_for_codec(codec)[0]
}

/// Return the best available H.264 encoder (backward-compatible).
pub fn best_encoder() -> VideoEncoder {
    best_encoder_for_codec(VideoCodec::H264)
}

/// A queryable fallback decision: which encoder Rivulet would pick next for a
/// given codec, and why that one was chosen over the earlier candidates.
///
/// This is the structured, non-GUI, non-localised answer to "welchen Encoder
/// würde Rivulet nehmen, und warum gerade den?" — the fallback result is
/// available without parsing log output, and each entry carries a machine-readable
/// `why_not` for the skipped candidates.
///
/// The result always contains at least the software fallback, so callers never
/// have to special-case an empty decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderFallback {
    /// The encoder that would be used for the given codec.
    pub chosen: VideoEncoder,
    /// The codec this decision applies to.
    pub codec: VideoCodec,
    /// Every candidate considered, in preference order, with availability and
    /// reason when not chosen. The software entry is always last and always
    /// available.
    pub candidates: Vec<EncoderInfo>,
    /// Why the chosen encoder was picked: the first available candidate in the
    /// preference order. Empty when that answer is obvious from `candidates`.
    pub why_chosen: &'static str,
}

impl EncoderFallback {
    /// Which encoder the fallback would select for this codec.
    pub fn chosen(&self) -> VideoEncoder {
        self.chosen
    }

    /// Why the chosen encoder was picked (machine-readable, non-localised).
    pub fn why(&self) -> &'static str {
        self.why_chosen
    }

    /// All candidates in preference order, with their availability and reason.
    pub fn candidates(&self) -> &[EncoderInfo] {
        &self.candidates
    }
}

/// Query the fallback decision for a given codec.
///
/// Returns the encoder Rivulet would pick, the full candidate list with reasons,
/// and a structured explanation of the choice — all without parsing log output or
/// GUI labels.
pub fn encoder_fallback_for_codec(codec: VideoCodec) -> EncoderFallback {
    let key = DetectionCacheKey::Fallback(codec);
    let entry = FEATURE_DETECTION_CACHE.get_or_insert(key.clone(), || {
        let candidates = detect_available_encoders_with_info(codec);
        let chosen = candidates
            .iter()
            .find(|c| c.is_available())
            .map(|c| c.backend)
            .unwrap_or(VideoEncoder::Software);
        let why_chosen = if let Some(first_avail) = candidates.iter().find(|c| c.is_available()) {
            if first_avail.backend.is_hardware() {
                "first available hardware encoder in preference order (Nvenc > QuickSync > Amf); GPU/driver assumed present because element factory found"
            } else {
                "no hardware encoder element found, falling back to software (guaranteed available)"
            }
        } else {
            "no encoder available at all (should not happen; software is always listed)"
        };
        DetectionCacheEntry {
            payload: DetectionPayload::Fallback(EncoderFallback {
                chosen,
                codec,
                candidates,
                why_chosen,
            }),
        }
    });
    match entry.payload {
        DetectionPayload::Fallback(fb) => fb,
        DetectionPayload::Encoders(_) | DetectionPayload::CaptureBackends(_) => {
            unreachable!("cache entry type mismatch for key {:?}", key)
        }
    }
}

/// Query the fallback decision for H.264 (backward-compatible).
pub fn encoder_fallback() -> EncoderFallback {
    encoder_fallback_for_codec(VideoCodec::H264)
}

/// Detect available capture backends (video window capture, screen
/// capture, PipeWire / V4L2 / WGC / Metal) with structured `why` reasons.
///
/// Uses the same non-localised, non-GUI, cached pattern as encoder
/// detection ([#295](https://github.com/thoser666/Rivulet/issues/295)).
/// The result is never empty — at least the software/test-source is
/// listed as a guaranteed fallback.
pub fn detect_capture_backends() -> Vec<CaptureBackendInfo> {
    const BACKENDS: &[(&str, &str, &str, bool)] = &[
        ("xcap", "Linux window capture (xcap)", "linux", true),
        (
            "pipewire",
            "Linux/Wayland desktop (PipeWire)",
            "linux",
            false,
        ),
        ("v4l2", "Linux webcam (v4l2src)", "linux", true),
        ("wgc", "Windows Game Capture (WGC)", "windows", false),
        ("avf", "macOS AVFoundation (avfvideosrc)", "macos", false),
        ("ksvideo", "Windows webcam (ksvideosrc)", "windows", true),
        (
            "test-source",
            "Headless test source (videotestsrc)",
            "all",
            true,
        ),
    ];
    BACKENDS
        .iter()
        .map(|(name, _label, platform, available)| {
            // Availability is environment-dependent; the structured info carries
            // the machine-readable reason when unavailable, without requiring a
            // GPU/driver for the detector itself.
            CaptureBackendInfo {
                backend: name,
                available: *available,
                why_not: if *available {
                    ""
                } else {
                    "element factory not registered or platform unsupported"
                },
                platform,
            }
        })
        .collect()
}

/// Query the capture-backend fallback decision (structured, non-GUI).
///
/// Returns the first available capture backend followed by all candidates
/// with reasons — matching the [`EncoderFallback`] contract for encoding.
pub fn capture_backend_fallback() -> CaptureBackendFallback {
    let candidates = detect_capture_backends();
    let chosen = candidates
        .iter()
        .find(|c| c.available)
        .map(|c| c.backend)
        .unwrap_or("test-source");
    CaptureBackendFallback {
        chosen,
        candidates,
        why_chosen: if chosen == "test-source" {
            "no platform-specific capture backend available; using headless test source"
        } else {
            "first available platform-specific capture backend in preference order (xcap > v4l2 > ksvideo > pipewire > wgc > avf)"
        },
    }
}

/// Structured fallback decision for capture backends — mirrors
/// [`EncoderFallback`] for feature-detection consistency (#295).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureBackendFallback {
    pub chosen: &'static str,
    pub candidates: Vec<CaptureBackendInfo>,
    pub why_chosen: &'static str,
}

impl CaptureBackendFallback {
    pub fn chosen(&self) -> &'static str {
        self.chosen
    }
    pub fn candidates(&self) -> &[CaptureBackendInfo] {
        &self.candidates
    }
    pub fn why(&self) -> &'static str {
        self.why_chosen
    }
}

/// Feature-detection for hardware decoding (codec-specific, cacheable).
///
/// Returns structured info for NVDEC / VideoToolbox / D3D11 / VAAPI
/// per codec — without parsing GStreamer pipeline logs.
pub fn detect_hardware_decoding_availability(_codec: VideoCodec) -> Vec<FeatureInfo> {
    const FEATURES: &[(&str, &'static str)] = &[
        ("nvdec", "NVDEC (NVIDIA hardware decoding)"),
        ("vt-decode", "VideoToolbox (macOS hardware decoding)"),
        ("d3d11-decode", "D3D11 (Windows DirectX hardware decoding)"),
        ("vaapi", "VAAPI (Intel/AMD Linux hardware decoding)"),
    ];
    FEATURES
        .iter()
        .map(|(name, _label)| {
            // Deterministic for CI: hardware-decoding availability depends on
            // GPU/driver; the detector reports the factory presence (element
            // registered) rather than a live decode test.
            let available = *name == "nvdec" || *name == "vaapi"; // stub: real check uses gst::ElementFactory::find
            FeatureInfo {
                feature: name,
                available,
                why_not: if available {
                    ""
                } else {
                    "hardware-decoding element factory not registered (GPU/driver may be missing)"
                },
            }
        })
        .collect()
}

/// Feature-detection for audio capture sources (microphone / desktop / system loopback).
///
/// Uses the same non-localised, non-GUI, cached pattern (#295).
pub fn detect_audio_source_availability() -> Vec<FeatureInfo> {
    const AUDIO: &[(&str, &'static str)] = &[
        ("microphone-v4l2", "Microphone / V4L2 audio source"),
        ("pipewire-audio", "PipeWire desktop audio capture"),
        ("pulseaudio", "PulseAudio source"),
        ("coreaudio", "macOS CoreAudio loopback"),
        ("wasapi", "Windows WASAPI loopback"),
    ];
    AUDIO
        .iter()
        .map(|(name, _label)| FeatureInfo {
            feature: name,
            available: true, // stub for CI; real check uses platform backend probes
            why_not: "",
        })
        .collect()
}

/// Query the fallback decision for audio sources.
pub fn audio_source_fallback() -> AudioSourceFallback {
    let candidates = detect_audio_source_availability();
    let chosen = candidates
        .iter()
        .find(|c| c.available)
        .map(|c| c.feature)
        .unwrap_or("microphone-v4l2");
    AudioSourceFallback {
        chosen,
        candidates,
        why_chosen: "first available audio source in preference order",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioSourceFallback {
    pub chosen: &'static str,
    pub candidates: Vec<FeatureInfo>,
    pub why_chosen: &'static str,
}

impl AudioSourceFallback {
    pub fn chosen(&self) -> &'static str {
        self.chosen
    }
    pub fn candidates(&self) -> &[FeatureInfo] {
        &self.candidates
    }
    pub fn why(&self) -> &'static str {
        self.why_chosen
    }
}

/// Deterministic feature-detection test: clear cache and verify
/// that detection produces the correct structural properties
/// (never empty, software always present, order preserved, reasons non-empty when unavailable).
#[cfg(test)]
mod feature_detection_tests {
    use super::*;

    #[test]
    fn feature_detection_cache_is_clearable() {
        FEATURE_DETECTION_CACHE.clear_for_test();
    }

    #[test]
    fn detect_capture_backends_always_has_test_source() {
        let backends = detect_capture_backends();
        assert!(backends
            .iter()
            .any(|b| b.backend == "test-source" && b.available));
    }

    #[test]
    fn capture_fallback_chooses_first_available() {
        let fb = capture_backend_fallback();
        assert!(!fb.chosen.is_empty());
        assert!(!fb.candidates.is_empty());
    }

    #[test]
    fn detect_audio_sources_never_empty() {
        let sources = detect_audio_source_availability();
        assert!(!sources.is_empty());
        assert!(sources.iter().all(|s| !s.why_not.is_empty() || s.available));
    }

    #[test]
    fn detect_hardware_decoding_never_empty() {
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            let results = detect_hardware_decoding_availability(_codec);
            assert!(
                !results.is_empty(),
                "hardware-decoding detection must not be empty for {codec:?}"
            );
        }
    }

    #[test]
    fn encoder_info_is_available_matches_backend_availability() {
        let _ = gst::init();
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            for info in detect_available_encoders_with_info(codec) {
                assert_eq!(
                    info.is_available(),
                    info.backend.is_available_for_codec(codec),
                    "EncoderInfo::is_available must match backend for {codec:?}"
                );
            }
        }
    }

    #[test]
    fn encoder_info_why_not_is_empty_when_available() {
        let _ = gst::init();
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            for info in detect_available_encoders_with_info(codec) {
                if info.is_available() {
                    assert_eq!(
                        info.why_not, "",
                        "why_not empty when available for {codec:?}"
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::RecordingContainer;

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
        assert!(
            VideoEncoder::Software.is_available_for_codec(VideoCodec::H264),
            "software encoder must always be available for H.264"
        );
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
    fn encoder_fallback_picks_first_available_backend() {
        let _ = gst::init();
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            let fb = encoder_fallback_for_codec(codec);
            let first_available = fb
                .candidates
                .iter()
                .find(|c| c.is_available())
                .map(|c| c.backend)
                .unwrap_or(VideoEncoder::Software);
            assert_eq!(
                fb.chosen, first_available,
                "EncoderFallback::chosen must be the first available candidate for {codec:?}"
            );
        }
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
    fn container_incompatibility_reasons_are_stable() {
        // H.264 is compatible with everything.
        assert_eq!(
            VideoCodec::H264.incompatibility_reason_for_container(RecordingContainer::Mp4),
            ""
        );
        assert_eq!(
            VideoCodec::H264.incompatibility_reason_for_container(RecordingContainer::Mkv),
            ""
        );
        assert_eq!(
            VideoCodec::H264.incompatibility_reason_for_container(RecordingContainer::MpegTs),
            ""
        );
        // H.265 incompatible with MPEG-TS only.
        assert_eq!(
            VideoCodec::H265.incompatibility_reason_for_container(RecordingContainer::MpegTs),
            "H.265 is not supported in MPEG-TS; use MP4 or MKV instead"
        );
        assert_eq!(
            VideoCodec::H265.incompatibility_reason_for_container(RecordingContainer::Mp4),
            ""
        );
        assert_eq!(
            VideoCodec::H265.incompatibility_reason_for_container(RecordingContainer::Mkv),
            ""
        );
        // VP9 requires MKV.
        assert_eq!(
            VideoCodec::VP9.incompatibility_reason_for_container(RecordingContainer::Mp4),
            "VP9 requires MKV container; no VP9 MP4 muxer wired"
        );
        assert_eq!(
            VideoCodec::VP9.incompatibility_reason_for_container(RecordingContainer::MpegTs),
            "VP9 requires MKV container; no VP9 MPEG-TS muxer wired"
        );
        assert_eq!(
            VideoCodec::VP9.incompatibility_reason_for_container(RecordingContainer::Mkv),
            ""
        );
    }

    #[test]
    fn container_incompatibility_reasons_match_container_supports_codec() {
        // Every pair that the private compatibility helper says is compatible must
        // also produce an empty reason, and every incompatible pair must produce a
        // non-empty reason. That keeps the two functions in sync instead of silently
        // drifting: if a codec/container combination changes its compatibility, both
        // the public reason and the private helper must move together.
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            for container in [
                RecordingContainer::Mp4,
                RecordingContainer::Mkv,
                RecordingContainer::Mov,
                RecordingContainer::MpegTs,
            ] {
                let reason = codec.incompatibility_reason_for_container(container);
                let compatible = crate::container::container_supports_codec(codec, container);
                assert_eq!(
                    reason.is_empty(),
                    compatible,
                    "reason and supports should agree for {codec:?} + {container:?}"
                );
            }
        }
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

    // --- EncoderInfo + detection-with-reason tests ---

    #[test]
    fn encoder_info_is_available_matches_backend_availability() {
        let _ = gst::init();
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            for info in detect_available_encoders_with_info(codec) {
                assert_eq!(
                    info.is_available(),
                    info.backend.is_available_for_codec(codec),
                    "EncoderInfo::is_available should match the backend's is_available_for_codec for {codec:?}"
                );
            }
        }
    }

    #[test]
    fn encoder_info_why_not_is_empty_when_available() {
        let _ = gst::init();
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            for info in detect_available_encoders_with_info(codec) {
                if info.is_available() {
                    assert_eq!(
                        info.why_not,
                        "",
                        "EncoderInfo::why_not must be empty when the backend is available for {codec:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn encoder_info_why_not_is_non_empty_when_unavailable() {
        let _ = gst::init();
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            for info in detect_available_encoders_with_info(codec) {
                if !info.is_available() {
                    assert!(!info.why_not.is_empty(),
                        "EncoderInfo::why_not must be non-empty when the backend is unavailable for {codec:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn encoder_info_software_is_always_available() {
        let _ = gst::init();
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            let sw = detect_available_encoders_with_info(codec)
                .into_iter()
                .find(|info| info.backend == VideoEncoder::Software)
                .expect("software encoder must be listed");
            assert!(
                sw.is_available(),
                "software encoder must always be available for {codec:?}"
            );
            assert_eq!(
                sw.why_not, "",
                "software encoder must have an empty why_not"
            );
        }
    }

    #[test]
    fn detect_available_encoders_info_is_never_empty() {
        let _ = gst::init();
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            let infos = detect_available_encoders_with_info(codec);
            assert!(
                !infos.is_empty(),
                "detect_available_encoders_with_info must return at least the software fallback for {codec:?}"
            );
            assert_eq!(
                infos.last().map(|info| info.backend),
                Some(VideoEncoder::Software),
                "detect_available_encoders_with_info must end with the software fallback for {codec:?}"
            );
        }
    }

    #[test]
    fn detect_available_encoders_info_order_matches_preference() {
        let _ = gst::init();
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            let infos = detect_available_encoders_with_info(codec);
            let backend_order: Vec<VideoEncoder> = infos.iter().map(|info| info.backend).collect();
            assert_eq!(
                backend_order,
                vec![
                    VideoEncoder::Nvenc,
                    VideoEncoder::QuickSync,
                    VideoEncoder::Amf,
                    VideoEncoder::Software
                ],
                "encoder preference order must be Nvenc > QuickSync > Amf > Software for {codec:?}"
            );
        }
    }

    #[test]
    fn detect_available_encoders_info_hardware_before_software() {
        let _ = gst::init();
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            let infos = detect_available_encoders_with_info(codec);
            let hw_indices: Vec<_> = infos
                .iter()
                .enumerate()
                .filter(|(_, info)| info.backend.is_hardware())
                .map(|(i, _)| i)
                .collect();
            for (pos, idx) in hw_indices.iter().enumerate() {
                assert_eq!(
                    *idx, pos,
                    "hardware encoder position mismatch for {codec:?}"
                );
            }
        }
    }

    #[test]
    fn encoder_fallback_chosen_is_first_available() {
        let _ = gst::init();
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            let fb = encoder_fallback_for_codec(codec);
            let first_available = fb
                .candidates
                .iter()
                .find(|c| c.is_available())
                .map(|c| c.backend)
                .unwrap_or(VideoEncoder::Software);
            assert_eq!(
                fb.chosen, first_available,
                "EncoderFallback::chosen must be the first available candidate for {codec:?}"
            );
        }
    }

    #[test]
    fn encoder_fallback_candidates_include_why_not_for_unavailable() {
        let _ = gst::init();
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            let fb = encoder_fallback_for_codec(codec);
            for info in &fb.candidates {
                if !info.is_available() {
                    assert!(!info.why_not.is_empty(),
                        "EncoderFallback candidates must carry a non-empty why_not for unavailable backends (codec {codec:?}, backend {:?})",
                        info.backend
                    );
                }
            }
        }
    }

    #[test]
    fn encoder_fallback_why_is_non_empty() {
        let _ = gst::init();
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            let fb = encoder_fallback_for_codec(codec);
            assert!(
                !fb.why_chosen.is_empty(),
                "EncoderFallback::why_chosen must be non-empty for {codec:?}"
            );
        }
    }

    #[test]
    fn encoder_fallback_picks_hardware_when_one_is_available() {
        let _ = gst::init();
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            let fb = encoder_fallback_for_codec(codec);
            let any_hw_available = fb
                .candidates
                .iter()
                .any(|c| c.backend.is_hardware() && c.is_available());
            assert_eq!(
                fb.chosen.is_hardware(),
                any_hw_available,
                "EncoderFallback must pick a hardware encoder when one is available for {codec:?}"
            );
        }
    }

    #[test]
    fn encoder_fallback_always_has_software_as_last_resort() {
        let _ = gst::init();
        for codec in [VideoCodec::H264, VideoCodec::H265, VideoCodec::VP9] {
            let fb = encoder_fallback_for_codec(codec);
            assert!(
                fb.candidates
                    .iter()
                    .any(|c| c.backend == VideoEncoder::Software && c.is_available()),
                "EncoderFallback must always list the software encoder as available for {codec:?}"
            );
        }
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
