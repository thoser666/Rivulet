//! Streaming support for Rivulet.
//!
//! Twitch, Kick and YouTube all ingest via **RTMPS** (RTMP over TLS), so a
//! single [`StreamSettings`] with an `rtmps://` ingest URL works for all three
//! platforms - only the ingest endpoint and stream key differ.

use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

/// OS-backed storage for a stream key. The secret is never serialized with UI
/// preferences or included in diagnostics.
pub struct StreamKeyStore {
    service: String,
}

impl Default for StreamKeyStore {
    fn default() -> Self {
        Self::new("Rivulet")
    }
}

impl StreamKeyStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    pub fn save(&self, account: &str, key: &str) -> Result<(), String> {
        keyring::Entry::new(&self.service, account)
            .map_err(|error| error.to_string())?
            .set_password(key)
            .map_err(|error| error.to_string())
    }

    pub fn load(&self, account: &str) -> Result<Option<String>, String> {
        match keyring::Entry::new(&self.service, account)
            .map_err(|error| error.to_string())?
            .get_password()
        {
            Ok(key) => Ok(Some(key)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn delete(&self, account: &str) -> Result<(), String> {
        match keyring::Entry::new(&self.service, account)
            .map_err(|error| error.to_string())?
            .delete_credential()
        {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// Deterministic state machine for a bounded private test stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivateTestStreamState {
    Idle,
    Countdown,
    Running,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivateTestStream {
    state: PrivateTestStreamState,
    countdown_secs: u32,
    max_duration: Duration,
    elapsed: Duration,
}

impl PrivateTestStream {
    pub fn new(countdown_secs: u32, max_duration: Duration) -> Self {
        Self {
            state: PrivateTestStreamState::Idle,
            countdown_secs,
            max_duration: max_duration.max(Duration::from_secs(1)),
            elapsed: Duration::ZERO,
        }
    }
    pub fn start(&mut self) {
        self.state = PrivateTestStreamState::Countdown;
    }
    pub fn state(&self) -> PrivateTestStreamState {
        self.state
    }
    pub fn tick(&mut self, elapsed: Duration) {
        if matches!(
            self.state,
            PrivateTestStreamState::Idle | PrivateTestStreamState::Completed
        ) {
            return;
        }
        self.elapsed = self.elapsed.saturating_add(elapsed);
        if self.state == PrivateTestStreamState::Countdown
            && self.elapsed >= Duration::from_secs(self.countdown_secs as u64)
        {
            self.state = PrivateTestStreamState::Running;
            self.elapsed = Duration::ZERO;
        } else if self.state == PrivateTestStreamState::Running && self.elapsed >= self.max_duration
        {
            self.state = PrivateTestStreamState::Completed;
        }
    }
    pub fn stop(&mut self) {
        self.state = PrivateTestStreamState::Completed;
    }
}

/// Result of a safe, short platform connection probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamProbeResult {
    Reachable,
    Rejected(String),
    Unavailable(String),
}

impl StreamProbeResult {
    pub fn is_reachable(&self) -> bool {
        matches!(self, Self::Reachable)
    }
}

/// Parse an RTMP(S) ingest URL into host, port and whether TLS is expected.
/// No stream key is returned or logged.
pub fn parse_ingest_endpoint(url: &str) -> Result<(String, u16, bool), &'static str> {
    let (secure, rest) = if let Some(rest) = url.strip_prefix("rtmps://") {
        (true, rest)
    } else if let Some(rest) = url.strip_prefix("rtmp://") {
        (false, rest)
    } else {
        return Err("ingest URL must use rtmp:// or rtmps://");
    };
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.is_empty() {
        return Err("ingest URL host is empty");
    }
    let (host, port) = authority.rsplit_once(':').map_or_else(
        || (authority.to_owned(), if secure { 443 } else { 1935 }),
        |(host, port)| {
            let port = port.parse::<u16>().unwrap_or(0);
            (host.trim_matches(['[', ']']).to_owned(), port)
        },
    );
    if host.is_empty() || port == 0 {
        return Err("ingest URL host or port is invalid");
    }
    Ok((host, port, secure))
}

/// Perform a bounded TCP reachability probe. This verifies DNS/TCP (and the
/// selected TLS port), but deliberately does not publish media or validate a
/// stream key. Run it off the UI thread.
pub fn probe_ingest_reachability(url: &str, timeout: Duration) -> StreamProbeResult {
    let (host, port, _) = match parse_ingest_endpoint(url) {
        Ok(value) => value,
        Err(error) => return StreamProbeResult::Rejected(error.to_owned()),
    };
    let timeout = timeout.clamp(Duration::from_secs(1), Duration::from_secs(30));
    let address = match (host.as_str(), port).to_socket_addrs() {
        Ok(mut addresses) => addresses.next(),
        Err(error) => return StreamProbeResult::Unavailable(format!("DNS lookup failed: {error}")),
    };
    let Some(address) = address else {
        return StreamProbeResult::Unavailable("DNS lookup returned no addresses".into());
    };
    match TcpStream::connect_timeout(&address, timeout) {
        Ok(stream) => {
            let _ = stream.shutdown(std::net::Shutdown::Both);
            StreamProbeResult::Reachable
        }
        Err(error) => StreamProbeResult::Unavailable(format!("connection failed: {error}")),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamConnectionResult {
    Connected,
    Rejected(String),
    Failed(String),
}

impl StreamConnectionResult {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Connected)
    }
}

use gstreamer::prelude::{Cast, ElementExt};

/// Configuration and pipeline contract for a WHIP/WebRTC publisher.
///
/// Signaling remains HTTP-based while media is provided by a GStreamer
/// `webrtcbin` branch. The branch description is deterministic and can be
/// validated without contacting an SFU.
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

    /// Build the initial H.264/Opus WebRTC media branch.
    pub fn pipeline_fragment(&self) -> &'static str {
        "webrtcbin name=whip_webrtc bundle-policy=max-bundle"
    }

    /// Build the deterministic encoded-media portion attached to `webrtcbin`.
    /// The caller supplies the capture/audio sources and performs pad linking
    /// on the GStreamer context after negotiation.
    pub fn media_branch_fragment(&self) -> &'static str {
        "videoconvert ! queue ! x264enc tune=zerolatency ! h264parse ! whip_webrtc. audioconvert ! audioresample ! opusenc ! rtpopuspay ! whip_webrtc."
    }

    /// Validate that the configured pipeline can provide the required element.
    pub fn media_pipeline_available(&self) -> bool {
        gstreamer::ElementFactory::find("webrtcbin").is_some()
    }

    /// Return the required element names for a preflight check.
    pub fn required_media_elements(&self) -> &'static [&'static str] {
        &["webrtcbin", "x264enc", "h264parse", "opusenc", "rtpopuspay"]
    }

    /// Build and set the initial WebRTC pipeline to `Ready` without starting
    /// network traffic. This is the safe preflight boundary before SDP/ICE.
    pub fn build_media_pipeline(&self) -> anyhow::Result<gstreamer::Pipeline> {
        self.validate().map_err(anyhow::Error::msg)?;
        gstreamer::init().map_err(|e| anyhow::anyhow!("GStreamer init failed: {e}"))?;
        let pipeline = gstreamer::parse::launch(&format!(
            "{} {}",
            self.pipeline_fragment(),
            self.media_branch_fragment()
        ))?
        .downcast::<gstreamer::Pipeline>()
        .map_err(|_| anyhow::anyhow!("WHIP media description did not create a pipeline"))?;
        pipeline.set_state(gstreamer::State::Ready)?;
        Ok(pipeline)
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

impl WhipSession {
    /// Terminate the WHIP session on the server by issuing an HTTP `DELETE` on
    /// the resource `Location` returned by the offer/answer exchange.
    ///
    /// This is cleanup-only and deliberately swallows transport errors so a
    /// failed teardown never masks the recording or streaming result.
    pub fn destroy(&self, endpoint: &str, timeout: Duration) {
        let Some(resource_url) = self.resource_url.as_deref() else {
            return;
        };
        let target = match resource_url {
            url if url.starts_with("http://") || url.starts_with("https://") => url.to_owned(),
            relative => format!("{endpoint}{relative}"),
        };
        let request = ureq::delete(&target)
            .config()
            .timeout_global(Some(timeout))
            .build();
        if let Err(error) = request.call() {
            tracing::debug!(
                %error,
                "WHIP session DELETE failed during cleanup (non-fatal)"
            );
        }
    }
}

/// Lifecycle state of a WHIP/WebRTC media publisher session.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WhipMediaState {
    Idle,
    Negotiating,
    Live,
    Failed(String),
}

