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
    pub const SDP_CONTENT_TYPE: &'static str = "application/sdp";

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

    /// Perform the WHIP SDP offer/answer exchange.
    ///
    /// The transport is kept here as a small, synchronous signaling operation;
    /// callers should run it off the UI thread. Media transport remains owned
    /// by the eventual GStreamer `webrtcbin` integration.
    pub fn post_offer(&self, offer_sdp: &str) -> anyhow::Result<WhipSession> {
        self.validate().map_err(anyhow::Error::msg)?;
        if offer_sdp.trim().is_empty() {
            anyhow::bail!("WHIP offer SDP is empty");
        }
        let mut request = ureq::post(&self.endpoint)
            .header("Content-Type", Self::SDP_CONTENT_TYPE)
            .header("Accept", Self::SDP_CONTENT_TYPE)
            .config()
            .timeout_global(Some(self.connect_timeout))
            .build();
        if let Some(token) = self
            .bearer_token
            .as_deref()
            .filter(|token| !token.is_empty())
        {
            request = request.header("Authorization", &format!("Bearer {token}"));
        }
        let response = request.send(offer_sdp.as_bytes())?;
        let status = response.status().into();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let resource_url = response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response.into_body().read_to_string()?;
        parse_whip_response(
            status,
            content_type.as_deref(),
            resource_url.as_deref(),
            &body,
        )
        .map_err(anyhow::Error::msg)
    }
}

/// Response from a WHIP offer/answer exchange.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhipSession {
    pub resource_url: Option<String>,
    pub answer_sdp: String,
}

/// Map an HTTP response from a WHIP endpoint to a deterministic result.
pub fn parse_whip_response(
    status: u16,
    content_type: Option<&str>,
    resource_url: Option<&str>,
    body: &str,
) -> Result<WhipSession, &'static str> {
    if status == 401 || status == 403 {
        return Err("WHIP authentication failed");
    }
    if status >= 500 {
        return Err("WHIP server failure; retry may succeed");
    }
    if !(200..300).contains(&status) {
        return Err("WHIP negotiation failed");
    }
    if !content_type
        .unwrap_or_default()
        .to_ascii_lowercase()
        .starts_with(WhipSettings::SDP_CONTENT_TYPE)
    {
        return Err("WHIP response must use application/sdp");
    }
    if body.trim().is_empty() {
        return Err("WHIP response contains an empty SDP answer");
    }
    Ok(WhipSession {
        resource_url: resource_url.map(str::to_owned),
        answer_sdp: body.to_owned(),
    })
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
    /// Compute and apply the next bounded bitrate for a live encoder.
    pub fn reconfigure_live_encoder(
        &self,
        current_kbps: &mut u32,
        status: crate::StreamHealthStatus,
    ) -> bool {
        let next = self.next_bitrate(*current_kbps, status);
        let changed = next != *current_kbps;
        *current_kbps = next;
        changed
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
    /// Return the delay duration used by a reconnecting pipeline.
    pub fn reconnect_delay(self) -> Duration {
        if self.enabled {
            self.duration
        } else {
            Duration::ZERO
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

/// A separately monitored RTMP/RTMPS output target for multistreaming.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamTarget {
    pub name: String,
    pub settings: StreamSettings,
    /// Optional contribution transport. `None` preserves RTMP/RTMPS behavior.
    pub contribution: Option<crate::ContributionSettings>,
    /// Whether this target's sink plugin was available when validated.
    pub plugin_status: TargetPluginStatus,
}

impl StreamTarget {
    pub fn new(name: impl Into<String>, settings: StreamSettings) -> Self {
        Self {
            name: name.into(),
            settings,
            contribution: None,
            plugin_status: TargetPluginStatus::NotApplicable,
        }
    }

    pub fn with_contribution(mut self, contribution: crate::ContributionSettings) -> Self {
        self.plugin_status = TargetPluginStatus::Unavailable;
        self.contribution = Some(contribution);
        self
    }

    pub fn refresh_plugin_status(&mut self) {
        self.plugin_status = self
            .contribution
            .as_ref()
            .map(|value| {
                if value.plugin_available() {
                    TargetPluginStatus::Available
                } else {
                    TargetPluginStatus::Unavailable
                }
            })
            .unwrap_or(TargetPluginStatus::NotApplicable);
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.name.trim().is_empty() {
            return Err("stream target name is empty");
        }
        self.settings.validate()?;
        if let Some(contribution) = &self.contribution {
            contribution.validate()?;
        }
        Ok(())
    }

    pub fn masked_key(&self) -> String {
        self.settings.masked_key()
    }
}

/// Availability of the sink plugin required by a target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPluginStatus {
    NotApplicable,
    Available,
    Unavailable,
}

/// Independent lifecycle state for one multistream output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamTargetState {
    Offline,
    Connecting,
    Live,
    Degraded,
    Failed,
}

