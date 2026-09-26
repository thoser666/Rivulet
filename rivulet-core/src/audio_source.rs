//! Audio source model for multi-track audio routing (issue #154, M6).
//!
//! A source captures audio from an application or a hardware device, carries
//! its own filter chain and volume, and has an independent routing decision
//! for the Record and Stream outputs. The design lives in
//! [`docs/m6-audio-routing.md`](../../docs/m6-audio-routing.md); the
//! versioned persistence schema is [`AudioRoutingConfig`].
//!
//! Phase 1 (engine core) provides the types, the engine API, and the
//! routing-aware pipeline composition. Per-platform capture backends and the
//! GUI mixer are later phases of the same issue.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identifies which outputs receive a source's audio.
///
/// The routing matrix is a simple cross product: every source independently
/// decides for the Record output and the Stream output. A source can go to
/// record only, stream only, both, or neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioRouting {
    /// The source is mixed into (or recorded as its own track of) the
    /// recording output.
    pub record: bool,
    /// The source is mixed into the streaming output (FLV carries a single
    /// audio track, so all stream-routed sources are mixed).
    pub stream: bool,
}

impl AudioRouting {
    /// To both outputs (the default for new sources).
    pub const BOTH: Self = Self {
        record: true,
        stream: true,
    };
    /// Recording only.
    pub const RECORD_ONLY: Self = Self {
        record: true,
        stream: false,
    };
    /// Streaming only.
    pub const STREAM_ONLY: Self = Self {
        record: false,
        stream: true,
    };
    /// No output (the source is captured but silent everywhere).
    pub const NONE: Self = Self {
        record: false,
        stream: false,
    };
}

impl Default for AudioRouting {
    fn default() -> Self {
        Self::BOTH
    }
}

/// Noise gate (downward expansion that closes fully below the threshold).
///
/// Realised with the `audiodynamic` element in `expander`/`hard-knee` mode.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NoiseGateConfig {
    /// Threshold in the 0.0–1.0 sample range below which the gate closes.
    pub threshold: f32,
    /// Expansion ratio (e.g. `10.0` ≈ a hard gate).
    pub ratio: f32,
}

impl Default for NoiseGateConfig {
    fn default() -> Self {
        Self {
            threshold: 0.03,
            ratio: 10.0,
        }
    }
}

/// Expander (gentle downward expansion below a mid threshold).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ExpanderConfig {
    pub threshold: f32,
    pub ratio: f32,
}

impl Default for ExpanderConfig {
    fn default() -> Self {
        Self {
            threshold: 0.3,
            ratio: 1.5,
        }
    }
}

/// Compressor (dynamic range compression).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CompressorConfig {
    pub threshold: f32,
    pub ratio: f32,
}

impl Default for CompressorConfig {
    fn default() -> Self {
        Self {
            threshold: 0.5,
            ratio: 4.0,
        }
    }
}

/// Hard limiter (prevents clipping).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LimiterConfig {
    pub threshold: f32,
    pub ratio: f32,
}

impl Default for LimiterConfig {
    fn default() -> Self {
        Self {
            threshold: 0.95,
            ratio: 20.0,
        }
    }
}

/// 10-band equalizer (`equalizer-10bands`), band gains in dB (`-24..=+12`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EqConfig {
    /// Gains for `band0` (lowest) … `band9` (highest).
    pub bands: [f32; 10],
}

impl Default for EqConfig {
    fn default() -> Self {
        Self { bands: [0.0; 10] }
    }
}

/// Per-source audio filter chain.
///
/// Every stage is optional; an all-`None` chain renders an empty filter
/// fragment. The GStreamer element parameters mirror the proven chain in
/// `rivulet-audio` (`filter_chain_str_with`) so a source's filters sound the
/// same as the legacy System/Microphone filters. `rivulet-audio` cannot be
/// reused directly because it depends on `rivulet-core` (no dependency
/// cycles), so the two implementations must stay in sync by review.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AudioFilterConfig {
    pub noise_gate: Option<NoiseGateConfig>,
    pub expander: Option<ExpanderConfig>,
    pub compressor: Option<CompressorConfig>,
    pub limiter: Option<LimiterConfig>,
    /// Makeup gain in decibels (`-30..=+30`); `0.0` (±0.1 tolerance)
    /// disables the `audioamplify` stage.
    pub gain_db: f64,
    pub eq: Option<EqConfig>,
}

impl Default for AudioFilterConfig {
    fn default() -> Self {
        Self {
            noise_gate: None,
            expander: None,
            compressor: None,
            limiter: None,
            gain_db: 0.0,
            eq: None,
        }
    }
}

impl AudioFilterConfig {
    /// True when no stage is active.
    pub fn is_empty(&self) -> bool {
        self.noise_gate.is_none()
            && self.expander.is_none()
            && self.compressor.is_none()
            && self.limiter.is_none()
            && self.gain_db.abs() < 0.1
            && !self
                .eq
                .map(|eq| eq.bands.iter().any(|db| db.abs() >= 0.5))
                .unwrap_or(false)
    }

    /// The GStreamer fragment for this chain (`element ! element ! …`),
    /// excluding the surrounding `audioconvert ! audioresample`.
    ///
    /// Deterministic: identical configs always render identical fragments
    /// (a requirement of the M7 deterministic-pipeline goal).
    pub fn chain_fragment(&self) -> String {
        self.chain_fragment_with_availability(|_| true).0
    }

    /// Availability-aware variant of [`Self::chain_fragment`]: the `available`
    /// predicate decides which GStreamer element factories exist (mirrors the
    /// graceful degradation in `rivulet-audio`). Returns the joined chain and
    /// the factory names that were requested but not available.
    pub fn chain_fragment_with_availability(
        &self,
        available: impl Fn(&str) -> bool,
    ) -> (String, Vec<&'static str>) {
        let mut elements: Vec<String> = Vec::new();
        let mut skipped: Vec<&'static str> = Vec::new();

        let mut push = |factory: &'static str, fragment: String| {
            if available(factory) {
                elements.push(fragment);
            } else if !skipped.contains(&factory) {
                skipped.push(factory);
            }
        };

