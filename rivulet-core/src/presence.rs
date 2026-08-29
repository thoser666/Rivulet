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
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "Bereit",
            Self::Recording => "Aufnahme",
            Self::Streaming => "Streamt",
            Self::RecordingAndStreaming => "Aufnahme + Stream",
            Self::Paused => "Pausiert",
            Self::Error => "Fehler",
        }
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
    /// Build a status without stream keys, URLs, file paths, or window titles.
    pub fn for_activity(activity: PresenceActivity) -> Self {
        let details = match activity {
            PresenceActivity::Idle => "Rivulet bereit".to_owned(),
            PresenceActivity::Recording => "Rivulet nimmt auf".to_owned(),
            PresenceActivity::Streaming => "Rivulet streamt".to_owned(),
            PresenceActivity::RecordingAndStreaming => "Rivulet nimmt auf und streamt".to_owned(),
            PresenceActivity::Paused => "Rivulet pausiert".to_owned(),
            PresenceActivity::Error => "Rivulet benötigt Aufmerksamkeit".to_owned(),
        };
        Self {
            application: "Rivulet",
            details,
            state: activity.label().to_owned(),
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
            "Aufnahme + Stream"
        );
    }

    #[test]
    fn payload_contains_no_sensitive_runtime_data() {
        let status = PresenceStatus::for_activity(PresenceActivity::Streaming);
        assert!(!status.details.contains("rtmp"));
        assert!(!status.details.contains("key"));
        assert!(!status.details.contains("/") && !status.details.contains("\\"));
    }
}