/// Configuration and deterministic state for multiple independent targets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MultistreamSettings {
    pub targets: Vec<StreamTarget>,
}

impl MultistreamSettings {
    pub const MAX_TARGETS: usize = 4;

    /// Build one independent sink description per configured target.
    ///
    /// The caller can attach each fragment to a shared encoded stream. Keeping
    /// the fragments separate makes target failures isolatable by the eventual
    /// transport supervisor.
    pub fn sink_fragments(&self) -> Result<Vec<String>, &'static str> {
        self.validate_all()?;
        Ok(self
            .targets
            .iter()
            .map(|target| {
                target
                    .contribution
                    .as_ref()
                    .and_then(|contribution| contribution.validated_pipeline_sink_fragment().ok())
                    .unwrap_or_else(|| {
                        format!("rtmp2sink location=\\\"{}\\\"", target.settings.location())
                    })
            })
            .collect())
    }

    /// Determine which targets may be retried without affecting healthy peers.
    pub fn retryable_targets(states: &[(String, StreamTargetState)]) -> Vec<String> {
        states
            .iter()
            .filter(|(_, state)| {
                matches!(
                    state,
                    StreamTargetState::Failed | StreamTargetState::Degraded
                )
            })
            .map(|(name, _)| name.clone())
            .collect()
    }

    pub fn add_target(&mut self, target: StreamTarget) -> Result<(), &'static str> {
        target.validate()?;
        if self.targets.len() >= Self::MAX_TARGETS {
            return Err("maximum number of stream targets reached");
        }
        if self
            .targets
            .iter()
            .any(|existing| existing.name == target.name)
        {
            return Err("stream target name already exists");
        }
        self.targets.push(target);
        Ok(())
    }

    pub fn remove_target(&mut self, name: &str) -> Option<StreamTarget> {
        self.targets
            .iter()
            .position(|target| target.name == name)
            .map(|index| self.targets.remove(index))
    }

    pub fn validate_all(&self) -> Result<(), &'static str> {
        if self.targets.is_empty() {
            return Err("at least one stream target is required");
        }
        self.targets.iter().try_for_each(StreamTarget::validate)
    }

    pub fn target_names(&self) -> Vec<&str> {
        self.targets
            .iter()
            .map(|target| target.name.as_str())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    fn adaptive_reconfiguration_updates_only_when_policy_changes() {
        let policy = AdaptiveBitrate::new(1500, 3000, 500);
        let mut current = 2500;
        assert!(policy.reconfigure_live_encoder(&mut current, crate::StreamHealthStatus::Poor));
        assert_eq!(current, 2000);
        assert!(!policy.reconfigure_live_encoder(&mut current, crate::StreamHealthStatus::Warning));
        assert_eq!(current, 2000);
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
    fn whip_response_requires_successful_sdp_content() {
        let response = parse_whip_response(
            201,
            Some("application/sdp; charset=utf-8"),
            Some("https://sfu.example/resource/1"),
            "v=0\r\no=- 1 1 IN IP4 0.0.0.0\r\n",
        )
        .unwrap();
        assert_eq!(
            response.resource_url.as_deref(),
            Some("https://sfu.example/resource/1")
        );
        assert!(response.answer_sdp.starts_with("v=0"));
        assert!(parse_whip_response(401, Some("application/sdp"), None, "v=0").is_err());
        assert!(parse_whip_response(503, Some("application/sdp"), None, "v=0").is_err());
        assert!(parse_whip_response(201, Some("text/plain"), None, "v=0").is_err());
        assert!(parse_whip_response(201, Some("application/sdp"), None, "").is_err());
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
    fn contribution_targets_refresh_plugin_status_without_affecting_rtmp() {
        let mut rtmp = StreamTarget::new("rtmp", StreamSettings::custom("rtmp://host/live", "key"));
        rtmp.refresh_plugin_status();
        assert_eq!(rtmp.plugin_status, TargetPluginStatus::NotApplicable);
        let mut srt = StreamTarget::new("srt", StreamSettings::custom("rtmp://host/live", "key"))
            .with_contribution(crate::ContributionSettings::new(
                crate::ContributionProtocol::Srt,
                "srt://host:9000",
            ));
        srt.refresh_plugin_status();
        assert!(matches!(
            srt.plugin_status,
            TargetPluginStatus::Available | TargetPluginStatus::Unavailable
        ));
    }

    #[test]
    fn multistream_sink_fragments_and_retry_selection_are_independent() {
        let mut settings = MultistreamSettings::default();
        settings
            .add_target(StreamTarget::new(
                "one",
                StreamSettings::custom("rtmp://one/live", "k1"),
            ))
            .unwrap();
        settings
            .add_target(StreamTarget::new(
                "two",
                StreamSettings::custom("rtmp://two/live", "k2"),
            ))
            .unwrap();
        let fragments = settings.sink_fragments().unwrap();
        assert_eq!(fragments.len(), 2);
        assert!(fragments[0].contains("rtmp://one/live/k1"));
        let retry = MultistreamSettings::retryable_targets(&[
            ("one".into(), StreamTargetState::Live),
            ("two".into(), StreamTargetState::Failed),
        ]);
        assert_eq!(retry, vec!["two"]);
    }

    #[test]
    fn reconnect_delay_preserves_configured_latency() {
        assert_eq!(
            StreamDelay::new(Duration::from_secs(4)).reconnect_delay(),
            Duration::from_secs(4)
        );
        assert_eq!(StreamDelay::default().reconnect_delay(), Duration::ZERO);
    }

    #[test]
    fn multistream_targets_are_validated_and_limited() {
        let mut settings = MultistreamSettings::default();
        for index in 0..MultistreamSettings::MAX_TARGETS {
            settings
                .add_target(StreamTarget::new(
                    format!("target-{index}"),
                    StreamSettings::custom("rtmp://localhost/live", format!("key-{index}")),
                ))
                .unwrap();
        }
        assert!(settings
            .add_target(StreamTarget::new(
                "overflow",
                StreamSettings::custom("rtmp://localhost/live", "overflow")
            ))
            .is_err());
        assert_eq!(
            settings.target_names().len(),
            MultistreamSettings::MAX_TARGETS
        );
        assert!(settings.validate_all().is_ok());
    }

    #[test]
    fn multistream_targets_isolate_duplicate_names_and_removal() {
        let target = StreamTarget::new("Twitch", StreamSettings::twitch("secret-twitch-key"));
        let mut settings = MultistreamSettings::default();
        settings.add_target(target.clone()).unwrap();
        assert!(settings.add_target(target).is_err());
        assert_eq!(
            settings.remove_target("Twitch").unwrap().masked_key(),
            "se••••"
        );
        assert!(settings.validate_all().is_err());
    }

    #[test]
    fn multistream_target_validation_rejects_invalid_target_without_mutating_state() {
        let mut settings = MultistreamSettings::default();
        let invalid = StreamTarget::new("", StreamSettings::twitch("key"));
        assert!(settings.add_target(invalid).is_err());
        assert!(settings.targets.is_empty());
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
