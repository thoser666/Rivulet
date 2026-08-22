//! DXGI Desktop Duplication backend (G2).
//!
//! Implements the zero-overhead fullscreen capture path recommended in
//! [docs/game-capture-strategy.md]: `IDXGIOutputDuplication` acquires the
//! desktop backbuffer as a GPU texture with no CPU copy in the hot path.
//! Frames are handed to the encoder as shared GPU textures (zero-copy); an
//! explicit CPU readback exists only for the compatibility RGBA path.
//!
//! Abort/fallback rules from the strategy (§6) are honoured:
//! - `DXGI_ERROR_ACCESS_LOST` (mode switch / lock / RDP) → re-create the
//!   duplication instead of silently producing nothing.
//! - `DXGI_ERROR_ACCESS_DENIED` (protected content) → explicit "not
//!   capturable" status, never a black frame.
//! - `DXGI_ERROR_UNSUPPORTED` → explicit "unsupported" status.
//! - Timeout (`DXGI_ERROR_WAIT_TIMEOUT`) → `Ok(None)` (no new frame).
//!
//! Windows-only module: gated via `#[cfg(target_os = "windows")]` in
//! `lib.rs`, so the workspace still builds on Linux/macOS (CI).

use anyhow::{bail, Context, Result};
use windows::core::Interface;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_10_0,
    D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_9_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
    D3D11_CREATE_DEVICE_FLAG, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_RESOURCE_MISC_SHARED,
    D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1,
    IDXGIOutputDuplication, IDXGIResource, DXGI_OUTDUPL_FRAME_INFO,
};

use crate::backend::{BackendKind, BackendStatus, DxgiFailure};
use crate::{CaptureSource, CapturedFrame};

/// A captured GPU frame, handed to the encoder via a shared texture handle
/// (zero-copy: no CPU readback happened).
#[derive(Debug, Clone, Copy)]
pub struct GpuFrame {
    /// Shared handle of the owned shared texture containing the frame. The
    /// encoder opens it with `ID3D11Device::OpenSharedResource`.
    pub shared_handle: HANDLE,
    pub width: u32,
    pub height: u32,
    /// Whether the frame contained protected content that was masked out.
    pub protected_masked: bool,
}

/// Metadata about a monitor/output, used for output selection in the GUI.
#[derive(Debug, Clone)]
pub struct DxgiOutputInfo {
    pub adapter_index: u32,
    pub output_index: u32,
    pub width: u32,
    pub height: u32,
    pub attached_to_desktop: bool,
    pub device_name: String,
}

/// DXGI Desktop Duplication capture for one output (monitor).
///
/// Create with [`DxgiDesktopDuplication::new`] (first adapter, first output)
/// or [`DxgiDesktopDuplication::for_output`] for a specific adapter/output.
pub struct DxgiDesktopDuplication {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    duplication: Option<IDXGIOutputDuplication>,
    width: u32,
    height: u32,
    capturing: bool,
    status: BackendStatus,
    /// Owned shared texture that receives the copied desktop backbuffer.
    /// Kept alive for the lifetime of the duplication so the shared handle
    /// stays valid for GPU encoders.
    shared_texture: Option<ID3D11Texture2D>,
    /// Staging texture for the CPU readback path (created lazily).
    staging: Option<ID3D11Texture2D>,
}

impl DxgiDesktopDuplication {
    /// Create a duplication for the first adapter's first output.
    pub fn new() -> Result<Self> {
        Self::for_output(0, 0)
    }