        if let Some(gate) = self.noise_gate {
            push(
                "audiodynamic",
                format!(
                    "audiodynamic mode=expander characteristics=hard-knee threshold={} ratio={}",
                    gate.threshold, gate.ratio
                ),
            );
        }
        if let Some(exp) = self.expander {
            push(
                "audiodynamic",
                format!(
                    "audiodynamic mode=expander characteristics=soft-knee threshold={} ratio={}",
                    exp.threshold, exp.ratio
                ),
            );
        }
        if let Some(comp) = self.compressor {
            push(
                "audiodynamic",
                format!(
                    "audiodynamic mode=compressor characteristics=soft-knee threshold={} ratio={}",
                    comp.threshold, comp.ratio
                ),
            );
        }
        if let Some(lim) = self.limiter {
            push(
                "audiodynamic",
                format!(
                    "audiodynamic mode=compressor characteristics=hard-knee threshold={} ratio={}",
                    lim.threshold, lim.ratio
                ),
            );
        }
        if self.gain_db.abs() >= 0.1 {
            let factor = 10f64.powf(self.gain_db / 20.0);
            push(
                "audioamplify",
                format!("audioamplify amplification={factor:.4}"),
            );
        }
        if let Some(eq) = self.eq {
            if eq.bands.iter().any(|db| db.abs() >= 0.5) {
                let mut frag = String::from("equalizer-10bands");
                for (i, db) in eq.bands.iter().enumerate() {
                    frag.push_str(&format!(" band{i}={db:.1}"));
                }
                push("equalizer-10bands", frag);
            }
        }

        (elements.join(" ! "), skipped)
    }
}

/// Parse the `pid:<number>` convention used by application audio sources.
///
/// Returns [`None`] for every other device id form (loopback placeholders,
/// device names, or the `pending_app` marker).
pub fn device_pid(device_id: &str) -> Option<u32> {
    let rest = device_id.strip_prefix("pid:")?;
    rest.parse::<u32>().ok().filter(|pid| *pid != 0)
}

/// Device-id conventions carried by device sources (issues #229 / #231).
///
/// `wasapi-out:<endpoint id>` selects a Windows render endpoint captured in
/// loopback mode (what that output device plays), `wasapi-in:<endpoint id>`
/// a Windows capture endpoint (microphone). On Linux, `pw-src:<node id>`
/// selects a PipeWire *source* node (microphone, virtual device) and
/// `pw-mon:<node id>` a sink *monitor* (what that sink plays). On macOS,
/// `core-audio-in:<name>` selects a Core Audio input device by its
/// cpal-reported name — the stable identifier cpal exposes (Core Audio
/// device UIDs are not available through cpal's public API). The id after
/// the prefix is the platform-stable target; friendly names live only in
/// the picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceTarget {
    /// A render endpoint captured in loopback mode.
    Output(String),
    /// A capture endpoint (e.g. microphone).
    Input(String),
    /// A PipeWire source node captured directly (issue #231).
    PwSource(u32),
    /// A PipeWire sink monitor captured in monitor mode (issue #231).
    PwMonitor(u32),
    /// A Core Audio input device addressed by name (issue #231).
    CoreAudioInput(String),
}

impl DeviceTarget {
    /// Parse a device id into a [`DeviceTarget`]. Returns [`None`] for every
    /// other form (legacy placeholders, `pid:` ids, or the pending markers).
    pub fn parse(device_id: &str) -> Option<Self> {
        if let Some(rest) = device_id.strip_prefix("wasapi-out:") {
            let id = rest.trim();
            (!id.is_empty()).then(|| Self::Output(id.to_owned()))
        } else if let Some(rest) = device_id.strip_prefix("wasapi-in:") {
            let id = rest.trim();
            (!id.is_empty()).then(|| Self::Input(id.to_owned()))
        } else if let Some(rest) = device_id.strip_prefix("pw-src:") {
            let id = rest.trim().parse::<u32>().ok().filter(|id| *id != 0);
            id.map(Self::PwSource)
        } else if let Some(rest) = device_id.strip_prefix("pw-mon:") {
            let id = rest.trim().parse::<u32>().ok().filter(|id| *id != 0);
            id.map(Self::PwMonitor)
        } else if let Some(rest) = device_id.strip_prefix("core-audio-in:") {
            let name = rest.trim();
            (!name.is_empty()).then(|| Self::CoreAudioInput(name.to_owned()))
        } else {
            None
        }
    }

    /// The device id this target round-trips to.
    pub fn device_id(&self) -> String {
        match self {
            Self::Output(id) => format!("wasapi-out:{id}"),
            Self::Input(id) => format!("wasapi-in:{id}"),
            Self::PwSource(id) => format!("pw-src:{id}"),
            Self::PwMonitor(id) => format!("pw-mon:{id}"),
            Self::CoreAudioInput(name) => format!("core-audio-in:{name}"),
        }
    }

    /// Whether the target selects a render (loopback) endpoint.
    pub fn is_output(&self) -> bool {
        matches!(self, Self::Output(_) | Self::PwMonitor(_))
    }
}

/// Extract the WASAPI device target from a device id, if any.
pub fn device_target(device_id: &str) -> Option<DeviceTarget> {
    DeviceTarget::parse(device_id)
}

/// The type of audio source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioSourceKind {
    /// Per-app audio capture (e.g. WASAPI per-app on Windows, PipeWire node
    /// on Linux).
    Application,
    /// System audio input device (e.g. microphone).
    InputDevice,
    /// System audio output device loopback (e.g. speaker capture). Also the
    /// fallback kind where per-app capture is unavailable (macOS system
    /// loopback).
    OutputDevice,
    /// Mixed (system + mic combined).
    Mixed,
}

impl AudioSourceKind {
    pub fn label(&self) -> &'static str {
        match self {
            AudioSourceKind::Application => "Application",
            AudioSourceKind::InputDevice => "Input Device",
            AudioSourceKind::OutputDevice => "Output Device",
            AudioSourceKind::Mixed => "Mixed",
        }
    }

    /// Icon hint for the GUI.
    pub fn icon(&self) -> &'static str {
        match self {
            AudioSourceKind::Application => "🖥️",
            AudioSourceKind::InputDevice => "🎤",
            AudioSourceKind::OutputDevice => "🔊",
            AudioSourceKind::Mixed => "🎵",
        }
    }
}

/// An audio source — captures audio from an application or hardware device,
/// with its own filter chain and an independent record/stream routing
/// decision.
///
/// Supports two modes:
/// - **Application**: Per-app audio capture (Windows WASAPI per-app, macOS
///   coreaudio).
/// - **Device**: System audio input/output device (microphone, speaker
///   loopback).
///
/// The actual capture is handled by the platform-specific audio layer
/// (later phase); the engine consumes PCM frames pushed per source id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioSource {
    /// Stable, persisted identity. Assigned by the engine on add when nil.
    #[serde(default = "Uuid::nil")]
    pub id: Uuid,
    /// User-visible name (e.g. "Discord", "Microphone (Realtek)").
    pub name: String,
    /// Platform-specific device/application identifier.
    pub device_id: String,
    /// Source kind.
    pub kind: AudioSourceKind,
    /// Capture volume (0.0–2.0).
    pub volume: f32,
    /// Whether the source is muted.
    pub muted: bool,
    /// Whether the source is currently active (capturing).
    #[serde(default)]
    pub active: bool,
    /// Which outputs receive this source.
    #[serde(default)]
    pub routing: AudioRouting,
    /// The audio buses this source feeds (issue #242, per-track model):
    /// 1-based bus ids; a source feeds every bus it belongs to. An empty
    /// list keeps the legacy record/stream booleans authoritative.
    #[serde(default)]
    pub track_members: Vec<u8>,
    /// Per-source filter chain.
    #[serde(default)]
    pub filters: AudioFilterConfig,
}

