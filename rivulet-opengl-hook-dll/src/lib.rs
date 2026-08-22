//! G4 – OpenGL wglSwapBuffers hook DLL.
//!
//! This crate is compiled as a cdylib (`.dll`) and injected into an OpenGL
//! game process. It intercepts `wglSwapBuffers` to read the back buffer and
//! write it to a named shared-memory region for Rivulet to consume.
//!
//! # SHM Protocol
//!
//! Identical to the Vulkan capture layer (`rivulet-core/src/capture_channel.rs`):
//! a 32-byte [`FrameHeader`] followed by raw BGRA pixel data.

use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};

use winapi::shared::minwindef::{BOOL, DWORD, HMODULE, LPVOID};
use winapi::shared::windef::HDC;
use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
use winapi::um::memoryapi::{MapViewOfFile, FILE_MAP_WRITE};
use winapi::um::winbase::CreateFileMappingA;
use winapi::um::winnt::HANDLE;
use winapi::um::winnt::PAGE_READWRITE;

// ---------------------------------------------------------------------------
// SHM protocol — must match rivulet-core/src/capture_channel.rs
// ---------------------------------------------------------------------------

const SHM_NAME: &str = "rivulet_capture_shm";
const SHM_MAGIC: u32 = 0x004C5652; // "RVL\0" — must match FrameHeader::MAGIC
const DEFAULT_SHM_SIZE: usize = 32 + 1920 * 1080 * 4;

#[repr(C)]
#[derive(Clone, Copy)]
struct FrameHeader {
    magic: u32,
    width: u32,
    height: u32,
    format: u32, // 0 = BGRA8
    sequence: u64,
    data_size: u32,
}

impl FrameHeader {
    const SIZE: usize = 32;

    fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut buf = [0u8; Self::SIZE];
        buf[0..4].copy_from_slice(&self.magic.to_le_bytes());
        buf[4..8].copy_from_slice(&self.width.to_le_bytes());
        buf[8..12].copy_from_slice(&self.height.to_le_bytes());
        buf[12..16].copy_from_slice(&self.format.to_le_bytes());
        buf[16..24].copy_from_slice(&self.sequence.to_le_bytes());
        buf[24..28].copy_from_slice(&self.data_size.to_le_bytes());
        buf
    }
}

// ---------------------------------------------------------------------------
// Shared memory writer
// ---------------------------------------------------------------------------

struct ShmWriter {
    ptr: *mut u8,
    size: usize,
}

unsafe impl Send for ShmWriter {}

static SHM: std::sync::Mutex<Option<ShmWriter>> = std::sync::Mutex::new(None);
static SEQUENCE: AtomicU64 = AtomicU64::new(1);
static HOOKED: AtomicBool = AtomicBool::new(false);

/// Ensure the shared memory region exists. Returns `true` on success.
///
/// After calling this, the SHM static is initialized and safe to read
/// via `SHM_PTR` / `SHM_SIZE`.
fn ensure_shm() -> bool {
    {
        let guard = SHM.lock().ok().unwrap();
        if guard.is_some() {
            return true;
        }
    }

    let name = std::ffi::CString::new(SHM_NAME).ok().unwrap();
    let size = DEFAULT_SHM_SIZE as u64;

    unsafe {
        let handle = CreateFileMappingA(
            INVALID_HANDLE_VALUE,
            std::ptr::null_mut(),
            PAGE_READWRITE,
            (size >> 32) as DWORD,
            size as DWORD,
            name.as_ptr(),
        );
        if handle.is_null() {
            return false;
        }

        let ptr = MapViewOfFile(handle, FILE_MAP_WRITE, 0, 0, 0);
        if ptr.is_null() {
            CloseHandle(handle);
            return false;
        }

        // Leak the handle — mapping stays alive for the process lifetime.
        let _ = handle;

        let writer = ShmWriter {
            ptr: ptr as *mut u8,
            size: DEFAULT_SHM_SIZE,
        };
        let mut guard = SHM.lock().ok().unwrap();
        guard.replace(writer);
        true
    }
}

