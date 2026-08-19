//! Lightweight internationalization (i18n) support.
//!
//! All user-visible strings live in translation tables keyed by stable,
//! semantic keys instead of being hard-coded in the UI. The default locale is
//! English; every other locale adds its own table. Missing translations fall
//! back to the key itself, so the UI never breaks.
//!
//! Keys may contain `{0}`, `{1}`, ... positional placeholders, rendered via
//! [`Locale::tr_fmt`]:
//!
//! ```
//! use rivulet_core::Locale;
//!
//! let msg = Locale::En.tr_fmt("recording_in_progress", &[42.to_string()]);
//! ```

use serde::{Deserialize, Serialize};

/// Supported user interface languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Locale {
    /// English (default).
    #[default]
    En,
    /// German.
    De,
}

impl Locale {
    /// All supported locales in display order (for language pickers).
    pub fn all() -> &'static [Locale] {
        &[Locale::En, Locale::De]
    }

    /// Parse a locale from a BCP-47-style language tag such as `"en"`,
    /// `"de"` or `"de-DE"`. Unknown tags fall back to English.
    pub fn from_code(code: &str) -> Locale {
        let lower = code.to_ascii_lowercase();
        let primary = lower.split('-').next().unwrap_or("");
        match primary {
            "de" => Locale::De,
            _ => Locale::En,
        }
    }

    /// BCP-47 language code identifying this locale (`"en"`, `"de"`).
    pub fn code(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::De => "de",
        }
    }

    /// Native name of the language, for language pickers.
    pub fn name(self) -> &'static str {
        match self {
            Locale::En => "English",
            Locale::De => "Deutsch",
        }
    }

    /// Translate a key into this locale.
    ///
    /// Returns the key itself when no translation exists, so missing entries
    /// degrade gracefully instead of panicking. The result borrows either the
    /// static translation table or the input key.
    pub fn tr(self, key: &str) -> &str {
        self.strings()
            .iter()
            .find(|(k, _)| **k == *key)
            .map_or(key, |(_, v)| v)
    }

    /// Translate a key and substitute `{0}`, `{1}`, ... positional
    /// placeholders with the given, already-formatted arguments.
    ///
    /// Falls back to the key itself when no translation exists.
    pub fn tr_fmt(self, key: &str, args: &[String]) -> String {
        let template = self.tr(key);
        let mut out = String::with_capacity(template.len() + 16);
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let tail = &rest[open + 1..];
            match tail.find('}') {
                Some(close) => {
                    let spec = &tail[..close];
                    if let Ok(idx) = spec.parse::<usize>() {
                        if let Some(arg) = args.get(idx) {
                            out.push_str(arg);
                            rest = &tail[close + 1..];
                            continue;
                        }
                    }
                    out.push('{');
                    rest = tail;
                }
                None => {
                    out.push_str(rest);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// Translation table for this locale.
    ///
    /// Every locale must define exactly the same keys in the same order; a
    /// test enforces this invariant.
    fn strings(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Locale::En => &[
                // Recording
                ("source", "Source:"),
                ("screen_recording", "Screen recording"),
                ("windows_screen_recording", "Windows Screen Recording"),
                ("select_monitor", "Select monitor"),
                ("select_window", "Select window"),
                ("unknown_monitor", "Unknown monitor"),
                ("refresh_sources", "Refresh sources"),
                ("start_recording", "Start recording"),
                ("stop_recording", "Stop recording"),
                ("recording_in_progress", "Recording in progress ({0}s)"),
                ("recording_metrics", "FPS {0} · Encoder load {1} · File {2}"),
                ("recording_saved", "Recording saved."),
                ("no_source_selected", "No capture source selected."),
                ("invalid_source", "Invalid selected source."),
                ("invalid_monitor", "Invalid selected monitor."),
                ("invalid_window", "Invalid selected window."),
                (
                    "recording_no_frames",
                    "No frames received for {0} seconds — recording stopped.",
                ),
                // Hotkeys
                ("hotkey_record", "Start/Stop recording"),
                ("hotkey_pause", "Pause/Resume recording"),
                ("hotkey_mute", "Mute/Unmute audio"),
                ("paused", "Paused"),
                ("muted", "Muted"),
                // Codec selection
                ("video_codec", "Video codec"),
                ("recording_preset", "Preset"),
                // Overlay
                ("overlay_toggle", "Overlay (timer + FPS)"),
                // Region capture
                ("capture_region", "Capture region"),
                ("region_select", "Select region…"),
                ("region_status", "{0}×{1} @ ({2},{3})"),
                ("region_editor_title", "Select capture region"),
                (
                    "region_hint",
                    "Drag on the preview to choose the recording area, or enter values below.",
                ),
                ("region_x", "X"),
                ("region_y", "Y"),
                ("region_width", "W"),
                ("region_height", "H"),
                ("region_full_monitor", "Full monitor"),
                ("region_apply", "Apply"),
                ("region_cancel", "Cancel"),
                ("region_preview_failed", "Preview unavailable: {0}"),
                ("region_preview_unavailable", "the monitor could not be captured"),
                // Audio
                ("audio_mixer", "Audio Mixer"),
                ("start_audio", "Start audio"),
                ("stop_audio", "Stop audio"),
                ("running", "running"),
                ("system_audio", "System audio"),
                ("microphone", "Microphone"),
                ("separate_tracks", "Separate tracks"),
                (
                    "audio_capture_unavailable",
                    "Audio capture unavailable: {0}",
                ),
                ("audio_start_failed", "Audio start failed: {0}"),
                (
                    "audio_filters_skipped",
                    "Audio filters skipped (missing GStreamer elements): {0}",
                ),
                ("peak", "Peak: {0} dB"),
                // Updates
                ("updates", "Updates"),
                ("updates_current_version", "Current version: {0}"),
                ("check_for_updates", "Check for updates"),
                ("update_checking", "Checking for updates..."),
                ("update_up_to_date", "You are up to date."),
                ("update_available", "Update {0} is available."),
                ("updates_new_version", "New version: {0}"),
                ("update_asset_size", "Size: {0}"),
                ("update_download_install", "Download & Install"),
                ("update_release_notes", "Release notes"),
                ("update_downloading", "Downloading {0}..."),
                ("update_downloaded", "Update downloaded."),
                ("update_install", "Install & Restart"),
                ("update_installing", "Installing update..."),
                (
                    "update_installed_restart",
                    "Update installed. Please restart Rivulet.",
                ),
                ("update_error", "Update check failed: {0}"),
                // Language picker
                ("language", "Language"),
                // Navigation
                ("nav_record", "Record"),
                ("nav_mixer", "Mixer"),
                ("nav_scenes", "Scenes"),
                ("nav_stream", "Stream"),
                ("nav_assistant", "Assistant"),
                ("nav_settings", "Settings"),
                ("section_planned", "This section is planned for milestone {0}."),
                ("mixer_unavailable", "The audio mixer is currently only available on Linux."),
                // Theme
                ("theme", "Theme"),
                ("theme_system", "System"),
                ("theme_dark", "Dark"),
                ("theme_light", "Light"),
            ],
            Locale::De => &[
                // Recording
                ("source", "Quelle:"),
                ("screen_recording", "Bildschirmaufnahme"),
                ("windows_screen_recording", "Windows-Bildschirmaufnahme"),
                ("select_monitor", "Monitor auswählen"),
                ("select_window", "Fenster auswählen"),
                ("unknown_monitor", "Unbekannter Monitor"),
                ("refresh_sources", "Quellen aktualisieren"),
                ("start_recording", "Aufnahme starten"),
                ("stop_recording", "Aufnahme stoppen"),
                ("recording_in_progress", "Aufnahme läuft ({0}s)"),
                (
                    "recording_metrics",
                    "FPS {0} · Encoder-Last {1} · Datei {2}",
                ),
                ("recording_saved", "Aufnahme gespeichert."),
                ("no_source_selected", "Keine Aufnahmequelle ausgewählt."),
                ("invalid_source", "Ausgewählte Quelle ist ungültig."),
                ("invalid_monitor", "Ausgewählter Monitor ist ungültig."),
                ("invalid_window", "Ausgewähltes Fenster ist ungültig."),
                (
                    "recording_no_frames",
                    "{0} Sekunden lang keine Frames empfangen — Aufnahme gestoppt.",
                ),
                // Hotkeys
                ("hotkey_record", "Aufnahme starten/stoppen"),
                ("hotkey_pause", "Aufnahme pausieren/fortsetzen"),
                ("hotkey_mute", "Audio stummschalten"),
                ("paused", "Pausiert"),
                ("muted", "Stumm"),
                // Codec selection
                ("video_codec", "Video-Codec"),
                ("recording_preset", "Preset"),
                // Overlay
                ("overlay_toggle", "Overlay (Timer + FPS)"),
                // Region capture
                ("capture_region", "Bereich aufnehmen"),
                ("region_select", "Bereich auswählen…"),
                ("region_status", "{0}×{1} @ ({2},{3})"),
                ("region_editor_title", "Aufnahmebereich auswählen"),
                (
                    "region_hint",
                    "Ziehen Sie auf der Vorschau, um den Aufnahmebereich zu wählen, oder geben Sie unten Werte ein.",
                ),
                ("region_x", "X"),
                ("region_y", "Y"),
                ("region_width", "B"),
                ("region_height", "H"),
                ("region_full_monitor", "Gesamter Monitor"),
                ("region_apply", "Übernehmen"),
                ("region_cancel", "Abbrechen"),
                ("region_preview_failed", "Vorschau nicht verfügbar: {0}"),
                ("region_preview_unavailable", "der Monitor konnte nicht erfasst werden"),
                // Audio
                ("audio_mixer", "Audio-Mixer"),
                ("start_audio", "Audio starten"),
                ("stop_audio", "Audio stoppen"),
                ("running", "läuft"),
                ("system_audio", "Systemaudio"),
                ("microphone", "Mikrofon"),
                ("separate_tracks", "Getrennte Tracks"),
                (
                    "audio_capture_unavailable",
                    "Audio-Capture nicht verfügbar: {0}",
                ),
                ("audio_start_failed", "Audio-Start fehlgeschlagen: {0}"),
                (
                    "audio_filters_skipped",
                    "Audiofilter übersprungen (fehlende GStreamer-Elemente): {0}",
                ),
                ("peak", "Peak: {0} dB"),
                // Updates
                ("updates", "Updates"),
                ("updates_current_version", "Aktuelle Version: {0}"),
                ("check_for_updates", "Auf Updates prüfen"),
                ("update_checking", "Suche nach Updates..."),
                ("update_up_to_date", "Du bist auf dem neuesten Stand."),
                ("update_available", "Update {0} ist verfügbar."),
                ("updates_new_version", "Neue Version: {0}"),
                ("update_asset_size", "Größe: {0}"),
                ("update_download_install", "Herunterladen & Installieren"),
                ("update_release_notes", "Versionshinweise"),
                ("update_downloading", "Lade {0} herunter..."),
                ("update_downloaded", "Update heruntergeladen."),
                ("update_install", "Installieren & Neu starten"),
                ("update_installing", "Installiere Update..."),
                (
                    "update_installed_restart",
                    "Update installiert. Bitte starte Rivulet neu.",
                ),
                ("update_error", "Update-Check fehlgeschlagen: {0}"),
                // Language picker
                ("language", "Sprache"),
                // Navigation
                ("nav_record", "Aufnehmen"),
                ("nav_mixer", "Mixer"),
                ("nav_scenes", "Szenen"),
                ("nav_stream", "Stream"),
                ("nav_assistant", "Assistant"),
                ("nav_settings", "Einstellungen"),
                ("section_planned", "Dieser Bereich ist für Meilenstein {0} geplant."),
                ("mixer_unavailable", "Der Audio-Mixer ist derzeit nur unter Linux verfügbar."),
                // Theme
                ("theme", "Farbschema"),
                ("theme_system", "System"),
                ("theme_dark", "Dunkel"),
                ("theme_light", "Hell"),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_locale_is_english() {
        assert_eq!(Locale::default(), Locale::En);
    }

    #[test]
    fn translates_english() {
        assert_eq!(Locale::En.tr("start_recording"), "Start recording");
    }

    #[test]
    fn translates_german() {
        assert_eq!(Locale::De.tr("start_recording"), "Aufnahme starten");
    }

    #[test]
    fn unknown_key_falls_back_to_key() {
        assert_eq!(Locale::En.tr("does_not_exist"), "does_not_exist");
        assert_eq!(Locale::De.tr("does_not_exist"), "does_not_exist");
    }

    #[test]
    fn parses_bcp47_codes() {
        assert_eq!(Locale::from_code("en"), Locale::En);
        assert_eq!(Locale::from_code("en-US"), Locale::En);
        assert_eq!(Locale::from_code("de"), Locale::De);
        assert_eq!(Locale::from_code("de-DE"), Locale::De);
        assert_eq!(Locale::from_code("fr"), Locale::En);
        assert_eq!(Locale::from_code(""), Locale::En);
    }

    #[test]
    fn lists_all_locales() {
        assert_eq!(Locale::all(), &[Locale::En, Locale::De]);
    }

    #[test]
    fn exposes_codes_and_native_names() {
        assert_eq!(Locale::En.code(), "en");
        assert_eq!(Locale::De.code(), "de");
        assert_eq!(Locale::En.name(), "English");
        assert_eq!(Locale::De.name(), "Deutsch");
    }

    #[test]
    fn formatted_translations_contain_placeholders() {
        let en = Locale::En.tr_fmt("recording_in_progress", &[7.to_string()]);
        assert_eq!(en, "Recording in progress (7s)");
        let de = Locale::De.tr_fmt("recording_in_progress", &[7.to_string()]);
        assert_eq!(de, "Aufnahme läuft (7s)");
        let missing = Locale::En.tr_fmt("does_not_exist", &["x".to_string()]);
        assert_eq!(missing, "does_not_exist");
    }

    #[test]
    fn formatted_metrics_translate_in_both_locales() {
        let args = ["29.8".to_string(), "41%".to_string(), "12.4 MB".to_string()];
        let en = Locale::En.tr_fmt("recording_metrics", &args);
        assert_eq!(en, "FPS 29.8 · Encoder load 41% · File 12.4 MB");
        let de = Locale::De.tr_fmt("recording_metrics", &args);
        assert_eq!(de, "FPS 29.8 · Encoder-Last 41% · Datei 12.4 MB");
    }

    #[test]
    fn all_locales_share_the_same_keys() {
        let en_keys: Vec<&str> = Locale::En.strings().iter().map(|(k, _)| *k).collect();
        let de_keys: Vec<&str> = Locale::De.strings().iter().map(|(k, _)| *k).collect();
        assert_eq!(
            en_keys, de_keys,
            "every locale must define exactly the same keys in the same order"
        );
    }

    #[test]
    fn navigation_keys_translate_in_both_locales() {
        assert_eq!(Locale::En.tr("nav_record"), "Record");
        assert_eq!(Locale::De.tr("nav_record"), "Aufnehmen");
        assert_eq!(Locale::En.tr("nav_mixer"), "Mixer");
        assert_eq!(Locale::De.tr("nav_mixer"), "Mixer");
        assert_eq!(Locale::En.tr("nav_scenes"), "Scenes");
        assert_eq!(Locale::De.tr("nav_scenes"), "Szenen");
        assert_eq!(Locale::En.tr("nav_stream"), "Stream");
        assert_eq!(Locale::De.tr("nav_stream"), "Stream");
        assert_eq!(Locale::En.tr("nav_assistant"), "Assistant");
        assert_eq!(Locale::De.tr("nav_assistant"), "Assistant");
        assert_eq!(Locale::En.tr("nav_settings"), "Settings");
        assert_eq!(Locale::De.tr("nav_settings"), "Einstellungen");
    }

    #[test]
    fn theme_keys_translate_in_both_locales() {
        assert_eq!(Locale::En.tr("theme"), "Theme");
        assert_eq!(Locale::De.tr("theme"), "Farbschema");
        assert_eq!(Locale::En.tr("theme_system"), "System");
        assert_eq!(Locale::De.tr("theme_system"), "System");
        assert_eq!(Locale::En.tr("theme_dark"), "Dark");
        assert_eq!(Locale::De.tr("theme_dark"), "Dunkel");
        assert_eq!(Locale::En.tr("theme_light"), "Light");
        assert_eq!(Locale::De.tr("theme_light"), "Hell");
    }

    #[test]
    fn planned_section_and_mixer_unavailable_translate() {
        let en = Locale::En.tr_fmt("section_planned", &["M9".to_string()]);
        assert_eq!(en, "This section is planned for milestone M9.");
        let de = Locale::De.tr_fmt("section_planned", &["M2".to_string()]);
        assert_eq!(de, "Dieser Bereich ist für Meilenstein M2 geplant.");
        assert_eq!(
            Locale::En.tr("mixer_unavailable"),
            "The audio mixer is currently only available on Linux."
        );
        assert_eq!(
            Locale::De.tr("mixer_unavailable"),
            "Der Audio-Mixer ist derzeit nur unter Linux verfügbar."
        );
    }
}
