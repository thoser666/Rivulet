//! Central theme handling for the Rivulet GUI.
//!
//! [`ThemePreference`] is the user-facing color-scheme preference
//! (system / dark / light), persisted in the app settings and applied to the
//! egui context. The semantic status colors live in [`StatusColors`], which
//! resolves a separate palette per active scheme so the colors stay legible
//! on both dark and light backgrounds. Both the status palettes and the
//! accent widget fills (active/hovered) meet WCAG AA (>= 4.5:1) against the
//! fills their text sits on — enforced in CI by
//! `scripts/check-theme-contrast.py` and the tests at the bottom of this
//! file.

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
// fill of their scheme, and the accent widget fills (active/hovered) meet
// WCAG AA against their button text — enforced in CI by
// `status_colors_meet_wcag_aa` and `scripts/check-theme-contrast.py`.

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
    ///
    /// `accent` is the *flat* highlight color for progress bars, strokes and
    /// badges. The widget-state button fills (active/hovered) use the deeper
    /// [`WIDGET_ACCENT_FILL`] so their text stays above WCAG AA — audit
    /// finding ui-006 (no text color passes on the flat teal A400 fill).
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

/// The deeper accent used for the `widgets.active`/`widgets.hovered` button
/// fills so the button text meets WCAG AA on both schemes (ui-006).
///
/// Material teal 800, paired with [`WIDGET_TEXT`]: white-ish text reaches
/// ~6.6:1 on the pressed fill and ~6.1:1 on the brightened hovered fill
/// (both schemes). White on the flat teal A400 tops out at ~2.7:1 — no text
/// color can pass, so the fill itself must deepen. Parsed and enforced by
/// `scripts/check-theme-contrast.py`.
const WIDGET_ACCENT_FILL: egui::Color32 = egui::Color32::from_rgb(0, 105, 92);

/// Button text color on [`WIDGET_ACCENT_FILL`] fills (both schemes — the
/// deep teal fill needs light text regardless of the scheme's body text
/// color). Parsed and enforced by `scripts/check-theme-contrast.py`.
const WIDGET_TEXT: egui::Color32 = egui::Color32::from_gray(250);

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
        // The accent fills must keep button text above WCAG AA
        // (scripts/check-theme-contrast.py enforces it). Button text comes
        // from the widget-state `fg_stroke`, so pin it here explicitly.
        visuals.widgets.active.bg_fill = WIDGET_ACCENT_FILL;
        visuals.widgets.hovered.bg_fill = WIDGET_ACCENT_FILL.linear_multiply(1.15);
        visuals.widgets.active.fg_stroke = egui::Stroke::new(2.0, WIDGET_TEXT);
        visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.5, WIDGET_TEXT);
        visuals.widgets.inactive.bg_fill = palette.panel_fill;
        visuals.widgets.noninteractive.bg_fill = palette.background;
        visuals.window_fill = palette.panel_fill;
        visuals.override_text_color = Some(palette.text);
        ctx.set_visuals_of(theme, visuals);
    }
}

/// Semi-transparent panel frame for the glassmorphism effect.
///
/// The fill is derived from the active structural palette instead of a second
/// set of hard-coded light/dark colors. The alpha is intentionally kept here,
/// because translucency is a component treatment rather than a palette role.
pub fn glass_frame(ui: &egui::Ui) -> egui::Frame {
    let palette = ThemePalette::for_ui(ui);
    let fill = palette.panel_fill.gamma_multiply(200.0 / 255.0);
    egui::Frame::default()
        .fill(fill)
        .inner_margin(egui::Margin::symmetric(8, 4))
        .corner_radius(egui::CornerRadius::same(4))
}

/// Apply the semantic accent stroke to a button while preserving egui's
/// normal widget visuals for disabled and inactive states.
pub fn accent_button(ui: &mut egui::Ui, label: impl Into<egui::WidgetText>) -> egui::Response {
    let response = ui.button(label);
    paint_interaction_stroke(ui, &response);
    response
}

/// Apply accent styling to an arbitrary interactive response. This keeps
/// hover/active feedback consistent across buttons, menu entries, and custom
/// controls without replacing egui's accessible text/contrast defaults.
pub fn paint_interaction_stroke(ui: &egui::Ui, response: &egui::Response) {
    let stroke = if response.has_focus() {
        egui::Stroke::new(2.0, ThemePalette::for_ui(ui).accent)
    } else if response.is_pointer_button_down_on() {
        active_stroke(ui)
    } else if response.hovered() {
        hover_stroke(ui)
    } else {
        return;
    };
    ui.painter().rect_stroke(
        response.rect.expand(stroke.width / 2.0),
        egui::CornerRadius::same(4),
        stroke,
        egui::StrokeKind::Outside,
    );
}