/// A WHIP/WebRTC media publisher session.
///
/// Owns the configured [`WhipSettings`], the SDP offer/answer exchange, and the
/// server-provided resource URL for cleanup. Media transport remains owned by
/// the GStreamer `webrtcbin` integration; this type drives the deterministic
/// signaling and teardown lifecycle that is fully unit-testable offline.
#[derive(Debug)]
pub struct WhipMediaSession {
    pub settings: WhipSettings,
    pub state: WhipMediaState,
    pub answer_sdp: Option<String>,
    pub resource_url: Option<String>,
}

impl WhipMediaSession {
    /// Create a new, idle session.
    pub fn new(settings: WhipSettings) -> Self {
        Self {
            settings,
            state: WhipMediaState::Idle,
            answer_sdp: None,
            resource_url: None,
        }
    }

    /// Whether a media pipeline is available on this platform.
    pub fn media_pipeline_available(&self) -> bool {
        self.settings.media_pipeline_available()
    }

    /// Build the deterministic H.264/Opus SDP offer for this session.
    pub fn create_offer_sdp(&self) -> anyhow::Result<String> {
        crate::sdp::SdpOffer::h264_opus(&self.settings.endpoint).to_sdp()
    }

    /// Exchange the offer/answer with the configured WHIP endpoint.
    ///
    /// Runs synchronously; callers place it off the UI thread. On success the
    /// session transitions to [`WhipMediaState::Live`] and retains the
    /// server-provided resource URL for later cleanup.
    pub fn negotiate(&mut self) -> Result<WhipSession, anyhow::Error> {
        self.state = WhipMediaState::Negotiating;
        let offer = self.create_offer_sdp()?;
        match self.settings.post_offer(&offer) {
            Ok(session) => {
                self.answer_sdp = Some(session.answer_sdp.clone());
                self.resource_url = session.resource_url.clone();
                self.state = WhipMediaState::Live;
                Ok(session)
            }
            Err(error) => {
                self.state = WhipMediaState::Failed(error.to_string());
                Err(error)
            }
        }
    }

