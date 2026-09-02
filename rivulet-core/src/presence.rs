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

    /// All states in display (and priority) order, used by the status legend
    /// in the Stream view.
    pub const fn all() -> [PresenceActivity; 6] {
        [
            PresenceActivity::Error,
            PresenceActivity::RecordingAndStreaming,
            PresenceActivity::Paused,
            PresenceActivity::Recording,
            PresenceActivity::Streaming,
            PresenceActivity::Idle,
        ]
    }

    /// Stable i18n key for the tooltip that explains when this state appears
    /// and what it means. Consumed by the status legend in the Stream view;
    /// both locales must translate every key (covered by the i18n parity test).
    pub const fn tooltip_i18n_key(self) -> &'static str {
        match self {
            Self::Idle => "presence_tooltip_ready",
            Self::Recording => "presence_tooltip_recording",
            Self::Streaming => "presence_tooltip_streaming",
            Self::RecordingAndStreaming => "presence_tooltip_recording_streaming",
            Self::Paused => "presence_tooltip_paused",
            Self::Error => "presence_tooltip_error",
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
    /// Build an English fallback status without sensitive runtime data.
    pub fn for_activity(activity: PresenceActivity) -> Self {
        Self::for_activity_localized(activity, crate::Locale::En, None)
    }

    /// Build a localized status and optionally include the current game name.
    /// The game name is user-selected metadata; callers must not pass window
    /// titles or other untrusted sensitive data.
    ///
    /// The presence is split into two lines: the status label (`details`, e.g.
    /// "Recording") and the game name (`state`, when a game source is
    /// selected). Discord renders `details` as the first line of the card and
    /// `state` as the second line beneath it. The application name "Rivulet"
    /// is supplied by the Discord application registration itself and must not
    /// be duplicated here — Discord renders it as the card title together
    /// with the configured large image.
    pub fn for_activity_localized(
        activity: PresenceActivity,
        locale: crate::Locale,
        game_name: Option<&str>,
    ) -> Self {
        let label = locale.tr(activity.i18n_key());
        let game = game_name.filter(|name| !name.trim().is_empty());
        // `details` is Discord's first card line, `state` the second, so the
        // status label lives in `details` and the game name in `state` (empty
        // when no game is selected — the serializer omits it, since Discord
        // rejects empty strings with 4000).
        let details = label.to_owned();
        let state = match game {
            Some(game) => game.to_owned(),
            None => String::new(),
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
            // The app name is rendered by the Discord application registration
            // (card title), never duplicated into state or details. Without a
            // game name the status label lives in `details`, `state` is empty.
            assert!(!status.details.contains("Rivulet"));
            assert!(status.details.contains(activity.fallback_label()));
            assert!(status.state.is_empty());
        }
    }

    #[test]
    fn recording_and_streaming_are_distinguished() {
        assert_ne!(
            PresenceStatus::for_activity(PresenceActivity::Recording),
            PresenceStatus::for_activity(PresenceActivity::Streaming)
        );
        assert_eq!(
            PresenceStatus::for_activity(PresenceActivity::RecordingAndStreaming).details,
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
        // Line layout: `details` carries the status label (first card line),
        // `state` the game name (second line), and the app name stays out of
        // both (Discord shows it as the title).
        assert_eq!(status.details, "Streamt");
        assert_eq!(status.state, "Elden Ring");
        assert!(!status.details.contains("rtmp"));
        assert!(!status.details.contains("/"));
    }

    #[test]
    fn without_game_name_state_stays_empty() {
        let status = PresenceStatus::for_activity_localized(
            PresenceActivity::Recording,
            crate::Locale::En,
            None,
        );
        assert_eq!(status.details, "Recording");
        assert!(status.state.is_empty());
    }

    #[test]
    fn payload_contains_no_sensitive_runtime_data() {
        let status = PresenceStatus::for_activity(PresenceActivity::Streaming);
        // Without a game name only the status label is on the wire.
        assert!(!status.details.contains("rtmp"));
        assert!(!status.details.contains("key"));
        assert!(!status.details.contains("/") && !status.details.contains("\\"));
        assert!(status.state.is_empty());
    }

    #[test]
    fn all_states_have_distinct_tooltips_in_both_locales() {
        // Every state in the legend needs a tooltip in both locales; the
        // i18n parity test additionally guarantees both tables agree on keys.
        let mut keys = Vec::new();
        for activity in PresenceActivity::all() {
            let key = activity.tooltip_i18n_key();
            assert!(key.starts_with("presence_tooltip_"), "bad key: {key}");
            assert!(
                !crate::Locale::En.tr(key).is_empty() && crate::Locale::En.tr(key) != key,
                "missing EN tooltip for {key}"
            );
            assert!(
                !crate::Locale::De.tr(key).is_empty() && crate::Locale::De.tr(key) != key,
                "missing DE tooltip for {key}"
            );
            keys.push(key);
        }
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 6, "legend must cover all six states exactly");
    }

    #[test]
    fn all_states_cover_every_activity_variant() {
        // The legend is the source of truth for display order; it must not
        // silently drop a variant when a new activity is added.
        for variant in [
            PresenceActivity::Idle,
            PresenceActivity::Recording,
            PresenceActivity::Streaming,
            PresenceActivity::RecordingAndStreaming,
            PresenceActivity::Paused,
            PresenceActivity::Error,
        ] {
            assert!(
                PresenceActivity::all().contains(&variant),
                "legend missing {variant:?}"
            );
        }
    }
}
