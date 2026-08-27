//! Streaming support for Rivulet.
//!
//! Twitch, Kick and YouTube all ingest via **RTMPS** (RTMP over TLS), so a
//! single [`StreamSettings`] with an `rtmps://` ingest URL works for all three
//! platforms - only the ingest endpoint and stream key differ.

/// Streaming platform presets used to build the ingest URL.
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

/// Named stream-quality preset. Bitrates are in kbit/s.
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
            Self::Low => Some(2_500),
            Self::Standard => Some(4_500),
            Self::High => Some(6_000),
            Self::Custom => None,
        }
    }

    pub fn all() -> &'static [Self] {
        &[Self::Low, Self::Standard, Self::High, Self::Custom]
    }
}

/// Adaptive bitrate policy based on stream health.
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
            minimum_kbps: 1_500,
            maximum_kbps: 6_000,
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

/// Settings describing a single RTMP/RTMPS stream output.
#[derive(Debug, Clone)]
pub struct StreamSettings {
    pub platform: StreamPlatform,
    pub ingest_url: String,
    pub stream_key: String,
    pub preset: StreamPreset,
    pub adaptive_bitrate: AdaptiveBitrate,
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
        }
    }

    pub fn twitch(stream_key: impl Into<String>) -> Self {
        Self::new(
            StreamPlatform::Twitch,
            "rtmps://live.twitch.tv/app",
            stream_key,
        )
    }
    pub fn youtube(stream_key: impl Into<String>) -> Self {
        Self::new(
            StreamPlatform::YouTube,
            "rtmps://a.rtmp.youtube.com/live2",
            stream_key,
        )
    }
    pub fn kick(ingest_url: impl Into<String>, stream_key: impl Into<String>) -> Self {
        Self::new(StreamPlatform::Kick, ingest_url, stream_key)
    }
    pub fn custom(ingest_url: impl Into<String>, stream_key: impl Into<String>) -> Self {
        Self::new(StreamPlatform::Custom, ingest_url, stream_key)
    }

    pub fn with_preset(mut self, preset: StreamPreset) -> Self {
        self.preset = preset;
        self
    }
    pub fn with_adaptive_bitrate(mut self, policy: AdaptiveBitrate) -> Self {
        self.adaptive_bitrate = policy;
        self
    }

    /// A masked value safe for UI and diagnostics. The secret is never returned.
    pub fn masked_key(&self) -> String {
        if self.stream_key.is_empty() {
            "Not configured".to_string()
        } else {
            format!(
                "{}••••",
                &self.stream_key[..self
                    .stream_key
                    .char_indices()
                    .nth(2)
                    .map(|(i, _)| i)
                    .unwrap_or(0)]
            )
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
            base.to_string()
        } else {
            format!("{}/{}", base, self.stream_key)
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
    fn presets_have_stable_bitrates() {
        assert_eq!(StreamPreset::Low.bitrate_kbps(), Some(2500));
        assert_eq!(StreamPreset::Standard.bitrate_kbps(), Some(4500));
        assert_eq!(StreamPreset::High.bitrate_kbps(), Some(6000));
        assert_eq!(StreamPreset::Custom.bitrate_kbps(), None);
    }

    #[test]
    fn masked_key_never_contains_the_secret() {
        let settings = StreamSettings::twitch("secret-key-123");
        let masked = settings.masked_key();
        assert!(!masked.contains("secret-key-123"));
        assert!(masked.ends_with("••••"));
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
    fn adaptive_bitrate_moves_only_inside_bounds() {
        let policy = AdaptiveBitrate::new(1500, 3000, 500);
        assert_eq!(
            policy.next_bitrate(2000, crate::StreamHealthStatus::Poor),
            1500
        );
        assert_eq!(
            policy.next_bitrate(2500, crate::StreamHealthStatus::Good),
            3000
        );
        assert_eq!(
            policy.next_bitrate(2200, crate::StreamHealthStatus::Warning),
            2200
        );
    }

    #[test]
    fn adaptive_bitrate_disabled_is_stable() {
        let policy = AdaptiveBitrate {
            enabled: false,
            ..AdaptiveBitrate::default()
        };
        assert_eq!(
            policy.next_bitrate(42_000, crate::StreamHealthStatus::Poor),
            42_000
        );
    }

    #[test]
    fn platform_defaults_are_known() {
        assert_eq!(
            StreamPlatform::Twitch.default_ingest_url(),
            Some("rtmps://live.twitch.tv/app")
        );
        assert_eq!(StreamPlatform::Custom.default_ingest_url(), None);
    }

    #[test]
    fn existing_location_behavior_is_preserved() {
        let s = StreamSettings::youtube("key").with_preset(StreamPreset::High);
        assert_eq!(s.location(), "rtmps://a.rtmp.youtube.com/live2/key");
        assert_eq!(s.preset, StreamPreset::High);
    }
}
