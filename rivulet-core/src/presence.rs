//! Product activity status for optional external presence integrations.
//!
//! This module only builds a status payload. It does not connect to Discord or
//! transmit anything; platform adapters can consume the payload later.

/// Current high-level activity shown to a presence provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceActivity {
    Idle,
    Recording,
    Streaming,
    RecordingAndStreaming,
    Paused,
    Error,
}

impl PresenceActivity {
    /// Stable i18n key consumed by the locale tables.
    pub const fn i18n_key(self) -> &'static str {
        match self {
            Self::Idle => "presence_ready",
            Self::Recording => "presence_recording",
            Self::Streaming => "presence_streaming",
            Self::RecordingAndStreaming => "presence_recording_streaming",
            Self::Paused => "presence_paused",
            Self::Error => "presence_error",
        }
    }

    /// English fallback for integrations that do not have a locale.
    pub const fn fallback_label(self) -> &'static str {
        match self {
            Self::Idle => "Ready",
            Self::Recording => "Recording",
            Self::Streaming => "Streaming",
            Self::RecordingAndStreaming => "Recording + streaming",
            Self::Paused => "Paused",
            Self::Error => "Error",
        }
    }

    pub fn label(self) -> &'static str {
        self.fallback_label()
    }
}

/// A privacy-safe activity payload suitable for Discord Rich Presence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresenceStatus {
    pub application: &'static str,
    pub details: String,
    pub state: String,
}

impl PresenceStatus {
    /// Build an English fallback status without sensitive runtime data.
    pub fn for_activity(activity: PresenceActivity) -> Self {
        Self::for_activity_localized(activity, crate::Locale::En, None)
    }

    /// Build a localized status and optionally include the current game name.
    /// The game name is user-selected metadata; callers must not pass window
    /// titles or other untrusted sensitive data.
    pub fn for_activity_localized(
        activity: PresenceActivity,
        locale: crate::Locale,
        game_name: Option<&str>,
    ) -> Self {
        let label = locale.tr(activity.i18n_key());
        let details = match game_name.filter(|name| !name.trim().is_empty()) {
            Some(_game) => format!("Rivulet · {label}"),
            None => format!("Rivulet · {label}"),
        };
        let state = match game_name.filter(|name| !name.trim().is_empty()) {
            Some(game) => format!("{label} · {game}"),
            None => label.to_owned(),
        };
        Self {
            application: "Rivulet",
            details,
            state,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_always_identifies_rivulet() {
        for activity in [
            PresenceActivity::Idle,
            PresenceActivity::Recording,
            PresenceActivity::Streaming,
            PresenceActivity::RecordingAndStreaming,
            PresenceActivity::Paused,
            PresenceActivity::Error,
        ] {
            let status = PresenceStatus::for_activity(activity);
            assert_eq!(status.application, "Rivulet");
            assert!(status.details.starts_with("Rivulet"));
            assert!(status.state.contains(activity.fallback_label()));
        }
    }

    #[test]
    fn recording_and_streaming_are_distinguished() {
        assert_ne!(
            PresenceStatus::for_activity(PresenceActivity::Recording),
            PresenceStatus::for_activity(PresenceActivity::Streaming)
        );
        assert_eq!(
            PresenceStatus::for_activity(PresenceActivity::RecordingAndStreaming).state,
            "Recording + streaming"
        );
    }

    #[test]
    fn localized_status_contains_game_name_without_sensitive_data() {
        let status = PresenceStatus::for_activity_localized(
            PresenceActivity::Streaming,
            crate::Locale::De,
            Some("Elden Ring"),
        );
        assert_eq!(status.details, "Rivulet · Streamt");
        assert_eq!(status.state, "Streamt · Elden Ring");
        assert!(!status.state.contains("rtmp"));
        assert!(!status.state.contains("/"));
    }

    #[test]
    fn payload_contains_no_sensitive_runtime_data() {
        let status = PresenceStatus::for_activity(PresenceActivity::Streaming);
        assert!(!status.details.contains("rtmp"));
        assert!(!status.details.contains("key"));
        assert!(!status.details.contains("/") && !status.details.contains("\\"));
    }
}
