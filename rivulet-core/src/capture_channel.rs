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

    /// The only pixel format the protocol defines (RGBA8 = 4 bytes/px).
    pub const FORMAT_RGBA8: u32 = 0;

    /// Largest single allocation the reader accepts for pixel data.
    /// 8 MiB covers 1920×1080×4 with headroom for 1440p; anything above
    /// comes from a corrupted or hostile header, not the shipped writer.
    pub const MAX_PIXEL_BYTES: usize = 16 * 1024 * 1024;

    /// Plausibility check for a header read out of shared memory.
    ///
    /// Shared memory is writable by every process in the session — and the
    /// writer runs inside the captured game, an untrusted host. The magic
    /// check alone would let a forged header drive unbounded allocation or
    /// geometry that disagrees with the data length, so `read_frame` must
    /// also reject:
    /// - zero or absurd geometry (width/height),
    /// - unknown pixel formats,
    /// - `data_size` != width × height × 4 (including multiplication
    ///   overflow),
    /// - pixel payloads above [`Self::MAX_PIXEL_BYTES`].
    ///
    /// `mapped_size` is the true size of the mapping; the header is
    /// additionally rejected when its data would read past the end.
    pub fn is_plausible(&self, mapped_size: usize) -> bool {
        if self.width == 0 || self.height == 0 {
            return false;
        }
        if self.width > 16384 || self.height > 16384 {
            return false;
        }
        if self.format != Self::FORMAT_RGBA8 {
            return false;
        }
        // u64 checked math: geometry is already capped at ≤16384² above, so
        // this cannot overflow — but the cap and the check must not depend on
        // each other's order to stay correct.
        let expected = match (self.width as u64).checked_mul(self.height as u64) {
            Some(px) => match px.checked_mul(4) {
                Some(bytes) => bytes as usize,
                None => return false,
            },
            None => return false,
        };
        if expected != self.data_size as usize {
            return false;
        }
        if expected > Self::MAX_PIXEL_BYTES {
            return false;
        }
        Self::SIZE + self.data_size as usize <= mapped_size
    }

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

    /// The size of the mapping this reader was opened with.
    pub fn mapped_size(&self) -> usize {
        self.size
    }

    /// Read the latest frame from shared memory.
    ///
    /// Returns `None` when the header is implausible (see
    /// [`FrameHeader::is_plausible`]) or the data would exceed the mapping.
    /// The pixels are copied out of shared memory immediately, so the caller
    /// works on a private, fully validated snapshot.
    pub fn read_frame(&self) -> Option<CapturedFrame> {
        unsafe {
            let header_buf = &*(self.ptr as *const [u8; FrameHeader::SIZE]);
            let header = FrameHeader::from_bytes(header_buf)?;
            if !header.is_plausible(self.size) {
                return None;
            }

            let data_offset = FrameHeader::SIZE;
            let data_len = header.data_size as usize;

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
/// True mapping size of the view at `ptr` (Windows: VirtualQuery region size).
/// The creator may map fewer bytes than DEFAULT_SHM_SIZE, so readers must
/// trust the queried size, never the constant, when bounding pixel reads.
#[cfg(target_os = "windows")]
fn mapped_region_size(ptr: *const u8) -> usize {
    use winapi::um::memoryapi::VirtualQuery;
    use winapi::um::winnt::MEMORY_BASIC_INFORMATION;

    unsafe {
        let mut info: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
        let queried = VirtualQuery(
            ptr as *const _,
            &mut info,
            std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        );
        if queried == 0 {
            return 0;
        }
        info.RegionSize
    }
}

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

        let size = mapped_region_size(ptr as *const u8);
        if size < FrameHeader::SIZE {
            UnmapViewOfFile(ptr);
            return None;
        }
        Some((ptr as *const u8, size))
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

        // The region may be smaller than DEFAULT_SHM_SIZE if a foreign or
        // older process created it — map min(object size, DEFAULT_SHM_SIZE)
        // so all reads stay inside the object.
        let mut stat: libc::stat = std::mem::zeroed();
        let object_size = if libc::fstat(fd, &mut stat) == 0 && stat.st_size > 0 {
            (stat.st_size as usize).min(DEFAULT_SHM_SIZE)
        } else {
            DEFAULT_SHM_SIZE
        };
        if object_size < FrameHeader::SIZE {
            libc::close(fd);
            return None;
        }

        let ptr = libc::mmap(
            std::ptr::null_mut(),
            object_size,
            libc::PROT_READ,
            libc::MAP_SHARED,
            fd,
            0,
        );
        libc::close(fd);
        if ptr == libc::MAP_FAILED {
            return None;
        }

        Some((ptr as *const u8, object_size))
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
fn close_shm(ptr: *mut u8, _size: usize) {
    use winapi::um::memoryapi::UnmapViewOfFile;
    // The mapping handle is closed right after MapViewOfFile (the view keeps
    // it alive); unmapping the view releases the last reference.
    unsafe {
        UnmapViewOfFile(ptr as *const _);
    }
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

    /// A header exactly as the shipped Vulkan layer writes it.
    #[allow(dead_code)]
    fn plausible_header(width: u32, height: u32) -> FrameHeader {
        FrameHeader {
            magic: FrameHeader::MAGIC,
            width,
            height,
            format: FrameHeader::FORMAT_RGBA8,
            sequence: 1,
            data_size: ((width as u128) * (height as u128) * 4) as u32,
        }
    }

    #[test]
    fn plausible_header_accepts_a_normal_frame() {
        let header = FrameHeader {
            magic: FrameHeader::MAGIC,
            width: 1920,
            height: 1080,
            format: FrameHeader::FORMAT_RGBA8,
            sequence: 7,
            data_size: 1920 * 1080 * 4,
        };
        assert!(header.is_plausible(DEFAULT_SHM_SIZE));
    }

    #[test]
    fn implausible_headers_are_rejected() {
        let mapped = DEFAULT_SHM_SIZE;
        // Zero geometry.
        assert!(!plausible_header(0, 1080).is_plausible(mapped));
        assert!(!plausible_header(1920, 0).is_plausible(mapped));
        // Absurd geometry (would overflow data_size math downstream).
        assert!(!plausible_header(u32::MAX, u32::MAX).is_plausible(mapped));
        // Unknown pixel format.
        let mut header = plausible_header(64, 64);
        header.format = 99;
        assert!(!header.is_plausible(mapped));
        // data_size disagrees with geometry.
        let mut header = plausible_header(64, 64);
        header.data_size = 1234;
        assert!(!header.is_plausible(mapped));
        // data_size above the allocation cap.
        let mut header = plausible_header(2048, 2048);
        header.data_size = (FrameHeader::MAX_PIXEL_BYTES * 2) as u32;
        assert!(!header.is_plausible(mapped));
        // Data extends past the mapping.
        let header = plausible_header(1920, 1080);
        assert!(!header.is_plausible(FrameHeader::SIZE + 16));
    }

    #[test]
    fn geometry_overflow_does_not_trick_the_check() {
        // width × height × 4 overflows u32 for these extents; the header
        // claims a small data_size, which must not match the (overflowing)
        // expected value.
        let mut header = plausible_header(65536, 65536);
        header.data_size = 4096;
        assert!(!header.is_plausible(usize::MAX));
    }

    #[test]
    fn max_pixel_cap_admits_1440p_and_rejects_16k() {
        let qhd = FrameHeader {
            magic: FrameHeader::MAGIC,
            width: 2560,
            height: 1440,
            format: FrameHeader::FORMAT_RGBA8,
            sequence: 1,
            data_size: 2560 * 1440 * 4,
        };
        // With a mapping that actually fits a 1440p frame (the default 8 MiB
        // mapping only fits 1080p), the header must pass.
        assert!(qhd.is_plausible(FrameHeader::SIZE + 2560 * 1440 * 4));
        // Against the default mapping it must be rejected — data beyond the
        // mapping end is unreadable, not merely "big".
        assert!(!qhd.is_plausible(DEFAULT_SHM_SIZE));
        // 16k×16k fits the cap math only in u64; the cap itself rejects it.
        let huge = FrameHeader {
            magic: FrameHeader::MAGIC,
            width: 16384,
            height: 16384,
            format: FrameHeader::FORMAT_RGBA8,
            sequence: 1,
            data_size: (16384u64 * 16384 * 4) as u32,
        };
        assert!(!huge.is_plausible(usize::MAX));
    }
}
