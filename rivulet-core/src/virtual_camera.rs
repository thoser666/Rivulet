use serde::{Deserialize, Serialize};
use std::fmt;

/// Supported frame formats for a virtual-camera output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VirtualCameraFormat {
    /// Packed 8-bit RGBA, the engine's appsrc layout.
    Rgba,
    /// Packed 8-bit BGRA; the default, and what camera consumers expect.
    #[default]
    Bgra,
    /// NV12: a full-size luma plane plus interleaved 4:2:0 chroma.
    Nv12,
}

/// Configuration shared by platform-specific virtual-camera backends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualCameraConfig {
    /// Device name other applications see; must not be blank.
    pub name: String,
    /// Frame width in pixels; non-zero, at most 7680.
    pub width: u32,
    /// Frame height in pixels; non-zero, at most 4320.
    pub height: u32,
    /// Target frame rate; validated to the range 1–240.
    pub fps: u32,
    /// Pixel format of the frames the backend is fed.
    pub format: VirtualCameraFormat,
}

impl Default for VirtualCameraConfig {
    fn default() -> Self {
        Self {
            name: "Rivulet Camera".into(),
            width: 1920,
            height: 1080,
            fps: 30,
            format: VirtualCameraFormat::default(),
        }
    }
}

impl VirtualCameraConfig {
    /// Check the config against the limits every backend shares: a non-empty
    /// name, non-zero dimensions up to 8K (7680×4320) and 1–240 fps.
    ///
    /// [`VirtualCamera::new`] calls this, so a controller can never hold an
    /// invalid config; backends that create the device themselves call it again
    /// because the config can outlive the controller.
    pub fn validate(&self) -> Result<(), VirtualCameraError> {
        if self.name.trim().is_empty() {
            return Err(VirtualCameraError::InvalidConfig("name must not be empty"));
        }
        if self.width == 0 || self.height == 0 {
            return Err(VirtualCameraError::InvalidConfig(
                "dimensions must be non-zero",
            ));
        }
        if self.width > 7680 || self.height > 4320 {
            return Err(VirtualCameraError::InvalidConfig("dimensions exceed 8K"));
        }
        if self.fps == 0 || self.fps > 240 {
            return Err(VirtualCameraError::InvalidConfig(
                "fps must be between 1 and 240",
            ));
        }
        Ok(())
    }
}

/// Lifecycle state of a virtual-camera output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VirtualCameraState {
    /// Not producing frames. The only state `start` accepts.
    #[default]
    Stopped,
    /// The backend driver call is in flight; `mark_running` continues from here.
    Starting,
    /// Producing frames; `stop` is allowed.
    Running,
    /// Shutdown is in flight; `mark_stopped` continues from here.
    Stopping,
    /// No backend can provide a camera on this machine; the string is the
    /// machine-readable reason.
    Unavailable(String),
    /// The backend failed; the string is its error message.
    Error(String),
}

/// Why a virtual-camera operation was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VirtualCameraError {
    /// The config violates a [`VirtualCameraConfig::validate`] limit; the string
    /// names the rule that failed.
    InvalidConfig(&'static str),
    /// The action is not legal from the current state.
    InvalidTransition {
        /// State the controller was in when the action arrived.
        state: VirtualCameraState,
        /// The action that was requested, e.g. `"start"`.
        action: &'static str,
    },
}

impl fmt::Display for VirtualCameraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig(message) => {
                write!(f, "invalid virtual-camera configuration: {message}")
            }
            Self::InvalidTransition { state, action } => {
                write!(f, "cannot {action} virtual camera from state {state:?}")
            }
        }
    }
}

impl std::error::Error for VirtualCameraError {}

/// Platform-neutral lifecycle controller. Platform backends can use the
/// `Starting`/`Stopping` states around their driver calls and transition to
/// `Running` only after the consumer is ready.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualCamera {
    config: VirtualCameraConfig,
    state: VirtualCameraState,
}

impl VirtualCamera {
    /// A controller in [`VirtualCameraState::Stopped`], after validating
    /// `config`.
    pub fn new(config: VirtualCameraConfig) -> Result<Self, VirtualCameraError> {
        config.validate()?;
        Ok(Self {
            config,
            state: VirtualCameraState::Stopped,
        })
    }

    /// The configuration this controller was built with.
    pub fn config(&self) -> &VirtualCameraConfig {
        &self.config
    }
    /// The current lifecycle state.
    pub fn state(&self) -> &VirtualCameraState {
        &self.state
    }

