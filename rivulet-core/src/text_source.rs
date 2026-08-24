use std::time::Instant;

/// RGBA color with values in 0.0 – 1.0.
#[derive(Debug, Clone, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self {
            r: r.clamp(0.0, 1.0),
            g: g.clamp(0.0, 1.0),
            b: b.clamp(0.0, 1.0),
            a: a.clamp(0.0, 1.0),
        }
    }

    /// Pure black, fully opaque.
    pub fn black() -> Self {
        Self::new(0.0, 0.0, 0.0, 1.0)
    }

    /// Pure white, fully opaque.
    pub fn white() -> Self {
        Self::new(1.0, 1.0, 1.0, 1.0)
    }

    /// Transparent.
    pub fn transparent() -> Self {
        Self::new(0.0, 0.0, 0.0, 0.0)
    }

    /// Create from a `#RRGGBB` hex string.
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim_start_matches('#');
        if hex.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
        let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
        let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
        Some(Self::new(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            1.0,
        ))
    }
}

impl Default for Rgba {
    fn default() -> Self {
        Self::white()
    }
}

/// Font weight for the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FontWeight {
    Light,
    #[default]
    Normal,
    Bold,
    ExtraBold,
}

impl FontWeight {
    pub fn label(&self) -> &'static str {
        match self {
            FontWeight::Light => "Light",
            FontWeight::Normal => "Normal",
            FontWeight::Bold => "Bold",
            FontWeight::ExtraBold => "Extra Bold",
        }
    }
}

/// Horizontal text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl TextAlign {
    pub fn label(&self) -> &'static str {
        match self {
            TextAlign::Left => "Left",
            TextAlign::Center => "Center",
            TextAlign::Right => "Right",
        }
    }
}

/// Scrolling direction for the text source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScrollDirection {
    /// No scrolling.
    #[default]
    None,
    /// Scroll vertically from bottom to top (credits-style).
    Up,
    /// Scroll vertically from top to bottom.
    Down,
    /// Scroll horizontally from right to left (ticker-style).
    Left,
    /// Scroll horizontally from left to right.
    Right,
}

impl ScrollDirection {
    pub fn label(&self) -> &'static str {
        match self {
            ScrollDirection::None => "None",
            ScrollDirection::Up => "Scroll Up",
            ScrollDirection::Down => "Scroll Down",
            ScrollDirection::Left => "Scroll Left",
            ScrollDirection::Right => "Scroll Right",
        }
    }
}

/// A text source — displays styled text in a scene.
///
/// Supports font styling (size, weight, color), background/outline colors,
/// text alignment, and optional scrolling animation.
#[derive(Debug, Clone)]
pub struct TextSource {
    /// The text content.
    pub text: String,
    /// Font size in points.
    pub font_size: u32,
    /// Font name (e.g. "Arial", "Fira Code"). Empty = system default.
    pub font_name: String,
    /// Font weight.
    pub font_weight: FontWeight,
    /// Text (foreground) color.
    pub color: Rgba,
    /// Background color. Transparent = no background.
    pub background: Rgba,
    /// Outline (border) color. Transparent = no outline.
    pub outline: Rgba,
    /// Outline thickness in pixels.
    pub outline_width: f32,
    /// Horizontal text alignment.
    pub alignment: TextAlign,
    /// Padding inside the bounding box, in pixels.
    pub padding: f32,
    /// Scrolling direction.
    pub scroll: ScrollDirection,
    /// Scroll speed in pixels per second. Only used when scroll != None.
    pub scroll_speed: f32,
    /// Internal: accumulated scroll offset (pixels).
    scroll_offset: f32,
    /// Internal: timestamp of last update.
    last_update: Instant,
}

impl TextSource {
    /// Create a new text source with sensible defaults.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            font_size: 36,
            font_name: String::new(),
            font_weight: FontWeight::Normal,
            color: Rgba::white(),
            background: Rgba::transparent(),
            outline: Rgba::transparent(),
            outline_width: 0.0,
            alignment: TextAlign::Left,
            padding: 4.0,
            scroll: ScrollDirection::None,
            scroll_speed: 60.0,
            scroll_offset: 0.0,
            last_update: Instant::now(),
        }
    }

    /// Returns true if the text is empty or whitespace-only.
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// Returns the number of characters (not bytes) in the text.
    pub fn char_count(&self) -> usize {
        self.text.chars().count()
    }

    /// Returns the number of lines in the text.
    pub fn line_count(&self) -> usize {
        if self.text.is_empty() {
            0
        } else {
            self.text.lines().count()
        }
    }

    /// Update the scroll offset based on elapsed time. Returns the new offset.
    pub fn update_scroll(&mut self) -> f32 {
        if self.scroll == ScrollDirection::None {
            self.scroll_offset = 0.0;
            return 0.0;
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last_update).as_secs_f32();
        self.last_update = now;
        self.scroll_offset += self.scroll_speed * dt;
        self.scroll_offset
    }

    /// Reset the scroll offset to zero.
    pub fn reset_scroll(&mut self) {
        self.scroll_offset = 0.0;
        self.last_update = Instant::now();
    }

    /// Returns the current scroll offset.
    pub fn scroll_offset(&self) -> f32 {
        self.scroll_offset
    }

    /// Returns a summary string for debugging/UI display.
    pub fn summary(&self) -> String {
        let lines = self.line_count();
        let chars = self.char_count();
        let preview = if self.text.len() > 40 {
            format!("{}…", &self.text[..40])
        } else {
            self.text.clone()
        };
        format!("{chars} chars, {lines} lines: {preview}")
    }
}

