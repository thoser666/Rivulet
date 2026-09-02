//! Sidechain audio ducking filter (M4, issue #79).
//!
//! Lowers music/crowd audio while the mic speaks. This module is the
//! deterministic policy model: it computes the gain to apply to the
//! program (music/crowd) channel from the mic's loudness, using
//! threshold/attenuation settings with bounded attack/release smoothing.
//!
//! The GStreamer-side application of this gain lives with the pipeline
//! integration; this module is pure and fully unit-testable.

/// Sidechain ducking configuration (issue #79 Definition of Done:
/// threshold/attenuation settings, tests + docs).
#[derive(Debug, Clone, PartialEq)]
pub struct DuckingConfig {
    /// Mic RMS level (dBFS) at or above which ducking engages.
    /// Must be negative (dBFS) and greater than `floor_db`.
    pub threshold_db: f64,
    /// Gain (dB) applied to the program channel while ducked.
    /// Must be negative or zero (attenuation only; never boosts).
    pub attenuation_db: f64,
    /// Mic RMS level below which ducking fully releases (dBFS).
    /// Must be <= `threshold_db`.
    pub floor_db: f64,
    /// Fraction of the remaining gain closed per duck frame (0..1].
    /// Higher = faster attack.
    pub attack: f64,
    /// Fraction of the remaining gain reopened per unducked frame (0..1].
    /// Higher = faster release.
    pub release: f64,
    /// Gain applied when fully released (dB). Typically 0.
    pub open_gain_db: f64,
}

impl Default for DuckingConfig {
    fn default() -> Self {
        Self {
            // Typical speech RMS sits around -30..-18 dBFS; -26 engages
            // ducking for normal speech while ignoring background noise.
            threshold_db: -26.0,
            // OBS-style ducking default: music drops by 12 dB.
            attenuation_db: -12.0,
            // Release fully once the mic is quiet.
            floor_db: -60.0,
            attack: 0.5,
            release: 0.25,
            open_gain_db: 0.0,
        }
    }
}

impl DuckingConfig {
    /// Validates the configuration, returning a human-readable error.
    pub fn validate(&self) -> Result<(), String> {
        if !self.threshold_db.is_finite() {
            return Err("threshold_db must be finite".to_string());
        }
        if !self.attenuation_db.is_finite() || self.attenuation_db > 0.0 {
            return Err("attenuation_db must be finite and <= 0 (attenuation only)".to_string());
        }
        if !self.floor_db.is_finite() || self.floor_db > self.threshold_db {
            return Err("floor_db must be finite and <= threshold_db".to_string());
        }
        for (name, v) in [
            ("attack", self.attack),
            ("release", self.release),
            ("open_gain_db", self.open_gain_db),
        ] {
            if !v.is_finite() {
                return Err(format!("{name} must be finite"));
            }
        }
        if !(0.0 < self.attack && self.attack <= 1.0) {
            return Err("attack must be in (0, 1]".to_string());
        }
        if !(0.0 < self.release && self.release <= 1.0) {
            return Err("release must be in (0, 1]".to_string());
        }
        Ok(())
    }
}

/// A sidechain ducking filter state machine.
///
/// Each `step` feeds one frame's mic RMS (dBFS) and returns the gain (dB)
/// to apply to the program (music/crowd) channel for that frame. The gain
/// moves smoothly between the ducked attenuation and the open gain using
/// bounded attack/release fractions, so the transition never clicks.
#[derive(Debug, Clone)]
pub struct DuckingFilter {
    config: DuckingConfig,
    current_gain_db: f64,
    ducked: bool,
}

impl DuckingFilter {
    /// Creates a filter, validating the configuration.
    pub fn new(config: DuckingConfig) -> Result<Self, String> {
        config.validate()?;
        Ok(Self {
            config,
            current_gain_db: 0.0,
            ducked: false,
        })
    }

    /// Creates a filter with default settings.
    pub fn with_defaults() -> Self {
        Self::new(DuckingConfig::default()).expect("default config is valid")
    }

    /// Current applied gain in dB.
    pub fn current_gain_db(&self) -> f64 {
        self.current_gain_db
    }

    /// Whether the filter is currently ducked.
    pub fn is_ducked(&self) -> bool {
        self.ducked
    }

    /// The configured target gain while ducked.
    pub fn ducked_target_db(&self) -> f64 {
        self.config.attenuation_db
    }

    /// Feeds one frame's mic RMS (dBFS) and returns the program-channel
    /// gain (dB) for that frame.
    ///
    /// Ducking engages when mic RMS >= threshold, releases when <= floor,
    /// and holds its previous state between floor and threshold (hysteresis,
    /// so a noisy mic does not stutter the gain).
    pub fn step(&mut self, mic_rms_db: f64) -> f64 {
        let target = if mic_rms_db >= self.config.threshold_db {
            self.ducked = true;
            self.config.attenuation_db
        } else if mic_rms_db <= self.config.floor_db {
            self.ducked = false;
            self.config.open_gain_db
        } else {
            // Between floor and threshold: hold previous state (hysteresis).
            return self.current_gain_db;
        };

        let rate = if self.ducked {
            self.config.attack
        } else {
            self.config.release
        };
        let remaining = target - self.current_gain_db;
        self.current_gain_db += remaining * rate;
        // Snap to the target once close enough to avoid an endless tail.
        if (target - self.current_gain_db).abs() < 0.05 {
            self.current_gain_db = target;
        }
        self.current_gain_db
    }