    /// Create a duplication for `adapter_index`'s `output_index` output.
    pub fn for_output(adapter_index: u32, output_index: u32) -> Result<Self> {
        tracing::info!(
            "Initializing DXGI Desktop Duplication (adapter {}, output {})",
            adapter_index,
            output_index
        );

        let factory: IDXGIFactory1 =
            unsafe { CreateDXGIFactory1() }.context("Failed to create DXGI factory")?;

        let adapter = unsafe { factory.EnumAdapters1(adapter_index) }
            .with_context(|| format!("Failed to enumerate adapter {}", adapter_index))?;

        let output = unsafe { adapter.EnumOutputs(output_index) }
            .with_context(|| format!("Failed to enumerate output {}", output_index))?;

        let output_desc =
            unsafe { output.GetDesc() }.context("Failed to get output description")?;

        let device = create_d3d11_device(Some(&adapter))?;
        let context =
            unsafe { device.GetImmediateContext() }.context("Failed to get immediate context")?;

        // `DuplicateOutput` requires IDXGIOutput1.
        let output1: IDXGIOutput1 = output.cast().context("Failed to cast to IDXGIOutput1")?;
        let duplication =
            unsafe { output1.DuplicateOutput(&device) }.context("Failed to duplicate output")?;

        let width = (output_desc.DesktopCoordinates.right - output_desc.DesktopCoordinates.left)
            .max(0) as u32;
        let height = (output_desc.DesktopCoordinates.bottom - output_desc.DesktopCoordinates.top)
            .max(0) as u32;

        tracing::info!(
            "DXGI duplication ready: {}x{} (attached to desktop: {})",
            width,
            height,
            output_desc.AttachedToDesktop.as_bool()
        );

        let mut status = BackendStatus::idle();
        status.switch_to(BackendKind::DesktopDuplication);

        Ok(Self {
            device,
            context,
            duplication: Some(duplication),
            width,
            height,
            capturing: false,
            status,
            shared_texture: None,
            staging: None,
        })
    }

    /// Enumerate all adapters and their outputs (monitors) reachable via
    /// DXGI, for output selection in the GUI.
    pub fn list_outputs() -> Result<Vec<DxgiOutputInfo>> {
        let factory: IDXGIFactory1 =
            unsafe { CreateDXGIFactory1() }.context("Failed to create DXGI factory")?;

        let mut outputs = Vec::new();
        let mut adapter_index = 0u32;
        loop {
            let adapter = match unsafe { factory.EnumAdapters1(adapter_index) } {
                Ok(a) => a,
                Err(_) => break, // no more adapters
            };
            let mut output_index = 0u32;
            loop {
                let output = match unsafe { adapter.EnumOutputs(output_index) } {
                    Ok(o) => o,
                    Err(_) => break, // no more outputs
                };
                let desc = unsafe { output.GetDesc() }.unwrap_or_default();
                let width = (desc.DesktopCoordinates.right - desc.DesktopCoordinates.left).max(0);
                let height = (desc.DesktopCoordinates.bottom - desc.DesktopCoordinates.top).max(0);
                let device_name = String::from_utf16_lossy(&desc.DeviceName)
                    .trim_end_matches('\0')
                    .to_string();
                outputs.push(DxgiOutputInfo {
                    adapter_index,
                    output_index,
                    width: width as u32,
                    height: height as u32,
                    attached_to_desktop: desc.AttachedToDesktop.as_bool(),
                    device_name,
                });
                output_index += 1;
            }
            adapter_index += 1;
        }
        Ok(outputs)
    }

    /// Acquire the next desktop frame as a GPU texture (zero-copy path).
    ///
    /// Returns `Ok(None)` when no new frame is available within the
    /// non-blocking timeout. On `DXGI_ERROR_ACCESS_LOST` the duplication is
    /// re-created automatically and `Ok(None)` is returned so the caller
    /// retries on the next iteration.
    pub fn acquire_gpu_frame(&mut self) -> Result<Option<GpuFrame>> {
        if !self.capturing {
            return Ok(None);
        }

        let (acquired, protected_masked) = self.acquire_raw()?;
        let Some(resource) = acquired else {
            return Ok(None);
        };

        // The duplicated desktop resource is not shareable, so OBS copies it
        // into an owned MISC_SHARED texture; the *shared* texture's handle is
        // what a GPU encoder can open. This copy is GPU→GPU, no CPU readback.
        self.ensure_shared_texture(self.width, self.height)?;
        let Some(shared) = self.shared_texture.as_ref() else {
            self.status.fail("shared texture is not initialized");
            return Ok(None);
        };
        let source: ID3D11Texture2D = resource
            .cast()
            .context("Failed to cast acquired resource to ID3D11Texture2D")?;
        unsafe { self.context.CopyResource(shared, &source) };

        // Get the shared handle of our owned texture.
        let shared_resource: IDXGIResource =
            shared.cast().context("Failed to cast to IDXGIResource")?;
        let shared_handle =
            unsafe { shared_resource.GetSharedHandle() }.context("Failed to get shared handle")?;

        Ok(Some(GpuFrame {
            shared_handle,
            width: self.width,
            height: self.height,
            protected_masked,
        }))
    }

