use anyhow::Result;

/// Platform-independent backend status / DXGI error classification (G2).
pub mod backend;
/// DXGI Desktop Duplication backend (G2) — Windows only.
#[cfg(target_os = "windows")]
pub mod dxgi;
/// PipeWire portal screen capture (G6) — Linux only.
#[cfg(target_os = "linux")]
pub mod pipewire_portal;
pub mod screen;

#[cfg(target_os = "windows")]
pub use dxgi::DxgiDesktopDuplication;
#[cfg(target_os = "linux")]
pub use pipewire_portal::{PipeWireCaptureHandle, PortalFrame, PortalStreamInfo};
pub use screen::XCapScreenCapture;

pub trait CaptureSource {
    /// Start capturing
    fn start(&mut self) -> Result<()>;

    /// Stop capturing
    fn stop(&mut self) -> Result<()>;

    /// Get next frame as RGBA bytes
    fn capture_frame(&mut self) -> Result<Option<CapturedFrame>>;

    /// Get capture dimensions
    fn dimensions(&self) -> (u32, u32);

    /// Check if currently capturing
    fn is_capturing(&self) -> bool;
}

#[derive(Debug, Clone)]
pub struct CapturedFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub timestamp: std::time::Instant,
}

impl CapturedFrame {
    pub fn new(data: Vec<u8>, width: u32, height: u32, stride: u32) -> Self {
        Self {
            data,
            width,
            height,
            stride,
            timestamp: std::time::Instant::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captured_frame_stores_fields() {
        let data = vec![0u8; 16];
        let frame = CapturedFrame::new(data.clone(), 2, 2, 8);
        assert_eq!(frame.data, data);
        assert_eq!(frame.width, 2);
        assert_eq!(frame.height, 2);
        assert_eq!(frame.stride, 8);
    }
}
