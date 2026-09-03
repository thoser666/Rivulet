//! Shared-memory capture channel for frame transfer from the Vulkan layer
//! to the Rivulet recording process.
//!
//! Protocol:
//! - Named shared memory region: `rivulet_capture_shm`
//! - Header (32 bytes): magic, width, height, format, sequence, data_size
//! - Frame data: RGBA pixels immediately after header
//!
//! The layer writes frames; Rivulet reads them.

use std::sync::atomic::{AtomicU64, Ordering};

/// Magic bytes identifying a valid frame: "RVL\0"
pub const SHM_MAGIC: u32 = 0x004C5652;

/// Shared memory region name (Windows: named file mapping, Linux: shm_open).
pub const SHM_NAME: &str = "rivulet_capture_shm";

/// Default shared memory size: header (32) + 1920×1080×4 (8,294,400) ≈ 8 MB.
pub const DEFAULT_SHM_SIZE: usize = 32 + 1920 * 1080 * 4;

/// Frame header — 32 bytes, stored at the start of the shared memory region.
///
/// All fields are stored as little-endian u32/u64 for cross-platform safety.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FrameHeader {
    /// Magic bytes: must be `SHM_MAGIC`.
    pub magic: u32,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Pixel format (0 = RGBA8).
    pub format: u32,
    /// Monotonically increasing frame sequence number.
    pub sequence: u64,
    /// Size of the frame data in bytes (width × height × 4).
    pub data_size: u32,
}

impl FrameHeader {
    pub const SIZE: usize = 32;

    pub fn new(width: u32, height: u32, sequence: u64) -> Self {
        // Checked multiplication: a hostile/corrupt extent (e.g. u32::MAX
        // from a broken swapchain) must wrap to None instead of overflowing
        // in release builds and writing a header whose data_size disagrees
        // with the geometry.
        let pixels = (width as u64)
            .checked_mul(height as u64)
            .and_then(|px| px.checked_mul(4))
            .and_then(|bytes| u32::try_from(bytes).ok());
        let data_size = pixels.expect("[Rivulet Layer] frame geometry overflows the SHM protocol");
        Self {
            magic: SHM_MAGIC,
            width,
            height,
            format: 0,
            sequence,
            data_size,
        }
    }

    /// Serialize the header to a byte slice (little-endian).
    pub fn to_bytes(&self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..8].copy_from_slice(&self.width.to_le_bytes());
        buf[8..12].copy_from_slice(&self.height.to_le_bytes());
        buf[12..16].copy_from_slice(&self.format.to_le_bytes());
        buf[16..24].copy_from_slice(&self.sequence.to_le_bytes());
        buf[24..28].copy_from_slice(&self.data_size.to_le_bytes());
        buf
    }

    /// Deserialize a header from a byte slice.
    pub fn from_bytes(buf: &[u8; Self::SIZE]) -> Option<Self> {
        let magic = u32::from_le_bytes(buf[0..4].try_into().ok()?);
        if magic != SHM_MAGIC {
            return None;
        }
        Some(Self {
            magic,
            width: u32::from_le_bytes(buf[4..8].try_into().ok()?),
            height: u32::from_le_bytes(buf[8..12].try_into().ok()?),
            format: u32::from_le_bytes(buf[12..16].try_into().ok()?),
            sequence: u64::from_le_bytes(buf[16..24].try_into().ok()?),
            data_size: u32::from_le_bytes(buf[24..28].try_into().ok()?),
        })
    }
}

/// Atomic sequence counter for generating unique frame IDs.
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Get the next frame sequence number.
pub fn next_sequence() -> u64 {
    SEQUENCE.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip() {
        let h = FrameHeader::new(1920, 1080, 42);
        let bytes = h.to_bytes();
        let h2 = FrameHeader::from_bytes(&bytes).unwrap();
        assert_eq!(h2.magic, SHM_MAGIC);
        assert_eq!(h2.width, 1920);
        assert_eq!(h2.height, 1080);
        assert_eq!(h2.format, 0);
        assert_eq!(h2.sequence, 42);
        assert_eq!(h2.data_size, 1920 * 1080 * 4);
    }

    #[test]
    fn header_invalid_magic_returns_none() {
        let mut buf = [0u8; FrameHeader::SIZE];
        buf[0..4].copy_from_slice(&999u32.to_le_bytes());
        assert!(FrameHeader::from_bytes(&buf).is_none());
    }

    #[test]
    fn header_size_is_32() {
        assert_eq!(FrameHeader::SIZE, 32);
    }

    #[test]
    fn sequence_increments() {
        let a = next_sequence();
        let b = next_sequence();
        assert!(b > a);
    }
}