impl AudioSource {
    /// Create a new audio source (fresh random id).
    pub fn new(
        name: impl Into<String>,
        device_id: impl Into<String>,
        kind: AudioSourceKind,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            device_id: device_id.into(),
            kind,
            volume: 1.0,
            muted: false,
            active: false,
            routing: AudioRouting::default(),
            track_members: Vec::new(),
            filters: AudioFilterConfig::default(),
        }
    }

    /// Create an application audio source (per-app capture).
    pub fn application(name: impl Into<String>, device_id: impl Into<String>) -> Self {
        Self::new(name, device_id, AudioSourceKind::Application)
    }

    /// Extract the OS process id from an application source's `device_id`.
    ///
    /// Application sources store their capture target as `pid:<number>` once
    /// the user picked a process (Phase 3 of the multi-track audio routing
    /// design, [`docs/m6-audio-routing.md`](../../docs/m6-audio-routing.md)).
    /// Sources created before a process was chosen carry the `pending_app`
    /// placeholder and yield [`None`].
    pub fn device_pid(&self) -> Option<u32> {
        device_pid(&self.device_id)
    }

    /// Extract the WASAPI/PipeWire device target from this source's
    /// `device_id` (issue #229: `wasapi-out:<id>` render-loopback /
    /// `wasapi-in:<id>` capture endpoints; issue #231: `pw-src:<node>`
    /// source nodes / `pw-mon:<node>` sink monitors). [`None`] for every
    /// other convention.
    pub fn wasapi_device(&self) -> Option<DeviceTarget> {
        device_target(&self.device_id)
    }

    /// Create an input device source (microphone).
    pub fn input_device(name: impl Into<String>, device_id: impl Into<String>) -> Self {
        Self::new(name, device_id, AudioSourceKind::InputDevice)
    }

    /// Create an output device loopback source.
    pub fn output_device(name: impl Into<String>, device_id: impl Into<String>) -> Self {
        Self::new(name, device_id, AudioSourceKind::OutputDevice)
    }

    /// The legacy default "System" source (backward compatibility: the
    /// hardcoded System/Microphone pair becomes two default sources).
    pub fn system_default() -> Self {
        Self::output_device("System", "system_loopback")
    }

    /// The legacy default "Microphone" source.
    pub fn microphone_default() -> Self {
        Self::input_device("Microphone", "default_input")
    }

    /// Builder: override the id (used when restoring a persisted config).
    pub fn with_id(mut self, id: Uuid) -> Self {
        self.id = id;
        self
    }

    /// Builder: set the routing decision.
    pub fn with_routing(mut self, routing: AudioRouting) -> Self {
        self.routing = routing;
        self
    }

    /// Builder: set the audio-bus membership (issue #242).
    pub fn with_track_members(mut self, members: Vec<u8>) -> Self {
        self.track_members = members;
        self
    }

    /// Builder: set the filter chain.
    pub fn with_filters(mut self, filters: AudioFilterConfig) -> Self {
        self.filters = filters;
        self
    }

    /// Returns true if the device_id is set and non-empty.
    pub fn has_device(&self) -> bool {
        !self.device_id.is_empty()
    }

    /// Effective volume: 0.0 when muted, otherwise the configured volume.
    pub fn effective_volume(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.volume
        }
    }

    /// Returns a summary string for UI display.
    pub fn summary(&self) -> String {
        let status = if self.active { "active" } else { "inactive" };
        let vol = if self.muted {
            "muted".to_string()
        } else {
            format!("vol={:.0}%", self.volume * 100.0)
        };
        format!("{} {} {} [{}]", self.kind.icon(), self.name, vol, status,)
    }
}

impl Default for AudioSource {
    fn default() -> Self {
        Self::new("", "", AudioSourceKind::Mixed)
    }
}

/// Schema version of [`AudioRoutingConfig`]. Bump on breaking changes and
/// add a migration; old versions are rejected rather than silently misread.
pub const AUDIO_ROUTING_SCHEMA_VERSION: u32 = 1;

/// The persisted multi-track audio routing configuration.
///
/// Serialized as JSON (`audio_routing_v1` in the app storage). Secrets and
/// stream keys are never stored here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioRoutingConfig {
    pub version: u32,
    pub sources: Vec<AudioSource>,
}

/// Errors from [`AudioRoutingConfig::from_json`].
#[derive(Debug)]
pub enum AudioRoutingConfigError {
    /// The JSON could not be parsed.
    Parse(serde_json::Error),
    /// The document declares a schema version this build does not understand.
    UnknownVersion { found: u32, supported: u32 },
}

impl std::fmt::Display for AudioRoutingConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "invalid audio routing config JSON: {e}"),
            Self::UnknownVersion { found, supported } => write!(
                f,
                "audio routing config schema v{found} is not supported (this build supports v{supported})"
            ),
        }
    }
}

impl std::error::Error for AudioRoutingConfigError {}

impl Default for AudioRoutingConfig {
    fn default() -> Self {
        Self::empty()
    }
}

impl AudioRoutingConfig {
    /// An empty v1 config.
    pub fn empty() -> Self {
        Self {
            version: AUDIO_ROUTING_SCHEMA_VERSION,
            sources: Vec::new(),
        }
    }

    /// The backward-compatible default: the legacy System + Microphone pair
    /// as two default sources, both routed to both outputs.
    pub fn legacy_defaults() -> Self {
        Self {
            version: AUDIO_ROUTING_SCHEMA_VERSION,
            sources: vec![
                AudioSource::system_default(),
                AudioSource::microphone_default(),
            ],
        }
    }

    /// Serialize to JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("audio routing config serializes")
    }

    /// Parse from JSON, rejecting unknown schema versions instead of
    /// silently misreading them.
    pub fn from_json(json: &str) -> Result<Self, AudioRoutingConfigError> {
        let config: Self = serde_json::from_str(json).map_err(AudioRoutingConfigError::Parse)?;
        if config.version != AUDIO_ROUTING_SCHEMA_VERSION {
            return Err(AudioRoutingConfigError::UnknownVersion {
                found: config.version,
                supported: AUDIO_ROUTING_SCHEMA_VERSION,
            });
        }
        Ok(config)
    }
}

