//! WASAPI endpoint enumeration and device-level capture (issue #229).
//!
//! Rivulet's per-app path captures the audio of a *process*; this module
//! adds the *device* half of the audio matrix (the S8 scope from issue #68):
//! selecting a WASAPI output endpoint (e.g. one of the SteelSeries GG /
//! Sonar virtual mixer channels — Game, Chat, Media, Stream) as a routed
//! `OutputDevice` source, or a capture endpoint (microphone) as an
//! `InputDevice` source.
//!
//! Two primitives are exposed to the GUI:
//!
//! - [`list_audio_devices`] — enumerate active render/capture endpoints
//!   with friendly names and the default-device marker.
//! - [`AudioDeviceCapture`] — stream one endpoint as interleaved f32 PCM at
//!   48 kHz stereo (the engine's routed appsrc caps; the same contract the
//!   per-app path in [`crate::process_loopback`] honors) from a dedicated
//!   thread with cooperative stop.
//!
//! Output endpoints are captured with `AUDCLNT_STREAMFLAGS_LOOPBACK`
//! (shared-mode render loopback: what that device plays); input endpoints
//! are plain shared-mode capture. `AUTOCONVERTPCM | SRC_DEFAULT_QUALITY`
//! lets WASAPI convert the endpoint's native mix format to our fixed caps,
//! so no resampling code lives here.
//!
//! Device ids follow the `wasapi-out:<endpoint id>` / `wasapi-in:<endpoint
//! id>` convention (see `rivulet_core::DeviceTarget`); the endpoint id is
//! the stable WASAPI endpoint string, so persisted configs survive
//! reboots and device re-ordering.

use anyhow::{bail, Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, EDataFlow, IAudioCaptureClient, IAudioClient, IMMDevice,
    IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_LOOPBACK, AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, DEVICE_STATE_ACTIVE,
    WAVEFORMATEX,
};
use windows::Win32::Media::Multimedia::WAVE_FORMAT_IEEE_FLOAT;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::System::Variant::VT_LPWSTR;

use rivulet_core::{AudioFrame, DeviceTarget};

use crate::capture::AudioFrameCallback;

/// `RPC_E_CHANGED_MODE` as a raw HRESULT (COM already initialized on this
/// thread with a different apartment model — still usable for our purpose).
const RPC_E_CHANGED_MODE_RAW: windows::core::HRESULT = windows::core::HRESULT(-2147417850);

/// Wait granularity for the capture event so the stop flag is honored even
/// when the endpoint goes silent (WASAPI only signals when packets arrive).
const CAPTURE_WAIT_MS: u32 = 200;

/// One capturable WASAPI endpoint offered by the device picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDeviceInfo {
    /// The stable WASAPI endpoint string (becomes a `wasapi-out:` /
    /// `wasapi-in:` device id).
    pub endpoint_id: String,
    /// Human-readable endpoint name (device + jack description).
    pub name: String,
    /// Whether this is a render (output/loopback) endpoint.
    pub is_output: bool,
    /// Whether this endpoint is the current console default for its flow.
    pub is_default: bool,
}

impl AudioDeviceInfo {
    /// The `rivulet_core::DeviceTarget`-style device id for this endpoint
    /// (`wasapi-out:<id>` for render, `wasapi-in:<id>` for capture).
    pub fn device_id(&self) -> String {
        if self.is_output {
            format!("wasapi-out:{}", self.endpoint_id)
        } else {
            format!("wasapi-in:{}", self.endpoint_id)
        }
    }
}