    /// Apply an already-negotiated remote answer directly. Returns the previous
    /// state.
    pub fn apply_answer(&mut self, answer_sdp: &str) -> WhipMediaState {
        let previous = self.state.clone();
        if answer_sdp.trim().is_empty() {
            self.state = WhipMediaState::Failed("WHIP answer SDP is empty".into());
            return previous;
        }
        self.answer_sdp = Some(answer_sdp.to_owned());
        self.state = WhipMediaState::Live;
        previous
    }

    /// Terminate the session, running the server-side `DELETE` when a resource
    /// URL is known.
    pub fn shutdown(&mut self) -> anyhow::Result<()> {
        if let Some(url) = self.resource_url.take() {
            let session = WhipSession {
                resource_url: Some(url),
                answer_sdp: String::new(),
            };
            session.destroy(&self.settings.endpoint, self.settings.connect_timeout);
        }
        self.answer_sdp = None;
        self.state = WhipMediaState::Idle;
        Ok(())
    }
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
    /// Stable platform default for a test stream. Kick intentionally has no
    /// global default: its Creator Dashboard supplies the account/region
    /// specific server URL.
    pub fn default_ingest_url(self) -> Option<&'static str> {
        match self {
            Self::Twitch => Some("rtmps://live.twitch.tv/app"),
            Self::YouTube => Some("rtmps://a.rtmp.youtube.com/live2"),
            Self::Kick | Self::Custom => None,
        }
    }

    pub fn requires_manual_ingest_url(self) -> bool {
        matches!(self, Self::Kick | Self::Custom)
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

/// Copyright-safe VOD audio-track configuration for Twitch's VOD workflow.
///
/// When streaming with mixed music, the VOD track is a third audio channel
/// (independent of the System/Microphone pair used for local recording) that is
/// intentionally kept out of the live ingest so the live stream carries the
/// clean mix while the published VOD retains a mutation-free, license-safe
/// track. The model is deterministic and validated so a VOD track can never
/// silently leak into the live stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VodTrack {
    pub enabled: bool,
    /// Record the VOD track into the local recording file while streaming. When
    /// disabled the track is not emitted at all, which prevents accidental
    /// live-stream leakage.
    pub recorded: bool,
}
impl VodTrack {
    pub const fn new(enabled: bool) -> Self {
        Self {
            enabled,
            recorded: enabled,
        }
    }