/// Write a BGRA frame to shared memory.
fn write_frame(bgra: &[u8], width: u32, height: u32) {
    if !ensure_shm() {
        return;
    }

    let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let header = FrameHeader {
        magic: SHM_MAGIC,
        width,
        height,
        format: 0,
        sequence: seq,
        data_size: bgra.len() as u32,
    };

    let header_bytes = header.to_bytes();

    // ensure_shm() guarantees the SHM is initialized.
    let (ptr, size) = {
        let guard = SHM.lock().ok().unwrap();
        let writer = guard.as_ref().unwrap();
        (writer.ptr, writer.size)
    };

    let total = FrameHeader::SIZE + bgra.len();
    let copy_len = total.min(size);

    unsafe {
        std::ptr::copy_nonoverlapping(header_bytes.as_ptr(), ptr, FrameHeader::SIZE);
        let data_len = (copy_len - FrameHeader::SIZE).min(bgra.len());
        std::ptr::copy_nonoverlapping(bgra.as_ptr(), ptr.add(FrameHeader::SIZE), data_len);
    }
}

// ---------------------------------------------------------------------------
// wglSwapBuffers hook
// ---------------------------------------------------------------------------

type WglSwapBuffersFn = unsafe extern "system" fn(HDC, HANDLE) -> BOOL;

static ORIGINAL: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());

/// Our hooked `wglSwapBuffers` — captures the back buffer then forwards.
///
/// # Safety
/// Called by the OpenGL driver when the game presents a frame.
#[no_mangle]
pub unsafe extern "system" fn rivulet_wgl_swap_buffers(hdc: HDC, _reserved: HANDLE) -> BOOL {
    capture_current_frame(hdc);

    let orig: WglSwapBuffersFn = std::mem::transmute(ORIGINAL.load(Ordering::Relaxed));
    orig(hdc, std::ptr::null_mut())
}

/// Capture the current OpenGL back buffer via GDI GetDIBits.
unsafe fn capture_current_frame(hdc: HDC) {
    use winapi::um::wingdi::*;

    let hdc_mem = CreateCompatibleDC(hdc);
    if hdc_mem.is_null() {
        return;
    }

    let width = GetDeviceCaps(hdc, HORZRES);
    let height = GetDeviceCaps(hdc, VERTRES);
    if width <= 0 || height <= 0 {
        DeleteDC(hdc_mem);
        return;
    }

    let bmi = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as DWORD,
        biWidth: width,
        biHeight: -height, // top-down
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        ..std::mem::zeroed()
    };

    let mut buf = vec![0u8; width as usize * height as usize * 4];
    let result = GetDIBits(
        hdc,
        std::ptr::null_mut(),
        0,
        height as u32,
        buf.as_mut_ptr() as LPVOID,
        &bmi as *const BITMAPINFOHEADER as *mut BITMAPINFO,
        DIB_RGB_COLORS,
    );

    DeleteDC(hdc_mem);

    if result != 0 {
        write_frame(&buf, width as u32, height as u32);
    }
}

// ---------------------------------------------------------------------------
// DLL entry — install the hook
// ---------------------------------------------------------------------------

/// # Safety
/// Called by the OS when the DLL is loaded.
#[no_mangle]
pub unsafe extern "system" fn DllMain(_hinst: HMODULE, reason: DWORD, _reserved: LPVOID) -> BOOL {
    // DLL_PROCESS_ATTACH = 1
    if reason == 1 && !HOOKED.load(Ordering::Relaxed) {
        HOOKED.store(true, Ordering::Relaxed);
        install_hook();
    }
    1
}

/// Resolve the original `wglSwapBuffers` and store it for forwarding.
unsafe fn install_hook() {
    use winapi::um::libloaderapi::{GetModuleHandleA, GetProcAddress};

    let opengl32 = GetModuleHandleA(c"opengl32.dll".as_ptr());
    if opengl32.is_null() {
        return;
    }

    let original = GetProcAddress(opengl32, c"wglSwapBuffers".as_ptr());
    if !original.is_null() {
        ORIGINAL.store(original as *mut (), Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_header_roundtrip() {
        let h = FrameHeader {
            magic: SHM_MAGIC,
            width: 1920,
            height: 1080,
            format: 0,
            sequence: 42,
            data_size: 1920 * 1080 * 4,
        };
        let bytes = h.to_bytes();
        assert_eq!(bytes[0..4], SHM_MAGIC.to_le_bytes());
        assert_eq!(bytes[4..8], 1920u32.to_le_bytes());
        assert_eq!(bytes[8..12], 1080u32.to_le_bytes());
        assert_eq!(bytes[16..24], 42u64.to_le_bytes());
    }

    #[test]
    fn shm_name_matches_capture_channel() {
        assert_eq!(SHM_NAME, "rivulet_capture_shm");
    }

    #[test]
    fn hook_flag_starts_false() {
        assert!(!HOOKED.load(Ordering::Relaxed));
    }
}
