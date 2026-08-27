//! Streaming support for Rivulet.
//!
//! Twitch, Kick and YouTube all ingest via **RTMPS** (RTMP over TLS), so a
//! single [`StreamSettings`] with an `rtmps://` ingest URL works for all three
//! platforms - only the ingest endpoint and stream key differ.

use std::time::Duration;

/// Configuration contract for a future WHIP/WebRTC publisher.
///
/// This is intentionally transport-neutral: signaling and GStreamer
/// integration are implemented after the strategy spike.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhipSettings {
    pub endpoint: String,
    pub bearer_token: Option<String>,
    pub connect_timeout: Duration,
    pub trickle_ice: bool,
}

impl WhipSettings {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            bearer_token: None,
            connect_timeout: Duration::from_secs(10),
            trickle_ice: false,
        }
    }

    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.bearer_token = Some(token.into());
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = timeout.clamp(Duration::from_secs(2), Duration::from_secs(60));
        self
    }

    pub fn with_trickle_ice(mut self, enabled: bool) -> Self {
        self.trickle_ice = enabled;
        self
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        let loopback = self.endpoint.starts_with("http://127.0.0.1")
            || self.endpoint.starts_with("http://localhost")
            || self.endpoint.starts_with("http://[::1]");
        if !self.endpoint.starts_with("https://") && !loopback {
            return Err("WHIP endpoint must use https:// unless it is loopback development HTTP");
        }
        if self.endpoint.trim_end_matches('/').len() <= "https://".len() {
            return Err("WHIP endpoint is empty");
        }
        if self.connect_timeout < Duration::from_secs(2)
            || self.connect_timeout > Duration::from_secs(60)
        {
            return Err("WHIP connect timeout must be between 2 and 60 seconds");
        }
        Ok(())
    }

    pub fn redacted_endpoint(&self) -> String {
        self.endpoint.clone()
    }

    pub fn has_token(&self) -> bool {
        self.bearer_token
            .as_deref()
            .is_some_and(|token| !token.is_empty())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamPlatform {
    Twitch,
    Kick,
    YouTube,
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
    pub fn default_ingest_url(self) -> Option<&'static str> {
        match self {
            Self::Twitch => Some("rtmps://live.twitch.tv/app"),
            Self::YouTube => Some("rtmps://a.rtmp.youtube.com/live2"),
            Self::Kick | Self::Custom => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamPreset {
    Low,
    Standard,
    High,
    Custom,
}
impl StreamPreset {
    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Standard => "Standard",
            Self::High => "High",
            Self::Custom => "Custom",
        }
    }
    pub fn bitrate_kbps(self) -> Option<u32> {
        match self {
            Self::Low => Some(2500),
            Self::Standard => Some(4500),
            Self::High => Some(6000),
            Self::Custom => None,
        }
    }
    pub fn all() -> &'static [Self] {
        &[Self::Low, Self::Standard, Self::High, Self::Custom]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdaptiveBitrate {
    pub enabled: bool,
    pub minimum_kbps: u32,
    pub maximum_kbps: u32,
    pub step_kbps: u32,
}
impl Default for AdaptiveBitrate {
    fn default() -> Self {
        Self {
            enabled: false,
            minimum_kbps: 1500,
            maximum_kbps: 6000,
            step_kbps: 500,
        }
    }
}
impl AdaptiveBitrate {
    pub fn new(minimum_kbps: u32, maximum_kbps: u32, step_kbps: u32) -> Self {
        let minimum_kbps = minimum_kbps.max(250);
        let maximum_kbps = maximum_kbps.max(minimum_kbps);
        Self {
            enabled: true,
            minimum_kbps,
            maximum_kbps,
            step_kbps: step_kbps.max(50),
        }
    }
    pub fn next_bitrate(self, current_kbps: u32, status: crate::StreamHealthStatus) -> u32 {
        if !self.enabled {
            return current_kbps;
        }
        match status {
            crate::StreamHealthStatus::Poor => current_kbps
                .saturating_sub(self.step_kbps)
                .max(self.minimum_kbps),
            crate::StreamHealthStatus::Good => current_kbps
                .saturating_add(self.step_kbps)
                .min(self.maximum_kbps),
            _ => current_kbps.clamp(self.minimum_kbps, self.maximum_kbps),
        }
    }
}

/// A bounded delay applied before packets enter the network sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamDelay {
    pub enabled: bool,
    pub duration: Duration,
}
impl Default for StreamDelay {
    fn default() -> Self {
        Self {
            enabled: false,
            duration: Duration::ZERO,
        }
    }
}
impl StreamDelay {
    pub fn new(duration: Duration) -> Self {
        let duration = duration.min(Duration::from_secs(300));
        Self {
            enabled: !duration.is_zero(),
            duration,
        }
    }
    pub fn latency_ms(self) -> u64 {
        if self.enabled {
            self.duration.as_millis() as u64
        } else {
            0
        }
    }
}

/// Additional encoded video representations for adaptive/multitrack delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultitrackVideo {
    pub enabled: bool,
    pub tracks: u8,
}
impl Default for MultitrackVideo {
    fn default() -> Self {
        Self {
            enabled: false,
            tracks: 1,
        }
    }
}
impl MultitrackVideo {
    pub fn new(tracks: u8) -> Self {
        Self {
            enabled: tracks > 1,
            tracks: tracks.clamp(1, 4),
        }
    }
    pub fn effective_tracks(self) -> u8 {
        if self.enabled {
            self.tracks.max(2)
        } else {
            1
        }
    }
}