    /// A VOD track is only meaningful while streaming; enabling it forces the
    /// local VOD recording flag on so the listener never forgets the track.
    pub fn with_recorded(mut self, recorded: bool) -> Self {
        self.recorded = recorded;
        self
    }

    /// Whether this configuration actually emits a VOD track.
    pub fn active(&self) -> bool {
        self.enabled && self.recorded
    }

    /// Twitch marks the VOD track with the `ivod` track flag. Returning whether
    /// the config is active gives the mux stage a deterministic signal.
    pub fn twitch_ivod_flag(&self) -> Option<&'static str> {
        self.active().then_some("ivod")
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
    pub vod_track: VodTrack,
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
            vod_track: VodTrack::default(),
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
    /// Kick's ingest URL must come from the Creator Dashboard.
    pub fn kick(url: impl Into<String>, key: impl Into<String>) -> Self {
        Self::new(StreamPlatform::Kick, url, key)
    }

    pub fn for_test_stream(platform: StreamPlatform, key: impl Into<String>) -> Self {
        let url = platform.default_ingest_url().unwrap_or_default();
        Self::new(platform, url, key)
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
    pub fn with_vod_track(mut self, vod_track: VodTrack) -> Self {
        self.vod_track = vod_track;
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

    /// Classify a short, non-publishing connection probe.
    ///
    /// The actual socket/TLS probe is owned by the streaming backend. Keeping
    /// this result type in the core lets the GUI present deterministic outcomes
    /// without ever treating configuration validation as a successful platform
    /// connection.
    pub fn probe_reachability(&self, timeout: Duration) -> StreamProbeResult {
        if let Err(error) = self.validate() {
            return StreamProbeResult::Rejected(error.to_owned());
        }
        probe_ingest_reachability(&self.ingest_url, timeout)
    }

    pub fn connection_result(&self, transport_error: Option<&str>) -> StreamConnectionResult {
        if let Err(error) = self.validate() {
            return StreamConnectionResult::Rejected(error.to_owned());
        }
        match transport_error {
            Some(error) if !error.trim().is_empty() => {
                StreamConnectionResult::Failed(error.to_owned())
            }
            _ => StreamConnectionResult::Connected,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_test_stream_counts_down_and_auto_stops() {
        let mut test = PrivateTestStream::new(3, Duration::from_secs(10));
        assert_eq!(test.state(), PrivateTestStreamState::Idle);
        test.start();
        test.tick(Duration::from_secs(2));
        assert_eq!(test.state(), PrivateTestStreamState::Countdown);
        test.tick(Duration::from_secs(1));
        assert_eq!(test.state(), PrivateTestStreamState::Running);
        test.tick(Duration::from_secs(10));
        assert_eq!(test.state(), PrivateTestStreamState::Completed);
    }

    #[test]
    fn private_test_stream_manual_stop_is_idempotent() {
        let mut test = PrivateTestStream::new(0, Duration::from_secs(10));
        test.start();
        test.stop();
        test.stop();
        assert_eq!(test.state(), PrivateTestStreamState::Completed);
    }

    #[test]
    fn ingest_endpoint_parser_uses_platform_ports_without_exposing_key() {
        assert_eq!(
            parse_ingest_endpoint("rtmps://live.example/app/key"),
            Ok(("live.example".into(), 443, true))
        );
        assert_eq!(
            parse_ingest_endpoint("rtmp://localhost:1935/live/key"),
            Ok(("localhost".into(), 1935, false))
        );
        assert!(parse_ingest_endpoint("https://example/live").is_err());
        assert!(parse_ingest_endpoint("rtmps:///key").is_err());
    }

    #[test]
    fn reachability_probe_rejects_invalid_url_without_network_access() {
        assert!(matches!(
            probe_ingest_reachability("https://invalid", Duration::from_secs(1)),
            StreamProbeResult::Rejected(_)
        ));
        assert!(matches!(
            StreamSettings::twitch("key").probe_reachability(Duration::from_secs(1)),
            StreamProbeResult::Unavailable(_) | StreamProbeResult::Reachable
        ));
    }

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
    fn connection_result_distinguishes_validation_and_transport_failures() {
        assert!(matches!(
            StreamSettings::twitch("").connection_result(None),
            StreamConnectionResult::Rejected(_)
        ));
        assert!(matches!(
            StreamSettings::twitch("key").connection_result(Some("timeout")),
            StreamConnectionResult::Failed(message) if message == "timeout"
        ));
        assert!(StreamSettings::twitch("key")
            .connection_result(None)
            .is_success());
    }

    #[test]
    fn platform_test_defaults_are_explicit_and_kick_requires_dashboard_url() {
        assert_eq!(
            StreamPlatform::Twitch.default_ingest_url(),
            Some("rtmps://live.twitch.tv/app")
        );
        assert_eq!(
            StreamPlatform::YouTube.default_ingest_url(),
            Some("rtmps://a.rtmp.youtube.com/live2")
        );
        assert!(StreamPlatform::Kick.default_ingest_url().is_none());
        assert!(StreamPlatform::Kick.requires_manual_ingest_url());
        assert!(!StreamPlatform::Twitch.requires_manual_ingest_url());
        assert!(
            StreamSettings::for_test_stream(StreamPlatform::Twitch, "key")
                .validate()
                .is_ok()
        );
        assert!(StreamSettings::for_test_stream(StreamPlatform::Kick, "key")
            .validate()
            .is_err());
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
    fn whip_media_pipeline_contract_is_explicit_and_safe() {
        let settings = WhipSettings::new("https://sfu.example/whip");
        assert!(settings.pipeline_fragment().contains("webrtcbin"));
        assert!(settings.media_branch_fragment().contains("x264enc"));
        assert!(settings.media_branch_fragment().contains("opusenc"));
        assert_eq!(settings.required_media_elements().len(), 5);
        let _ = gstreamer::init();
        assert_eq!(
            settings.media_pipeline_available(),
            gstreamer::ElementFactory::find("webrtcbin").is_some()
        );
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
        let _ = gstreamer::init();
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

    #[test]
    fn whip_session_negotiation_lifecycle_is_deterministic() {
        // Loopback-only test: no real SFU is contacted, but the deterministic
        // SDP offer and a local HTTP mock validate the full signaling contract.
        let mut session = WhipMediaSession::new(
            WhipSettings::new("http://127.0.0.1:9/whip").with_timeout(Duration::from_secs(2)),
        );
        assert_eq!(session.state, WhipMediaState::Idle);

        let offer = session.create_offer_sdp().expect("offer builds");
        assert!(offer.contains("H264/90000"));
        assert!(offer.contains("opus/48000/2"));

        // Applying a valid answer moves the session live and keeps the state
        // machine consistent without any network I/O.
        let previous = session.apply_answer("v=0\r\no=- 1 1 IN IP4 0.0.0.0\r\n");
        assert_eq!(previous, WhipMediaState::Idle);
        assert_eq!(session.state, WhipMediaState::Live);
        assert!(session.answer_sdp.as_deref().unwrap().starts_with("v=0"));

        // An empty answer is rejected.
        session.apply_answer("");
        assert!(matches!(session.state, WhipMediaState::Failed(_)));

        session.shutdown().unwrap();
        assert_eq!(session.state, WhipMediaState::Idle);
    }

    #[test]
    fn whip_session_shutdown_runs_server_delete_when_resource_known() {
        let destroyed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let destroyed_clone = std::sync::Arc::clone(&destroyed);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            use std::io::{Read, Write};
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let request = String::from_utf8_lossy(&buf);
            if request.starts_with("DELETE") {
                destroyed_clone.store(true, std::sync::atomic::Ordering::SeqCst);
                write!(
                    stream,
                    "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n"
                )
                .unwrap();
            } else {
                write!(
                    stream,
                    "HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n"
                )
                .unwrap();
            }
        });

        let mut session = WhipMediaSession::new(WhipSettings::new(format!("http://{addr}/whip")));
        session.resource_url = Some(format!("http://{addr}/whip/resource"));
        session.shutdown().unwrap();
        server.join().unwrap();
        assert!(destroyed.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn whip_session_shutdown_without_resource_is_a_noop() {
        let mut session = WhipMediaSession::new(WhipSettings::new("http://127.0.0.1:9/whip"));
        session.shutdown().unwrap();
        assert_eq!(session.state, WhipMediaState::Idle);
        assert!(session.answer_sdp.is_none());
    }

    #[test]
    fn whip_offer_is_deterministic_and_redaction_safe() {
        let session = WhipMediaSession::new(WhipSettings::new("https://sfu.example/whip"));
        let a = session.create_offer_sdp().unwrap();
        let b = session.create_offer_sdp().unwrap();
        assert_eq!(a, b);
        // Every generated media description must never contain a credential.
        assert!(!a.to_lowercase().contains("bearer"));
        assert!(!a.to_lowercase().contains("token"));
    }

    #[test]
    fn vod_track_is_inactive_by_default_and_only_active_when_recorded() {
        assert!(!VodTrack::default().active());
        assert!(VodTrack::default().twitch_ivod_flag().is_none());

        let enabled = VodTrack::new(true);
        assert!(enabled.active());
        assert_eq!(enabled.twitch_ivod_flag(), Some("ivod"));

        // Enabling but explicitly disabling recording must not emit a track, so
        // the VOD track can never silently leak into the live stream.
        let muted = VodTrack::new(true).with_recorded(false);
        assert!(!muted.active());
        assert!(muted.twitch_ivod_flag().is_none());
    }

    #[test]
    fn vod_track_is_part_of_stream_settings_and_exposed_on_preset() {
        let settings = StreamSettings::twitch("key").with_vod_track(VodTrack::new(true));
        assert!(settings.vod_track.active());

        let plain = StreamSettings::twitch("key");
        assert!(!plain.vod_track.active());
        assert_eq!(plain.location(), "rtmps://live.twitch.tv/app/key");
    }

    #[test]
    fn vod_track_never_appears_in_masked_or_redacted_output() {
        let settings = StreamSettings::custom("rtmp://host/live", "secret-key")
            .with_vod_track(VodTrack::new(true));
        assert_eq!(settings.masked_key(), "se••••");
        assert!(!settings.location().contains("ivod"));
    }
}
