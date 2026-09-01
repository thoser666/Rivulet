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
                ("recording_preview_title", "Recording preview"),
                ("recording_preview_ready", "Preview ready"),
                ("recording_preview_active", "Recording preview active"),
                ("recording_preview_waiting", "Waiting for the first frame…"),
                ("recording_preview_select_source", "Select a source to preview it."),
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
                ("alert_overlay_title", "Alert overlay import"),
                ("alert_provider_streamlabs", "Streamlabs"),
                ("alert_provider_streamelements", "StreamElements"),
                ("alert_provider_custom", "Custom URL"),
                ("alert_token", "Widget token"),
                ("alert_token_hint", "paste the widget token from your dashboard"),
                ("alert_import", "Import overlay"),
                ("alert_imported", "Alert overlay imported into the browser source."),
                ("alert_overlay_note", "The browser source renders the widget page; disable interaction so the overlay never steals focus."),
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
                ("hotkeys_section", "Hotkeys"),
                ("hotkeys_hint", "Rebind in-app hotkeys. On Windows these also work while the app is unfocused."),
                ("modifier_ctrl", "Ctrl"),
                ("modifier_alt", "Alt"),
                ("modifier_shift", "Shift"),
                ("paused", "Paused"),
                ("muted", "Muted"),
                // Codec selection
                ("video_codec", "Video codec"),
                ("recording_container", "Container"),
                ("auto_remux", "Remux to MP4 after stop"),
                ("rate_control", "Rate control"),
                ("rate_mode", "Mode"),
                ("video_effects", "Video effects"),
                ("vf_brightness", "Brightness"),
                ("vf_contrast", "Contrast"),
                ("vf_saturation", "Saturation"),
                ("vf_hue", "Hue"),
                ("vf_blur", "Blur"),
                ("vf_sharpen", "Sharpen"),
                ("rate_quality", "Quality (0-51, lower is better)"),
                ("rate_max_bitrate", "Max bitrate (kbit/s)"),
                ("rate_custom_options", "Extra encoder options"),
                ("split_after", "Split after (s):"),
                ("seconds", "s"),
                ("auto_record_with_stream", "Record automatically with stream"),
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
                ("export_track_system", "Export system audio on its own track"),
                ("export_track_microphone", "Export microphone on its own track"),
                ("multi_track_mapping", "Each source is stored on its own track in the output file (decoupled from the live mix); a track is exported only while the source is captured."),
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
                ("filter_noise_gate", "Gate (System)"),
                ("filter_expander", "Exp (System)"),
                ("filter_gain", "Gain (dB)"),
                ("filter_eq", "10-band EQ"),
                ("audio_monitoring", "Monitoring"),
                ("monitoring_system", "Monitor system audio"),
                ("monitoring_microphone", "Monitor microphone"),
                ("monitoring_volume", "Monitoring volume"),
                ("master_volume", "Master volume"),
                ("master_output", "Master output"),
                ("output_vu", "Output VU: {0} dB"),
                ("powered_by", "Powered by"),
                ("screen_recording_unavailable", "Screen recording is currently only available on Linux and Windows."),
                // Activity presence
                ("presence_ready", "Ready"),
                ("presence_recording", "Recording"),
                ("presence_streaming", "Streaming"),
                ("presence_recording_streaming", "Recording + streaming"),
                ("presence_paused", "Paused"),
                ("presence_error", "Error"),
                ("presence_legend", "What the status means"),
                ("presence_tooltip_ready", "Nothing is being recorded or streamed. This is the default state on startup and after recording/streaming stops."),
                ("presence_tooltip_recording", "A recording is active. Shown while capture is running and frames are being written."),
                ("presence_tooltip_streaming", "A stream to your ingest server is active. Shown while streaming is running."),
                ("presence_tooltip_recording_streaming", "Recording and streaming are both active at the same time."),
                ("presence_tooltip_paused", "A recording is active but paused: frames are captured but not written, and the elapsed timer is suspended."),
                ("presence_tooltip_error", "The engine or capture reported an error (e.g. a pipeline failure or no frames). Shown until the next recording/streaming start."),
                ("discord_presence_enable", "Show activity on Discord (Rich Presence)"),
                ("discord_presence_hint", "Share your recording/streaming status as
Discord Rich Presence. Never includes stream keys, URLs, paths or window titles."),
                ("discord_presence_privacy", "Only Rivulet and a localized status are shown; no stream keys, URLs, paths or window titles are ever transmitted."),
                ("discord_conn_off", "Not connected to the Discord desktop app (or activity disabled / no client ID)."),
                ("discord_conn_connecting", "Connecting to the Discord desktop app…"),
                ("discord_conn_connected", "Connected — Discord shows the Rivulet status."),
                ("discord_reconnect", "Reconnect"),
                ("discord_section", "Discord Rich Presence"),
                ("discord_client_id", "Application client ID"),
                ("discord_client_id_hint", "e.g. 1234567890123456789"),
                ("discord_client_id_apply", "Apply"),
                ("discord_client_id_error_not_numeric", "Invalid client ID: it must be a number. Paste only the numeric ID (not the application URL) without spaces."),
                ("discord_client_id_error_length", "Invalid client ID: Discord IDs are 17-20 digits. Check that the whole ID was copied."),
                ("discord_client_id_note", "Create a Discord application at discord.com/developers/applications, enable Rich Presence, and enter its client ID here. Empty keeps the adapter off."),
                ("discord_large_image", "Art asset key (optional)"),
                ("discord_large_image_hint", "e.g. rivulet_logo"),
                ("discord_large_image_note", "Upload an image under Rich Presence → Art Assets in the Discord Developer Portal and enter its asset name here. Discord then shows the Rivulet artwork on the activity card instead of the generic placeholder icon."),
                // OBS WebSocket remote control (Stream Deck / TouchPortal)
                ("obs_ws_section", "OBS WebSocket (Stream Deck)"),
                ("obs_ws_enable", "Enable remote control"),
                ("obs_ws_port", "Port"),
                ("obs_ws_password", "Password (optional)"),
                ("obs_ws_password_hint", "Empty disables authentication."),
                ("obs_ws_running", "Server running on ws://127.0.0.1:{0}"),
                ("obs_ws_stopped", "Server stopped"),
                ("obs_ws_error", "Could not start server: {0}"),
                ("obs_ws_hint", "OBS WebSocket v5 compatible endpoint for Stream Deck / TouchPortal. Scenes, sources, recording and streaming control, with optional password authentication."),
                // MIDI controller mapping
                ("midi_section", "MIDI controller"),
                ("midi_enable", "Enable MIDI input"),
                ("midi_device", "Device"),
                ("midi_no_devices", "No MIDI input devices found"),
                ("midi_channel", "Channel"),
                ("midi_kind", "Message"),
                ("midi_number", "Number"),
                ("midi_action", "Action"),
                ("midi_remove", "Remove"),
                ("midi_add", "Add binding"),
                ("midi_scene", "Scene"),
                ("midi_scene_none", "(no scene)"),
                ("midi_action_record", "Toggle recording"),
                ("midi_action_stream", "Toggle streaming"),
                ("midi_action_mute", "Toggle mute"),
                ("midi_action_volume", "Master volume"),
                ("midi_action_chroma", "Toggle chroma key"),
                ("midi_action_scene", "Switch scene"),
                ("midi_hint", "Map MIDI messages (Note/CC on a channel) to actions: scene switches, master volume fader (CC value 0-127), mute and chroma-key toggles. Great for controllers like the Korg NanoKontrol."),
                ("midi_learn", "Learn"),
                ("midi_learn_waiting", "Move a control on the controller…"),
                ("midi_learn_captured", "Captured"),
                ("midi_learn_clear", "Clear"),
                ("midi_presets", "Device presets"),
                ("midi_preset_name_hint", "Preset name"),
                ("midi_preset_save", "Save as preset"),
                ("midi_preset_load", "Apply"),
                ("midi_preset_apply", "Apply"),
                ("midi_preset_apply_hint", "Replace the current bindings with this preset"),
                ("midi_preset_delete", "Delete"),
                ("midi_preset_delete_hint", "Delete this preset from the selected device"),
                // Navigation
                ("nav_record", "Record"),
                ("nav_mixer", "Mixer"),
                ("nav_scenes", "Scenes"),
                ("nav_stream", "Stream"),
                ("nav_chat", "Chat"),
                ("nav_assistant", "Assistant"),
                ("nav_settings", "Settings"),
                ("nav_help", "Help"),
                ("chat_title", "Twitch Chat"),
                ("chat_channel_label", "Channel"),
                ("chat_channel_hint", "e.g. yourtwitchname"),
                ("chat_connect", "Connect"),
                ("chat_disconnect", "Disconnect"),
                ("chat_state_connected", "Connected"),
                ("chat_state_disconnected", "Disconnected — retrying"),
                ("chat_state_off", "Off"),
                ("chat_oauth_label", "OAuth token (optional)"),
                ("chat_oauth_hint", "oauth:..."),
                ("chat_note", "Anonymous read-only works without a token. With an OAuth token (chat:read) you get colors and badges. Tokens are never logged."),
                ("chat_empty", "No messages yet. Chat will appear here while connected."),
                ("help_user_guide", "User guide"),
                ("help_stream_setup", "Stream setup"),
                ("help_first_stream", "First-stream checklist"),
                ("help_troubleshooting", "Troubleshooting"),
                ("help_intro", "Help and documentation"),
                ("help_open_from_repository", "Open these Markdown pages from the repository or packaged documentation.") ,
                ("stream_health", "Stream health"),
                ("stream_status", "Status"),
                ("stream_m3_note", "Streaming controls and diagnostics are available."),
                ("stopped", "Stopped"),
                ("stream_status_connecting", "Connecting"),
                ("stream_status_good", "Good"),
                ("stream_status_warning", "Warning"),
                ("stream_status_poor", "Poor"),
                ("stream_status_offline", "Offline"),
                ("stream_rate", "Rate"),
                ("stream_queue_fill", "Queue fill"),
                ("stream_queue_underflows", "Queue underflows"),
                ("stream_queue_overflows", "Queue overflows"),
                ("stream_sink_latency", "Sink latency"),
                ("stream_health_help", "Queue and delay counters are cumulative for the active stream."),
                ("stream_ingest", "Ingest endpoint"),
                ("stream_key", "Stream key"),
                ("stream_key_save", "Save key securely"),
                ("stream_key_delete", "Delete saved key"),
                ("stream_key_saved", "Stream key saved in the operating system keyring."),
                ("stream_key_deleted", "Saved stream key deleted."),
                ("stream_key_store_unavailable", "Operating system keyring is unavailable."),
                ("stream_setup_assistant", "Setup assistant"),
                ("stream_test_connection", "Test connection"),
                ("stream_probe_running", "Testing ingest reachability…"),
                ("stream_probe_reachable", "Ingest endpoint is reachable (authentication not tested)."),
                ("stream_probe_rejected", "Stream configuration is invalid."),
                ("stream_probe_unavailable", "Ingest endpoint is unavailable."),
                ("stream_setup_assistant_title", "Rivulet stream setup"),
                ("stream_setup_assistant_active", "Setup assistant open"),
                ("stream_setup_step", "Step {0}"),
                ("stream_setup_choose_platform", "Choose a streaming platform."),
                ("stream_setup_credentials", "Enter the stream key."),
                ("stream_setup_key_never_logged", "The key is masked and never logged."),
                ("stream_setup_test_stream", "Run a short private test stream."),
                ("stream_setup_run_test", "Prepare test stream"),
                ("stream_setup_test_prepared", "Test stream prepared; start it with the Stream button."),
                ("stream_setup_test_private_only", "Use a private or unlisted stream only."),
                ("stream_setup_summary", "Review the setup and finish."),
                ("back", "Back"),
                ("next", "Next"),
                ("done", "Done"),
                ("cancel", "Cancel"),
                ("stream_start", "Start stream"),
                ("stream_stop", "Stop stream"),
                ("stream_configure_first", "Configure a valid secure endpoint and stream key first."),
                ("not_available", "n/a"),
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
                ("scenes_save_snapshot", "Save scene snapshot"),
                ("scenes_save_snapshot_hint", "Export the active scene layout as a PNG image."),
                ("scenes_snapshot_no_scene", "Select or create a scene before exporting a snapshot."),
                ("scenes_snapshot_saved", "Scene snapshot saved to {0}."),
                ("scenes_snapshot_failed", "Could not save scene snapshot: {0}"),
                ("scenes_collection", "Collection"),
                ("scenes_profile", "Profile"),
                ("scenes_export", "Export"),
                ("scenes_import", "Import"),
                ("scenes_exported", "Scene collection exported to {0}."),
                ("scenes_export_failed", "Could not export scene collection: {0}"),
                ("scenes_imported", "Scene collection imported from {0}."),
                ("scenes_import_failed", "Could not import scene collection: {0}"),
                ("scenes_hotkey", "Scene hotkey"),
                ("scene_overlay", "Scene overlay"),
                ("scene_overlay_enabled", "Enable overlay"),
                ("scene_overlay_text", "Overlay text"),
                ("scene_overlay_hint", "Text overlays are shown above the scene preview."),
                ("chroma_key", "Chroma key"),
                ("chroma_key_enabled", "Enable green-screen removal"),
                ("chroma_key_similarity", "Similarity"),
                ("chroma_key_smoothness", "Smoothness"),
                ("multi_view_enabled", "Show multi-view scene grid"),
                ("projector_open", "Open projector preview"),
                ("projector_close", "Close projector preview"),
                ("projector_fallback", "Projector preview is available in this window; a separate-display backend is pending."),
                ("scenes_assign_hotkey", "Assign"),
                ("scenes_hotkey_assigned", "Scene hotkey assigned."),
                ("scenes_auto_switch", "Auto-switch by window title"),
                ("scenes_add_auto_switch", "Add rule"),
                ("scenes_duplicate", "Duplicate"),
                ("scenes_duplicate_name", "Copy of {0}"),
                ("transition", "Transition"),
                ("transition_duration", "Duration"),
                ("transition_in_progress", "Transitioning…"),
                ("studio_mode", "Studio Mode"),
                ("studio_mode_enabled", "Studio Mode enabled."),
                ("studio_mode_disabled", "Studio Mode disabled."),
                ("studio_program_label", "Program: {0}"),
                ("studio_preview_label", "Preview: {0}"),
                ("studio_take", "Take"),
                ("studio_take_hint", "Take the selected Preview scene live with the configured transition."),
                ("studio_preview_selected", "Preview selected: \"{0}\"."),
                ("studio_taken", "Preview taken live: \"{0}\"."),
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
                ("recording_preview_title", "Aufnahmevorschau"),
                ("recording_preview_ready", "Vorschau bereit"),
                ("recording_preview_active", "Aufnahmevorschau aktiv"),
                ("recording_preview_waiting", "Warte auf den ersten Frame…"),
                ("recording_preview_select_source", "Wähle eine Quelle für die Vorschau aus."),
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
                ("alert_overlay_title", "Alert-Overlay importieren"),
                ("alert_provider_streamlabs", "Streamlabs"),
                ("alert_provider_streamelements", "StreamElements"),
                ("alert_provider_custom", "Eigene URL"),
                ("alert_token", "Widget-Token"),
                ("alert_token_hint", "Token aus dem Dashboard einfügen"),
                ("alert_import", "Overlay importieren"),
                ("alert_imported", "Alert-Overlay in die Browserquelle importiert."),
                ("alert_overlay_note", "Die Browserquelle rendert die Widget-Seite; deaktiviere Interaktion, damit das Overlay nie den Fokus stiehlt."),
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
                ("hotkeys_section", "Tastenkürzel"),
                ("hotkeys_hint", "In-App-Tastenkürzel umbelegen. Unter Windows funktionieren sie auch, wenn die App nicht fokussiert ist."),
                ("modifier_ctrl", "Strg"),
                ("modifier_alt", "Alt"),
                ("modifier_shift", "Umschalt"),
                ("paused", "Pausiert"),
                ("muted", "Stumm"),
                // Codec selection
                ("video_codec", "Video-Codec"),
                ("recording_container", "Container"),
                ("auto_remux", "Nach Stopp zu MP4 remuxen"),
                ("rate_control", "Rate-Control"),
                ("rate_mode", "Modus"),
                ("video_effects", "Video-Effekte"),
                ("vf_brightness", "Helligkeit"),
                ("vf_contrast", "Kontrast"),
                ("vf_saturation", "Sättigung"),
                ("vf_hue", "Farbton"),
                ("vf_blur", "Weichzeichnen"),
                ("vf_sharpen", "Schärfen"),
                ("rate_quality", "Qualität (0-51, niedriger ist besser)"),
                ("rate_max_bitrate", "Max. Bitrate (kbit/s)"),
                ("rate_custom_options", "Zusätzliche Encoder-Optionen"),
                ("split_after", "Aufteilen nach (s):"),
                ("seconds", "s"),
                ("auto_record_with_stream", "Automatisch mit Stream aufnehmen"),
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
                ("export_track_system", "Systemaudio auf eigenem Track exportieren"),
                ("export_track_microphone", "Mikrofon auf eigenem Track exportieren"),
                ("multi_track_mapping", "Jede Quelle wird auf einem eigenen Track in der Ausgabedatei gespeichert (getrennt vom Live-Mix); ein Track wird nur exportiert, solange die Quelle erfasst wird."),
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
                ("filter_noise_gate", "Gate (System)"),
                ("filter_expander", "Exp (System)"),
                ("filter_gain", "Verstärkung (dB)"),
                ("filter_eq", "10-Band-EQ"),
                ("audio_monitoring", "Monitoring"),
                ("monitoring_system", "Systemaudio abhören"),
                ("monitoring_microphone", "Mikrofon abhören"),
                ("monitoring_volume", "Monitoring-Lautstärke"),
                ("master_volume", "Master-Lautstärke"),
                ("master_output", "Master-Ausgabe"),
                ("output_vu", "Ausgangs-VU: {0} dB"),
                ("powered_by", "Unterstützt von"),
                ("screen_recording_unavailable", "Bildschirmaufnahme ist derzeit nur unter Linux und Windows verfügbar."),
                // Activity presence
                ("presence_ready", "Bereit"),
                ("presence_recording", "Aufnahme"),
                ("presence_streaming", "Streamt"),
                ("presence_recording_streaming", "Aufnahme + Stream"),
                ("presence_paused", "Pausiert"),
                ("presence_error", "Fehler"),
                ("presence_legend", "Was der Status bedeutet"),
                ("presence_tooltip_ready", "Es wird weder aufgenommen noch gestreamt. Standardzustand beim Start und nach dem Stoppen von Aufnahme/Stream."),
                ("presence_tooltip_recording", "Eine Aufnahme ist aktiv. Wird angezeigt, solange Capture läuft und Frames geschrieben werden."),
                ("presence_tooltip_streaming", "Ein Stream zu deinem Ingest-Server ist aktiv. Wird angezeigt, solange gestreamt wird."),
                ("presence_tooltip_recording_streaming", "Aufnahme und Stream laufen gleichzeitig."),
                ("presence_tooltip_paused", "Eine Aufnahme ist aktiv, aber pausiert: Frames werden erfasst, aber nicht geschrieben; der Zeitmesser läuft nicht weiter."),
                ("presence_tooltip_error", "Die Engine oder der Capture meldet einen Fehler (z. B. Pipeline-Fehler oder keine Frames). Wird angezeigt, bis der nächste Aufnahme-/Stream-Start beginnt."),
                ("discord_presence_enable", "Aktivität auf Discord anzeigen (Rich Presence)"),
                ("discord_presence_hint", "Teile deinen Aufnahme-/Stream-Status als
Discord Rich Presence. Enthält nie Stream-Keys, URLs, Pfade oder Fenstertitel."),
                ("discord_presence_privacy", "Es werden nur Rivulet und ein lokalisierter Status angezeigt; Stream-Keys, URLs, Pfade oder Fenstertitel werden nie übertragen."),
                ("discord_conn_off", "Nicht mit der Discord-Desktop-App verbunden (oder Aktivität deaktiviert / keine Client-ID)."),
                ("discord_conn_connecting", "Verbinde mit der Discord-Desktop-App…"),
                ("discord_conn_connected", "Verbunden — Discord zeigt den Rivulet-Status."),
                ("discord_reconnect", "Erneut verbinden"),
                ("discord_section", "Discord Rich Presence"),
                ("discord_client_id", "Anwendungs-Client-ID"),
                ("discord_client_id_hint", "z. B. 1234567890123456789"),
                ("discord_client_id_apply", "Übernehmen"),
                ("discord_client_id_error_not_numeric", "Ungültige Client-ID: Sie muss eine Zahl sein. Füge nur die numerische ID ein (nicht die App-URL) und ohne Leerzeichen."),
                ("discord_client_id_error_length", "Ungültige Client-ID: Discord-IDs haben 17-20 Ziffern. Prüfe, ob die gesamte ID kopiert wurde."),
                ("discord_client_id_note", "Erstelle eine Discord-App unter discord.com/developers/applications, aktiviere Rich Presence und trage hier ihre Client-ID ein. Leer hält den Adapter deaktiviert."),
                ("discord_large_image", "Art-Asset-Schlüssel (optional)"),
                ("discord_large_image_hint", "z. B. rivulet_logo"),
                ("discord_large_image_note", "Lade unter Rich Presence → Art Assets im Discord-Developer-Portal ein Bild hoch und trage hier seinen Asset-Namen ein. Discord zeigt dann das Rivulet-Artwork auf der Activity-Karte statt des generischen Platzhalter-Icons."),
                // OBS WebSocket Fernbedienung (Stream Deck / TouchPortal)
                ("obs_ws_section", "OBS WebSocket (Stream Deck)"),
                ("obs_ws_enable", "Fernbedienung aktivieren"),
                ("obs_ws_port", "Port"),
                ("obs_ws_password", "Passwort (optional)"),
                ("obs_ws_password_hint", "Leer deaktiviert die Authentifizierung."),
                ("obs_ws_running", "Server läuft unter ws://127.0.0.1:{0}"),
                ("obs_ws_stopped", "Server gestoppt"),
                ("obs_ws_error", "Server konnte nicht gestartet werden: {0}"),
                ("obs_ws_hint", "OBS-WebSocket-v5-kompatibler Endpunkt für Stream Deck / TouchPortal. Szenen, Quellen, Aufnahme- und Streaming-Steuerung, optional mit Passwort-Authentifizierung."),
                // MIDI-Controller-Mapping
                ("midi_section", "MIDI-Controller"),
                ("midi_enable", "MIDI-Eingang aktivieren"),
                ("midi_device", "Gerät"),
                ("midi_no_devices", "Keine MIDI-Eingabegeräte gefunden"),
                ("midi_channel", "Kanal"),
                ("midi_kind", "Nachricht"),
                ("midi_number", "Nummer"),
                ("midi_action", "Aktion"),
                ("midi_remove", "Entfernen"),
                ("midi_add", "Binding hinzufügen"),
                ("midi_scene", "Szene"),
                ("midi_scene_none", "(keine Szene)"),
                ("midi_action_record", "Aufnahme umschalten"),
                ("midi_action_stream", "Streaming umschalten"),
                ("midi_action_mute", "Stummschaltung umschalten"),
                ("midi_action_volume", "Master-Lautstärke"),
                ("midi_action_chroma", "Chroma-Key umschalten"),
                ("midi_action_scene", "Szene wechseln"),
                ("midi_hint", "MIDI-Nachrichten (Note/CC auf einem Kanal) auf Aktionen mappen: Szenenwechsel, Master-Lautstärke-Fader (CC-Wert 0-127), Stummschaltung und Chroma-Key-Toggles. Ideal für Controller wie den Korg NanoKontrol."),
                ("midi_learn", "Lernen"),
                ("midi_learn_waiting", "Bewege ein Bedienelement am Controller…"),
                ("midi_learn_captured", "Erfasst"),
                ("midi_learn_clear", "Zurücksetzen"),
                ("midi_presets", "Geräte-Presets"),
                ("midi_preset_name_hint", "Preset-Name"),
                ("midi_preset_save", "Als Preset speichern"),
                ("midi_preset_load", "Übernehmen"),
                ("midi_preset_apply", "Übernehmen"),
                ("midi_preset_apply_hint", "Ersetzt die aktuellen Bindings durch dieses Preset"),
                ("midi_preset_delete", "Löschen"),
                ("midi_preset_delete_hint", "Preset für das ausgewählte Gerät löschen"),
                // Navigation
                ("nav_record", "Aufnehmen"),
                ("nav_mixer", "Mixer"),
                ("nav_scenes", "Szenen"),
                ("nav_stream", "Stream"),
                ("nav_chat", "Chat"),
                ("nav_assistant", "Assistant"),
                ("nav_settings", "Einstellungen"),
                ("nav_help", "Hilfe"),
                ("chat_title", "Twitch-Chat"),
                ("chat_channel_label", "Kanal"),
                ("chat_channel_hint", "z. B. deintwitchname"),
                ("chat_connect", "Verbinden"),
                ("chat_disconnect", "Trennen"),
                ("chat_state_connected", "Verbunden"),
                ("chat_state_disconnected", "Getrennt — erneuter Versuch"),
                ("chat_state_off", "Aus"),
                ("chat_oauth_label", "OAuth-Token (optional)"),
                ("chat_oauth_hint", "oauth:..."),
                ("chat_note", "Anonymes Lesen funktioniert ohne Token. Mit OAuth-Token (chat:read) erhältst du Farben und Badges. Tokens werden nie geloggt."),
                ("chat_empty", "Noch keine Nachrichten. Chat erscheint hier, solange verbunden."),
                ("help_user_guide", "Bedienungsanleitung"),
                ("help_stream_setup", "Stream-Einrichtung"),
                ("help_first_stream", "Checkliste für den ersten Stream"),
                ("help_troubleshooting", "Fehlerbehebung"),
                ("help_intro", "Hilfe und Dokumentation"),
                ("help_open_from_repository", "Diese Markdown-Seiten im Repository oder in der mitgelieferten Dokumentation öffnen."),
                ("stream_health", "Stream-Zustand"),
                ("stream_status", "Status"),
                ("stream_m3_note", "Streaming-Steuerung und Diagnose sind verfügbar."),
                ("stopped", "Gestoppt"),
                ("stream_status_connecting", "Verbindung wird hergestellt"),
                ("stream_status_good", "Gut"),
                ("stream_status_warning", "Warnung"),
                ("stream_status_poor", "Kritisch"),
                ("stream_status_offline", "Offline"),
                ("stream_rate", "Rate"),
                ("stream_queue_fill", "Warteschlangenfüllstand"),
                ("stream_queue_underflows", "Queue-Underflows"),
                ("stream_queue_overflows", "Queue-Overflows"),
                ("stream_sink_latency", "Sink-Latenz"),
                ("stream_health_help", "Queue- und Delay-Zähler gelten kumulativ für den aktiven Stream."),
                ("stream_ingest", "Ingest-Endpunkt"),
                ("stream_key", "Stream-Key"),
                ("stream_key_save", "Key sicher speichern"),
                ("stream_key_delete", "Gespeicherten Key löschen"),
                ("stream_key_saved", "Stream-Key im Betriebssystem-Schlüsselbund gespeichert."),
                ("stream_key_deleted", "Gespeicherter Stream-Key gelöscht."),
                ("stream_key_store_unavailable", "Betriebssystem-Schlüsselbund ist nicht verfügbar."),
                ("stream_setup_assistant", "Einrichtungsassistent"),
                ("stream_test_connection", "Verbindung testen"),
                ("stream_probe_running", "Ingest-Erreichbarkeit wird geprüft …"),
                ("stream_probe_reachable", "Ingest-Endpunkt ist erreichbar (Authentifizierung nicht geprüft)."),
                ("stream_probe_rejected", "Die Stream-Konfiguration ist ungültig."),
                ("stream_probe_unavailable", "Der Ingest-Endpunkt ist nicht erreichbar."),
                ("stream_setup_assistant_title", "Rivulet-Stream-Einrichtung"),
                ("stream_setup_assistant_active", "Assistent geöffnet"),
                ("stream_setup_step", "Schritt {0}"),
                ("stream_setup_choose_platform", "Streaming-Plattform ausw�hlen."),
                ("stream_setup_credentials", "Stream-Key eingeben."),
                ("stream_setup_key_never_logged", "Der Key wird maskiert und nie geloggt."),
                ("stream_setup_test_stream", "Kurzen privaten Teststream vorbereiten."),
                ("stream_setup_run_test", "Teststream vorbereiten"),
                ("stream_setup_test_prepared", "Teststream vorbereitet; mit dem Stream-Button starten."),
                ("stream_setup_test_private_only", "Nur privaten oder nicht gelisteten Stream verwenden."),
                ("stream_setup_summary", "Einrichtung pr�fen und abschlie�en."),
                ("back", "Zur�ck"),
                ("next", "Weiter"),
                ("done", "Fertig"),
                ("cancel", "Abbrechen"),
                ("stream_start", "Stream starten"),
                ("stream_stop", "Stream stoppen"),
                ("stream_configure_first", "Zuerst einen gültigen sicheren Endpunkt und Stream-Key konfigurieren."),
                ("not_available", "n. a."),
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
                ("scenes_save_snapshot", "Szenen-Snapshot speichern"),
                ("scenes_save_snapshot_hint", "Das Layout der aktiven Szene als PNG-Bild exportieren."),
                ("scenes_snapshot_no_scene", "Vor dem Export zuerst eine Szene auswählen oder erstellen."),
                ("scenes_snapshot_saved", "Szenen-Snapshot gespeichert unter {0}."),
                ("scenes_snapshot_failed", "Szenen-Snapshot konnte nicht gespeichert werden: {0}"),
                ("scenes_collection", "Sammlung"),
                ("scenes_profile", "Profil"),
                ("scenes_export", "Exportieren"),
                ("scenes_import", "Importieren"),
                ("scenes_exported", "Szenensammlung exportiert nach {0}."),
                ("scenes_export_failed", "Szenensammlung konnte nicht exportiert werden: {0}"),
                ("scenes_imported", "Szenensammlung importiert aus {0}."),
                ("scenes_import_failed", "Szenensammlung konnte nicht importiert werden: {0}"),
                ("scenes_hotkey", "Szenen-Hotkey"),
                ("scene_overlay", "Szenen-Overlay"),
                ("scene_overlay_enabled", "Overlay aktivieren"),
                ("scene_overlay_text", "Overlay-Text"),
                ("scene_overlay_hint", "Text-Overlays werden über der Szenenvorschau angezeigt."),
                ("chroma_key", "Chroma-Key"),
                ("chroma_key_enabled", "Greenscreen-Entfernung aktivieren"),
                ("chroma_key_similarity", "Ähnlichkeit"),
                ("chroma_key_smoothness", "Weichheit"),
                ("multi_view_enabled", "Szenen-Multiview anzeigen"),
                ("projector_open", "Projektorvorschau öffnen"),
                ("projector_close", "Projektorvorschau schließen"),
                ("projector_fallback", "Die Projektorvorschau ist in diesem Fenster verfügbar; ein Backend für separate Displays folgt."),
                ("scenes_assign_hotkey", "Zuweisen"),
                ("scenes_hotkey_assigned", "Szenen-Hotkey zugewiesen."),
                ("scenes_auto_switch", "Automatischer Wechsel nach Fenstertitel"),
                ("scenes_add_auto_switch", "Regel hinzufügen"),
                ("scenes_duplicate", "Duplizieren"),
                ("scenes_duplicate_name", "Kopie von {0}"),
                ("transition", "Übergang"),
                ("transition_duration", "Dauer"),
                ("transition_in_progress", "Übergang läuft…"),
                ("studio_mode", "Studio-Modus"),
                ("studio_mode_enabled", "Studio-Modus aktiviert."),
                ("studio_mode_disabled", "Studio-Modus deaktiviert."),
                ("studio_program_label", "Programm: {0}"),
                ("studio_preview_label", "Vorschau: {0}"),
                ("studio_take", "Übernehmen"),
                ("studio_take_hint", "Die ausgewählte Vorschau mit dem konfigurierten Übergang live schalten."),
                ("studio_preview_selected", "Vorschau ausgewählt: \"{0}\"."),
                ("studio_taken", "Vorschau live geschaltet: \"{0}\"."),
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
    fn recording_preview_keys_translate_in_both_locales() {
        assert_eq!(
            Locale::En.tr("recording_preview_title"),
            "Recording preview"
        );
        assert_eq!(Locale::De.tr("recording_preview_title"), "Aufnahmevorschau");
        assert_eq!(
            Locale::En.tr("recording_preview_select_source"),
            "Select a source to preview it."
        );
        assert_eq!(
            Locale::De.tr("recording_preview_select_source"),
            "Wähle eine Quelle für die Vorschau aus."
        );
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
    fn scene_snapshot_keys_translate_in_both_locales() {
        assert_eq!(Locale::En.tr("scenes_save_snapshot"), "Save scene snapshot");
        assert_eq!(
            Locale::De.tr("scenes_save_snapshot"),
            "Szenen-Snapshot speichern"
        );
        assert_eq!(
            Locale::En.tr_fmt("scenes_snapshot_saved", &["scene.png".to_string()]),
            "Scene snapshot saved to scene.png."
        );
        assert_eq!(
            Locale::De.tr_fmt("scenes_snapshot_saved", &["scene.png".to_string()]),
            "Szenen-Snapshot gespeichert unter scene.png."
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
        assert_eq!(Locale::En.tr("studio_mode"), "Studio Mode");
        assert_eq!(Locale::De.tr("studio_mode"), "Studio-Modus");
        assert_eq!(
            Locale::En.tr_fmt("studio_taken", &["Live".to_string()]),
            "Preview taken live: \"Live\"."
        );
        assert_eq!(
            Locale::De.tr_fmt("studio_taken", &["Live".to_string()]),
            "Vorschau live geschaltet: \"Live\"."
        );
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

    #[test]
    fn master_mix_keys_translate_in_both_locales() {
        assert_eq!(Locale::En.tr("master_volume"), "Master volume");
        assert_eq!(Locale::En.tr("master_output"), "Master output");
        assert_eq!(
            Locale::En.tr_fmt("output_vu", &["-6.0".to_string()]),
            "Output VU: -6.0 dB"
        );
        assert_eq!(Locale::De.tr("master_volume"), "Master-Lautstärke");
        assert_eq!(Locale::De.tr("master_output"), "Master-Ausgabe");
        assert_eq!(
            Locale::De.tr_fmt("output_vu", &["-6.0".to_string()]),
            "Ausgangs-VU: -6.0 dB"
        );
    }

    #[test]
    fn rate_control_keys_translate_in_both_locales() {
        assert_eq!(Locale::En.tr("rate_control"), "Rate control");
        assert_eq!(Locale::En.tr("rate_mode"), "Mode");
        assert_eq!(
            Locale::En.tr("rate_quality"),
            "Quality (0-51, lower is better)"
        );
        assert_eq!(Locale::En.tr("rate_max_bitrate"), "Max bitrate (kbit/s)");
        assert_eq!(
            Locale::En.tr("rate_custom_options"),
            "Extra encoder options"
        );
        assert_eq!(Locale::De.tr("rate_control"), "Rate-Control");
        assert_eq!(Locale::De.tr("rate_mode"), "Modus");
        assert_eq!(
            Locale::De.tr("rate_quality"),
            "Qualität (0-51, niedriger ist besser)"
        );
        assert_eq!(Locale::De.tr("rate_max_bitrate"), "Max. Bitrate (kbit/s)");
        assert_eq!(
            Locale::De.tr("rate_custom_options"),
            "Zusätzliche Encoder-Optionen"
        );
    }

    #[test]
    fn filter_keys_translate_in_both_locales() {
        assert_eq!(Locale::En.tr("video_effects"), "Video effects");
        assert_eq!(Locale::En.tr("vf_brightness"), "Brightness");
        assert_eq!(Locale::En.tr("vf_contrast"), "Contrast");
        assert_eq!(Locale::En.tr("vf_saturation"), "Saturation");
        assert_eq!(Locale::En.tr("vf_hue"), "Hue");
        assert_eq!(Locale::En.tr("vf_blur"), "Blur");
        assert_eq!(Locale::En.tr("vf_sharpen"), "Sharpen");
        assert_eq!(Locale::De.tr("video_effects"), "Video-Effekte");
        assert_eq!(Locale::De.tr("vf_brightness"), "Helligkeit");
        assert_eq!(Locale::De.tr("vf_saturation"), "Sättigung");
        assert_eq!(Locale::De.tr("filter_gain"), "Verstärkung (dB)");
        assert_eq!(Locale::De.tr("filter_eq"), "10-Band-EQ");
        assert_eq!(Locale::En.tr("filter_gain"), "Gain (dB)");
    }
}
