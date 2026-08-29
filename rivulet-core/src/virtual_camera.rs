use serde::{Deserialize, Serialize};
use std::fmt;

/// Supported frame formats for a virtual-camera output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VirtualCameraFormat {
    Rgba,
    #[default]
    Bgra,
    Nv12,
}

/// Configuration shared by platform-specific virtual-camera backends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualCameraConfig {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
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
    #[default]
    Stopped,
    Starting,
    Running,
    Stopping,
    Unavailable(String),
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VirtualCameraError {
    InvalidConfig(&'static str),
    InvalidTransition {
        state: VirtualCameraState,
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
    pub fn new(config: VirtualCameraConfig) -> Result<Self, VirtualCameraError> {
        config.validate()?;
        Ok(Self {
            config,
            state: VirtualCameraState::Stopped,
        })
    }

    pub fn config(&self) -> &VirtualCameraConfig {
        &self.config
    }
    pub fn state(&self) -> &VirtualCameraState {
        &self.state
    }

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

    pub fn mark_unavailable(&mut self, reason: impl Into<String>) {
        self.state = VirtualCameraState::Unavailable(reason.into());
    }

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
