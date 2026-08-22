//! Shared-memory capture channel reader for receiving frames from the
//! Vulkan capture layer.
//!
//! The layer writes frames to a named shared memory region; this module
//! reads them for the recording pipeline.

/// Frame header — 32 bytes, stored at the start of the shared memory region.
/// Must match the layout in `rivulet-vulkan-layer/src/capture_channel.rs`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FrameHeader {
    pub magic: u32,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub sequence: u64,
    pub data_size: u32,
}

impl FrameHeader {
    pub const SIZE: usize = 32;
    pub const MAGIC: u32 = 0x004C5652; // "RVL\0"

    pub fn from_bytes(buf: &[u8; Self::SIZE]) -> Option<Self> {
        let magic = u32::from_le_bytes(buf[0..4].try_into().ok()?);
        if magic != Self::MAGIC {
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

/// Shared memory region name — must match the layer's `SHM_NAME`.
pub const SHM_NAME: &str = "rivulet_capture_shm";

/// Default shared memory size — must match the layer's `DEFAULT_SHM_SIZE`.
pub const DEFAULT_SHM_SIZE: usize = 32 + 1920 * 1080 * 4;

/// A captured frame read from shared memory.
#[derive(Debug, Clone)]
pub struct CapturedFrame {
    pub width: u32,
    pub height: u32,
    pub sequence: u64,
    pub pixels: Vec<u8>,
}

/// Handle to the shared memory region for reading frames.
pub struct ShmReader {
    ptr: *const u8,
    #[allow(dead_code)]
    size: usize,
}

unsafe impl Send for ShmReader {}

impl ShmReader {
    /// Open the shared memory region for reading.
    ///
    /// Returns `None` if the region doesn't exist or can't be mapped.
    pub fn open() -> Option<Self> {
        open_shm().map(|(ptr, size)| Self { ptr, size })
    }

    /// Read the latest frame from shared memory.
    ///
    /// Returns `None` if the header is invalid or the region is too small.
    pub fn read_frame(&self) -> Option<CapturedFrame> {
        unsafe {
            let header_buf = &*(self.ptr as *const [u8; FrameHeader::SIZE]);
            let header = FrameHeader::from_bytes(header_buf)?;

            let data_offset = FrameHeader::SIZE;
            let data_len = header.data_size as usize;
            if data_offset + data_len > self.size {
                return None;
            }

            let pixels = std::slice::from_raw_parts(self.ptr.add(data_offset), data_len).to_vec();

            Some(CapturedFrame {
                width: header.width,
                height: header.height,
                sequence: header.sequence,
                pixels,
            })
        }
    }
}

impl Drop for ShmReader {
    fn drop(&mut self) {
        close_shm(self.ptr as *mut u8, self.size);
    }
}

/// Platform-specific shared memory opening.
#[cfg(target_os = "windows")]
fn open_shm() -> Option<(*const u8, usize)> {
    use std::ffi::CString;
    use winapi::um::handleapi::*;
    use winapi::um::memoryapi::*;
    use winapi::um::winbase::*;

    let name = CString::new(SHM_NAME).ok()?;

    unsafe {
        let handle = OpenFileMappingA(FILE_MAP_READ, 0, name.as_ptr());
        if handle.is_null() {
            return None;
        }

        let ptr = MapViewOfFile(handle, FILE_MAP_READ, 0, 0, 0);
        CloseHandle(handle);
        if ptr.is_null() {
            return None;
        }

        Some((ptr as *const u8, DEFAULT_SHM_SIZE))
    }
}

#[cfg(target_os = "linux")]
fn open_shm() -> Option<(*const u8, usize)> {
    use std::ffi::CString;

    let name = CString::new(format!("/{}", SHM_NAME)).ok()?;

    unsafe {
        let fd = libc::shm_open(name.as_ptr(), libc::O_RDONLY, 0);
        if fd < 0 {
            return None;
        }

        let ptr = libc::mmap(
            std::ptr::null_mut(),
            DEFAULT_SHM_SIZE,
            libc::PROT_READ,
            libc::MAP_SHARED,
            fd,
            0,
        );
        libc::close(fd);
        if ptr == libc::MAP_FAILED {
            return None;
        }

        Some((ptr as *const u8, DEFAULT_SHM_SIZE))
    }
}

/// The Vulkan layer is not shipped on other platforms (macOS etc.), so the
/// shared-memory channel is simply unavailable there.
#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn open_shm() -> Option<(*const u8, usize)> {
    None
}

/// Platform-specific shared memory cleanup.
#[cfg(target_os = "windows")]
fn close_shm(_ptr: *mut u8, _size: usize) {
    // On Windows, UnmapViewOfFile is needed but the handle is already closed.
    // For simplicity, we rely on process exit for cleanup.
    // A production implementation would track the mapping handle.
}

#[cfg(target_os = "linux")]
fn close_shm(ptr: *mut u8, size: usize) {
    unsafe {
        libc::munmap(ptr as *mut libc::c_void, size);
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn close_shm(_ptr: *mut u8, _size: usize) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip() {
        let header = FrameHeader {
            magic: FrameHeader::MAGIC,
            width: 1920,
            height: 1080,
            format: 0,
            sequence: 42,
            data_size: 1920 * 1080 * 4,
        };
        let mut buf = [0u8; FrameHeader::SIZE];
        buf[0..4].copy_from_slice(&header.magic.to_le_bytes());
        buf[4..8].copy_from_slice(&header.width.to_le_bytes());
        buf[8..12].copy_from_slice(&header.height.to_le_bytes());
        buf[12..16].copy_from_slice(&header.format.to_le_bytes());
        buf[16..24].copy_from_slice(&header.sequence.to_le_bytes());
        buf[24..28].copy_from_slice(&header.data_size.to_le_bytes());

        let parsed = FrameHeader::from_bytes(&buf).unwrap();
        assert_eq!(parsed.width, 1920);
        assert_eq!(parsed.height, 1080);
        assert_eq!(parsed.sequence, 42);
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
    fn shm_name_matches_layer() {
        assert_eq!(SHM_NAME, "rivulet_capture_shm");
    }

    #[test]
    fn shm_size_matches_layer() {
        assert_eq!(DEFAULT_SHM_SIZE, 32 + 1920 * 1080 * 4);
    }
}
