use crate::text_source::Rgba;

/// A color source — renders a solid color rectangle as a scene background or banner.
///
/// Used for lower thirds, backgrounds, overlays, or transition screens.
/// The actual rendering is handled by the scene compositor.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorSource {
    /// Background color.
    pub color: Rgba,
    /// Width in pixels (0 = scene width).
    pub width: u32,
    /// Height in pixels (0 = scene height).
    pub height: u32,
    /// Opacity override (0.0–1.0). If `None`, uses the color's alpha channel.
    pub opacity: Option<f32>,
}

impl ColorSource {
    /// Create a new color source with the given color and full-scene dimensions.
    pub fn new(color: Rgba) -> Self {
        Self {
            color,
            width: 0,
            height: 0,
            opacity: None,
        }
    }

    /// Create a color source with explicit dimensions.
    pub fn with_size(color: Rgba, width: u32, height: u32) -> Self {
        Self {
            color,
            width,
            height,
            opacity: None,
        }
    }

    /// Create from a hex color string (e.g. "#FF8800" or "FF8800").
    pub fn from_hex(hex: &str) -> Option<Self> {
        Rgba::from_hex(hex).map(Self::new)
    }

    /// Effective opacity: explicit override or the color's own alpha.
    pub fn effective_opacity(&self) -> f32 {
        self.opacity.unwrap_or(self.color.a)
    }

    /// Whether the dimensions are scene-filling (0 = scene size).
    pub fn is_scene_fill(&self) -> bool {
        self.width == 0 && self.height == 0
    }

    /// Returns a summary string for UI display.
    pub fn summary(&self) -> String {
        let (r, g, b) = (
            (self.color.r * 255.0) as u8,
            (self.color.g * 255.0) as u8,
            (self.color.b * 255.0) as u8,
        );
        let size = if self.is_scene_fill() {
            "fill".to_string()
        } else {
            format!("{}×{}", self.width, self.height)
        };
        format!("#{:02X}{:02X}{:02X} ({})", r, g, b, size)
    }
}

impl Default for ColorSource {
    fn default() -> Self {
        Self::new(Rgba::black())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Creation ─────────────────────────────────────────────────

    #[test]
    fn color_source_new_defaults() {
        let cs = ColorSource::new(Rgba::white());
        assert_eq!(cs.color, Rgba::white());
        assert_eq!(cs.width, 0);
        assert_eq!(cs.height, 0);
        assert_eq!(cs.opacity, None);
    }

    #[test]
    fn color_source_with_size() {
        let cs = ColorSource::with_size(Rgba::new(1.0, 0.0, 0.0, 1.0), 1920, 1080);
        assert_eq!(cs.width, 1920);
        assert_eq!(cs.height, 1080);
        assert_eq!(cs.color.r, 1.0);
    }

    #[test]
    fn color_source_from_hex_valid() {
        let cs = ColorSource::from_hex("#00FF88").unwrap();
        assert!((cs.color.r - 0.0).abs() < 0.01);
        assert!((cs.color.g - 1.0).abs() < 0.01);
        assert!((cs.color.b - 0x88 as f32 / 255.0).abs() < 0.01);
    }

    #[test]
    fn color_source_from_hex_invalid() {
        assert!(ColorSource::from_hex("GGG").is_none());
    }

    #[test]
    fn color_source_default_is_black_fill() {
        let cs = ColorSource::default();
        assert_eq!(cs.color, Rgba::black());
        assert!(cs.is_scene_fill());
    }

    // ── Effective opacity ────────────────────────────────────────

    #[test]
    fn effective_opacity_uses_color_alpha_when_none() {
        let cs = ColorSource::new(Rgba::new(1.0, 0.0, 0.0, 0.5));
        assert!((cs.effective_opacity() - 0.5).abs() < 0.01);
    }

    #[test]
    fn effective_opacity_prefers_explicit_override() {
        let mut cs = ColorSource::new(Rgba::new(1.0, 0.0, 0.0, 0.5));
        cs.opacity = Some(0.8);
        assert!((cs.effective_opacity() - 0.8).abs() < 0.01);
    }

    // ── Scene fill ───────────────────────────────────────────────

    #[test]
    fn is_scene_fill_when_zero() {
        let cs = ColorSource::new(Rgba::white());
        assert!(cs.is_scene_fill());
    }

    #[test]
    fn is_not_scene_fill_when_sized() {
        let cs = ColorSource::with_size(Rgba::white(), 640, 480);
        assert!(!cs.is_scene_fill());
    }

    // ── Summary ──────────────────────────────────────────────────

    #[test]
    fn summary_fill() {
        let cs = ColorSource::new(Rgba::black());
        let s = cs.summary();
        assert!(s.contains("#000000"));
        assert!(s.contains("fill"));
    }

    #[test]
    fn summary_sized() {
        let cs = ColorSource::with_size(Rgba::white(), 800, 600);
        let s = cs.summary();
        assert!(s.contains("#FFFFFF"));
        assert!(s.contains("800×600"));
    }

    #[test]
    fn summary_custom_color() {
        let cs = ColorSource::new(Rgba::new(1.0, 0.5, 0.0, 1.0));
        let s = cs.summary();
        assert!(s.contains("#FF7F00"));
    }

    // ── Clone, PartialEq, Debug ──────────────────────────────────

    #[test]
    fn color_source_clone() {
        let cs = ColorSource::with_size(Rgba::new(0.5, 0.5, 0.5, 1.0), 100, 100);
        let cs2 = cs.clone();
        assert_eq!(cs, cs2);
    }

    #[test]
    fn color_source_debug() {
        let cs = ColorSource::new(Rgba::new(1.0, 0.0, 0.0, 1.0));
        let dbg = format!("{cs:?}");
        assert!(dbg.contains("ColorSource"));
    }
}
