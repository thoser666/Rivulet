#![warn(missing_docs)]
//! Screen capture backends and the frame type they hand out.
//!
//! [`CaptureSource`] is the shape every backend here implements:
//! `XCapScreenCapture` on every platform, `DxgiDesktopDuplication` on Windows
//! and the PipeWire portal handle on Linux. The backends behind `rivulet-core`'s
//! `CaptureBackend` trait build on these; frames arrive either as a
//! [`CapturedFrame`] in system memory or, on the DXGI path, as a shared GPU
//! handle in `GpuFrame`.
use anyhow::Result;

/// Platform-independent backend status / DXGI error classification (G2).
pub mod backend;
/// DXGI Desktop Duplication backend (G2) — Windows only.
#[cfg(target_os = "windows")]
pub mod dxgi;
/// PipeWire portal screen capture (G6) — Linux only.
#[cfg(target_os = "linux")]
pub mod pipewire_portal;
/// Monitor capture through xcap: the cross-platform full-screen path.
pub mod screen;

#[cfg(target_os = "windows")]
pub use dxgi::DxgiDesktopDuplication;
#[cfg(target_os = "linux")]
pub use pipewire_portal::{PipeWireCaptureHandle, PortalFrame, PortalStreamInfo};
pub use screen::XCapScreenCapture;

/// A screen source that yields RGBA frames on demand.
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

/// One frame in system memory, as the encoder consumes it.
#[derive(Debug, Clone)]
pub struct CapturedFrame {
    /// RGBA8 pixels, `stride` bytes per row.
    pub data: Vec<u8>,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Bytes per row. The backends in this crate fill it with `width * 4`, but
    /// a consumer should honour the field rather than assume it.
    pub stride: u32,
    /// When the frame was read out of the source.
    pub timestamp: std::time::Instant,
}

impl CapturedFrame {
    /// Build a frame from raw RGBA8 bytes, stamped with `Instant::now()`.
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
