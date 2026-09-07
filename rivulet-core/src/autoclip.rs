//! Chat-driven auto-clip: save replay-buffer highlights when chat activity
//! spikes or a `!clip` command is received.
//!
//! The [`SpikeDetector`] monitors incoming [`ChatMessage`]s through a sliding
//! time window and fires a callback when the message rate exceeds the
//! configured threshold. A separate [`handle_clip_command`] helper checks
//! every message for the `!clip` trigger and returns `true` when the clip
//! command was recognised.
//!
//! Both triggers feed into the same action — save the current replay buffer
//! snapshot — but are independent: a spike can fire even when the user hasn't
//! typed `!clip`, and `!clip` works even at low chat activity.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Configuration for the auto-clip feature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoClipConfig {
    /// Whether auto-clipping is enabled.
    pub enabled: bool,
    /// Minimum number of messages within the window to trigger a spike clip.
    pub spike_threshold: usize,
    /// Sliding window duration for the spike detector.
    pub window: Duration,
    /// Minimum time between two spike-triggered clips.
    pub cooldown: Duration,
    /// Command that triggers an instant clip (without the `!`).
    pub clip_command: String,
}

impl Default for AutoClipConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            spike_threshold: 15,
            window: Duration::from_secs(30),
            cooldown: Duration::from_secs(60),
            clip_command: "clip".to_owned(),
        }
    }
}

impl AutoClipConfig {
    /// Validate the configuration. Returns an error message on invalid values.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.spike_threshold == 0 {
            return Err("spike threshold must be at least 1");
        }
        if self.window.as_secs() < 1 {
            return Err("window must be at least 1 second");
        }
        if self.cooldown.as_secs() < 1 {
            return Err("cooldown must be at least 1 second");
        }
        Ok(())
    }

    /// The full command string including the `!` prefix.
    pub fn full_command(&self) -> String {
        format!("!{}", self.clip_command.trim_start_matches('!'))
    }
}

/// Sliding-window spike detector that tracks message timestamps.
///
/// Call [`SpikeDetector::record`] for every incoming chat message, then
/// [`SpikeDetector::should_clip`] to check whether the threshold was crossed
/// and the cooldown has elapsed.
#[derive(Debug)]
pub struct SpikeDetector {
    /// Timestamps of messages within the sliding window.
    timestamps: VecDeque<Instant>,
    /// When the last spike-triggered clip was fired.
    last_clip: Option<Instant>,
    config: AutoClipConfig,
}

impl SpikeDetector {
    /// Create a new detector with the given configuration.
    pub fn new(config: AutoClipConfig) -> Self {
        Self {
            timestamps: VecDeque::new(),
            last_clip: None,
            config,
        }
    }

    /// Record an incoming message timestamp and prune the window.
    pub fn record(&mut self, now: Instant) {
        self.timestamps.push_back(now);
        self.prune(now);
    }

    /// Check whether a spike clip should fire right now.
    ///
    /// Returns `true` when:
    /// - the message count within the window >= threshold, **and**
    /// - the cooldown since the last clip has elapsed (or no clip has fired yet).
    pub fn should_clip(&mut self, now: Instant) -> bool {
        self.prune(now);
        if self.timestamps.len() < self.config.spike_threshold {
            return false;
        }
        match self.last_clip {
            Some(last) if now.duration_since(last) < self.config.cooldown => false,
            _ => {
                self.last_clip = Some(now);
                // Drain the window so we don't re-fire immediately.
                self.timestamps.clear();
                true
            }
        }
    }

    /// Remove timestamps older than the sliding window.
    fn prune(&mut self, now: Instant) {
        let cutoff = now.checked_sub(self.config.window).unwrap_or(now);
        while self.front().is_some_and(|&t| t < cutoff) {
            self.timestamps.pop_front();
        }
    }

    fn front(&self) -> Option<&Instant> {
        self.timestamps.front()
    }

    /// Current message count within the window (for diagnostics / UI display).
    pub fn window_count(&self) -> usize {
        self.timestamps.len()
    }

    /// Update the configuration at runtime (e.g. when the user changes
    /// settings).
    pub fn set_config(&mut self, config: AutoClipConfig) {
        self.config = config;
    }
}

/// Check whether a chat message text is the clip command.
///
/// Matches `!clip`, `!clip <anything>`, case-insensitively. Returns `true`
/// when the message is a clip command.
pub fn handle_clip_command(message_text: &str, command: &str) -> bool {
    let cmd = command.trim_start_matches('!').to_lowercase();
    let text = message_text.trim().to_lowercase();
    text == format!("!{}", cmd) || text.starts_with(&format!("!{} ", cmd))
}

