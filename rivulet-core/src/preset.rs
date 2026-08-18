//! Recording presets that bundle resolution, FPS, and bitrate into a single
//! selectable configuration. Each preset targets a common recording scenario
//! (e.g. 1080p60 for high-quality game capture, 720p30 for lightweight
//! tutorials).

use serde::{Deserialize, Serialize};

/// A predefined recording configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingPreset {
    /// Human-readable label shown in the UI.
    pub label: &'static str,
    /// Target width in pixels. `None` means "use source resolution".
    pub width: Option<u32>,
    /// Target height in pixels. `None` means "use source resolution".
    pub height: Option<u32>,
    /// Target frame rate. `None` means "use source FPS" (30 fps fallback).
    pub fps: Option<u32>,
    /// Target bitrate in kbit/s. `None` means "use engine default" (5000).
    pub bitrate_kbps: Option<u32>,
}

impl RecordingPreset {
    /// All built-in presets in display order.
    pub fn all() -> &'static [RecordingPreset] {
        &[
            Self::ORIGINAL,
            Self::P1080P60,
            Self::P1080P30,
            Self::P720P60,
            Self::P720P30,
            Self::P480P30,
        ]
    }

    /// Effective width: preset value or fallback to source.
    pub fn effective_width(self, source_width: u32) -> u32 {
        self.width.unwrap_or(source_width)
    }

    /// Effective height: preset value or fallback to source.
    pub fn effective_height(self, source_height: u32) -> u32 {
        self.height.unwrap_or(source_height)
    }

    /// Effective FPS: preset value or fallback.
    pub fn effective_fps(self, fallback_fps: u32) -> u32 {
        self.fps.unwrap_or(fallback_fps)
    }

    /// Effective bitrate: preset value or fallback to engine default.
    pub fn effective_bitrate_kbps(self, fallback_kbps: u32) -> u32 {
        self.bitrate_kbps.unwrap_or(fallback_kbps)
    }

    /// Whether the preset changes resolution or FPS from the source.
    pub fn transforms_video(self) -> bool {
        self.width.is_some() || self.height.is_some() || self.fps.is_some()
    }

    /// Build the GStreamer caps fragment for this preset, or `None` when no
    /// transformation is needed.
    pub fn caps_fragment(self) -> Option<String> {
        if !self.transforms_video() {
            return None;
        }
        let mut parts = Vec::new();
        if let Some(w) = self.width {
            parts.push(format!("width={}", w));
        }
        if let Some(h) = self.height {
            parts.push(format!("height={}", h));
        }
        if let Some(fps) = self.fps {
            parts.push(format!("framerate={}/1", fps));
        }
        Some(format!("video/x-raw,{}", parts.join(",")))
    }

    /// Build the `videoscale ! videorate ! <caps>` pipeline fragment, or an
    /// empty string when the preset matches the source.
    pub fn transform_fragment(self) -> String {
        match self.caps_fragment() {
            Some(caps) => format!("! videoscale ! videorate ! {} ", caps),
            None => String::new(),
        }
    }
}

/// "Original" preset — no transformation, use source resolution and FPS.
pub const ORIGINAL: RecordingPreset = RecordingPreset {
    label: "Original",
    width: None,
    height: None,
    fps: None,
    bitrate_kbps: None,
};

/// "Custom" placeholder for user-defined settings in the future.
pub const CUSTOM: RecordingPreset = RecordingPreset {
    label: "Custom",
    width: None,
    height: None,
    fps: None,
    bitrate_kbps: None,
};