// ── Per-track audio model (issue #242) ───────────────────────

/// Schema version of [`AudioTrackConfig`]. Bump on breaking changes and
/// add a migration; old versions are rejected rather than silently misread.
pub const AUDIO_TRACK_SCHEMA_VERSION: u32 = 1;

/// The maximum number of audio buses a session can configure (OBS parity).
pub const AUDIO_TRACK_MAX: u8 = 6;

/// One mixing bus ("track") of the per-track audio model (issue #242).
///
/// Sources feed every bus they are a member of
/// (`AudioSource::track_members`); each bus is edited independently — its
/// own master gain and mute — and the recording mux receives every enabled
/// bus while the stream encodes the send bus
/// ([`AudioTrackConfig::send_track`]).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AudioBus {
    /// Stable identity within the track list (1-based track number).
    pub id: u8,
    /// Disabled buses are not built into any output pipeline.
    pub enabled: bool,
    /// Master gain in decibels (`-30.0..=30.0`).
    pub gain_db: f64,
    /// Whether the whole bus is muted.
    pub muted: bool,
}

impl AudioBus {
    /// A default bus with the given 1-based id: enabled, unity gain.
    pub fn new(id: u8) -> Self {
        Self {
            id,
            enabled: true,
            gain_db: 0.0,
            muted: false,
        }
    }

    /// The effective linear volume factor of this bus: `0.0` when muted,
    /// otherwise the dB gain converted to the `volume` element scale.
    pub fn effective_volume(&self) -> f64 {
        if self.muted {
            0.0
        } else {
            10.0_f64.powf(self.gain_db / 20.0)
        }
    }
}

/// The per-track audio configuration (issue #242).
///
/// Serialized as JSON (`audio_tracks_v1` in the app storage). Complements
/// [`AudioRoutingConfig`]: the source list stays there, while this struct
/// carries the bus list and the streaming send-bus selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioTrackConfig {
    pub version: u32,
    /// The mixing buses in track order (track 1 is `tracks[0]`).
    pub tracks: Vec<AudioBus>,
    /// 1-based index of the bus encoded into the streaming output (FLV/RTMP
    /// carries a single audio track).
    pub send_track: u8,
}

/// Errors from [`AudioTrackConfig::from_json`].
#[derive(Debug)]
pub enum AudioTrackConfigError {
    /// The JSON could not be parsed.
    Parse(serde_json::Error),
    /// The document declares a schema version this build does not understand.
    UnknownVersion { found: u32, supported: u32 },
}

impl std::fmt::Display for AudioTrackConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "invalid audio track config JSON: {e}"),
            Self::UnknownVersion { found, supported } => write!(
                f,
                "audio track config schema v{found} is not supported (this build supports v{supported})"
            ),
        }
    }
}

impl std::error::Error for AudioTrackConfigError {}

impl Default for AudioTrackConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioTrackConfig {
    /// The default config: four enabled, unity-gain buses and track 1 as the
    /// streaming send bus (OBS defaults).
    pub fn new() -> Self {
        Self {
            version: AUDIO_TRACK_SCHEMA_VERSION,
            tracks: (1..=4).map(AudioBus::new).collect(),
            send_track: 1,
        }
    }

    /// Clamp the track count to [`AUDIO_TRACK_MAX`] and keep `send_track`
    /// inside the remaining range. An empty bus list resets to the default
    /// four buses.
    pub fn sanitized(mut self) -> Self {
        if self.tracks.len() > AUDIO_TRACK_MAX as usize {
            self.tracks.truncate(AUDIO_TRACK_MAX as usize);
        }
        if self.tracks.is_empty() {
            return Self::new();
        }
        if self.send_track == 0 || self.send_track as usize > self.tracks.len() {
            self.send_track = 1;
        }
        self
    }

    /// The currently selected send bus, if any.
    pub fn send_bus(&self) -> Option<&AudioBus> {
        self.tracks.get(self.send_track.checked_sub(1)? as usize)
    }

    /// Serialize to JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("audio track config serializes")
    }

    /// Parse from JSON, rejecting unknown schema versions instead of
    /// silently misreading them.
    pub fn from_json(json: &str) -> Result<Self, AudioTrackConfigError> {
        let config: Self = serde_json::from_str(json).map_err(AudioTrackConfigError::Parse)?;
        if config.version != AUDIO_TRACK_SCHEMA_VERSION {
            return Err(AudioTrackConfigError::UnknownVersion {
                found: config.version,
                supported: AUDIO_TRACK_SCHEMA_VERSION,
            });
        }
        Ok(config)
    }
}