/// Check whether a chat message is a clip command based on the config.
pub fn is_clip_command(message_text: &str, config: &AutoClipConfig) -> bool {
    handle_clip_command(message_text, &config.clip_command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_disabled() {
        let config = AutoClipConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.spike_threshold, 15);
        assert_eq!(config.window, Duration::from_secs(30));
        assert_eq!(config.cooldown, Duration::from_secs(60));
    }

    #[test]
    fn validate_rejects_zero_threshold() {
        let config = AutoClipConfig {
            spike_threshold: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_window() {
        let config = AutoClipConfig {
            window: Duration::from_secs(0),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_cooldown() {
        let config = AutoClipConfig {
            cooldown: Duration::from_secs(0),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn validate_accepts_valid_config() {
        assert!(AutoClipConfig::default().validate().is_ok());
    }

    #[test]
    fn full_command_adds_bang() {
        let config = AutoClipConfig {
            clip_command: "clip".to_owned(),
            ..Default::default()
        };
        assert_eq!(config.full_command(), "!clip");
    }

    #[test]
    fn full_command_preserves_existing_bang() {
        let config = AutoClipConfig {
            clip_command: "!clip".to_owned(),
            ..Default::default()
        };
        assert_eq!(config.full_command(), "!clip");
    }

    #[test]
    fn spike_detector_fires_when_threshold_reached() {
        let config = AutoClipConfig {
            enabled: true,
            spike_threshold: 3,
            window: Duration::from_secs(10),
            cooldown: Duration::from_secs(5),
            ..Default::default()
        };
        let mut det = SpikeDetector::new(config);
        let t0 = Instant::now();
        det.record(t0);
        det.record(t0);
        assert!(!det.should_clip(t0));
        det.record(t0);
        assert!(det.should_clip(t0));
    }

    #[test]
    fn spike_detector_respects_cooldown() {
        let config = AutoClipConfig {
            enabled: true,
            spike_threshold: 2,
            window: Duration::from_secs(10),
            cooldown: Duration::from_secs(5),
            ..Default::default()
        };
        let mut det = SpikeDetector::new(config);
        let t0 = Instant::now();
        det.record(t0);
        det.record(t0);
        assert!(det.should_clip(t0)); // fires, drains window
                                      // Re-add messages immediately — cooldown should block
        det.record(t0);
        det.record(t0);
        assert!(!det.should_clip(t0));
        // After cooldown passes
        assert!(det.should_clip(t0 + Duration::from_secs(6)));
    }

    #[test]
    fn spike_detector_prunes_old_messages() {
        let config = AutoClipConfig {
            spike_threshold: 2,
            window: Duration::from_secs(5),
            cooldown: Duration::from_secs(0),
            ..Default::default()
        };
        let mut det = SpikeDetector::new(config);
        let t0 = Instant::now();
        det.record(t0);
        det.record(t0 + Duration::from_secs(1));
        // Messages outside the window
        det.record(t0 + Duration::from_secs(10));
        assert_eq!(det.window_count(), 1);
    }

    #[test]
    fn clip_command_matches_exact() {
        assert!(handle_clip_command("!clip", "clip"));
        assert!(handle_clip_command("!Clip", "clip"));
        assert!(handle_clip_command("!CLIP", "clip"));
    }

    #[test]
    fn clip_command_matches_with_args() {
        assert!(handle_clip_command("!clip please", "clip"));
        assert!(handle_clip_command("!clip 30s", "clip"));
    }

    #[test]
    fn clip_command_rejects_non_match() {
        assert!(!handle_clip_command("nice clip!", "clip"));
        assert!(!handle_clip_command("!clips", "clip"));
        assert!(!handle_clip_command("!clipping", "clip"));
    }

    #[test]
    fn clip_command_custom_name() {
        assert!(handle_clip_command("!save", "save"));
        assert!(!handle_clip_command("!clip", "save"));
    }

    #[test]
    fn is_clip_command_delegates_to_config() {
        let config = AutoClipConfig {
            clip_command: "clip".to_owned(),
            ..Default::default()
        };
        assert!(is_clip_command("!clip", &config));
        assert!(!is_clip_command("!save", &config));
    }

    #[test]
    fn set_config_updates_threshold() {
        let mut det = SpikeDetector::new(AutoClipConfig::default());
        let new_config = AutoClipConfig {
            spike_threshold: 5,
            ..Default::default()
        };
        det.set_config(new_config);
        // After update, we need 5 messages not 15
        let t0 = Instant::now();
        for _ in 0..4 {
            det.record(t0);
        }
        assert!(!det.should_clip(t0));
        det.record(t0);
        assert!(det.should_clip(t0));
    }
}