impl RecordingPreset {
    pub const ORIGINAL: RecordingPreset = ORIGINAL;
    pub const P1080P60: RecordingPreset = RecordingPreset {
        label: "1080p60",
        width: Some(1920),
        height: Some(1080),
        fps: Some(60),
        bitrate_kbps: Some(8_000),
    };
    pub const P1080P30: RecordingPreset = RecordingPreset {
        label: "1080p30",
        width: Some(1920),
        height: Some(1080),
        fps: Some(30),
        bitrate_kbps: Some(5_000),
    };
    pub const P720P60: RecordingPreset = RecordingPreset {
        label: "720p60",
        width: Some(1280),
        height: Some(720),
        fps: Some(60),
        bitrate_kbps: Some(4_500),
    };
    pub const P720P30: RecordingPreset = RecordingPreset {
        label: "720p30",
        width: Some(1280),
        height: Some(720),
        fps: Some(30),
        bitrate_kbps: Some(2_500),
    };
    pub const P480P30: RecordingPreset = RecordingPreset {
        label: "480p30",
        width: Some(640),
        height: Some(480),
        fps: Some(30),
        bitrate_kbps: Some(1_500),
    };
}

impl Default for RecordingPreset {
    fn default() -> Self {
        Self::ORIGINAL
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_has_no_transform() {
        assert!(!RecordingPreset::ORIGINAL.transforms_video());
        assert!(!RecordingPreset::ORIGINAL.caps_fragment().is_some());
        assert!(RecordingPreset::ORIGINAL.transform_fragment().is_empty());
    }

    #[test]
    fn preset_1080p60_dimensions() {
        let p = RecordingPreset::P1080P60;
        assert_eq!(p.effective_width(3840), 1920);
        assert_eq!(p.effective_height(2160), 1080);
        assert_eq!(p.effective_fps(30), 60);
        assert_eq!(p.effective_bitrate_kbps(5000), 8000);
    }

    #[test]
    fn preset_falls_back_to_source() {
        let p = RecordingPreset::ORIGINAL;
        assert_eq!(p.effective_width(3840), 3840);
        assert_eq!(p.effective_height(2160), 2160);
        assert_eq!(p.effective_fps(30), 30);
        assert_eq!(p.effective_bitrate_kbps(5000), 5000);
    }

    #[test]
    fn preset_720p30_dimensions() {
        let p = RecordingPreset::P720P30;
        assert_eq!(p.effective_width(1920), 1280);
        assert_eq!(p.effective_height(1080), 720);
        assert_eq!(p.effective_fps(60), 30);
        assert_eq!(p.effective_bitrate_kbps(8000), 2500);
    }

    #[test]
    fn preset_480p30_dimensions() {
        let p = RecordingPreset::P480P30;
        assert_eq!(p.effective_width(1920), 640);
        assert_eq!(p.effective_height(1080), 480);
        assert_eq!(p.effective_bitrate_kbps(5000), 1500);
    }

    #[test]
    fn caps_fragment_contains_resolution_and_fps() {
        let caps = RecordingPreset::P1080P60.caps_fragment().unwrap();
        assert!(caps.contains("width=1920"), "{caps}");
        assert!(caps.contains("height=1080"), "{caps}");
        assert!(caps.contains("framerate=60/1"), "{caps}");
    }

    #[test]
    fn transform_fragment_includes_videoscale_and_videorate() {
        let frag = RecordingPreset::P720P30.transform_fragment();
        assert!(frag.contains("videoscale"), "{frag}");
        assert!(frag.contains("videorate"), "{frag}");
        assert!(frag.contains("width=1280"), "{frag}");
    }

    #[test]
    fn all_presets_are_distinct() {
        let labels: Vec<_> = RecordingPreset::all().iter().map(|p| p.label).collect();
        let mut sorted = labels.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(labels.len(), sorted.len(), "duplicate preset labels");
    }

    #[test]
    fn default_is_original() {
        assert_eq!(RecordingPreset::default(), RecordingPreset::ORIGINAL);
    }

    #[test]
    fn bitrate_presets_are_sensible() {
        for p in RecordingPreset::all() {
            if let Some(bitrate) = p.bitrate_kbps {
                assert!(bitrate > 0 && bitrate <= 50_000, "{}: {}", p.label, bitrate);
            }
        }
    }
}
