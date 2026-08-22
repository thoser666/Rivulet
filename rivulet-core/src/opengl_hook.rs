//! G4 – OpenGL wglSwapBuffers hook (host side).
//!
//! This module provides the host-side logic for the OpenGL capture backend:
//! - DLL injection into the target process
//! - SHM-based frame reading (same protocol as the Vulkan capture layer)
//! - Backend status reporting
//!
//! The actual hook lives in `rivulet-opengl-hook-dll` (a cdylib that gets
//! injected into the game process).

#[cfg(target_os = "windows")]
use std::ffi::CString;
#[cfg(target_os = "windows")]
use std::path::PathBuf;

use crate::capture_channel::{CapturedFrame, FrameHeader, DEFAULT_SHM_SIZE};

/// Errors specific to the OpenGL hook backend.
#[derive(Debug, Clone)]
pub enum OpenGLHookError {
    /// The DLL could not be found next to the executable.
    DllNotFound(PathBuf),
    /// Failed to inject the DLL into the target process.
    InjectionFailed(String),
    /// Shared memory region is not available (DLL not loaded, no game writing yet).
    ShmNotAvailable,
    /// Frame data is corrupted or too large.
    FrameCorrupt(String),
}

impl std::fmt::Display for OpenGLHookError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenGLHookError::DllNotFound(path) => {
                write!(f, "OpenGL hook DLL not found: {}", path.display())
            }
            OpenGLHookError::InjectionFailed(msg) => {
                write!(f, "DLL injection failed: {msg}")
            }
            OpenGLHookError::ShmNotAvailable => {
                write!(f, "OpenGL capture SHM not available")
            }
            OpenGLHookError::FrameCorrupt(msg) => {
                write!(f, "Frame data corrupt: {msg}")
            }
        }
    }
}

impl std::error::Error for OpenGLHookError {}

/// Configuration for the OpenGL hook backend.
#[derive(Debug, Clone)]
pub struct OpenGLHookConfig {
    /// Target frames per second for the capture poll loop.
    pub fps: u32,
    /// Path to the injected DLL. If `None`, looks for
    /// `rivulet_opengl_hook_dll.dll` next to the current executable.
    pub dll_path: Option<PathBuf>,
}

impl Default for OpenGLHookConfig {
    fn default() -> Self {
        Self {
            fps: 60,
            dll_path: None,
        }
    }
}

/// Probe whether the OpenGL hook is available on this platform.
///
/// Returns `Ok(())` if the DLL exists and injection is likely to succeed,
/// or an error describing why it won't work.
pub fn probe_opengl_hook(config: &OpenGLHookConfig) -> Result<(), OpenGLHookError> {
    let dll_path = resolve_dll_path(config)?;
    if !dll_path.exists() {
        return Err(OpenGLHookError::DllNotFound(dll_path));
    }
    Ok(())
}

/// Resolve the path to the hook DLL.
fn resolve_dll_path(config: &OpenGLHookConfig) -> Result<PathBuf, OpenGLHookError> {
    if let Some(ref p) = config.dll_path {
        return Ok(p.clone());
    }

    // Look next to the current executable.
    let exe_dir = std::env::current_exe()
        .map_err(|e| OpenGLHookError::InjectionFailed(e.to_string()))?
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();

    Ok(exe_dir.join("rivulet_opengl_hook_dll.dll"))
}

// ---------------------------------------------------------------------------
// SHM frame reader (Windows only — uses Win32 file mapping)
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
pub struct ShmFrameReader {
    ptr: *const u8,
    size: usize,
}

#[cfg(target_os = "windows")]
unsafe impl Send for ShmFrameReader {}

#[cfg(target_os = "windows")]
impl ShmFrameReader {
    /// Open the shared memory region created by the injected DLL.
    ///
    /// Returns `None` if the region doesn't exist yet (DLL not loaded or
    /// no game writing).
    pub fn open() -> Option<Self> {
        use winapi::um::handleapi::CloseHandle;
        use winapi::um::memoryapi::{MapViewOfFile, FILE_MAP_READ};
        use winapi::um::winbase::OpenFileMappingA;

        let name = CString::new(crate::capture_channel::SHM_NAME).ok()?;
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
            Some(ShmFrameReader {
                ptr: ptr as *const u8,
                size: DEFAULT_SHM_SIZE,
            })
        }
    }

    /// Read the latest frame from shared memory.
    pub fn read_frame(&self) -> Option<CapturedFrame> {
        unsafe {
            let header_bytes = std::slice::from_raw_parts(self.ptr, FrameHeader::SIZE);
            let mut buf = [0u8; FrameHeader::SIZE];
            buf.copy_from_slice(header_bytes);

            let header = FrameHeader::from_bytes(&buf)?;
            let data_size = header.data_size as usize;
            if data_size > self.size - FrameHeader::SIZE {
                return None;
            }

            let pixels =
                std::slice::from_raw_parts(self.ptr.add(FrameHeader::SIZE), data_size).to_vec();

            Some(CapturedFrame {
                width: header.width,
                height: header.height,
                sequence: header.sequence,
                pixels,
            })
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for ShmFrameReader {
    fn drop(&mut self) {
        use winapi::um::memoryapi::UnmapViewOfFile;
        unsafe {
            UnmapViewOfFile(self.ptr as *const _);
        }
    }
}

// ---------------------------------------------------------------------------
// Non-Windows stubs
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "windows"))]
pub struct ShmFrameReader;

#[cfg(not(target_os = "windows"))]
impl ShmFrameReader {
    pub fn open() -> Option<Self> {
        None
    }

    pub fn read_frame(&self) -> Option<CapturedFrame> {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opengl_hook_error_display() {
        let e = OpenGLHookError::DllNotFound(PathBuf::from("/foo/bar.dll"));
        assert!(e.to_string().contains("bar.dll"));

        let e2 = OpenGLHookError::InjectionFailed("timeout".into());
        assert!(e2.to_string().contains("timeout"));

        let e3 = OpenGLHookError::ShmNotAvailable;
        assert!(e3.to_string().contains("SHM"));

        let e4 = OpenGLHookError::FrameCorrupt("bad magic".into());
        assert!(e4.to_string().contains("bad magic"));
    }

    #[test]
    fn default_config_has_sane_fps() {
        let cfg = OpenGLHookConfig::default();
        assert_eq!(cfg.fps, 60);
        assert!(cfg.dll_path.is_none());
    }

    #[test]
    fn resolve_dll_path_uses_config_override() {
        let cfg = OpenGLHookConfig {
            dll_path: Some(PathBuf::from("/custom/path.dll")),
            ..Default::default()
        };
        let p = resolve_dll_path(&cfg).unwrap();
        assert_eq!(p, PathBuf::from("/custom/path.dll"));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn resolve_dll_path_defaults_to_exe_dir() {
        let cfg = OpenGLHookConfig::default();
        let p = resolve_dll_path(&cfg).unwrap();
        assert!(p.file_name().unwrap() == "rivulet_opengl_hook_dll.dll");
    }

    #[test]
    fn shm_reader_not_available_on_non_windows() {
        #[cfg(not(target_os = "windows"))]
        {
            assert!(ShmFrameReader::open().is_none());
        }
    }
}
