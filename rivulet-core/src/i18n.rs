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
                ("camera_source", "Camera:"),
                ("select_camera", "Select camera"),
                ("game_capture", "Game Capture"),
                ("select_game_window", "Select game window"),
                ("refresh_game_windows", "Refresh window list"),
                ("game_preview_title", "Window preview (live)"),
                ("game_preview_loading", "Loading window preview…"),
                (
                    "game_preview_unavailable",
                    "Window preview unavailable (window closed or minimized)."
                ),
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
                // Capture backend (G2)
                (
                    "backend_desktop_duplication",
                    "Capture backend: DXGI Desktop Duplication (zero-copy GPU)",
                ),
                (
                    "backend_wgc",
                    "Capture backend: Windows Graphics Capture (fallback)",
                ),
                (
                    "backend_wgc_fallback",
                    "Capture backend: Windows Graphics Capture (fallback — {0})",
                ),
                (
                    "backend_vulkan_layer",
                    "Capture backend: Vulkan layer (zero-copy fullscreen)",
                ),
                (
                    "backend_opengl_hook",
                    "Capture backend: OpenGL hook (wglSwapBuffers)",
                ),
                (
                    "backend_opengl_hook_fallback",
                    "Capture backend: OpenGL hook (wglSwapBuffers — {0})",
                ),
                (
                    "backend_pipewire_portal",
                    "Capture backend: PipeWayland portal (fullscreen)",
                ),
                (
                    "backend_pipewire_portal_fallback",
                    "Capture backend: PipeWire portal ({0})",
                ),
                ("backend_none", "Capture backend: not active"),
                // Source kinds (S1)
                ("source_image", "Image"),
                ("source_image_single", "Single image"),
                ("source_image_folder", "Image folder (slideshow)"),
                ("source_image_interval", "Slideshow interval"),
                ("source_image_no_images", "No images found"),
                // Text source (S3)
                ("source_text_font_size", "Font size"),
                ("source_text_font_name", "Font"),
                ("source_text_color", "Text color"),
                ("source_text_background", "Background"),
                ("source_text_outline", "Outline"),
                ("source_text_alignment", "Alignment"),
                ("source_text_scroll", "Scroll"),
                ("source_text_scroll_speed", "Scroll speed"),
                // Webcam source (S4)
                ("source_webcam_device", "Device"),
                ("source_webcam_resolution", "Resolution"),
                ("source_webcam_fps", "Frame rate"),
                ("source_webcam_format", "Pixel format"),
                ("source_webcam_mirror", "Mirror"),
                ("source_webcam_active", "Camera active"),
                ("source_webcam_inactive", "Camera inactive"),
                // Color source (S7)
                ("source_color_name", "Color source"),
                ("source_color_fill", "Fill scene"),
                ("source_color_custom_size", "Custom size"),
                // Media source (S6)
                ("source_media_file", "File"),
                ("source_media_playback", "Playback"),
                ("source_media_speed", "Speed"),
                ("source_media_volume", "Volume"),
                ("source_media_loop", "Loop"),
                ("source_media_once", "Once"),
                ("source_media_freeze", "Freeze on last frame"),
                ("source_media_restart", "Restart on scene entry"),
                ("source_media_muted", "Muted"),
                // Audio source (S8)
                ("source_audio_application", "Application"),
                ("source_audio_input", "Input Device"),
                ("source_audio_output", "Output Device"),
                ("source_audio_mixed", "Mixed"),
                ("source_audio_active", "Audio active"),
                ("source_audio_inactive", "Audio inactive"),
                ("source_text", "Text"),
                ("source_webcam", "Webcam"),
                ("source_browser", "Browser"),
                ("browser_source", "Browser source"),
                ("browser_url", "URL"),
                ("browser_apply_url", "Apply URL"),
                ("browser_url_applied", "Browser URL updated."),
                ("browser_viewport", "Viewport"),
                ("browser_apply_viewport", "Apply size"),
                ("browser_interaction", "Allow interaction"),
                ("browser_transparent", "Transparent background"),
                ("browser_zoom", "Zoom"),
                ("browser_css", "Custom CSS"),
                ("browser_frame_ready", "Browser frame ready: {0}×{1} (#{2})"),
                ("browser_preview_pending", "Waiting for the webview renderer…"),
                ("source_media", "Media"),
                ("source_color", "Color"),
                ("source_game_capture", "Game Capture"),
                ("source_screen_capture", "Screen Capture"),
                ("source_audio", "Audio"),
                // Replay buffer
                ("replay_buffer", "Replay buffer"),
                ("replay_off", "Off"),
                ("replay_seconds", "{0} seconds"),
                (
                    "replay_hint",
                    "Keeps the last {0} seconds of video in memory while recording. Press {1} (or the button below) to save them as an MP4 clip.",
                ),
                ("save_replay", "Save Replay"),
                ("replay_saved", "Replay saved: {0}"),
                ("replay_save_failed", "Could not save replay: {0}"),
                // Hotkeys
                ("hotkey_record", "Start/Stop recording"),
                ("hotkey_pause", "Pause/Resume recording"),
                ("hotkey_mute", "Mute/Unmute audio"),
                ("hotkey_save_replay", "Save replay"),
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
                ("file_menu", "File"),
                ("quit", "Quit"),
                ("audio_filters", "Filters"),
                ("audio_monitoring", "Monitoring"),
                ("monitoring_system", "Monitor system audio"),
                ("monitoring_microphone", "Monitor microphone"),
                ("monitoring_volume", "Monitoring volume"),
                ("powered_by", "Powered by"),
                ("screen_recording_unavailable", "Screen recording is currently only available on Linux and Windows."),
                // Navigation
                ("nav_record", "Record"),
                ("nav_mixer", "Mixer"),
                ("nav_scenes", "Scenes"),
                ("nav_stream", "Stream"),
                ("nav_assistant", "Assistant"),
                ("nav_settings", "Settings"),
                ("scenes_add", "Add scene"),
                ("scenes_name_placeholder", "Scene name…"),
                ("scenes_active", "Active"),
                ("scenes_no_scenes", "No scenes yet. Add one to start composing."),
                ("scenes_switch_back", "Switch back"),
                ("scenes_undo", "Undo"),
                ("scenes_redo", "Redo"),
                ("scenes_undo_hint", "Undo the last scene change (Ctrl+Z)"),
                ("scenes_redo_hint", "Redo the last scene change (Ctrl+Y)"),
                ("scenes_rename", "Rename"),
                ("scenes_remove", "Remove"),
                ("scenes_renamed", "Renamed to \"{0}\"."),
                ("scenes_added", "Scene \"{0}\" added."),
                ("scenes_removed", "Scene \"{0}\" removed."),
                ("scenes_switched", "Switched to \"{0}\"."),
                ("scenes_active_label", "Active scene: {0}"),
                ("scenes_collection", "Collection"),
                ("scenes_profile", "Profile"),
                ("scenes_duplicate", "Duplicate"),
                ("scenes_duplicate_name", "Copy of {0}"),
                ("transition", "Transition"),
                ("transition_duration", "Duration"),
                ("composition", "Source composition"),
                ("composition_no_scene", "Select or create a scene first."),
                ("composition_add_source", "Add source"),
                ("composition_add", "Add"),
                ("composition_empty", "No sources in this scene yet."),
                ("composition_layers", "Layers"),
                ("composition_properties", "Source properties"),
                ("composition_visible", "Visible"),
                ("composition_locked", "Locked"),
                ("composition_position", "Position"),
                ("composition_size", "Size"),
                ("composition_opacity", "Opacity"),
                ("composition_crop", "Crop"),
                ("composition_lower", "Lower"),
                ("composition_raise", "Raise"),
                ("composition_duplicate_source", "Duplicate source"),
                ("composition_hidden", "hidden"),
                ("composition_locked_state", "locked"),
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
                ("camera_source", "Kamera:"),
                ("select_camera", "Kamera auswählen"),
                ("game_capture", "Game-Capture"),
                ("select_game_window", "Spiel-Fenster auswählen"),
                ("refresh_game_windows", "Fensterliste aktualisieren"),
                ("game_preview_title", "Fenster-Vorschau (live)"),
                ("game_preview_loading", "Fenster-Vorschau wird geladen…"),
                (
                    "game_preview_unavailable",
                    "Fenster-Vorschau nicht verfügbar (Fenster geschlossen oder minimiert)."
                ),
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
                // Capture backend (G2)
                (
                    "backend_desktop_duplication",
                    "Capture-Backend: DXGI Desktop Duplication (Zero-Copy-GPU)",
                ),
                ("backend_wgc", "Capture-Backend: Windows Graphics Capture (Fallback)"),
                (
                    "backend_wgc_fallback",
                    "Capture-Backend: Windows Graphics Capture (Fallback — {0})",
                ),
                (
                    "backend_vulkan_layer",
                    "Capture-Backend: Vulkan-Layer (Zero-Copy-Fullscreen)",
                ),
                (
                    "backend_opengl_hook",
                    "Capture-Backend: OpenGL-Hook (wglSwapBuffers)",
                ),
                (
                    "backend_opengl_hook_fallback",
                    "Capture-Backend: OpenGL-Hook (wglSwapBuffers — {0})",
                ),
                (
                    "backend_pipewire_portal",
                    "Capture-Backend: PipeWire-Portal (Fullscreen)",
                ),
                (
                    "backend_pipewire_portal_fallback",
                    "Capture-Backend: PipeWire-Portal ({0})",
                ),
                ("backend_none", "Capture-Backend: nicht aktiv"),
                // Source kinds (S1)
                ("source_image", "Bild"),
                ("source_image_single", "Einzelbild"),
                ("source_image_folder", "Bilderordner (Slideshow)"),
                ("source_image_interval", "Slideshow-Intervall"),
                ("source_image_no_images", "Keine Bilder gefunden"),
                // Text source (S3)
                ("source_text_font_size", "Schriftgröße"),
                ("source_text_font_name", "Schriftart"),
                ("source_text_color", "Textfarbe"),
                ("source_text_background", "Hintergrund"),
                ("source_text_outline", "Umriss"),
                ("source_text_alignment", "Ausrichtung"),
                ("source_text_scroll", "Scrollen"),
                ("source_text_scroll_speed", "Scrollgeschwindigkeit"),
                // Webcam source (S4)
                ("source_webcam_device", "Gerät"),
                ("source_webcam_resolution", "Auflösung"),
                ("source_webcam_fps", "Bildrate"),
                ("source_webcam_format", "Pixelformat"),
                ("source_webcam_mirror", "Spiegeln"),
                ("source_webcam_active", "Kamera aktiv"),
                ("source_webcam_inactive", "Kamera inaktiv"),
                // Color source (S7)
                ("source_color_name", "Farbquelle"),
                ("source_color_fill", "Szene füllen"),
                ("source_color_custom_size", "Benutzergröße"),
                // Media source (S6)
                ("source_media_file", "Datei"),
                ("source_media_playback", "Wiedergabe"),
                ("source_media_speed", "Geschwindigkeit"),
                ("source_media_volume", "Lautstärke"),
                ("source_media_loop", "Schleife"),
                ("source_media_once", "Einmal"),
                ("source_media_freeze", "Auf letztem Frame frieren"),
                ("source_media_restart", "Bei Szenenstart neu starten"),
                ("source_media_muted", "Stumm"),
                // Audio source (S8)
                ("source_audio_application", "Anwendung"),
                ("source_audio_input", "Eingabegerät"),
                ("source_audio_output", "Ausgabegerät"),
                ("source_audio_mixed", "Gemischt"),
                ("source_audio_active", "Audio aktiv"),
                ("source_audio_inactive", "Audio inaktiv"),
                ("source_text", "Text"),
                ("source_webcam", "Webcam"),
                ("source_browser", "Browser"),
                ("browser_source", "Browserquelle"),
                ("browser_url", "URL"),
                ("browser_apply_url", "URL übernehmen"),
                ("browser_url_applied", "Browser-URL aktualisiert."),
                ("browser_viewport", "Ansichtsgröße"),
                ("browser_apply_viewport", "Größe übernehmen"),
                ("browser_interaction", "Interaktion erlauben"),
                ("browser_transparent", "Transparenter Hintergrund"),
                ("browser_zoom", "Zoom"),
                ("browser_css", "Benutzerdefiniertes CSS"),
                ("browser_frame_ready", "Browser-Frame bereit: {0}×{1} (#{2})"),
                ("browser_preview_pending", "Warte auf den Webview-Renderer…"),
                ("source_media", "Medien"),
                ("source_color", "Farbe"),
                ("source_game_capture", "Game-Capture"),
                ("source_screen_capture", "Bildschirm-Capture"),
                ("source_audio", "Audio"),
                // Replay buffer
                ("replay_buffer", "Replay-Puffer"),
                ("replay_off", "Aus"),
                ("replay_seconds", "{0} Sekunden"),
                (
                    "replay_hint",
                    "Hält die letzten {0} Sekunden des Videos im Speicher, während du aufnimmst. Drücke {1} (oder den Button), um sie als MP4-Clip zu speichern.",
                ),
                ("save_replay", "Replay speichern"),
                ("replay_saved", "Replay gespeichert: {0}"),
                ("replay_save_failed", "Replay konnte nicht gespeichert werden: {0}"),
                // Hotkeys
                ("hotkey_record", "Aufnahme starten/stoppen"),
                ("hotkey_pause", "Aufnahme pausieren/fortsetzen"),
                ("hotkey_mute", "Audio stummschalten"),
                ("hotkey_save_replay", "Replay speichern"),
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
                ("file_menu", "Datei"),
                ("quit", "Beenden"),
                ("audio_filters", "Filter"),
                ("audio_monitoring", "Monitoring"),
                ("monitoring_system", "Systemaudio abhören"),
                ("monitoring_microphone", "Mikrofon abhören"),
                ("monitoring_volume", "Monitoring-Lautstärke"),
                ("powered_by", "Unterstützt von"),
                ("screen_recording_unavailable", "Bildschirmaufnahme ist derzeit nur unter Linux und Windows verfügbar."),
                // Navigation
                ("nav_record", "Aufnehmen"),
                ("nav_mixer", "Mixer"),
                ("nav_scenes", "Szenen"),
                ("nav_stream", "Stream"),
                ("nav_assistant", "Assistant"),
                ("nav_settings", "Einstellungen"),
                ("scenes_add", "Szene hinzufügen"),
                ("scenes_name_placeholder", "Szenenname…"),
                ("scenes_active", "Aktiv"),
                ("scenes_no_scenes", "Noch keine Szenen. Füge eine hinzu, um mit dem Komponieren zu beginnen."),
                ("scenes_switch_back", "Zurückwechseln"),
                ("scenes_undo", "Rückgängig"),
                ("scenes_redo", "Wiederholen"),
                ("scenes_undo_hint", "Letzte Szenenänderung rückgängig machen (Strg+Z)"),
                ("scenes_redo_hint", "Letzte Szenenänderung wiederholen (Strg+Y)"),
                ("scenes_rename", "Umbenennen"),
                ("scenes_remove", "Entfernen"),
                ("scenes_renamed", "Umbenannt in \"{0}\"."),
                ("scenes_added", "Szene \"{0}\" hinzugefügt."),
                ("scenes_removed", "Szene \"{0}\" entfernt."),
                ("scenes_switched", "Gewechselt zu \"{0}\"."),
                ("scenes_active_label", "Aktive Szene: {0}"),
                ("scenes_collection", "Sammlung"),
                ("scenes_profile", "Profil"),
                ("scenes_duplicate", "Duplizieren"),
                ("scenes_duplicate_name", "Kopie von {0}"),
                ("transition", "Übergang"),
                ("transition_duration", "Dauer"),
                ("composition", "Quellenkomposition"),
                ("composition_no_scene", "Bitte zuerst eine Szene auswählen oder erstellen."),
                ("composition_add_source", "Quelle hinzufügen"),
                ("composition_add", "Hinzufügen"),
                ("composition_empty", "Diese Szene enthält noch keine Quellen."),
                ("composition_layers", "Ebenen"),
                ("composition_properties", "Quelleneigenschaften"),
                ("composition_visible", "Sichtbar"),
                ("composition_locked", "Gesperrt"),
                ("composition_position", "Position"),
                ("composition_size", "Größe"),
                ("composition_opacity", "Deckkraft"),
                ("composition_crop", "Beschnitt"),
                ("composition_lower", "Nach unten"),
                ("composition_raise", "Nach oben"),
                ("composition_duplicate_source", "Quelle duplizieren"),
                ("composition_hidden", "ausgeblendet"),
                ("composition_locked_state", "gesperrt"),
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
    fn source_composition_keys_translate_in_both_locales() {
        assert_eq!(Locale::En.tr("composition"), "Source composition");
        assert_eq!(Locale::De.tr("composition"), "Quellenkomposition");
        assert_eq!(Locale::En.tr("composition_visible"), "Visible");
        assert_eq!(Locale::De.tr("composition_visible"), "Sichtbar");
        assert_eq!(Locale::En.tr("composition_crop"), "Crop");
        assert_eq!(Locale::De.tr("composition_crop"), "Beschnitt");
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
    fn capture_backend_keys_translate_in_both_locales() {
        assert_eq!(
            Locale::En.tr("backend_desktop_duplication"),
            "Capture backend: DXGI Desktop Duplication (zero-copy GPU)"
        );
        assert_eq!(
            Locale::De.tr("backend_desktop_duplication"),
            "Capture-Backend: DXGI Desktop Duplication (Zero-Copy-GPU)"
        );
        assert_eq!(
            Locale::En.tr("backend_wgc"),
            "Capture backend: Windows Graphics Capture (fallback)"
        );
        assert_eq!(
            Locale::De.tr("backend_wgc"),
            "Capture-Backend: Windows Graphics Capture (Fallback)"
        );
        assert_eq!(
            Locale::En.tr_fmt("backend_wgc_fallback", &["access denied".to_string()]),
            "Capture backend: Windows Graphics Capture (fallback — access denied)"
        );
        assert_eq!(
            Locale::De.tr_fmt("backend_wgc_fallback", &["Zugriff verweigert".to_string()]),
            "Capture-Backend: Windows Graphics Capture (Fallback — Zugriff verweigert)"
        );
        assert_eq!(
            Locale::En.tr("backend_vulkan_layer"),
            "Capture backend: Vulkan layer (zero-copy fullscreen)"
        );
        assert_eq!(
            Locale::De.tr("backend_vulkan_layer"),
            "Capture-Backend: Vulkan-Layer (Zero-Copy-Fullscreen)"
        );
        assert_eq!(
            Locale::En.tr("backend_opengl_hook"),
            "Capture backend: OpenGL hook (wglSwapBuffers)"
        );
        assert_eq!(
            Locale::De.tr("backend_opengl_hook"),
            "Capture-Backend: OpenGL-Hook (wglSwapBuffers)"
        );
        assert_eq!(
            Locale::En.tr_fmt("backend_opengl_hook_fallback", &["injected".to_string()]),
            "Capture backend: OpenGL hook (wglSwapBuffers — injected)"
        );
        assert_eq!(
            Locale::De.tr_fmt("backend_opengl_hook_fallback", &["injiziert".to_string()]),
            "Capture-Backend: OpenGL-Hook (wglSwapBuffers — injiziert)"
        );
        assert_eq!(
            Locale::En.tr("backend_pipewire_portal"),
            "Capture backend: PipeWayland portal (fullscreen)"
        );
        assert_eq!(
            Locale::De.tr("backend_pipewire_portal"),
            "Capture-Backend: PipeWire-Portal (Fullscreen)"
        );
        assert_eq!(
            Locale::En.tr_fmt("backend_pipewire_portal_fallback", &["denied".to_string()]),
            "Capture backend: PipeWire portal (denied)"
        );
        assert_eq!(
            Locale::De.tr_fmt(
                "backend_pipewire_portal_fallback",
                &["verweigert".to_string()]
            ),
            "Capture-Backend: PipeWire-Portal (verweigert)"
        );
        assert_eq!(Locale::En.tr("backend_none"), "Capture backend: not active");
        assert_eq!(
            Locale::De.tr("backend_none"),
            "Capture-Backend: nicht aktiv"
        );
        // Source kinds (S1)
        assert_eq!(Locale::En.tr("source_image"), "Image");
        assert_eq!(Locale::De.tr("source_image"), "Bild");
        assert_eq!(Locale::En.tr("source_text"), "Text");
        assert_eq!(Locale::De.tr("source_text"), "Text");
        assert_eq!(Locale::En.tr("source_webcam"), "Webcam");
        assert_eq!(Locale::De.tr("source_webcam"), "Webcam");
        assert_eq!(Locale::En.tr("source_browser"), "Browser");
        assert_eq!(Locale::De.tr("source_browser"), "Browser");
        assert_eq!(Locale::En.tr("browser_source"), "Browser source");
        assert_eq!(Locale::De.tr("browser_source"), "Browserquelle");
        assert_eq!(Locale::En.tr("source_media"), "Media");
        assert_eq!(Locale::De.tr("source_media"), "Medien");
        assert_eq!(Locale::En.tr("source_color"), "Color");
        assert_eq!(Locale::De.tr("source_color"), "Farbe");
        assert_eq!(Locale::En.tr("source_game_capture"), "Game Capture");
        assert_eq!(Locale::De.tr("source_game_capture"), "Game-Capture");
        // Game capture live preview keys
        assert_eq!(Locale::En.tr("game_preview_title"), "Window preview (live)");
        assert_eq!(
            Locale::De.tr("game_preview_title"),
            "Fenster-Vorschau (live)"
        );
        assert_eq!(
            Locale::En.tr("game_preview_unavailable"),
            "Window preview unavailable (window closed or minimized)."
        );
        assert_eq!(
            Locale::De.tr("game_preview_unavailable"),
            "Fenster-Vorschau nicht verfügbar (Fenster geschlossen oder minimiert)."
        );
        assert_eq!(Locale::En.tr("source_screen_capture"), "Screen Capture");
        assert_eq!(Locale::De.tr("source_screen_capture"), "Bildschirm-Capture");
        assert_eq!(Locale::En.tr("source_audio"), "Audio");
        assert_eq!(Locale::De.tr("source_audio"), "Audio");
        // S2 image source keys
        assert_eq!(Locale::En.tr("source_image_single"), "Single image");
        assert_eq!(Locale::De.tr("source_image_single"), "Einzelbild");
        assert_eq!(
            Locale::En.tr("source_image_folder"),
            "Image folder (slideshow)"
        );
        assert_eq!(
            Locale::De.tr("source_image_folder"),
            "Bilderordner (Slideshow)"
        );
        assert_eq!(Locale::En.tr("source_image_interval"), "Slideshow interval");
        assert_eq!(
            Locale::De.tr("source_image_interval"),
            "Slideshow-Intervall"
        );
        assert_eq!(Locale::En.tr("source_image_no_images"), "No images found");
        assert_eq!(
            Locale::De.tr("source_image_no_images"),
            "Keine Bilder gefunden"
        );
        // S3 text source keys
        assert_eq!(Locale::En.tr("source_text_font_size"), "Font size");
        assert_eq!(Locale::De.tr("source_text_font_size"), "Schriftgröße");
        assert_eq!(Locale::En.tr("source_text_font_name"), "Font");
        assert_eq!(Locale::De.tr("source_text_font_name"), "Schriftart");
        assert_eq!(Locale::En.tr("source_text_color"), "Text color");
        assert_eq!(Locale::De.tr("source_text_color"), "Textfarbe");
        assert_eq!(Locale::En.tr("source_text_background"), "Background");
        assert_eq!(Locale::De.tr("source_text_background"), "Hintergrund");
        assert_eq!(Locale::En.tr("source_text_outline"), "Outline");
        assert_eq!(Locale::De.tr("source_text_outline"), "Umriss");
        assert_eq!(Locale::En.tr("source_text_alignment"), "Alignment");
        assert_eq!(Locale::De.tr("source_text_alignment"), "Ausrichtung");
        assert_eq!(Locale::En.tr("source_text_scroll"), "Scroll");
        assert_eq!(Locale::De.tr("source_text_scroll"), "Scrollen");
        assert_eq!(Locale::En.tr("source_text_scroll_speed"), "Scroll speed");
        assert_eq!(
            Locale::De.tr("source_text_scroll_speed"),
            "Scrollgeschwindigkeit"
        );
        // S4 webcam source keys
        assert_eq!(Locale::En.tr("source_webcam_device"), "Device");
        assert_eq!(Locale::De.tr("source_webcam_device"), "Gerät");
        assert_eq!(Locale::En.tr("source_webcam_resolution"), "Resolution");
        assert_eq!(Locale::De.tr("source_webcam_resolution"), "Auflösung");
        assert_eq!(Locale::En.tr("source_webcam_fps"), "Frame rate");
        assert_eq!(Locale::De.tr("source_webcam_fps"), "Bildrate");
        assert_eq!(Locale::En.tr("source_webcam_format"), "Pixel format");
        assert_eq!(Locale::De.tr("source_webcam_format"), "Pixelformat");
        assert_eq!(Locale::En.tr("source_webcam_mirror"), "Mirror");
        assert_eq!(Locale::De.tr("source_webcam_mirror"), "Spiegeln");
        assert_eq!(Locale::En.tr("source_webcam_active"), "Camera active");
        assert_eq!(Locale::De.tr("source_webcam_active"), "Kamera aktiv");
        assert_eq!(Locale::En.tr("source_webcam_inactive"), "Camera inactive");
        assert_eq!(Locale::De.tr("source_webcam_inactive"), "Kamera inaktiv");
        // S7 color source keys
        assert_eq!(Locale::En.tr("source_color_name"), "Color source");
        assert_eq!(Locale::De.tr("source_color_name"), "Farbquelle");
        assert_eq!(Locale::En.tr("source_color_fill"), "Fill scene");
        assert_eq!(Locale::De.tr("source_color_fill"), "Szene füllen");
        assert_eq!(Locale::En.tr("source_color_custom_size"), "Custom size");
        assert_eq!(Locale::De.tr("source_color_custom_size"), "Benutzergröße");
        // S6 media source keys
        assert_eq!(Locale::En.tr("source_media_file"), "File");
        assert_eq!(Locale::De.tr("source_media_file"), "Datei");
        assert_eq!(Locale::En.tr("source_media_playback"), "Playback");
        assert_eq!(Locale::De.tr("source_media_playback"), "Wiedergabe");
        assert_eq!(Locale::En.tr("source_media_speed"), "Speed");
        assert_eq!(Locale::De.tr("source_media_speed"), "Geschwindigkeit");
        assert_eq!(Locale::En.tr("source_media_volume"), "Volume");
        assert_eq!(Locale::De.tr("source_media_volume"), "Lautstärke");
        assert_eq!(Locale::En.tr("source_media_loop"), "Loop");
        assert_eq!(Locale::De.tr("source_media_loop"), "Schleife");
        assert_eq!(Locale::En.tr("source_media_once"), "Once");
        assert_eq!(Locale::De.tr("source_media_once"), "Einmal");
        assert_eq!(Locale::En.tr("source_media_freeze"), "Freeze on last frame");
        assert_eq!(
            Locale::De.tr("source_media_freeze"),
            "Auf letztem Frame frieren"
        );
        assert_eq!(
            Locale::En.tr("source_media_restart"),
            "Restart on scene entry"
        );
        assert_eq!(
            Locale::De.tr("source_media_restart"),
            "Bei Szenenstart neu starten"
        );
        assert_eq!(Locale::En.tr("source_media_muted"), "Muted");
        assert_eq!(Locale::De.tr("source_media_muted"), "Stumm");
        // S8 audio source keys
        assert_eq!(Locale::En.tr("source_audio_application"), "Application");
        assert_eq!(Locale::De.tr("source_audio_application"), "Anwendung");
        assert_eq!(Locale::En.tr("source_audio_input"), "Input Device");
        assert_eq!(Locale::De.tr("source_audio_input"), "Eingabegerät");
        assert_eq!(Locale::En.tr("source_audio_output"), "Output Device");
        assert_eq!(Locale::De.tr("source_audio_output"), "Ausgabegerät");
        assert_eq!(Locale::En.tr("source_audio_mixed"), "Mixed");
        assert_eq!(Locale::De.tr("source_audio_mixed"), "Gemischt");
        assert_eq!(Locale::En.tr("source_audio_active"), "Audio active");
        assert_eq!(Locale::De.tr("source_audio_active"), "Audio aktiv");
        assert_eq!(Locale::En.tr("source_audio_inactive"), "Audio inactive");
        assert_eq!(Locale::De.tr("source_audio_inactive"), "Audio inaktiv");
    }

    #[test]
    fn replay_keys_translate_in_both_locales() {
        assert_eq!(Locale::En.tr("replay_buffer"), "Replay buffer");
        assert_eq!(Locale::De.tr("replay_buffer"), "Replay-Puffer");
        assert_eq!(Locale::En.tr("replay_off"), "Off");
        assert_eq!(Locale::De.tr("replay_off"), "Aus");
        assert_eq!(
            Locale::En.tr_fmt("replay_seconds", &["30".to_string()]),
            "30 seconds"
        );
        assert_eq!(
            Locale::De.tr_fmt("replay_seconds", &["30".to_string()]),
            "30 Sekunden"
        );
        assert_eq!(Locale::En.tr("save_replay"), "Save Replay");
        assert_eq!(Locale::De.tr("save_replay"), "Replay speichern");
        assert_eq!(Locale::En.tr("hotkey_save_replay"), "Save replay");
        assert_eq!(Locale::De.tr("hotkey_save_replay"), "Replay speichern");
        let en = Locale::En.tr_fmt("replay_saved", &["/tmp/clip.mp4".to_string()]);
        assert_eq!(en, "Replay saved: /tmp/clip.mp4");
        let de = Locale::De.tr_fmt("replay_saved", &["/tmp/clip.mp4".to_string()]);
        assert_eq!(de, "Replay gespeichert: /tmp/clip.mp4");
        let en_fail = Locale::En.tr_fmt("replay_save_failed", &["boom".to_string()]);
        assert_eq!(en_fail, "Could not save replay: boom");
        let de_fail = Locale::De.tr_fmt("replay_save_failed", &["boom".to_string()]);
        assert_eq!(de_fail, "Replay konnte nicht gespeichert werden: boom");
    }

    #[test]
    fn scenes_keys_translate_in_both_locales() {
        assert_eq!(Locale::En.tr("scenes_add"), "Add scene");
        assert_eq!(Locale::De.tr("scenes_add"), "Szene hinzufügen");
        assert_eq!(Locale::En.tr("scenes_active"), "Active");
        assert_eq!(Locale::De.tr("scenes_active"), "Aktiv");
        assert_eq!(Locale::En.tr("scenes_switch_back"), "Switch back");
        assert_eq!(Locale::De.tr("scenes_switch_back"), "Zurückwechseln");
        let en = Locale::En.tr_fmt("scenes_switched", &["Game".to_string()]);
        assert_eq!(en, "Switched to \"Game\".");
        let de = Locale::De.tr_fmt("scenes_switched", &["Game".to_string()]);
        assert_eq!(de, "Gewechselt zu \"Game\".");
        let en_removed = Locale::En.tr_fmt("scenes_removed", &["Old".to_string()]);
        assert_eq!(en_removed, "Scene \"Old\" removed.");
        let de_removed = Locale::De.tr_fmt("scenes_removed", &["Old".to_string()]);
        assert_eq!(de_removed, "Szene \"Old\" entfernt.");
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
