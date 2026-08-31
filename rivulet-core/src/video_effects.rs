//! Video filters & effects applied to the recording pipeline before encoding.
//!
//! Effects are expressed as a GStreamer pipeline fragment so they can be
//! inserted between `videoconvert` and the encoder-input caps filter. This
//! offers colour correction (brightness/contrast/saturation/hue via
//! `videobalance`), a Gaussian blur (`gaussianblur`) and optional sharpening
//! (`cas`). **LUT colour grading** is deliberately not mapped here: there is no
//! portable `lut`/`lut3d` element in the core plugins, so loading `.cube` LUTs
//! is tracked separately and left for a follow-up.

use gstreamer as gst;
use serde::{Deserialize, Serialize};

/// Video filter/effect settings applied to the capture before encoding.
///
/// All values are neutral by default, so an untouched [`VideoEffects`]
/// contributes nothing to the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct VideoEffects {
    /// Brightness, `-1.0..=1.0`, `0.0` neutral (videobalance).
    pub brightness: f32,
    /// Contrast, `-1.0..=1.0`, `0.0` neutral (videobalance).
    pub contrast: f32,
    /// Saturation, `-1.0..=1.0`, `0.0` neutral (videobalance).
    pub saturation: f32,
    /// Hue rotation, `-1.0..=1.0`, `0.0` neutral (videobalance).
    pub hue: f32,
    /// Gaussian blur (softens the image). Uses `gaussianblur`.
    pub blur: bool,
    /// Contrast-adaptive sharpening. Uses `cas`; skipped when the element is
    /// not installed so a missing optional effect never fails the capture.
    pub sharpen: bool,
}

impl Default for VideoEffects {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 0.0,
            saturation: 0.0,
            hue: 0.0,
            blur: false,
            sharpen: false,
        }
    }
}

impl VideoEffects {
    /// Whether any effect is active.
    pub fn is_empty(&self) -> bool {
        self.brightness.abs() < 0.01
            && self.contrast.abs() < 0.01
            && self.saturation.abs() < 0.01
            && self.hue.abs() < 0.01
            && !self.blur
            && !self.sharpen
    }

    /// Build the pipeline fragment (elements joined by ` ! `) using the real
    /// availability of the optional elements. Empty when no effect is active.
    pub fn fragment(&self) -> String {
        let _ = gst::init();
        self.fragment_with_available(|name| gst::ElementFactory::find(name).is_some())
    }

    /// Availability-aware variant used by [`VideoEffects::fragment`] and the
    /// tests: the `available` predicate decides which element factories exist.
    /// Optional elements that are not available are skipped rather than
    /// breaking the pipeline.
    pub fn fragment_with_available(&self, available: impl Fn(&str) -> bool) -> String {
        let mut parts: Vec<String> = Vec::new();

        let mut balance = "videobalance".to_string();
        let mut has_balance = false;
        if self.brightness.abs() >= 0.01 {
            balance.push_str(&format!(" brightness={:.3}", self.brightness));
            has_balance = true;
        }
        if self.contrast.abs() >= 0.01 {
            balance.push_str(&format!(" contrast={:.3}", self.contrast));
            has_balance = true;
        }
        if self.saturation.abs() >= 0.01 {
            balance.push_str(&format!(" saturation={:.3}", self.saturation));
            has_balance = true;
        }
        if self.hue.abs() >= 0.01 {
            balance.push_str(&format!(" hue={:.3}", self.hue));
            has_balance = true;
        }
        if has_balance {
            parts.push(balance);
        }
        if self.blur && available("gaussianblur") {
            parts.push("gaussianblur".to_string());
        }
        if self.sharpen && available("cas") {
            parts.push("cas".to_string());
        }

        parts.join(" ! ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_effects_are_empty() {
        let e = VideoEffects::default();
        assert!(e.is_empty());
        assert_eq!(e.fragment_with_available(|_| true), "");
    }

    #[test]
    fn balance_emits_only_active_properties() {
        let e = VideoEffects {
            brightness: 0.1,
            saturation: -0.25,
            ..Default::default()
        };
        let frag = e.fragment_with_available(|_| true);
        assert!(frag.contains("videobalance"), "{frag}");
        assert!(frag.contains("brightness=0.100"), "{frag}");
        assert!(frag.contains("saturation=-0.250"), "{frag}");
        assert!(!frag.contains("contrast="), "{frag}");
        assert!(!frag.contains("hue="), "{frag}");
    }

    #[test]
    fn blur_and_sharpen_are_availability_gated() {
        let e = VideoEffects {
            blur: true,
            sharpen: true,
            ..Default::default()
        };
        // All available -> both present.
        let frag = e.fragment_with_available(|_| true);
        assert!(frag.contains("gaussianblur"), "{frag}");
        assert!(frag.contains("cas"), "{frag}");
        // Neither available -> silently dropped (chain empty).
        let frag = e.fragment_with_available(|_| false);
        assert_eq!(frag, "");
        // cas missing (e.g. older gst-plugins-bad) -> only blur stays.
        let frag = e.fragment_with_available(|name| name != "cas");
        assert!(frag.contains("gaussianblur"), "{frag}");
        assert!(!frag.contains("cas"), "{frag}");
    }

    #[test]
    fn effects_chain_joins_with_pipeline_operator() {
        let e = VideoEffects {
            brightness: 0.2,
            blur: true,
            ..Default::default()
        };
        let frag = e.fragment_with_available(|_| true);
        assert!(frag.contains("brightness=0.200 ! gaussianblur"), "{frag}");
        // Balance stages come before the blur/sharpening stages.
        let ok_pos = frag.find("videobalance").unwrap();
        let blur_pos = frag.find("gaussianblur").unwrap();
        assert!(ok_pos < blur_pos, "{frag}");
        // Empty still returns empty (no dangling "!").
        assert_eq!(
            VideoEffects::default().fragment_with_available(|_| true),
            ""
        );
    }
}
