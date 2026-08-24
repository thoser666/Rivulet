/// An audio source — captures audio from an application or hardware device.
///
/// Supports two modes:
/// - **Application**: Per-app audio capture (Windows WASAPI loopback, macOS coreaudio).
/// - **Device**: System audio input/output device (microphone, speaker loopback).
///
/// The actual capture is handled by the platform-specific audio layer.
#[derive(Debug, Clone)]
pub struct AudioSource {
    /// User-visible name (e.g. "Discord", "Microphone (Realtek)").
    pub name: String,
    /// Platform-specific device/application identifier.
    pub device_id: String,
    /// Source kind.
    pub kind: AudioSourceKind,
    /// Capture volume (0.0–1.0).
    pub volume: f32,
    /// Whether the source is muted.
    pub muted: bool,
    /// Whether the source is currently active (capturing).
    pub active: bool,
}

/// The type of audio source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSourceKind {
    /// Per-app audio capture (e.g. WASAPI loopback on Windows).
    Application,
    /// System audio input device (e.g. microphone).
    InputDevice,
    /// System audio output device loopback (e.g. speaker capture).
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

impl AudioSource {
    /// Create a new audio source.
    pub fn new(
        name: impl Into<String>,
        device_id: impl Into<String>,
        kind: AudioSourceKind,
    ) -> Self {
        Self {
            name: name.into(),
            device_id: device_id.into(),
            kind,
            volume: 1.0,
            muted: false,
            active: false,
        }
    }

    /// Create an application audio source (per-app capture).
    pub fn application(name: impl Into<String>, device_id: impl Into<String>) -> Self {
        Self::new(name, device_id, AudioSourceKind::Application)
    }

    /// Create an input device source (microphone).
    pub fn input_device(name: impl Into<String>, device_id: impl Into<String>) -> Self {
        Self::new(name, device_id, AudioSourceKind::InputDevice)
    }

    /// Create an output device loopback source.
    pub fn output_device(name: impl Into<String>, device_id: impl Into<String>) -> Self {
        Self::new(name, device_id, AudioSourceKind::OutputDevice)
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
    fn audio_source_default() {
        let asrc = AudioSource::default();
        assert!(asrc.name.is_empty());
        assert!(asrc.device_id.is_empty());
        assert_eq!(asrc.kind, AudioSourceKind::Mixed);
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

    // ── Summary ──────────────────────────────────────────────────

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

    // ── Clone, Debug ─────────────────────────────────────────────

    #[test]
    fn audio_source_clone() {
        let mut asrc = AudioSource::new("Clone", "id", AudioSourceKind::Application);
        asrc.volume = 0.5;
        asrc.active = true;
        let asrc2 = asrc.clone();
        assert_eq!(asrc2.volume, 0.5);
        assert!(asrc2.active);
    }

    #[test]
    fn audio_source_debug() {
        let asrc = AudioSource::new("Debug", "id", AudioSourceKind::Application);
        let dbg = format!("{asrc:?}");
        assert!(dbg.contains("AudioSource"));
        assert!(dbg.contains("Debug"));
    }
}