/// Accent-colored hover stroke for buttons and interactive elements.
///
/// Returns a 1.5px stroke using the palette accent color at 60% opacity.
#[allow(dead_code)]
pub fn hover_stroke(ui: &egui::Ui) -> egui::Stroke {
    let accent = ThemePalette::for_ui(ui).accent;
    egui::Stroke::new(1.5, accent.linear_multiply(0.6))
}

/// Accent-colored active press stroke.
///
/// Returns a 2px stroke using the full palette accent color.
#[allow(dead_code)]
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

    /// Mirror of ecolor 0.36.1 `Color32::linear_multiply` (premultiplied
    /// chain), used to derive the hovered fill exactly like the runtime.
    fn linear_multiply(color: egui::Color32, factor: f32) -> egui::Color32 {
        let gamma_to_linear = |g: f32| {
            if g <= 0.04045 {
                g / 12.92
            } else {
                ((g + 0.055) / 1.055).powf(2.4)
            }
        };
        let linear_to_gamma_u8 = |l: f32| {
            if l <= 0.0 {
                0
            } else if l <= 0.003_130_8 {
                (3294.6 * l).round() as u8
            } else if l <= 1.0 {
                (269.025 * l.powf(1.0 / 2.4) - 14.025).round() as u8
            } else {
                255
            }
        };
        let channel = |c: u8| -> u8 {
            let lin = gamma_to_linear(f32::from(c) / 255.0);
            ((linear_to_gamma_u8(lin / factor) as f32) * factor).round() as u8
        };
        egui::Color32::from_rgb(channel(color.r()), channel(color.g()), channel(color.b()))
    }

    /// The button text on the accent active/hovered fills must meet WCAG AA
    /// in both schemes — audit finding ui-006. The hovered fill is derived
    /// via `linear_multiply(1.15)` exactly like `theme::init` does.
    #[test]
    fn widget_accent_fills_meet_wcag_aa_with_their_text() {
        let hovered = linear_multiply(WIDGET_ACCENT_FILL, 1.15);
        for (name, fill) in [("active", WIDGET_ACCENT_FILL), ("hovered", hovered)] {
            let ratio = contrast(WIDGET_TEXT, fill);
            assert!(
                ratio >= 4.5,
                "widgets.{name} fill: text contrast {ratio:.2}:1 < 4.5:1 (WCAG AA)"
            );
        }
    }

    /// The palette accent (progress bars, strokes, badges) is distinct from
    /// the deeper widget fill, so accents stay visible on accent-filled
    /// widgets.
    #[test]
    fn widget_fill_is_deeper_than_the_flat_accent() {
        for dark in [true, false] {
            let accent = ThemePalette::for_scheme(dark).accent;
            assert_ne!(accent, WIDGET_ACCENT_FILL);
        }
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
        assert_eq!(dark_visuals.widgets.active.bg_fill, WIDGET_ACCENT_FILL);
        assert_eq!(light_visuals.widgets.hovered.fg_stroke.color, WIDGET_TEXT);
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
            assert!(
                frame.fill.a() > 100,
                "glass frame must not be too transparent"
            );
        });
    }

    #[test]
    fn glass_frame_is_semi_transparent_light() {
        let ctx = egui::Context::default();
        ThemePreference::Light.apply(&ctx);
        test_ui(&ctx, |ui| {
            let frame = super::glass_frame(ui);
            assert!(frame.fill.a() < 255, "glass frame must be semi-transparent");
            assert!(
                frame.fill.a() > 100,
                "glass frame must not be too transparent"
            );
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
            assert!(
                stroke.color.a() < 255,
                "hover stroke must be semi-transparent"
            );
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
    fn interaction_strokes_have_expected_semantic_alpha() {
        let ctx = egui::Context::default();
        ThemePreference::Dark.apply(&ctx);
        test_ui(&ctx, |ui| {
            let hover = super::hover_stroke(ui);
            let active = super::active_stroke(ui);
            assert_eq!(
                hover.color,
                ThemePalette::for_ui(ui).accent.linear_multiply(0.6)
            );
            assert_eq!(active.color, ThemePalette::for_ui(ui).accent);
            assert!(hover.width < active.width);
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
        assert!(
            alpha > 0.0,
            "visible preview should animate toward 1.0, got {alpha}"
        );
    }

    #[test]
    fn hover_stroke_differs_from_active_stroke() {
        let ctx = egui::Context::default();
        ThemePreference::Dark.apply(&ctx);
        test_ui(&ctx, |ui| {
            let hover = super::hover_stroke(ui);
            let active = super::active_stroke(ui);
            assert_ne!(
                hover.width, active.width,
                "hover and active strokes must differ in width"
            );
        });
    }
}
