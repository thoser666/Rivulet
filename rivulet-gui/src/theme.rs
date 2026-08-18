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
}
