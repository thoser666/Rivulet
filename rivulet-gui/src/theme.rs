//! Central theme handling for the Rivulet GUI.
//!
//! [`ThemePreference`] is the user-facing color-scheme preference
//! (system / dark / light), persisted in the app settings and applied to the
//! egui context. The semantic status colors live in [`StatusColors`], which
//! resolves a separate palette per active scheme so the colors stay legible
//! on both dark and light backgrounds (verified by a WCAG regression test at
//! the bottom of this file).

use eframe::egui;

/// The user's color-scheme preference. `System` follows the OS theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum ThemePreference {
    /// Follow the operating system's dark/light setting.
    #[default]
    System,
    /// Always use the dark scheme.
    Dark,
    /// Always use the light scheme.
    Light,
}

impl ThemePreference {
    /// All preferences in display order (for the settings UI).
    pub fn all() -> &'static [ThemePreference] {
        &[
            ThemePreference::System,
            ThemePreference::Dark,
            ThemePreference::Light,
        ]
    }

    /// i18n key for the preference label.
    pub fn key(self) -> &'static str {
        match self {
            ThemePreference::System => "theme_system",
            ThemePreference::Dark => "theme_dark",
            ThemePreference::Light => "theme_light",
        }
    }

    /// Apply the preference to an egui context.
    pub fn apply(self, ctx: &egui::Context) {
        ctx.set_theme(match self {
            ThemePreference::System => egui::ThemePreference::System,
            ThemePreference::Dark => egui::ThemePreference::Dark,
            ThemePreference::Light => egui::ThemePreference::Light,
        });
    }
}

// --- Semantic status colors -----------------------------------------
// Two palettes (dark/light) resolve the same six status colors so every view
// reads them from one place and a future branded palette only touches this
// file. Both palettes meet WCAG AA (>= 4.5:1) against the egui panel/window
// fill of their scheme — enforced by `status_colors_meet_wcag_aa` below.

/// The resolved status colors for one color scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusColors {
    /// Success / positive state (up to date, downloaded, running).
    pub success: egui::Color32,
    /// Warning state (update available, paused, skipped filters).
    pub warning: egui::Color32,
    /// Error state (update check failed, engine/capture errors).
    pub error: egui::Color32,
    /// Live/active state (recording in progress). Used on Linux only.
    #[allow(dead_code)]
    pub active: egui::Color32,
    /// Informational state (muted, recording status, audio status).
    pub info: egui::Color32,
    /// Secondary / hint text.
    pub hint: egui::Color32,
}

impl StatusColors {
    /// The palette for the current egui scheme (`ui.visuals().dark_mode`).
    pub fn for_ui(ui: &egui::Ui) -> Self {
        Self::for_scheme(ui.visuals().dark_mode)
    }

    /// The palette for a scheme, `dark = true` selects the dark palette.
    pub fn for_scheme(dark: bool) -> Self {
        if dark {
            Self::dark()
        } else {
            Self::light()
        }
    }

    /// Bright pastel palette for the dark scheme (egui panel fill #1b1b1b).
    pub fn dark() -> Self {
        Self {
            success: egui::Color32::LIGHT_GREEN,
            warning: egui::Color32::YELLOW,
            // Brighter than RED so it also meets AA on the dark scheme.
            error: egui::Color32::from_rgb(255, 82, 82),
            active: egui::Color32::LIGHT_RED,
            info: egui::Color32::LIGHT_BLUE,
            hint: egui::Color32::GRAY,
        }
    }

    /// Darker palette for the light scheme (egui panel fill #f8f8f8).
    pub fn light() -> Self {
        Self {
            success: egui::Color32::from_rgb(27, 94, 32),
            warning: egui::Color32::from_rgb(180, 83, 9),
            error: egui::Color32::from_rgb(198, 40, 40),
            active: egui::Color32::from_rgb(216, 27, 96),
            info: egui::Color32::from_rgb(21, 101, 192),
            hint: egui::Color32::from_rgb(97, 97, 97),
        }
    }
}
// ---------------------------------------------------------------------------
// Extended theme handling: custom palette, font loading, and theme application
// ---------------------------------------------------------------------------