    /// Feeds a frame and returns the multiplied program samples.
    ///
    /// Applies the current gain (dB, linearized) to every sample. Returns
    /// a new vector; the input is not modified.
    pub fn apply(&mut self, mic_rms_db: f64, program: &[f32]) -> Vec<f32> {
        let gain_db = self.step(mic_rms_db);
        let linear = 10.0_f64.powf(gain_db / 20.0);
        program
            .iter()
            .map(|s| (*s as f64 * linear) as f32)
            .collect()
    }
}

/// Computes the RMS level (dBFS) of a frame of samples.
pub fn rms_db(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return f64::NEG_INFINITY;
    }
    let sum_squares: f64 = samples.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    let rms = (sum_squares / samples.len() as f64).sqrt();
    20.0 * rms.max(f64::EPSILON).log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        assert!(DuckingConfig::default().validate().is_ok());
    }

    #[test]
    fn rejects_positive_attenuation() {
        let config = DuckingConfig {
            attenuation_db: 3.0,
            ..DuckingConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_floor_above_threshold() {
        let config = DuckingConfig {
            floor_db: -20.0,
            ..DuckingConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_zero_attack_and_release() {
        let config = DuckingConfig {
            attack: 0.0,
            ..DuckingConfig::default()
        };
        assert!(config.validate().is_err());
        let config = DuckingConfig {
            release: 0.0,
            ..DuckingConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_non_finite_settings() {
        let config = DuckingConfig {
            threshold_db: f64::NAN,
            ..DuckingConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn loud_mic_ducks_toward_attenuation() {
        let mut filter = DuckingFilter::with_defaults();
        // Loud mic: -10 dBFS is well above the -26 dB threshold.
        let gain = filter.step(-10.0);
        assert!(gain < 0.0, "gain must attenuate while ducked, got {gain}");
        assert!(gain >= -12.0, "gain must not overshoot attenuation");
        assert!(filter.is_ducked());
    }

    #[test]
    fn quiet_mic_releases_toward_open_gain() {
        let mut filter = DuckingFilter::with_defaults();
        filter.step(-10.0); // duck
                            // Silent mic: below the -60 dB floor.
        for _ in 0..20 {
            filter.step(-80.0);
        }
        assert!(!filter.is_ducked());
        assert!((filter.current_gain_db() - 0.0).abs() < 0.05);
    }

    #[test]
    fn hysteresis_holds_state_between_floor_and_threshold() {
        let mut filter = DuckingFilter::with_defaults();
        filter.step(-10.0); // duck
        let ducked_gain = filter.current_gain_db();
        // Mic between floor (-60) and threshold (-26): hold state.
        let held = filter.step(-40.0);
        assert_eq!(held, ducked_gain);
        assert!(filter.is_ducked());
    }

    #[test]
    fn gain_converges_to_ducked_target() {
        let mut filter = DuckingFilter::with_defaults();
        for _ in 0..50 {
            filter.step(-10.0);
        }
        assert!((filter.current_gain_db() - (-12.0)).abs() < 0.05);
    }

    #[test]
    fn apply_scales_program_samples_by_linear_gain() {
        let mut filter = DuckingFilter::with_defaults();
        let program = vec![1.0_f32, 0.5, -0.5];
        let ducked = filter.apply(-10.0, &program);
        // After one attack step the gain is 0.5 * -12 dB = -6 dB.
        let linear = 10.0_f64.powf(-6.0 / 20.0);
        let expected = 1.0_f32 * linear as f32;
        assert!((ducked[0] - expected).abs() < 1e-4);
        // Never boosts above the input while ducked.
        assert!(ducked[0] <= program[0]);
    }

    #[test]
    fn rms_db_of_sine_is_reasonable() {
        // Full-scale sine RMS is -3.01 dBFS.
        let samples: Vec<f32> = (0..1000)
            .map(|i| (2.0 * std::f64::consts::PI * i as f64 / 1000.0).sin() as f32)
            .collect();
        let rms = rms_db(&samples);
        assert!((rms - (-3.01)).abs() < 0.1, "got {rms}");
    }

    #[test]
    fn rms_db_of_silence_is_very_quiet() {
        assert_eq!(rms_db(&[]), f64::NEG_INFINITY);
        // Digital silence clamps to EPSILON before log10, so the value is a
        // large negative number rather than exactly -inf.
        let rms = rms_db(&[0.0; 100]);
        assert!(rms < -120.0, "silence must read below -120 dBFS, got {rms}");
    }

    #[test]
    fn custom_threshold_and_attenuation_are_honored() {
        let config = DuckingConfig {
            threshold_db: -20.0,
            attenuation_db: -6.0,
            ..DuckingConfig::default()
        };
        let mut filter = DuckingFilter::new(config).unwrap();
        // -25 dBFS is below the -20 threshold: no ducking.
        let gain = filter.step(-25.0);
        assert!((gain - 0.0).abs() < 0.05);
        // -15 dBFS is above the -20 threshold: duck toward -6 dB.
        for _ in 0..50 {
            filter.step(-15.0);
        }
        assert!((filter.current_gain_db() - (-6.0)).abs() < 0.05);
    }
}