    /// Begin starting up: `Stopped` → `Starting`.
    ///
    /// Validates the config again and rejects every other state, so a double
    /// `start` cannot silently reset a running camera.
    pub fn start(&mut self) -> Result<(), VirtualCameraError> {
        self.config.validate()?;
        if !matches!(self.state, VirtualCameraState::Stopped) {
            return Err(VirtualCameraError::InvalidTransition {
                state: self.state.clone(),
                action: "start",
            });
        }
        self.state = VirtualCameraState::Starting;
        Ok(())
    }

    /// Finish starting up: `Starting` → `Running`.
    ///
    /// The backend calls this once its consumer is ready; every other state is
    /// rejected.
    pub fn mark_running(&mut self) -> Result<(), VirtualCameraError> {
        if self.state != VirtualCameraState::Starting {
            return Err(VirtualCameraError::InvalidTransition {
                state: self.state.clone(),
                action: "mark running",
            });
        }
        self.state = VirtualCameraState::Running;
        Ok(())
    }

    /// Begin shutting down: `Running` → `Stopping`. Rejects every other state.
    pub fn stop(&mut self) -> Result<(), VirtualCameraError> {
        if self.state != VirtualCameraState::Running {
            return Err(VirtualCameraError::InvalidTransition {
                state: self.state.clone(),
                action: "stop",
            });
        }
        self.state = VirtualCameraState::Stopping;
        Ok(())
    }

    /// Finish shutting down: `Stopping` → `Stopped`, once the backend released
    /// the device. Rejects every other state.
    pub fn mark_stopped(&mut self) -> Result<(), VirtualCameraError> {
        if self.state != VirtualCameraState::Stopping {
            return Err(VirtualCameraError::InvalidTransition {
                state: self.state.clone(),
                action: "mark stopped",
            });
        }
        self.state = VirtualCameraState::Stopped;
        Ok(())
    }

    /// Record that no backend can provide a camera here, with the
    /// machine-readable `reason`.
    ///
    /// This leaves the normal sequence: the state becomes
    /// [`VirtualCameraState::Unavailable`], so `start` no longer applies until a
    /// new controller is built.
    pub fn mark_unavailable(&mut self, reason: impl Into<String>) {
        self.state = VirtualCameraState::Unavailable(reason.into());
    }

    /// Record a backend failure. Like [`VirtualCamera::mark_unavailable`] this
    /// jumps out of the normal sequence instead of advancing it.
    pub fn mark_error(&mut self, reason: impl Into<String>) {
        self.state = VirtualCameraState::Error(reason.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_stopped() {
        let camera = VirtualCamera::new(VirtualCameraConfig::default()).unwrap();
        assert_eq!(camera.state(), &VirtualCameraState::Stopped);
        assert_eq!(camera.config().fps, 30);
    }

    #[test]
    fn lifecycle_requires_backend_acknowledgements() {
        let mut camera = VirtualCamera::new(VirtualCameraConfig::default()).unwrap();
        camera.start().unwrap();
        assert_eq!(camera.state(), &VirtualCameraState::Starting);
        camera.mark_running().unwrap();
        camera.stop().unwrap();
        camera.mark_stopped().unwrap();
        assert_eq!(camera.state(), &VirtualCameraState::Stopped);
    }

    #[test]
    fn repeated_start_and_stop_are_rejected() {
        let mut camera = VirtualCamera::new(VirtualCameraConfig::default()).unwrap();
        assert!(camera.start().is_ok());
        assert!(camera.start().is_err());
        camera.mark_running().unwrap();
        assert!(camera.stop().is_ok());
        assert!(camera.stop().is_err());
    }

    #[test]
    fn invalid_config_is_rejected() {
        let config = VirtualCameraConfig {
            width: 0,
            ..Default::default()
        };
        assert!(matches!(
            VirtualCamera::new(config),
            Err(VirtualCameraError::InvalidConfig(_))
        ));
        let config = VirtualCameraConfig {
            fps: 241,
            ..Default::default()
        };
        assert!(VirtualCamera::new(config).is_err());
    }

    #[test]
    fn unavailable_and_error_states_keep_reason() {
        let mut camera = VirtualCamera::new(VirtualCameraConfig::default()).unwrap();
        camera.mark_unavailable("driver missing");
        assert_eq!(
            camera.state(),
            &VirtualCameraState::Unavailable("driver missing".into())
        );
        camera.mark_error("permission denied");
        assert_eq!(
            camera.state(),
            &VirtualCameraState::Error("permission denied".into())
        );
    }
}