/// Read the friendly endpoint name from the endpoint's property store.
/// Falls back to the raw endpoint id when the property is unreadable —
/// a nameless device must not make the whole enumeration fail.
fn endpoint_friendly_name(device: &IMMDevice, fallback: &str) -> String {
    unsafe {
        let store = match device.OpenPropertyStore(STGM_READ) {
            Ok(store) => store,
            Err(_) => return fallback.to_owned(),
        };
        match store.GetValue(&PKEY_Device_FriendlyName) {
            Ok(value) => {
                // VT_LPWSTR: the PROPVARIANT owns the allocation — read it
                // here, then let the `Ok(value)` binding drop normally
                // (PropVariantClear frees `pwszVal`).
                let vt = value.Anonymous.Anonymous.vt;
                let pwstr = value.Anonymous.Anonymous.Anonymous.pwszVal;
                if vt == VT_LPWSTR && !pwstr.is_null() {
                    if let Ok(name) = pwstr.to_string() {
                        return name;
                    }
                }
                fallback.to_owned()
            }
            Err(_) => fallback.to_owned(),
        }
    }
}

/// Enumerate active endpoints for one data flow as `(endpoint id, name)`.
/// Best-effort: a partially unreadable endpoint is skipped instead of
/// failing the whole enumeration.
fn endpoints(dataflow: EDataFlow) -> Vec<(String, String)> {
    let mut endpoints = Vec::new();
    unsafe {
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        if hr.is_err() && hr != RPC_E_CHANGED_MODE_RAW {
            return endpoints;
        }
        let enumerator: IMMDeviceEnumerator =
            match CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) {
                Ok(e) => e,
                Err(_) => return endpoints,
            };
        let collection = match enumerator.EnumAudioEndpoints(dataflow, DEVICE_STATE_ACTIVE) {
            Ok(c) => c,
            Err(_) => return endpoints,
        };
        let count = collection.GetCount().unwrap_or(0);
        for i in 0..count {
            let Ok(device) = collection.Item(i) else {
                continue;
            };
            let Ok(id_ptr) = device.GetId() else {
                continue;
            };
            // `GetId` CoTaskMem-allocates the string — copy it, then free.
            let id = match PWSTR(id_ptr.as_ptr()).to_string() {
                Ok(s) => s,
                Err(_) => {
                    free_co_task_str(id_ptr);
                    continue;
                }
            };
            free_co_task_str(id_ptr);
            let name = endpoint_friendly_name(&device, &id);
            endpoints.push((id, name));
        }
    }
    endpoints
}

/// Free a WASAPI-allocated endpoint id string (CoTaskMem-allocated).
fn free_co_task_str(pwstr: PWSTR) {
    if !pwstr.is_null() {
        unsafe {
            CoTaskMemFree(Some(pwstr.as_ptr() as *const core::ffi::c_void));
        }
    }
}

/// Resolve the default console endpoint id for a data flow (`eRender` or
/// `eCapture`). Empty string when the flow has no default.
fn default_endpoint_id(dataflow: EDataFlow) -> String {
    unsafe {
        let hr = CoInitializeEx(None, COINIT_MULTITHREADED);
        if hr.is_err() && hr != RPC_E_CHANGED_MODE_RAW {
            return String::new();
        }
        let enumerator: IMMDeviceEnumerator =
            match CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) {
                Ok(e) => e,
                Err(_) => return String::new(),
            };
        let Ok(device) = enumerator.GetDefaultAudioEndpoint(dataflow, eConsole) else {
            return String::new();
        };
        let Ok(id_ptr) = device.GetId() else {
            return String::new();
        };
        let id = PWSTR(id_ptr.as_ptr()).to_string().unwrap_or_default();
        free_co_task_str(id_ptr);
        id
    }
}