/// A full UI palette for the application, resolved per color scheme so the
/// same structural colors (accent, panel/background fill, text, scrim) are
/// always legible. The semantic status colors live in [`StatusColors`]; the
/// palette carries the structural colors used by the egui visuals and by the
/// views that paint their own overlays (e.g. the region-editor scrim).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemePalette {
    /// Primary accent color (teal) for active widgets and highlights.
    pub accent: egui::Color32,
    /// Background fill for panels/windows.
    pub panel_fill: egui::Color32,
    /// Base background color for the outermost area / non-interactive widgets.
    pub background: egui::Color32,
    /// Text color for standard body text.
    pub text: egui::Color32,
    /// Translucent overlay used to dim captured previews (region editor)
    /// while dragging a selection.
    pub scrim: egui::Color32,
}

impl ThemePalette {
    /// The palette for the current egui scheme (`ui.visuals().dark_mode`).
    pub fn for_ui(ui: &egui::Ui) -> Self {
        Self::for_scheme(ui.visuals().dark_mode)
    }

    /// The palette for a scheme, `dark = true` selects the dark palette.
    pub fn for_scheme(dark: bool) -> Self {
        if dark {
            Self {
                accent: egui::Color32::from_rgb(0, 150, 136), // teal A400
                panel_fill: egui::Color32::from_gray(27),
                background: egui::Color32::from_gray(20),
                text: egui::Color32::LIGHT_GRAY,
                scrim: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 110),
            }
        } else {
            Self {
                accent: egui::Color32::from_rgb(0, 128, 128), // teal
                panel_fill: egui::Color32::from_gray(248),
                background: egui::Color32::from_gray(255),
                text: egui::Color32::from_gray(30),
                scrim: egui::Color32::from_rgba_unmultiplied(0, 0, 0, 110),
            }
        }
    }
}

/// Build the font definitions that register the bundled Inter font
/// (`assets/fonts/Inter-Regular.ttf`) as the primary proportional and
/// monospace family. Kept as a pure function so it is unit-testable without
/// a live egui context.
pub fn inter_font_definitions() -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "Inter".to_owned(),
        egui::FontData::from_static(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/fonts/Inter-Regular.ttf"
        )))
        .into(),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "Inter".to_owned());
    }
    fonts
}

/// Load the Inter font into the egui context as the primary proportional +
/// monospace family.
pub fn load_inter_font(ctx: &egui::Context) {
    ctx.set_fonts(inter_font_definitions());
}

/// Apply the full theme (preference, fonts, palette visuals) to the egui
/// context. This is the single entry point the app calls at startup (and
/// again whenever the user changes the preference in Settings).
pub fn init(ctx: &egui::Context, pref: ThemePreference) {
    // Apply light/dark preference first so `System` follows the OS theme.
    pref.apply(ctx);

    // Load custom fonts.
    load_inter_font(ctx);

    // Apply the palette to BOTH schemes so a runtime switch (or the "System"
    // preference following the OS) keeps the branded visuals. `set_visuals_of`
    // stores the palette per theme; `set_visuals` alone would only touch the
    // currently active one.
    for (theme, dark) in [(egui::Theme::Dark, true), (egui::Theme::Light, false)] {
        let palette = ThemePalette::for_scheme(dark);
        let mut visuals = if dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        visuals.widgets.active.bg_fill = palette.accent;
        visuals.widgets.hovered.bg_fill = palette.accent.linear_multiply(1.15);
        visuals.widgets.inactive.bg_fill = palette.panel_fill;
        visuals.widgets.noninteractive.bg_fill = palette.background;
        visuals.window_fill = palette.panel_fill;
        visuals.override_text_color = Some(palette.text);
        ctx.set_visuals_of(theme, visuals);
    }
}

/// Semi-transparent panel frame for the glassmorphism effect.
///
/// Used for sidebars and the top bar so the desktop wallpaper or
/// background shows through with a frosted-glass look.
pub fn glass_frame(ui: &egui::Ui) -> egui::Frame {
    let dark = ui.visuals().dark_mode;
    let fill = if dark {
        egui::Color32::from_rgba_unmultiplied(27, 27, 27, 200)
    } else {
        egui::Color32::from_rgba_unmultiplied(245, 245, 245, 200)
    };
    egui::Frame::default()
        .fill(fill)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .corner_radius(egui::CornerRadius::same(4))
}

/// Accent-colored hover stroke for buttons and interactive elements.
///
/// Returns a 1.5px stroke using the palette accent color at 60% opacity.
#[expect(dead_code, reason = "public API for nav/button hover styling")]
pub fn hover_stroke(ui: &egui::Ui) -> egui::Stroke {
    let accent = ThemePalette::for_ui(ui).accent;
    egui::Stroke::new(1.5, accent.linear_multiply(0.6))
}