    /// Acquire a raw desktop frame, applying the abort/fallback rules from
    /// the strategy. Returns the resource (if any) plus the protected-content
    /// flag. The desktop frame is released immediately; callers must copy it
    /// before returning.
    fn acquire_raw(&mut self) -> Result<(Option<IDXGIResource>, bool)> {
        if !self.capturing {
            return Ok((None, false));
        }
        let Some(duplication) = self.duplication.as_ref() else {
            self.status.fail("duplication is not initialized");
            return Ok((None, false));
        };

        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;

        // Non-blocking acquire (0 ms) matches the recording loop's
        // frame-rate-driven pacing.
        if let Err(e) = unsafe { duplication.AcquireNextFrame(0, &mut frame_info, &mut resource) } {
            let hr = e.code().0;
            match DxgiFailure::classify(hr) {
                DxgiFailure::Timeout => return Ok((None, false)),
                DxgiFailure::AccessLost => {
                    tracing::warn!(
                        "DXGI access lost (mode switch / lock) — re-creating duplication"
                    );
                    self.status.fail(format!(
                        "desktop session changed (DXGI_ERROR_ACCESS_LOST, {:#010x})",
                        hr
                    ));
                    self.recreate_duplication()?;
                    return Ok((None, false));
                }
                DxgiFailure::AccessDenied => {
                    self.status.fail(format!(
                        "protected content or non-duplicable desktop (DXGI_ERROR_ACCESS_DENIED, {:#010x})",
                        hr
                    ));
                    bail!("not capturable: protected content (DXGI_ERROR_ACCESS_DENIED)");
                }
                DxgiFailure::Unsupported => {
                    self.status.fail(format!(
                        "Desktop Duplication unavailable (DXGI_ERROR_UNSUPPORTED, {:#010x})",
                        hr
                    ));
                    bail!("unsupported: Desktop Duplication unavailable (DXGI_ERROR_UNSUPPORTED)");
                }
                DxgiFailure::Other(code) => {
                    self.status
                        .fail(format!("DXGI acquire failed ({:#010x})", code));
                    return Err(anyhow::anyhow!(
                        "DXGI acquire failed with HRESULT {:#010x}",
                        code
                    ));
                }
            }
        }

        let protected_masked = frame_info.ProtectedContentMaskedOut.as_bool();

        let Some(resource) = resource else {
            self.status.fail("acquire returned no resource");
            return Ok((None, false));
        };

        // Release the desktop frame immediately; callers have copied the
        // pixels/texture into their own resource by the time this returns.
        if let Err(e) = unsafe { duplication.ReleaseFrame() } {
            self.status
                .fail(format!("failed to release DXGI frame ({})", e));
        }

        Ok((Some(resource), protected_masked))
    }

