//! Streaming support for Rivulet.
//!
//! Twitch, Kick and YouTube all ingest via **RTMPS** (RTMP over TLS), so a
//! single [`StreamSettings`] with an `rtmps://` ingest URL works for all three
//! platforms - only the ingest endpoint and stream key differ.

/// Streaming platform presets used to build the ingest URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamPlatform {
    /// `rtmps://live.twitch.tv/app/<stream key>`
    Twitch,
    /// AWS IVS based ingest; the endpoint is streamer-specific and is taken
    /// from the Kick dashboard, e.g.
    /// `rtmps://<id>.global-contribute.live-video.net/app`
    Kick,
    /// `rtmps://a.rtmp.youtube.com/live2/<stream key>`
    YouTube,
    /// Any other RTMP/RTMPS ingest server.
    Custom,
}

impl StreamPlatform {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Twitch => "Twitch",
            Self::Kick => "Kick",
            Self::YouTube => "YouTube",
            Self::Custom => "Custom",
        }
    }
}

/// Settings describing a single RTMP/RTMPS stream output.
#[derive(Debug, Clone)]
pub struct StreamSettings {
    /// Platform label used for display purposes.
    pub platform: StreamPlatform,
    /// Ingest base URL **without** the stream key, e.g.
    /// `rtmps://live.twitch.tv/app`.
    pub ingest_url: String,
    /// The stream key as shown in the platform dashboard.
    pub stream_key: String,
}

impl StreamSettings {
    /// Create a settings object from an explicit ingest base URL and key.
    pub fn new(
        platform: StreamPlatform,
        ingest_url: impl Into<String>,
        stream_key: impl Into<String>,
    ) -> Self {
        Self {
            platform,
            ingest_url: ingest_url.into(),
            stream_key: stream_key.into(),
        }
    }

    /// Preset for Twitch: `rtmps://live.twitch.tv/app/<key>`.
    pub fn twitch(stream_key: impl Into<String>) -> Self {
        Self::new(
            StreamPlatform::Twitch,
            "rtmps://live.twitch.tv/app",
            stream_key,
        )
    }

    /// Preset for YouTube: `rtmps://a.rtmp.youtube.com/live2/<key>`.
    pub fn youtube(stream_key: impl Into<String>) -> Self {
        Self::new(
            StreamPlatform::YouTube,
            "rtmps://a.rtmp.youtube.com/live2",
            stream_key,
        )
    }

    /// Preset for Kick. The ingest endpoint is streamer-specific (AWS IVS),
    /// so it must be taken from the Kick dashboard, e.g.
    /// `rtmps://<id>.global-contribute.live-video.net/app`.
    pub fn kick(ingest_url: impl Into<String>, stream_key: impl Into<String>) -> Self {
        Self::new(StreamPlatform::Kick, ingest_url, stream_key)
    }

    /// Use a fully custom RTMP/RTMPS ingest server.
    pub fn custom(ingest_url: impl Into<String>, stream_key: impl Into<String>) -> Self {
        Self::new(StreamPlatform::Custom, ingest_url, stream_key)
    }

    /// The full ingest location passed to the RTMP sink, including the stream
    /// key. Returns the base URL unchanged when no stream key is set.
    pub fn location(&self) -> String {
        let base = self.ingest_url.trim_end_matches('/');
        if self.stream_key.is_empty() {
            base.to_string()
        } else {
            format!("{}/{}", base, self.stream_key)
        }
    }

    /// Whether this stream uses a TLS-encrypted connection (`rtmps://`).
    pub fn is_rtmps(&self) -> bool {
        self.ingest_url.starts_with("rtmps://")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twitch_uses_rtmps_endpoint() {
        let s = StreamSettings::twitch("live_123456_abc");
        assert_eq!(s.platform, StreamPlatform::Twitch);
        assert!(s.is_rtmps());
        assert_eq!(s.location(), "rtmps://live.twitch.tv/app/live_123456_abc");
    }

    #[test]
    fn youtube_uses_rtmps_endpoint() {
        let s = StreamSettings::youtube("abcd-efgh-1234");
        assert_eq!(s.platform, StreamPlatform::YouTube);
        assert!(s.is_rtmps());
        assert_eq!(
            s.location(),
            "rtmps://a.rtmp.youtube.com/live2/abcd-efgh-1234"
        );
    }

    #[test]
    fn kick_uses_provided_ivs_endpoint() {
        let s = StreamSettings::kick(
            "rtmps://fa7231ca50b7.global-contribute.live-video.net/app",
            "kickkey123",
        );
        assert_eq!(s.platform, StreamPlatform::Kick);
        assert!(s.is_rtmps());
        assert_eq!(
            s.location(),
            "rtmps://fa7231ca50b7.global-contribute.live-video.net/app/kickkey123"
        );
    }

    #[test]
    fn custom_uses_url_verbatim() {
        let s = StreamSettings::custom("rtmp://localhost/live", "mykey");
        assert!(!s.is_rtmps());
        assert_eq!(s.location(), "rtmp://localhost/live/mykey");
    }

    #[test]
    fn location_omits_trailing_slash() {
        let s = StreamSettings::new(StreamPlatform::Custom, "rtmps://host/app/", "key");
        assert_eq!(s.location(), "rtmps://host/app/key");
    }

    #[test]
    fn location_without_key_is_ingest_url() {
        let s = StreamSettings::twitch("");
        assert_eq!(s.location(), "rtmps://live.twitch.tv/app");
    }

    #[test]
    fn platform_labels_are_distinct() {
        assert_eq!(StreamPlatform::Twitch.label(), "Twitch");
        assert_eq!(StreamPlatform::Kick.label(), "Kick");
        assert_eq!(StreamPlatform::YouTube.label(), "YouTube");
        assert_eq!(StreamPlatform::Custom.label(), "Custom");
    }

    #[test]
    fn is_rtmps_distinguishes_encrypted_and_plain() {
        assert!(
            StreamSettings::custom("rtmps://host/app", "k").is_rtmps(),
            "rtmps:// must be recognized as encrypted"
        );
        assert!(
            !StreamSettings::custom("rtmp://host/app", "k").is_rtmps(),
            "rtmp:// must not be considered encrypted"
        );
    }

    #[test]
    fn settings_keep_platform_and_parts() {
        let s = StreamSettings::new(StreamPlatform::Kick, "rtmps://host/app", "key");
        assert_eq!(s.platform, StreamPlatform::Kick);
        assert_eq!(s.ingest_url, "rtmps://host/app");
        assert_eq!(s.stream_key, "key");
        assert_eq!(s.location(), "rtmps://host/app/key");
    }

    #[test]
    fn location_joins_with_single_separator_even_with_missing_key() {
        let s = StreamSettings::new(StreamPlatform::Custom, "rtmps://host/app", "");
        assert_eq!(s.location(), "rtmps://host/app");
    }

    #[test]
    fn keys_are_not_mangled_by_location_builder() {
        let s = StreamSettings::twitch("live_123456-ab-cd.ef");
        assert_eq!(
            s.location(),
            "rtmps://live.twitch.tv/app/live_123456-ab-cd.ef"
        );
    }
}