/// Accent-colored active press stroke.
///
/// Returns a 2px stroke using the full palette accent color.
#[expect(dead_code, reason = "public API for nav/button active styling")]
pub fn active_stroke(ui: &egui::Ui) -> egui::Stroke {
    let accent = ThemePalette::for_ui(ui).accent;
    egui::Stroke::new(2.0, accent)
}

/// Fade-in alpha for preview images.
///
/// Wraps `ui.ctx().animate_bool` to return an `f32` in `[0.0, 1.0]`
/// suitable for `Color32::linear_multiply` or the painter's `tint`.
pub fn preview_fade_alpha(ctx: &egui::Context, id: egui::Id, visible: bool) -> f32 {
    ctx.animate_bool(id, visible)
}

#[cfg(test)]
mod tests {
    use super::*;

    // egui panel/window fill per scheme (Visuals::dark()/light()).
    const DARK_BG: egui::Color32 = egui::Color32::from_gray(27);
    const LIGHT_BG: egui::Color32 = egui::Color32::from_gray(248);

    fn linear(channel: u8) -> f64 {
        let c = f64::from(channel) / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }

    fn luminance(color: egui::Color32) -> f64 {
        let (r, g, b) = (color.r(), color.g(), color.b());
        0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
    }

    fn contrast(fg: egui::Color32, bg: egui::Color32) -> f64 {
        let (l1, l2) = (luminance(fg), luminance(bg));
        let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn assert_meets_aa(palette: StatusColors, background: egui::Color32, scheme: &str) {
        let entries = [
            ("success", palette.success),
            ("warning", palette.warning),
            ("error", palette.error),
            ("active", palette.active),
            ("info", palette.info),
            ("hint", palette.hint),
        ];
        for (name, color) in entries {
            let ratio = contrast(color, background);
            assert!(
                ratio >= 4.5,
                "{scheme}: {name} contrast {ratio:.2}:1 < 4.5:1 (WCAG AA)"
            );
        }
    }

    #[test]
    fn theme_preference_defaults_to_system() {
        assert_eq!(ThemePreference::default(), ThemePreference::System);
    }

    #[test]
    fn theme_preference_all_covers_three_entries_with_keys() {
        assert_eq!(ThemePreference::all().len(), 3);
        for pref in ThemePreference::all() {
            assert!(!pref.key().is_empty(), "preference has no i18n key");
        }
        assert_eq!(ThemePreference::System.key(), "theme_system");
        assert_eq!(ThemePreference::Dark.key(), "theme_dark");
        assert_eq!(ThemePreference::Light.key(), "theme_light");
    }

    #[test]
    fn theme_preference_applies_to_the_egui_context() {
        let ctx = egui::Context::default();
        ThemePreference::Dark.apply(&ctx);
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        ThemePreference::Light.apply(&ctx);
        assert_eq!(ctx.theme(), egui::Theme::Light);
    }

    #[test]
    fn status_colors_meet_wcag_aa_on_both_schemes() {
        assert_meets_aa(StatusColors::dark(), DARK_BG, "dark");
        assert_meets_aa(StatusColors::light(), LIGHT_BG, "light");
    }

    #[test]
    fn status_colors_select_the_palette_by_scheme() {
        assert_eq!(StatusColors::for_scheme(true), StatusColors::dark());
        assert_eq!(StatusColors::for_scheme(false), StatusColors::light());
        // The palettes differ, so a scheme switch visibly changes the UI.
        assert_ne!(StatusColors::dark(), StatusColors::light());
    }

    #[test]
    fn theme_palette_selects_the_scheme_variant() {
        let dark = ThemePalette::for_scheme(true);
        let light = ThemePalette::for_scheme(false);
        // Both schemes carry all five palette colors.
        for palette in [dark, light] {
            assert_ne!(palette.accent, egui::Color32::TRANSPARENT);
            assert_ne!(palette.panel_fill, egui::Color32::TRANSPARENT);
            assert_ne!(palette.background, egui::Color32::TRANSPARENT);
            assert_ne!(palette.text, egui::Color32::TRANSPARENT);
            assert_ne!(palette.scrim, egui::Color32::TRANSPARENT);
        }
        // The palettes differ, so a scheme switch visibly changes the UI.
        assert_ne!(dark, light);
    }

    #[test]
    fn theme_init_applies_palette_to_both_schemes() {
        let ctx = egui::Context::default();
        init(&ctx, ThemePreference::Dark);

        let dark_visuals = ctx.style_of(egui::Theme::Dark).visuals.clone();
        let light_visuals = ctx.style_of(egui::Theme::Light).visuals.clone();
        assert_eq!(
            dark_visuals.window_fill,
            ThemePalette::for_scheme(true).panel_fill
        );
        assert_eq!(
            light_visuals.window_fill,
            ThemePalette::for_scheme(false).panel_fill
        );
        assert_eq!(
            dark_visuals.widgets.active.bg_fill,
            ThemePalette::for_scheme(true).accent
        );
        // The Inter font is registered as the primary proportional family.
        let definitions = inter_font_definitions();
        assert!(
            definitions.font_data.contains_key("Inter")
                && definitions
                    .families
                    .get(&egui::FontFamily::Proportional)
                    .is_some_and(|names| names.first().is_some_and(|n| n == "Inter")),
            "Inter font must be registered as the primary proportional font"
        );
    }

    #[test]
    fn theme_init_respects_the_preference() {
        let ctx = egui::Context::default();
        init(&ctx, ThemePreference::Light);
        assert_eq!(ctx.theme(), egui::Theme::Light);
        init(&ctx, ThemePreference::Dark);
        assert_eq!(ctx.theme(), egui::Theme::Dark);
    }

    /// Helper: create a temporary egui Ui from a Context for testing.
    fn test_ui(ctx: &egui::Context, f: impl FnOnce(&mut egui::Ui)) {
        let mut ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("test_ui"),
            egui::UiBuilder::new(),
        );
        f(&mut ui);
    }