    /// Ensure the owned shared texture exists (created once, reused).
    fn ensure_shared_texture(&mut self, width: u32, height: u32) -> Result<()> {
        if self.shared_texture.is_some() {
            return Ok(());
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: 0,
            CPUAccessFlags: 0,
            MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
        };
        let mut texture: Option<ID3D11Texture2D> = None;
        unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut texture)) }
            .context("Failed to create shared texture")?;
        self.shared_texture = texture;
        Ok(())
    }

    /// Ensure the staging texture for the CPU readback path exists.
    fn ensure_staging(&mut self, width: u32, height: u32) -> Result<()> {
        if self.staging.is_some() {
            return Ok(());
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };
        let mut staging: Option<ID3D11Texture2D> = None;
        unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut staging)) }
            .context("Failed to create staging texture")?;
        self.staging = staging;
        Ok(())
    }

    /// Re-create the output duplication after `DXGI_ERROR_ACCESS_LOST`.
    fn recreate_duplication(&mut self) -> Result<()> {
        self.duplication = None;

        let factory: IDXGIFactory1 =
            unsafe { CreateDXGIFactory1() }.context("Failed to create DXGI factory")?;
        let adapter = unsafe { factory.EnumAdapters1(0) }
            .context("Failed to enumerate adapter 0 during re-creation")?;
        let output = unsafe { adapter.EnumOutputs(0) }
            .context("Failed to enumerate output 0 during re-creation")?;
        let output1: IDXGIOutput1 = output
            .cast()
            .context("Failed to cast to IDXGIOutput1 during re-creation")?;
        let duplication = unsafe { output1.DuplicateOutput(&self.device) }
            .context("Failed to duplicate output during re-creation")?;
        self.duplication = Some(duplication);
        self.status.last_error = None;
        tracing::info!("DXGI duplication re-created after access loss");
        Ok(())
    }

    /// Current backend status: which backend is active, whether a fallback
    /// occurred, and the last error (if any). Mirrors the strategy's
    /// requirement to always report the active backend.
    pub fn status(&self) -> &BackendStatus {
        &self.status
    }
}

/// Create a D3D11 device for duplication, preferring the given adapter.
fn create_d3d11_device(adapter: Option<&IDXGIAdapter1>) -> Result<ID3D11Device> {
    let feature_levels = [
        D3D_FEATURE_LEVEL_11_0,
        D3D_FEATURE_LEVEL_10_0,
        D3D_FEATURE_LEVEL_9_1,
    ];
    let mut device: Option<ID3D11Device> = None;

    // Per the D3D11 docs, when an explicit adapter is supplied the driver
    // type must be D3D_DRIVER_TYPE_UNKNOWN (OBS does exactly this). With no
    // adapter, use hardware so the runtime picks the default GPU.
    let driver_type = if adapter.is_some() {
        D3D_DRIVER_TYPE_UNKNOWN
    } else {
        D3D_DRIVER_TYPE_HARDWARE
    };

    // The Param impl accepts `Option<&IDXGIAdapter>`; IDXGIAdapter1 derefs
    // to IDXGIAdapter, so map through the deref.
    let adapter_ref: Option<&IDXGIAdapter> = adapter.map(|a| &**a);
    unsafe {
        D3D11CreateDevice(
            adapter_ref,
            driver_type,
            Default::default(),
            D3D11_CREATE_DEVICE_FLAG(0),
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )
    }
    .context("Failed to create D3D11 device")?;

    device.context("D3D11 device creation returned no device")
}

impl CaptureSource for DxgiDesktopDuplication {
    fn start(&mut self) -> Result<()> {
        tracing::info!("Starting DXGI desktop duplication capture");
        self.capturing = true;
        self.status.last_error = None;
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        tracing::info!("Stopping DXGI desktop duplication capture");
        self.capturing = false;
        Ok(())
    }

    fn capture_frame(&mut self) -> Result<Option<CapturedFrame>> {
        // CPU readback: copy the current desktop frame into the staging
        // texture and read the pixels. This is the compatibility path; the
        // zero-copy path is `acquire_gpu_frame`.
        let (acquired, _protected) = self.acquire_raw()?;
        let Some(resource) = acquired else {
            return Ok(None);
        };

        self.ensure_staging(self.width, self.height)?;
        let Some(staging) = self.staging.as_ref() else {
            return Ok(None);
        };

        let source: ID3D11Texture2D = resource
            .cast()
            .context("Failed to cast acquired resource to ID3D11Texture2D")?;
        unsafe { self.context.CopyResource(staging, &source) };

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            self.context
                .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
        }
        .context("Failed to map staging texture")?;

        let row_pitch = mapped.RowPitch as usize;
        let bytes_per_row = self.width as usize * 4;
        let mut bgra = vec![0u8; bytes_per_row * self.height as usize];
        unsafe {
            for row in 0..self.height as usize {
                let src = (mapped.pData as *const u8).add(row * row_pitch);
                let dst = &mut bgra[row * bytes_per_row..(row + 1) * bytes_per_row];
                std::ptr::copy_nonoverlapping(src, dst.as_mut_ptr(), bytes_per_row);
            }
            self.context.Unmap(staging, 0);
        }