/// Migrate a legacy `audio_routing_v1` config to the per-track model
/// (issue #242): sources with `routing.record` join track 1, sources with
/// `routing.stream` join the send track, sources routed to neither keep an
/// empty membership (they stay silent everywhere, as before). Each matching
/// source's `track_members` is updated in place; the legacy booleans remain
/// authoritative until the pipeline builder consumes the track model.
pub fn migrate_routing_to_tracks(
    routing: &AudioRoutingConfig,
    sources: &mut [AudioSource],
) -> AudioTrackConfig {
    let config = AudioTrackConfig::new();
    let send = config.send_track;
    for source in sources.iter_mut() {
        let Some(saved) = routing.sources.iter().find(|s| s.id == source.id) else {
            continue;
        };
        let mut members = Vec::new();
        if saved.routing.record {
            members.push(1);
        }
        if saved.routing.stream && !members.contains(&send) {
            members.push(send);
        }
        source.track_members = members;
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── AudioSourceKind ──────────────────────────────────────────

    #[test]
    fn audio_source_kind_labels() {
        assert_eq!(AudioSourceKind::Application.label(), "Application");
        assert_eq!(AudioSourceKind::InputDevice.label(), "Input Device");
        assert_eq!(AudioSourceKind::OutputDevice.label(), "Output Device");
        assert_eq!(AudioSourceKind::Mixed.label(), "Mixed");
    }

    #[test]
    fn audio_source_kind_icons() {
        assert_eq!(AudioSourceKind::Application.icon(), "🖥️");
        assert_eq!(AudioSourceKind::InputDevice.icon(), "🎤");
        assert_eq!(AudioSourceKind::OutputDevice.icon(), "🔊");
        assert_eq!(AudioSourceKind::Mixed.icon(), "🎵");
    }

    // ── AudioSource creation ─────────────────────────────────────

    #[test]
    fn audio_source_new_defaults() {
        let asrc = AudioSource::new("Discord", "pid:1234", AudioSourceKind::Application);
        assert_eq!(asrc.name, "Discord");
        assert_eq!(asrc.device_id, "pid:1234");
        assert_eq!(asrc.kind, AudioSourceKind::Application);
        assert_eq!(asrc.volume, 1.0);
        assert!(!asrc.muted);
        assert!(!asrc.active);
        // Phase-1 invariants: fresh id, both-outputs routing, empty filters.
        assert!(!asrc.id.is_nil(), "new sources get a fresh id");
        assert_eq!(asrc.routing, AudioRouting::BOTH);
        assert!(asrc.filters.is_empty());
    }

    #[test]
    fn audio_source_ids_are_unique() {
        let a = AudioSource::application("A", "pid:1");
        let b = AudioSource::application("B", "pid:2");
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn audio_source_application_factory() {
        let asrc = AudioSource::application("Spotify", "pid:5678");
        assert_eq!(asrc.kind, AudioSourceKind::Application);
        assert_eq!(asrc.name, "Spotify");
    }

    #[test]
    fn audio_source_input_device_factory() {
        let asrc = AudioSource::input_device("Realtek Mic", "hw:0,0");
        assert_eq!(asrc.kind, AudioSourceKind::InputDevice);
    }

    #[test]
    fn audio_source_output_device_factory() {
        let asrc = AudioSource::output_device("Speakers", "hw:0,1");
        assert_eq!(asrc.kind, AudioSourceKind::OutputDevice);
    }

    #[test]
    fn legacy_default_sources_exist() {
        let sys = AudioSource::system_default();
        let mic = AudioSource::microphone_default();
        assert_eq!(sys.kind, AudioSourceKind::OutputDevice);
        assert_eq!(mic.kind, AudioSourceKind::InputDevice);
        assert_eq!(sys.routing, AudioRouting::BOTH);
        assert_eq!(mic.routing, AudioRouting::BOTH);
        assert_ne!(sys.id, mic.id);
    }

    #[test]
    fn audio_source_default() {
        let asrc = AudioSource::default();
        assert!(asrc.name.is_empty());
        assert!(asrc.device_id.is_empty());
        assert_eq!(asrc.kind, AudioSourceKind::Mixed);
    }

    #[test]
    fn builders_override_routing_and_filters() {
        let filters = AudioFilterConfig {
            compressor: Some(CompressorConfig::default()),
            ..AudioFilterConfig::default()
        };
        let asrc = AudioSource::application("Game", "pid:1")
            .with_routing(AudioRouting::RECORD_ONLY)
            .with_filters(filters);
        assert_eq!(asrc.routing, AudioRouting::RECORD_ONLY);
        assert!(asrc.filters.compressor.is_some());
        assert!(!asrc.id.is_nil(), "application() assigns a fresh id");
    }

    // ── Device check ─────────────────────────────────────────────

    #[test]
    fn has_device_true() {
        let asrc = AudioSource::new("Discord", "pid:1234", AudioSourceKind::Application);
        assert!(asrc.has_device());
    }

    #[test]
    fn has_device_false() {
        let asrc = AudioSource::new("Empty", "", AudioSourceKind::Application);
        assert!(!asrc.has_device());
    }

    // ── Volume ───────────────────────────────────────────────────

    #[test]
    fn effective_volume_normal() {
        let mut asrc = AudioSource::new("Test", "id", AudioSourceKind::Application);
        asrc.volume = 0.75;
        assert!((asrc.effective_volume() - 0.75).abs() < 0.01);
    }

    #[test]
    fn effective_volume_muted() {
        let mut asrc = AudioSource::new("Test", "id", AudioSourceKind::Application);
        asrc.volume = 0.75;
        asrc.muted = true;
        assert_eq!(asrc.effective_volume(), 0.0);
    }

    // ── Routing ──────────────────────────────────────────────────

    #[test]
    fn routing_constants() {
        assert_eq!(
            (AudioRouting::BOTH.record, AudioRouting::BOTH.stream),
            (true, true)
        );
        assert_eq!(
            (
                AudioRouting::RECORD_ONLY.record,
                AudioRouting::RECORD_ONLY.stream
            ),
            (true, false)
        );
        assert_eq!(
            (
                AudioRouting::STREAM_ONLY.record,
                AudioRouting::STREAM_ONLY.stream
            ),
            (false, true)
        );
        assert_eq!(
            (AudioRouting::NONE.record, AudioRouting::NONE.stream),
            (false, false)
        );
    }

    #[test]
    fn routing_default_is_both() {
        assert_eq!(AudioRouting::default(), AudioRouting::BOTH);
    }

    // ── Filter chain ─────────────────────────────────────────────

    #[test]
    fn filter_config_default_is_empty() {
        let filters = AudioFilterConfig::default();
        assert!(filters.is_empty());
        assert_eq!(filters.chain_fragment(), "");
    }

    #[test]
    fn filter_chain_single_stage() {
        let filters = AudioFilterConfig {
            compressor: Some(CompressorConfig::default()),
            ..AudioFilterConfig::default()
        };
        let frag = filters.chain_fragment();
        assert_eq!(
            frag,
            "audiodynamic mode=compressor characteristics=soft-knee threshold=0.5 ratio=4"
        );
    }

    #[test]
    fn filter_chain_full_order_is_deterministic() {
        let filters = AudioFilterConfig {
            noise_gate: Some(NoiseGateConfig::default()),
            expander: Some(ExpanderConfig::default()),
            compressor: Some(CompressorConfig::default()),
            limiter: Some(LimiterConfig::default()),
            gain_db: 3.0,
            eq: Some(EqConfig {
                bands: [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -2.0],
            }),
        };
        let frag = filters.chain_fragment();
        assert_eq!(frag, "audiodynamic mode=expander characteristics=hard-knee threshold=0.03 ratio=10 ! audiodynamic mode=expander characteristics=soft-knee threshold=0.3 ratio=1.5 ! audiodynamic mode=compressor characteristics=soft-knee threshold=0.5 ratio=4 ! audiodynamic mode=compressor characteristics=hard-knee threshold=0.95 ratio=20 ! audioamplify amplification=1.4125 ! equalizer-10bands band0=1.0 band1=0.0 band2=0.0 band3=0.0 band4=0.0 band5=0.0 band6=0.0 band7=0.0 band8=0.0 band9=-2.0");
        // Determinism: identical config → identical fragment.
        assert_eq!(frag, filters.chain_fragment());
    }

    #[test]
    fn filter_chain_tiny_gain_and_tiny_eq_are_skipped() {
        let filters = AudioFilterConfig {
            gain_db: 0.05,
            eq: Some(EqConfig {
                bands: [0.2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            }),
            ..AudioFilterConfig::default()
        };
        assert!(filters.is_empty());
        assert_eq!(filters.chain_fragment(), "");
    }

    #[test]
    fn filter_chain_negative_gain() {
        let filters = AudioFilterConfig {
            gain_db: -6.0,
            ..AudioFilterConfig::default()
        };
        let frag = filters.chain_fragment();
        assert_eq!(frag, "audioamplify amplification=0.5012");
    }

    #[test]
    fn filter_chain_availability_skips_missing_factories() {
        let filters = AudioFilterConfig {
            noise_gate: Some(NoiseGateConfig::default()),
            compressor: Some(CompressorConfig::default()),
            gain_db: 2.0,
            ..AudioFilterConfig::default()
        };
        // Only `audiodynamic` is available: the gate and compressor survive,
        // the `audioamplify` gain stage is skipped and reported.
        let (frag, skipped) = filters.chain_fragment_with_availability(|f| f == "audiodynamic");
        assert!(frag.starts_with("audiodynamic"));
        assert!(frag.contains("mode=compressor"));
        assert!(!frag.contains("audioamplify"));
        assert_eq!(skipped, vec!["audioamplify"]);
    }

    // ── Persistence round-trip ───────────────────────────────────

    #[test]
    fn routing_config_round_trip_preserves_everything() {
        let mut source = AudioSource::application("Spotify", "pid:5678")
            .with_routing(AudioRouting::STREAM_ONLY)
            .with_id(Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0));
        source.volume = 1.75;
        source.muted = true;
        source.filters = AudioFilterConfig {
            noise_gate: Some(NoiseGateConfig {
                threshold: 0.02,
                ratio: 12.0,
            }),
            expander: None,
            compressor: Some(CompressorConfig::default()),
            limiter: None,
            gain_db: -3.5,
            eq: Some(EqConfig {
                bands: [0.0, 1.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
            }),
        };
        let config = AudioRoutingConfig {
            version: AUDIO_ROUTING_SCHEMA_VERSION,
            sources: vec![
                source,
                AudioSource::microphone_default().with_routing(AudioRouting::RECORD_ONLY),
            ],
        };

        let json = config.to_json();
        let restored = AudioRoutingConfig::from_json(&json).expect("round-trip parses");

        assert_eq!(restored.version, AUDIO_ROUTING_SCHEMA_VERSION);
        assert_eq!(restored.sources.len(), 2);
        let back = &restored.sources[0];
        assert_eq!(
            back.id,
            Uuid::from_u128(0x1234_5678_9abc_def0_1234_5678_9abc_def0)
        );
        assert_eq!(back.name, "Spotify");
        assert_eq!(back.device_id, "pid:5678");
        assert_eq!(back.kind, AudioSourceKind::Application);
        assert!((back.volume - 1.75).abs() < f32::EPSILON);
        assert!(back.muted);
        assert_eq!(back.routing, AudioRouting::STREAM_ONLY);
        assert_eq!(
            back.filters.noise_gate,
            Some(NoiseGateConfig {
                threshold: 0.02,
                ratio: 12.0,
            })
        );
        assert!(back.filters.compressor.is_some());
        assert!((back.filters.gain_db + 3.5).abs() < 1e-9);
        assert_eq!(back.filters.eq.unwrap().bands[1], 1.5);
        let mic = &restored.sources[1];
        assert_eq!(mic.routing, AudioRouting::RECORD_ONLY);
    }

    #[test]
    fn routing_config_rejects_unknown_version() {
        let json = r#"{"version": 99, "sources": []}"#;
        let err = AudioRoutingConfig::from_json(json).expect_err("unknown version rejected");
        match err {
            AudioRoutingConfigError::UnknownVersion { found, supported } => {
                assert_eq!(found, 99);
                assert_eq!(supported, AUDIO_ROUTING_SCHEMA_VERSION);
            }
            other => panic!("expected UnknownVersion, got {other:?}"),
        }
        let msg = format!("{err}");
        assert!(msg.contains("v99"), "error mentions the found version");
    }

    #[test]
    fn routing_config_rejects_garbage() {
        assert!(AudioRoutingConfig::from_json("not json at all").is_err());
    }

    #[test]
    fn routing_config_legacy_defaults() {
        let config = AudioRoutingConfig::legacy_defaults();
        assert_eq!(config.version, 1);
        assert_eq!(config.sources.len(), 2);
        assert_eq!(config.sources[0].name, "System");
        assert_eq!(config.sources[1].name, "Microphone");
        // Round-trips like any other config.
        let restored = AudioRoutingConfig::from_json(&config.to_json()).unwrap();
        assert_eq!(restored, config);
    }

    #[test]
    fn routing_config_missing_fields_get_defaults() {
        // A minimal legacy-ish document (no routing/filters/active/id).
        let json = r#"{"version": 1, "sources": [{"id": "00000000-0000-0000-0000-000000000000", "name": "Game", "device_id": "pid:1", "kind": "Application", "volume": 1.0, "muted": false}]}"#;
        let config = AudioRoutingConfig::from_json(json).expect("parses");
        let source = &config.sources[0];
        assert_eq!(
            source.routing,
            AudioRouting::BOTH,
            "routing defaults to both"
        );
        assert!(source.filters.is_empty(), "filters default to empty");
        assert!(!source.active);
    }

    // ── Summary, Clone, Debug ────────────────────────────────────

    #[test]
    fn summary_inactive() {
        let asrc = AudioSource::new("Discord", "id", AudioSourceKind::Application);
        let s = asrc.summary();
        assert!(s.contains("Discord"));
        assert!(s.contains("inactive"));
        assert!(s.contains("vol=100%"));
    }

    #[test]
    fn summary_active() {
        let mut asrc = AudioSource::new("Discord", "id", AudioSourceKind::Application);
        asrc.active = true;
        let s = asrc.summary();
        assert!(s.contains("active"));
    }

    #[test]
    fn summary_muted() {
        let mut asrc = AudioSource::new("Mic", "id", AudioSourceKind::InputDevice);
        asrc.muted = true;
        let s = asrc.summary();
        assert!(s.contains("muted"));
    }

    #[test]
    fn summary_custom_volume() {
        let mut asrc = AudioSource::new("Spotify", "id", AudioSourceKind::Application);
        asrc.volume = 0.5;
        let s = asrc.summary();
        assert!(s.contains("vol=50%"));
    }

    #[test]
    fn audio_source_clone() {
        let mut asrc = AudioSource::new("Clone", "id", AudioSourceKind::Application);
        asrc.volume = 0.5;
        asrc.active = true;
        let asrc2 = asrc.clone();
        assert_eq!(asrc2.volume, 0.5);
        assert!(asrc2.active);
        assert_eq!(asrc2.id, asrc.id, "id is cloned");
    }

    #[test]
    fn audio_source_debug() {
        let asrc = AudioSource::new("Debug", "id", AudioSourceKind::Application);
        let dbg = format!("{asrc:?}");
        assert!(dbg.contains("AudioSource"));
        assert!(dbg.contains("Debug"));
    }

    #[test]
    fn device_pid_parses_pid_prefix() {
        assert_eq!(device_pid("pid:1234"), Some(1234));
        assert_eq!(device_pid("pid:0"), None, "pid 0 is the idle process");
        assert_eq!(device_pid("pid:4294967295"), Some(u32::MAX));
    }

    #[test]
    fn device_pid_rejects_non_pid_forms() {
        assert_eq!(device_pid("pending_app"), None);
        assert_eq!(device_pid("default_input"), None);
        assert_eq!(device_pid("system_loopback"), None);
        assert_eq!(device_pid("pid:notanumber"), None);
        assert_eq!(device_pid("pid:"), None);
        assert_eq!(device_pid(""), None);
        assert_eq!(device_pid("PID:1234"), None, "prefix is case sensitive");
    }

    #[test]
    fn audio_source_device_pid_delegates() {
        let app = AudioSource::application("Game", "pid:4711");
        assert_eq!(app.device_pid(), Some(4711));

        let pending = AudioSource::application("Pending", "pending_app");
        assert_eq!(pending.device_pid(), None);

        let mic = AudioSource::input_device("Mic", "default_input");
        assert_eq!(mic.device_pid(), None);
    }

    #[test]
    fn device_target_parses_wasapi_conventions() {
        // Issue #229: render and capture endpoint conventions round-trip.
        let out = DeviceTarget::parse("wasapi-out:{0.0.0.00000000}.{abc-123}")
            .expect("wasapi-out id parses");
        assert_eq!(
            out,
            DeviceTarget::Output("{0.0.0.00000000}.{abc-123}".to_owned())
        );
        assert!(out.is_output());
        assert_eq!(
            out.device_id(),
            "wasapi-out:{0.0.0.00000000}.{abc-123}",
            "round-trip is stable"
        );

        let input = DeviceTarget::parse("wasapi-in:{0.0.1.00000000}.{def-456}")
            .expect("wasapi-in id parses");
        assert_eq!(
            input,
            DeviceTarget::Input("{0.0.1.00000000}.{def-456}".to_owned())
        );
        assert!(!input.is_output());
        assert_eq!(input.device_id(), "wasapi-in:{0.0.1.00000000}.{def-456}");
    }

    #[test]
    fn device_target_rejects_other_forms_and_empties() {
        // Legacy placeholders and other conventions must not be mistaken
        // for WASAPI endpoint selections.
        assert_eq!(DeviceTarget::parse("system_loopback"), None);
        assert_eq!(DeviceTarget::parse("default_input"), None);
        assert_eq!(DeviceTarget::parse("pending_app"), None);
        assert_eq!(DeviceTarget::parse("pid:1234"), None);
        assert_eq!(DeviceTarget::parse(""), None);
        assert_eq!(DeviceTarget::parse("WASAPI-OUT:x"), None, "case sensitive");
        // Empty endpoint ids are meaningless selections, not defaults.
        assert_eq!(DeviceTarget::parse("wasapi-out:"), None);
        assert_eq!(DeviceTarget::parse("wasapi-out:   "), None);
        assert_eq!(DeviceTarget::parse("wasapi-in:"), None);
    }

    #[test]
    fn device_target_parses_coreaudio_convention() {
        // Issue #231: the macOS convention addresses input devices by name
        // (cpal's stable public identifier) and round-trips.
        let mic =
            DeviceTarget::parse("core-audio-in:BlackHole 2ch").expect("core-audio-in id parses");
        assert_eq!(
            mic,
            DeviceTarget::CoreAudioInput("BlackHole 2ch".to_owned())
        );
        assert!(!mic.is_output(), "an input device is input-kind");
        assert_eq!(
            mic.device_id(),
            "core-audio-in:BlackHole 2ch",
            "round-trip is stable (names may contain spaces)"
        );
        // Names are trimmed; empty names are meaningless selections.
        assert_eq!(
            DeviceTarget::parse("core-audio-in:  MacBook Pro Microphone  "),
            Some(DeviceTarget::CoreAudioInput(
                "MacBook Pro Microphone".to_owned()
            ))
        );
        assert_eq!(DeviceTarget::parse("core-audio-in:"), None);
        assert_eq!(DeviceTarget::parse("core-audio-in:   "), None);
        assert_eq!(DeviceTarget::parse("CORE-AUDIO-IN:x"), None);
        assert_eq!(DeviceTarget::parse("core-audio"), None);
    }

    #[test]
    fn device_target_parses_pipewire_conventions() {
        // Issue #231: PipeWire source-node and sink-monitor conventions
        // round-trip; the node id is a positive decimal number.
        let src = DeviceTarget::parse("pw-src:42").expect("pw-src id parses");
        assert_eq!(src, DeviceTarget::PwSource(42));
        assert!(!src.is_output(), "a source node is an input-kind target");
        assert_eq!(src.device_id(), "pw-src:42", "round-trip is stable");

        let mon = DeviceTarget::parse("pw-mon:7").expect("pw-mon id parses");
        assert_eq!(mon, DeviceTarget::PwMonitor(7));
        assert!(
            mon.is_output(),
            "a sink monitor carries what an output plays"
        );
        assert_eq!(mon.device_id(), "pw-mon:7");

        // Node id 0 is PipeWire's ID_ANY — never a capturable target.
        assert_eq!(DeviceTarget::parse("pw-src:0"), None);
        assert_eq!(DeviceTarget::parse("pw-mon:0"), None);
        // Malformed or negative ids do not parse.
        assert_eq!(DeviceTarget::parse("pw-src:"), None);
        assert_eq!(DeviceTarget::parse("pw-src:   "), None);
        assert_eq!(DeviceTarget::parse("pw-src:abc"), None);
        assert_eq!(DeviceTarget::parse("pw-src:-1"), None);
        assert_eq!(DeviceTarget::parse("pw-src:4.2"), None);
        // Prefixes are case sensitive and prefix-distinct.
        assert_eq!(DeviceTarget::parse("PW-SRC:42"), None);
        assert_eq!(DeviceTarget::parse("pw"), None);
    }

    #[test]
    fn audio_source_wasapi_device_delegates() {
        let out = AudioSource::output_device(
            "Sonar Stream",
            "wasapi-out:{0.0.0.00000000}.{sonar-stream}",
        );
        assert_eq!(
            out.wasapi_device(),
            Some(DeviceTarget::Output(
                "{0.0.0.00000000}.{sonar-stream}".to_owned()
            ))
        );

        let mic = AudioSource::input_device("Podcast Mic", "wasapi-in:{mic-endpoint}");
        assert_eq!(
            mic.wasapi_device(),
            Some(DeviceTarget::Input("{mic-endpoint}".to_owned()))
        );

        // Legacy sources keep yielding None.
        assert_eq!(AudioSource::system_default().wasapi_device(), None);
        assert_eq!(AudioSource::microphone_default().wasapi_device(), None);
        let app = AudioSource::application("Game", "pid:42");
        assert_eq!(app.wasapi_device(), None);
    }

    #[test]
    fn wasapi_device_accessor_carries_pipewire_ids() {
        // Issue #231: the same accessor carries the PipeWire conventions,
        // so the GUI picker/lifecycle wiring stays platform-agnostic.
        let mic = AudioSource::input_device("USB Mic", "pw-src:42");
        assert_eq!(mic.wasapi_device(), Some(DeviceTarget::PwSource(42)));

        let desktop = AudioSource::output_device("Desktop", "pw-mon:7");
        assert_eq!(desktop.wasapi_device(), Some(DeviceTarget::PwMonitor(7)));
    }

    #[test]
    fn wasapi_device_accessor_carries_coreaudio_ids() {
        // Issue #231: the macOS convention rides the same accessor.
        let mic = AudioSource::input_device("Podcast Mic", "core-audio-in:MacBook Pro Microphone");
        assert_eq!(
            mic.wasapi_device(),
            Some(DeviceTarget::CoreAudioInput(
                "MacBook Pro Microphone".to_owned()
            ))
        );
    }

    // ── Per-track audio model (issue #242) ───────────────────────

    #[test]
    fn audio_track_config_defaults_match_obs() {
        let config = AudioTrackConfig::new();
        assert_eq!(config.version, 1);
        assert_eq!(config.tracks.len(), 4);
        assert_eq!(config.send_track, 1);
        for (i, bus) in config.tracks.iter().enumerate() {
            assert_eq!(bus.id as usize, i + 1);
            assert!(bus.enabled);
            assert_eq!(bus.gain_db, 0.0);
            assert!(!bus.muted);
        }
        assert_eq!(config.send_bus().map(|b| b.id), Some(1));
    }

    #[test]
    fn audio_bus_gain_converts_to_linear_volume() {
        let mut bus = AudioBus::new(1);
        assert_eq!(bus.effective_volume(), 1.0);
        bus.gain_db = 6.0;
        assert!((bus.effective_volume() - 10.0_f64.powf(0.3)).abs() < 1e-9);
        bus.gain_db = -6.0;
        assert!((bus.effective_volume() - 10.0_f64.powf(-0.3)).abs() < 1e-9);
        bus.muted = true;
        assert_eq!(bus.effective_volume(), 0.0);
    }

    #[test]
    fn audio_track_config_sanitizes_count_and_send_track() {
        let mut config = AudioTrackConfig::new();
        for id in 5..=9u8 {
            config.tracks.push(AudioBus::new(id));
        }
        config.send_track = 9;
        let config = config.sanitized();
        assert_eq!(config.tracks.len(), AUDIO_TRACK_MAX as usize);
        assert_eq!(config.send_track, 1, "out-of-range send track resets");

        let mut empty = AudioTrackConfig::new();
        empty.tracks.clear();
        empty.send_track = 3;
        let empty = empty.sanitized();
        assert_eq!(empty.tracks.len(), 4, "empty bus list resets to default");
        assert_eq!(empty.send_track, 1);

        let mut zero = AudioTrackConfig::new();
        zero.send_track = 0;
        assert_eq!(zero.sanitized().send_track, 1);
    }

    #[test]
    fn audio_track_config_json_round_trip_and_version_guard() {
        let mut config = AudioTrackConfig::new();
        config.tracks[0].gain_db = -3.5;
        config.tracks[1].muted = true;
        config.send_track = 2;
        let restored = AudioTrackConfig::from_json(&config.to_json()).expect("round-trip parses");
        assert_eq!(restored, config);

        let future = config.to_json().replace("\"version\":1", "\"version\":99");
        let err = AudioTrackConfig::from_json(&future).expect_err("unknown version rejected");
        assert!(matches!(
            err,
            AudioTrackConfigError::UnknownVersion {
                found: 99,
                supported: 1
            }
        ));
        assert!(AudioTrackConfig::from_json("not json").is_err());
    }

    #[test]
    fn migration_maps_routing_bools_onto_track_members() {
        let mut routing = AudioRoutingConfig::legacy_defaults();
        let game =
            AudioSource::application("Game", "pid:42").with_routing(AudioRouting::RECORD_ONLY);
        routing.sources.push(game);
        let silent = AudioSource::output_device("Silent", "system_loopback")
            .with_routing(AudioRouting::NONE);
        routing.sources.push(silent);

        let mut sources = routing.sources.clone();
        let config = migrate_routing_to_tracks(&routing, &mut sources);

        assert_eq!(config.tracks.len(), 4);
        assert_eq!(config.send_track, 1);
        // BOTH: record track == send track → single membership.
        assert_eq!(sources[0].track_members, vec![1]);
        assert_eq!(sources[1].track_members, vec![1]);
        // RECORD_ONLY: record track only.
        assert_eq!(sources[2].track_members, vec![1]);
        // NONE: stays silent everywhere.
        assert!(sources[3].track_members.is_empty());

        // Ids without a persisted counterpart are left alone.
        let mut partial = vec![AudioSource::application("New", "pid:7")];
        migrate_routing_to_tracks(&routing, &mut partial);
        assert!(partial[0].track_members.is_empty());
    }

    #[test]
    fn track_members_field_is_additive_over_audio_routing_v1() {
        // Old configs (no `track_members` key) keep parsing with an empty
        // membership; new configs round-trip the field.
        let source = AudioSource::application("Game", "pid:42");
        let json = AudioRoutingConfig {
            version: 1,
            sources: vec![source],
        }
        .to_json();
        let stripped = json.replace(",\"track_members\":[]", "");
        assert_ne!(stripped, json, "track_members serializes by default");
        let parsed =
            AudioRoutingConfig::from_json(&stripped).expect("v1 JSON without track_members parses");
        assert!(parsed.sources[0].track_members.is_empty());

        let member = AudioSource::application("Game", "pid:42").with_track_members(vec![1, 3]);
        let round = AudioRoutingConfig::from_json(
            &AudioRoutingConfig {
                version: 1,
                sources: vec![member],
            }
            .to_json(),
        )
        .expect("round-trip parses");
        assert_eq!(round.sources[0].track_members, vec![1, 3]);
    }
}