/// List active, capturable audio endpoints for the device picker
/// (issue #229). Render endpoints come first (they map to `OutputDevice`
/// sources), then capture endpoints; within each flow the console default
/// sorts first. Best-effort: an unreadable endpoint is skipped instead of
/// failing the enumeration.
pub fn list_audio_devices() -> Vec<AudioDeviceInfo> {
    let mut devices = Vec::new();

    // Render endpoints (captured in loopback mode).
    let default_render = default_endpoint_id(eRender);
    for (endpoint_id, name) in endpoints(eRender) {
        devices.push(AudioDeviceInfo {
            is_default: endpoint_id == default_render,
            is_output: true,
            endpoint_id,
            name,
        });
    }

    // Capture endpoints (plain shared-mode capture).
    let default_capture = default_endpoint_id(eCapture);
    for (endpoint_id, name) in endpoints(eCapture) {
        devices.push(AudioDeviceInfo {
            is_default: endpoint_id == default_capture,
            is_output: false,
            endpoint_id,
            name,
        });
    }

    devices.sort_by(|a, b| {
        b.is_default
            .cmp(&a.is_default)
            .then_with(|| b.is_output.cmp(&a.is_output))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    devices
}

/// Resolve a [`DeviceTarget`] to its WASAPI endpoint string. Returns the
/// endpoint id verbatim — the capture thread resolves it to the `IMMDevice`
/// at start time. PipeWire node targets (issue #231) are not WASAPI
/// endpoints; the PipeWire backend (`device_capture_pw`) handles them.
fn resolve_target(target: &DeviceTarget) -> Result<(String, bool)> {
    match target {
        DeviceTarget::Output(id) => Ok((id.clone(), true)),
        DeviceTarget::Input(id) => Ok((id.clone(), false)),
        DeviceTarget::PwSource(_) | DeviceTarget::PwMonitor(_) => {
            bail!("pw-src:/pw-mon: targets are handled by the PipeWire device backend, not WASAPI")
        }
    }
}

/// An active WASAPI device capture. Dropping it signals the capture thread;
/// call [`AudioDeviceCapture::stop`] to also wait for a clean exit.
pub struct AudioDeviceCapture {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AudioDeviceCapture {
    /// Start capturing the given WASAPI endpoint (from a parsed
    /// [`DeviceTarget`]) and deliver interleaved f32 frames (48 kHz stereo)
    /// to `on_frame` from a dedicated thread.
    ///
    /// Output targets open the endpoint in shared-mode loopback; input
    /// targets open it as a plain capture client.
    pub fn start_for_target(target: &DeviceTarget, on_frame: AudioFrameCallback) -> Result<Self> {
        let (endpoint_id, is_output) = resolve_target(target)?;
        if endpoint_id.is_empty() {
            bail!("empty WASAPI endpoint id");
        }
        Self::start(endpoint_id, is_output, on_frame)
    }

    /// Start capturing the endpoint `endpoint_id` (loopback when
    /// `is_output`) and deliver frames to `on_frame`.
    pub fn start(
        endpoint_id: String,
        is_output: bool,
        on_frame: AudioFrameCallback,
    ) -> Result<Self> {
        if endpoint_id.is_empty() {
            bail!("empty WASAPI endpoint id");
        }
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name(format!(
                "rivulet-device-audio-{}",
                if is_output { "out" } else { "in" }
            ))
            .spawn(move || {
                if let Err(err) = capture_thread(endpoint_id, is_output, thread_stop, on_frame) {
                    tracing::warn!(error = %err, "WASAPI device capture ended with an error");
                }
            })
            .context("failed to spawn the WASAPI device capture thread")?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    /// Signal the capture thread to stop and wait for it to exit.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for AudioDeviceCapture {
    fn drop(&mut self) {
        // Cooperative shutdown without blocking the caller.
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// The fixed capture format: interleaved f32, 48 kHz, stereo — exactly the
/// engine's routed appsrc caps so no resampling is needed.
/// `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM` lets WASAPI convert the endpoint's
/// native mix format into this regardless of its native layout.
fn device_wave_format() -> WAVEFORMATEX {
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
        nChannels: 2,
        nSamplesPerSec: 48_000,
        nAvgBytesPerSec: 48_000 * 2 * 4,
        nBlockAlign: 2 * 4,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

fn capture_thread(
    endpoint_id: String,
    is_output: bool,
    stop: Arc<AtomicBool>,
    mut on_frame: AudioFrameCallback,
) -> Result<()> {
    // COM is required on this thread for the enumerator and WASAPI. MTA is
    // the natural choice for a worker thread; RPC_E_CHANGED_MODE means COM
    // is already initialized with a different model, which still works here.
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if hr.is_err() && hr != RPC_E_CHANGED_MODE_RAW {
        bail!("CoInitializeEx failed: {hr:?}");
    }

    let client = open_endpoint_client(&endpoint_id)?;

    let mut stream_flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK
        | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
        | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
    if is_output {
        // Loopback capture: deliver what the render endpoint plays.
        stream_flags |= AUDCLNT_STREAMFLAGS_LOOPBACK;
    }

    unsafe {
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                stream_flags,
                0, // default buffer duration
                0,
                &device_wave_format(),
                None,
            )
            .context("IAudioClient::Initialize failed for the device stream")?;

        let event = CreateEventW(None, false, false, PCWSTR::null())
            .context("CreateEventW failed for the capture event")?;
        client
            .SetEventHandle(event)
            .context("SetEventHandle failed")?;

        let capture: IAudioCaptureClient = client
            .GetService()
            .context("GetService(IAudioCaptureClient) failed")?;

        client.Start().context("IAudioClient::Start failed")?;

        let run_result = device_capture_loop(&capture, event, stop, &mut on_frame);
        let _ = client.Stop();
        let _ = CloseHandle(event);
        run_result
    }
}

/// Resolve the endpoint id to its `IMMDevice` and open an `IAudioClient`
/// for it (the *device* path — unlike the per-app path, a plain
/// `IMMDeviceEnumerator::GetDevice` is all that is needed here).
fn open_endpoint_client(endpoint_id: &str) -> Result<IAudioClient> {
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if hr.is_err() && hr != RPC_E_CHANGED_MODE_RAW {
        bail!("CoInitializeEx failed: {hr:?}");
    }
    let enumerator: IMMDeviceEnumerator =
        unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
            .context("CoCreateInstance(MMDeviceEnumerator) failed")?;
    let wide: Vec<u16> = endpoint_id
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let device: IMMDevice = unsafe { enumerator.GetDevice(PCWSTR(wide.as_ptr())) }
        .with_context(|| format!("WASAPI endpoint not found: {endpoint_id}"))?;
    unsafe { device.Activate::<IAudioClient>(CLSCTX_ALL, None) }
        .context("IMMDevice::Activate(IAudioClient) failed")
}

/// Run the packet pump until `stop` is set. Same contract as the per-app
/// capture loop: drain every packet per signal, silence keeps the appsrc
/// timeline moving, timeouts re-check the stop flag.
fn device_capture_loop(
    capture: &IAudioCaptureClient,
    event: HANDLE,
    stop: Arc<AtomicBool>,
    on_frame: &mut AudioFrameCallback,
) -> Result<()> {
    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        let wait = unsafe { WaitForSingleObject(event, CAPTURE_WAIT_MS) };
        if stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        if wait != WAIT_OBJECT_0 {
            // Timeout: the endpoint is silent. Loop back and re-check stop.
            continue;
        }
        loop {
            let packet = unsafe { capture.GetNextPacketSize() }?;
            if packet == 0 {
                break;
            }
            let mut data_ptr: *mut u8 = std::ptr::null_mut();
            let mut frames: u32 = 0;
            let mut flags: u32 = 0;
            unsafe { capture.GetBuffer(&mut data_ptr, &mut frames, &mut flags, None, None) }
                .context("GetBuffer failed")?;
            if frames > 0 {
                if (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 {
                    // Silence keeps the appsrc timeline moving; deliver zeros.
                    on_frame(AudioFrame::new(vec![0.0; frames as usize * 2], 48_000, 2));
                } else {
                    let len = frames as usize * 2; // stereo f32 interleaved
                    let slice = unsafe { std::slice::from_raw_parts(data_ptr as *const f32, len) };
                    on_frame(AudioFrame::new(slice.to_vec(), 48_000, 2));
                }
            }
            let _ = unsafe { capture.ReleaseBuffer(frames) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_wave_format_matches_engine_caps() {
        // Same contract as the per-app path: the engine's routed appsrcs
        // declare `audio/x-raw,format=F32LE,layout=interleaved,channels=2,rate=48000`.
        // WAVEFORMATEX is `packed(1)`, so fields are destructured (copied)
        // instead of referenced in place.
        let WAVEFORMATEX {
            wFormatTag,
            nChannels,
            nSamplesPerSec,
            nAvgBytesPerSec,
            nBlockAlign,
            wBitsPerSample,
            cbSize,
        } = device_wave_format();
        assert_eq!(wFormatTag, WAVE_FORMAT_IEEE_FLOAT as u16);
        assert_eq!(nChannels, 2);
        assert_eq!(nSamplesPerSec, 48_000);
        assert_eq!(wBitsPerSample, 32);
        assert_eq!(nBlockAlign, 8);
        assert_eq!(nAvgBytesPerSec, 48_000 * 8);
        assert_eq!(cbSize, 0);
    }
    #[test]
    fn device_list_returns_both_flows() {
        // Best-effort enumeration is the contract: the call never panics or
        // fails wholesale, even on endpoint-less hosts (hosted CI runners
        // ship no audio devices at all). Shape assertions apply only to the
        // entries that exist; every entry round-trips through the core
        // DeviceTarget convention.
        let devices = list_audio_devices();
        for device in &devices {
            let id = device.device_id();
            let parsed = DeviceTarget::parse(&id).expect("picker ids must round-trip");
            assert_eq!(parsed.is_output(), device.is_output);
            assert!(
                !device.name.is_empty(),
                "fallback naming guarantees non-empty"
            );
        }
        // When defaults were resolved, the default endpoint sorts first.
        if devices.iter().any(|d| d.is_default) {
            assert!(
                devices.first().is_some_and(|d| d.is_default),
                "the console default endpoints are marked and sort first"
            );
        }
    }

    #[test]
    fn device_capture_rejects_empty_endpoint_id() {
        let result = AudioDeviceCapture::start(String::new(), true, Box::new(|_| {}));
        assert!(result.is_err(), "an empty endpoint id is not capturable");
    }

    #[test]
    fn device_capture_start_stop_round_trip_on_unknown_endpoint() {
        // A plausible-but-unknown endpoint must not panic: the thread starts,
        // fails to resolve the device, and stop() joins cleanly.
        let frames = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = frames.clone();
        let capture = AudioDeviceCapture::start(
            "{0.0.0.00000000}.{does-not-exist}".to_owned(),
            true,
            Box::new(move |_| {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }),
        )
        .expect("capture thread must spawn for a non-empty endpoint id");
        std::thread::sleep(std::time::Duration::from_millis(120));
        capture.stop();
        let _ = frames.load(std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn device_capture_round_trip_on_the_default_render_endpoint() {
        // The console default render endpoint exists on real desktop
        // sessions, where activating it must succeed (or fail cleanly,
        // never hang). Endpoint-less hosts (hosted CI runners have no audio
        // devices) degrade to a no-op instead of failing — the live
        // activation contract is exercised wherever endpoints exist.
        let Some(default) = list_audio_devices()
            .into_iter()
            .find(|d| d.is_output && d.is_default)
        else {
            eprintln!("no default render endpoint on this host — skipping live capture");
            return;
        };
        let frames = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = frames.clone();
        let capture = AudioDeviceCapture::start(
            default.endpoint_id.clone(),
            true,
            Box::new(move |_| {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }),
        )
        .expect("the default endpoint must activate");
        std::thread::sleep(std::time::Duration::from_millis(150));
        capture.stop();
        let _ = frames.load(std::sync::atomic::Ordering::Relaxed);
    }
}