#[derive(Debug, Clone)]
pub struct StreamSettings {
    pub platform: StreamPlatform,
    pub ingest_url: String,
    pub stream_key: String,
    pub preset: StreamPreset,
    pub adaptive_bitrate: AdaptiveBitrate,
    pub delay: StreamDelay,
    pub multitrack_video: MultitrackVideo,
}
impl StreamSettings {
    pub fn new(
        platform: StreamPlatform,
        ingest_url: impl Into<String>,
        stream_key: impl Into<String>,
    ) -> Self {
        Self {
            platform,
            ingest_url: ingest_url.into(),
            stream_key: stream_key.into(),
            preset: StreamPreset::Standard,
            adaptive_bitrate: AdaptiveBitrate::default(),
            delay: StreamDelay::default(),
            multitrack_video: MultitrackVideo::default(),
        }
    }
    pub fn twitch(key: impl Into<String>) -> Self {
        Self::new(StreamPlatform::Twitch, "rtmps://live.twitch.tv/app", key)
    }
    pub fn youtube(key: impl Into<String>) -> Self {
        Self::new(
            StreamPlatform::YouTube,
            "rtmps://a.rtmp.youtube.com/live2",
            key,
        )
    }
    pub fn kick(url: impl Into<String>, key: impl Into<String>) -> Self {
        Self::new(StreamPlatform::Kick, url, key)
    }
    pub fn custom(url: impl Into<String>, key: impl Into<String>) -> Self {
        Self::new(StreamPlatform::Custom, url, key)
    }
    pub fn with_preset(mut self, preset: StreamPreset) -> Self {
        self.preset = preset;
        self
    }
    pub fn with_adaptive_bitrate(mut self, policy: AdaptiveBitrate) -> Self {
        self.adaptive_bitrate = policy;
        self
    }
    pub fn with_delay(mut self, delay: StreamDelay) -> Self {
        self.delay = delay;
        self
    }
    pub fn with_multitrack_video(mut self, video: MultitrackVideo) -> Self {
        self.multitrack_video = video;
        self
    }
    pub fn masked_key(&self) -> String {
        if self.stream_key.is_empty() {
            "Not configured".into()
        } else {
            let end = self
                .stream_key
                .char_indices()
                .nth(2)
                .map(|(i, _)| i)
                .unwrap_or(0);
            format!("{}••••", &self.stream_key[..end])
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.stream_key.trim().is_empty() {
            return Err("stream key is empty");
        }
        if !(self.ingest_url.starts_with("rtmp://") || self.ingest_url.starts_with("rtmps://")) {
            return Err("ingest URL must use rtmp:// or rtmps://");
        }
        if self.platform != StreamPlatform::Custom && !self.is_rtmps() {
            return Err("platform presets require TLS (rtmps://)");
        }
        Ok(())
    }
    pub fn location(&self) -> String {
        let base = self.ingest_url.trim_end_matches('/');
        if self.stream_key.is_empty() {
            base.into()
        } else {
            format!("{base}/{}", self.stream_key)
        }
    }
    pub fn is_rtmps(&self) -> bool {
        self.ingest_url.starts_with("rtmps://")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delay_is_bounded_and_disabled_for_zero() {
        assert_eq!(StreamDelay::new(Duration::ZERO).latency_ms(), 0);
        assert_eq!(
            StreamDelay::new(Duration::from_secs(600)).latency_ms(),
            300_000
        );
    }
    #[test]
    fn multitrack_video_clamps_to_supported_range() {
        assert_eq!(MultitrackVideo::new(0).effective_tracks(), 1);
        assert_eq!(MultitrackVideo::new(9).effective_tracks(), 4);
        assert_eq!(MultitrackVideo::new(3).effective_tracks(), 3);
    }
    #[test]
    fn presets_have_stable_bitrates() {
        assert_eq!(StreamPreset::Low.bitrate_kbps(), Some(2500));
        assert_eq!(StreamPreset::Standard.bitrate_kbps(), Some(4500));
        assert_eq!(StreamPreset::High.bitrate_kbps(), Some(6000));
        assert_eq!(StreamPreset::Custom.bitrate_kbps(), None);
    }
    #[test]
    fn masked_key_never_contains_secret() {
        let s = StreamSettings::twitch("secret-key-123");
        assert!(!s.masked_key().contains("secret-key-123"));
    }
    #[test]
    fn validation_rejects_empty_or_insecure_preset() {
        assert!(StreamSettings::twitch("").validate().is_err());
        assert!(
            StreamSettings::new(StreamPlatform::Twitch, "rtmp://host/app", "key")
                .validate()
                .is_err()
        );
        assert!(StreamSettings::custom("rtmp://host/app", "key")
            .validate()
            .is_ok());
    }
    #[test]
    fn adaptive_bitrate_moves_inside_bounds() {
        let p = AdaptiveBitrate::new(1500, 3000, 500);
        assert_eq!(p.next_bitrate(2000, crate::StreamHealthStatus::Poor), 1500);
        assert_eq!(p.next_bitrate(2500, crate::StreamHealthStatus::Good), 3000);
    }
    #[test]
    fn adaptive_bitrate_disabled_is_stable() {
        let p = AdaptiveBitrate {
            enabled: false,
            ..Default::default()
        };
        assert_eq!(
            p.next_bitrate(42000, crate::StreamHealthStatus::Poor),
            42000
        );
    }
    #[test]
    fn whip_settings_enforce_secure_endpoints_and_bounded_timeout() {
        assert!(WhipSettings::new("https://sfu.example/whip")
            .with_timeout(Duration::from_secs(1))
            .validate()
            .is_ok());
        assert_eq!(
            WhipSettings::new("https://sfu.example/whip")
                .with_timeout(Duration::from_secs(1))
                .connect_timeout,
            Duration::from_secs(2)
        );
        assert!(WhipSettings::new("http://remote.example/whip")
            .validate()
            .is_err());
        assert!(WhipSettings::new("http://127.0.0.1:8889/whip")
            .validate()
            .is_ok());
    }

    #[test]
    fn whip_credentials_are_never_part_of_the_redacted_endpoint() {
        let settings =
            WhipSettings::new("https://sfu.example/whip").with_bearer_token("super-secret-token");
        assert!(settings.has_token());
        assert!(!settings.redacted_endpoint().contains("super-secret-token"));
    }

    #[test]
    fn whip_trickle_ice_is_explicitly_configurable() {
        assert!(!WhipSettings::new("https://sfu.example/whip").trickle_ice);
        assert!(
            WhipSettings::new("https://sfu.example/whip")
                .with_trickle_ice(true)
                .trickle_ice
        );
    }

    #[test]
    fn existing_location_behavior_is_preserved() {
        let s = StreamSettings::youtube("key")
            .with_preset(StreamPreset::High)
            .with_delay(StreamDelay::new(Duration::from_secs(3)))
            .with_multitrack_video(MultitrackVideo::new(2));
        assert_eq!(s.location(), "rtmps://a.rtmp.youtube.com/live2/key");
        assert_eq!(s.delay.latency_ms(), 3000);
        assert_eq!(s.multitrack_video.effective_tracks(), 2);
    }
}