impl Default for TextSource {
    fn default() -> Self {
        Self::new("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // ── Rgba ──────────────────────────────────────────────────────

    #[test]
    fn rgba_new_clamps_values() {
        let c = Rgba::new(2.0, -0.5, 0.5, 1.5);
        assert_eq!(c.r, 1.0);
        assert_eq!(c.g, 0.0);
        assert_eq!(c.b, 0.5);
        assert_eq!(c.a, 1.0);
    }

    #[test]
    fn rgba_black_white_transparent() {
        assert_eq!(Rgba::black(), Rgba::new(0.0, 0.0, 0.0, 1.0));
        assert_eq!(Rgba::white(), Rgba::new(1.0, 1.0, 1.0, 1.0));
        assert_eq!(Rgba::transparent(), Rgba::new(0.0, 0.0, 0.0, 0.0));
    }

    #[test]
    fn rgba_default_is_white() {
        assert_eq!(Rgba::default(), Rgba::white());
    }

    #[test]
    fn rgba_from_hex_valid() {
        let c = Rgba::from_hex("#FF8800").unwrap();
        assert!((c.r - 1.0).abs() < 0.01);
        assert!((c.g - 0x88 as f32 / 255.0).abs() < 0.01);
        assert!((c.b - 0.0).abs() < 0.01);
        assert_eq!(c.a, 1.0);
    }

    #[test]
    fn rgba_from_hex_without_hash() {
        let c = Rgba::from_hex("00FF00").unwrap();
        assert!((c.r - 0.0).abs() < 0.01);
        assert!((c.g - 1.0).abs() < 0.01);
        assert!((c.b - 0.0).abs() < 0.01);
    }

    #[test]
    fn rgba_from_hex_invalid() {
        assert!(Rgba::from_hex("#GGG").is_none());
        assert!(Rgba::from_hex("#12").is_none());
        assert!(Rgba::from_hex("longstring").is_none());
    }

    // ── FontWeight ────────────────────────────────────────────────

    #[test]
    fn font_weight_labels() {
        assert_eq!(FontWeight::Light.label(), "Light");
        assert_eq!(FontWeight::Normal.label(), "Normal");
        assert_eq!(FontWeight::Bold.label(), "Bold");
        assert_eq!(FontWeight::ExtraBold.label(), "Extra Bold");
    }

    #[test]
    fn font_weight_default_is_normal() {
        assert_eq!(FontWeight::default(), FontWeight::Normal);
    }

    // ── TextAlign ─────────────────────────────────────────────────

    #[test]
    fn text_align_labels() {
        assert_eq!(TextAlign::Left.label(), "Left");
        assert_eq!(TextAlign::Center.label(), "Center");
        assert_eq!(TextAlign::Right.label(), "Right");
    }

    #[test]
    fn text_align_default_is_left() {
        assert_eq!(TextAlign::default(), TextAlign::Left);
    }

    // ── ScrollDirection ───────────────────────────────────────────

    #[test]
    fn scroll_direction_labels() {
        assert_eq!(ScrollDirection::None.label(), "None");
        assert_eq!(ScrollDirection::Up.label(), "Scroll Up");
        assert_eq!(ScrollDirection::Down.label(), "Scroll Down");
        assert_eq!(ScrollDirection::Left.label(), "Scroll Left");
        assert_eq!(ScrollDirection::Right.label(), "Scroll Right");
    }

    #[test]
    fn scroll_direction_default_is_none() {
        assert_eq!(ScrollDirection::default(), ScrollDirection::None);
    }

    // ── TextSource creation ───────────────────────────────────────

    #[test]
    fn text_source_new_defaults() {
        let ts = TextSource::new("Hello");
        assert_eq!(ts.text, "Hello");
        assert_eq!(ts.font_size, 36);
        assert_eq!(ts.font_weight, FontWeight::Normal);
        assert_eq!(ts.color, Rgba::white());
        assert_eq!(ts.background, Rgba::transparent());
        assert_eq!(ts.alignment, TextAlign::Left);
        assert_eq!(ts.scroll, ScrollDirection::None);
        assert_eq!(ts.scroll_speed, 60.0);
    }

    #[test]
    fn text_source_default_is_empty() {
        let ts = TextSource::default();
        assert!(ts.is_empty());
        assert_eq!(ts.text, "");
    }

    #[test]
    fn text_source_is_empty_whitespace() {
        let ts = TextSource::new("   \n\t  ");
        assert!(ts.is_empty());
    }

    #[test]
    fn text_source_is_not_empty() {
        let ts = TextSource::new("Hello World");
        assert!(!ts.is_empty());
    }

    #[test]
    fn text_source_char_count() {
        let ts = TextSource::new("Hello");
        assert_eq!(ts.char_count(), 5);
        let ts2 = TextSource::new("Ünïcödé");
        assert_eq!(ts2.char_count(), 7);
    }

    #[test]
    fn text_source_line_count() {
        let ts = TextSource::new("Line1\nLine2\nLine3");
        assert_eq!(ts.line_count(), 3);
    }

    #[test]
    fn text_source_line_count_empty() {
        let ts = TextSource::new("");
        assert_eq!(ts.line_count(), 0);
    }

    #[test]
    fn text_source_line_count_single() {
        let ts = TextSource::new("Single line");
        assert_eq!(ts.line_count(), 1);
    }

    // ── TextSource scrolling ──────────────────────────────────────

    #[test]
    fn text_source_no_scroll_stays_zero() {
        let mut ts = TextSource::new("Static text");
        ts.scroll = ScrollDirection::None;
        let offset = ts.update_scroll();
        assert_eq!(offset, 0.0);
        assert_eq!(ts.scroll_offset(), 0.0);
    }

    #[test]
    fn text_source_scroll_increments_offset() {
        let mut ts = TextSource::new("Scrolling");
        ts.scroll = ScrollDirection::Up;
        ts.scroll_speed = 100.0;
        // First call sets baseline
        ts.update_scroll();
        std::thread::sleep(Duration::from_millis(50));
        let offset = ts.update_scroll();
        assert!(
            offset > 0.0,
            "scroll offset should be positive, got {offset}"
        );
    }

    #[test]
    fn text_source_reset_scroll() {
        let mut ts = TextSource::new("Reset me");
        ts.scroll = ScrollDirection::Up;
        ts.scroll_speed = 100.0;
        ts.update_scroll();
        std::thread::sleep(Duration::from_millis(20));
        ts.update_scroll();
        assert!(ts.scroll_offset() > 0.0);

        ts.reset_scroll();
        assert_eq!(ts.scroll_offset(), 0.0);
    }

    // ── TextSource summary ────────────────────────────────────────

    #[test]
    fn text_source_summary_short() {
        let ts = TextSource::new("Hi");
        let s = ts.summary();
        assert!(s.contains("2 chars"));
        assert!(s.contains("1 lines"));
        assert!(s.contains("Hi"));
    }

    #[test]
    fn text_source_summary_long_truncates() {
        let long_text = "A".repeat(60);
        let ts = TextSource::new(&long_text);
        let s = ts.summary();
        assert!(s.contains("60 chars"));
        assert!(s.ends_with('…'));
    }

    #[test]
    fn text_source_summary_multiline() {
        let ts = TextSource::new("a\nb\nc");
        let s = ts.summary();
        assert!(s.contains("3 lines"));
    }

    // ── TextSource builder-style setters (via field mutation) ─────

    #[test]
    fn text_source_custom_styling() {
        let mut ts = TextSource::new("Styled");
        ts.font_size = 72;
        ts.font_name = "Fira Code".to_string();
        ts.font_weight = FontWeight::Bold;
        ts.color = Rgba::new(1.0, 0.0, 0.0, 1.0);
        ts.background = Rgba::new(0.0, 0.0, 0.0, 0.8);
        ts.outline = Rgba::white();
        ts.outline_width = 2.0;
        ts.alignment = TextAlign::Center;
        ts.padding = 16.0;

        assert_eq!(ts.font_size, 72);
        assert_eq!(ts.font_name, "Fira Code");
        assert_eq!(ts.font_weight, FontWeight::Bold);
        assert_eq!(ts.color, Rgba::new(1.0, 0.0, 0.0, 1.0));
        assert_eq!(ts.background.a, 0.8);
        assert_eq!(ts.outline_width, 2.0);
        assert_eq!(ts.alignment, TextAlign::Center);
        assert_eq!(ts.padding, 16.0);
    }

    #[test]
    fn text_source_scrolling_config() {
        let mut ts = TextSource::new("Ticker");
        ts.scroll = ScrollDirection::Left;
        ts.scroll_speed = 200.0;
        assert_eq!(ts.scroll, ScrollDirection::Left);
        assert_eq!(ts.scroll_speed, 200.0);
    }

    // ── clone and debug ───────────────────────────────────────────

    #[test]
    fn text_source_clone_preserves_all_fields() {
        let mut ts = TextSource::new("Clone me");
        ts.font_size = 48;
        ts.scroll = ScrollDirection::Up;
        ts.scroll_offset = 42.0;

        let ts2 = ts.clone();
        assert_eq!(ts2.text, "Clone me");
        assert_eq!(ts2.font_size, 48);
        assert_eq!(ts2.scroll, ScrollDirection::Up);
        assert_eq!(ts2.scroll_offset, 42.0);
    }

    #[test]
    fn text_source_debug_impl() {
        let ts = TextSource::new("Debug");
        let dbg = format!("{ts:?}");
        assert!(dbg.contains("TextSource"));
        assert!(dbg.contains("Debug"));
    }
}