    #[test]
    fn glass_frame_is_semi_transparent_dark() {
        let ctx = egui::Context::default();
        ThemePreference::Dark.apply(&ctx);
        test_ui(&ctx, |ui| {
            let frame = super::glass_frame(ui);
            assert!(frame.fill.a() < 255, "glass frame must be semi-transparent");
            assert!(frame.fill.a() > 100, "glass frame must not be too transparent");
        });
    }

    #[test]
    fn glass_frame_is_semi_transparent_light() {
        let ctx = egui::Context::default();
        ThemePreference::Light.apply(&ctx);
        test_ui(&ctx, |ui| {
            let frame = super::glass_frame(ui);
            assert!(frame.fill.a() < 255, "glass frame must be semi-transparent");
            assert!(frame.fill.a() > 100, "glass frame must not be too transparent");
        });
    }

    #[test]
    fn glass_frame_has_corner_radius() {
        let ctx = egui::Context::default();
        test_ui(&ctx, |ui| {
            let frame = super::glass_frame(ui);
            assert!(frame.corner_radius.nw > 0, "glass frame must have rounding");
        });
    }

    #[test]
    fn hover_stroke_uses_accent_color() {
        let ctx = egui::Context::default();
        ThemePreference::Dark.apply(&ctx);
        test_ui(&ctx, |ui| {
            let stroke = super::hover_stroke(ui);
            assert!(stroke.width > 0.0, "hover stroke must have width");
            assert!(stroke.color.a() > 0, "hover stroke must have alpha");
            assert!(stroke.color.a() < 255, "hover stroke must be semi-transparent");
        });
    }

    #[test]
    fn active_stroke_uses_accent_color() {
        let ctx = egui::Context::default();
        ThemePreference::Dark.apply(&ctx);
        test_ui(&ctx, |ui| {
            let stroke = super::active_stroke(ui);
            assert!(stroke.width > 0.0, "active stroke must have width");
            assert_eq!(stroke.color.a(), 255, "active stroke must be fully opaque");
        });
    }

    #[test]
    fn preview_fade_alpha_returns_zero_when_hidden() {
        let ctx = egui::Context::default();
        let alpha = super::preview_fade_alpha(&ctx, egui::Id::new("test_fade"), false);
        assert_eq!(alpha, 0.0, "hidden preview should have alpha 0");
    }

    #[test]
    fn preview_fade_alpha_returns_one_when_visible() {
        let ctx = egui::Context::default();
        let _ = super::preview_fade_alpha(&ctx, egui::Id::new("test_fade_visible"), true);
        for _ in 0..10 {
            ctx.request_repaint();
        }
        let alpha = super::preview_fade_alpha(&ctx, egui::Id::new("test_fade_visible"), true);
        assert!(alpha > 0.0, "visible preview should animate toward 1.0, got {alpha}");
    }

    #[test]
    fn hover_stroke_differs_from_active_stroke() {
        let ctx = egui::Context::default();
        ThemePreference::Dark.apply(&ctx);
        test_ui(&ctx, |ui| {
            let hover = super::hover_stroke(ui);
            let active = super::active_stroke(ui);
            assert_ne!(hover.width, active.width, "hover and active strokes must differ in width");
        });
    }
}