        // Convert BGRA → RGBA for the rest of the pipeline.
        let mut rgba = bgra;
        for [b, _g, r, _a] in rgba.as_chunks_mut::<4>().0 {
            std::mem::swap(b, r);
        }

        Ok(Some(CapturedFrame::new(
            rgba,
            self.width,
            self.height,
            self.width * 4,
        )))
    }

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn is_capturing(&self) -> bool {
        self.capturing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_info_collects_fields() {
        let info = DxgiOutputInfo {
            adapter_index: 0,
            output_index: 1,
            width: 1920,
            height: 1080,
            attached_to_desktop: true,
            device_name: r"\\.\DISPLAY1".to_string(),
        };
        assert_eq!(info.adapter_index, 0);
        assert_eq!(info.output_index, 1);
        assert_eq!(info.width, 1920);
        assert_eq!(info.height, 1080);
        assert!(info.attached_to_desktop);
        assert_eq!(info.device_name, r"\\.\DISPLAY1");
    }

    #[test]
    fn gpu_frame_reports_dimensions_and_protection() {
        let frame = GpuFrame {
            shared_handle: HANDLE::default(),
            width: 2560,
            height: 1440,
            protected_masked: true,
        };
        assert_eq!(frame.width, 2560);
        assert_eq!(frame.height, 1440);
        assert!(frame.protected_masked);
    }

    /// End-to-end smoke test against the real DXGI API. Requires an
    /// interactive desktop session with at least one attached output, so it
    /// is ignored in CI (headless runners have no duplicatable desktop). Run
    /// locally with `cargo test -p rivulet-capture -- --ignored`.
    #[test]
    #[ignore = "requires an interactive desktop session"]
    fn smoke_duplication_acquires_gpu_frame() {
        let outputs = DxgiDesktopDuplication::list_outputs().expect("list_outputs failed");
        assert!(!outputs.is_empty(), "no DXGI outputs — headless session?");
        let attached: Vec<_> = outputs.iter().filter(|o| o.attached_to_desktop).collect();
        assert!(
            !attached.is_empty(),
            "no output attached to desktop — cannot duplicate"
        );
        tracing::info!("outputs: {:?}", outputs);

        let mut dup = DxgiDesktopDuplication::new().expect("create duplication");
        let (w, h) = dup.dimensions();
        assert!(
            w > 0 && h > 0,
            "dimensions must be non-zero, got {}x{}",
            w,
            h
        );
        assert_eq!(dup.status().active, BackendKind::DesktopDuplication);
        assert!(dup.status().is_healthy());

        dup.start().expect("start");
        assert!(dup.is_capturing());

        // Poll for up to ~1 s for the first GPU frame. The screen may be
        // static, in which case AcquireNextFrame times out until something
        // changes; Desktop Duplication delivers at least one frame after
        // DuplicateOutput, so the first acquire usually succeeds.
        let mut got_frame = false;
        for _ in 0..20 {
            if let Some(frame) = dup.acquire_gpu_frame().expect("acquire_gpu_frame") {
                assert_eq!(frame.width, w);
                assert_eq!(frame.height, h);
                assert!(
                    !frame.shared_handle.is_invalid(),
                    "shared handle must be valid"
                );
                got_frame = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(got_frame, "no GPU frame acquired within 1 s");

        // RGBA compatibility path must produce a correctly-sized frame.
        let mut found_rgba = false;
        for _ in 0..20 {
            if let Some(frame) = dup.capture_frame().expect("capture_frame") {
                assert_eq!(frame.width, w);
                assert_eq!(frame.height, h);
                assert_eq!(frame.stride, w * 4);
                assert_eq!(frame.data.len(), (w * h * 4) as usize);
                found_rgba = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(found_rgba, "no RGBA frame acquired within 1 s");

        dup.stop().expect("stop");
        assert!(!dup.is_capturing());
    }
}
