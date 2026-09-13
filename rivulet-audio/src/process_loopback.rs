//! Per-application audio capture via WASAPI process loopback (Windows).
//!
//! Phase 3 of the multi-track audio routing design
//! ([`docs/m6-audio-routing.md`](../../docs/m6-audio-routing.md)): an
//! [`AudioSource`](rivulet_core::audio_source::AudioSource) of kind
//! [`Application`](rivulet_core::audio_source::AudioSourceKind::Application)
//! targets a process id (`pid:<n>` device id) and this module captures exactly
//! that process tree's audio with the WASAPI "virtual audio device" process
//! loopback (`VAD\Process_Loopback`, Windows 10 2004+).
//!
//! Flow: `ActivateAudioInterfaceAsync` activates an `IAudioClient` for the
//! loopback VAD with the target pid in a `VT_BLOB` `PROPVARIANT`; the capture
//! thread then reads event-driven shared-mode packets and delivers interleaved
//! f32 frames (48 kHz stereo — the engine's routed appsrc caps) through the
//! caller's callback.

use anyhow::{bail, Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use windows::core::{implement, Interface, Ref, HRESULT, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation,
    IActivateAudioInterfaceCompletionHandler, IActivateAudioInterfaceCompletionHandler_Impl,
    IAudioCaptureClient, IAudioClient, AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, AUDIOCLIENT_ACTIVATION_PARAMS,
    AUDIOCLIENT_ACTIVATION_PARAMS_0, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
    VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK, WAVEFORMATEX,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{CoInitializeEx, BLOB, COINIT_MULTITHREADED};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};
use windows::Win32::System::Variant::VT_BLOB;

use windows::Win32::Media::Multimedia::WAVE_FORMAT_IEEE_FLOAT;

use rivulet_core::AudioFrame;

use crate::capture::AudioFrameCallback;

/// `RPC_E_CHANGED_MODE` as a raw HRESULT (COM already initialized on this
/// thread with a different apartment model — still usable for our purpose).
const RPC_E_CHANGED_MODE_RAW: HRESULT = HRESULT(-2147417850);

/// The activation may take a moment on loaded systems; this is generous but
/// bounded so a wedged audio service cannot hang the capture thread forever.
const ACTIVATION_TIMEOUT_MS: u32 = 10_000;

/// Wait granularity for the capture event so the stop flag is honored even
/// when the target goes silent (WASAPI only signals when packets arrive).
const CAPTURE_WAIT_MS: u32 = 200;

/// A running process offered by the Application-source picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppAudioProcess {
    /// The OS process id (becomes the `pid:<n>` device id).
    pub pid: u32,
    /// Executable file name (e.g. `spotify.exe`).
    pub name: String,
}

/// Build the fixed loopback format: interleaved f32, 48 kHz, stereo —
/// exactly the engine's routed appsrc caps so no resampling is needed.
/// `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM` lets WASAPI convert the loopback mix
/// into this format regardless of the device's native layout.
pub(crate) fn loopback_wave_format() -> WAVEFORMATEX {
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

/// An agile completion handler: the async activation signals our event when
/// it finishes; the waiting thread then reads the result from the operation
/// itself (the documented pattern — the callback runs on an RPC thread and
/// must not block). `windows-implement` marks handlers agile by default, as
/// `ActivateAudioInterfaceAsync` requires.
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct ActivationHandler {
    /// Raw event HANDLE stored as `isize` so the handler stays `Send`.
    event: isize,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for ActivationHandler_Impl {
    fn ActivateCompleted(
        &self,
        _activateoperation: Ref<'_, IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        // Wake the waiter. Errors are ignored: the waiter also applies its
        // timeout and reads the operation result either way.
        unsafe {
            let _ = SetEvent(HANDLE(self.event as *mut core::ffi::c_void));
        }
        Ok(())
    }
}

/// An active per-application capture. Dropping it signals the capture thread;
/// call [`AppAudioCapture::stop`] to also wait for a clean exit.
pub struct AppAudioCapture {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AppAudioCapture {
    /// Start capturing the audio of `pid` (and its child process tree) and
    /// deliver interleaved f32 frames (48 kHz stereo) to `on_frame` from a
    /// dedicated thread.
    pub fn start(pid: u32, on_frame: AudioFrameCallback) -> Result<Self> {
        if pid == 0 {
            bail!("pid 0 (idle process) cannot be captured");
        }
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name(format!("rivulet-app-audio-{pid}"))
            .spawn(move || {
                if let Err(err) = capture_thread(pid, thread_stop, on_frame) {
                    tracing::warn!(pid, error = %err, "per-app audio capture ended with an error");
                }
            })
            .context("failed to spawn the per-app audio capture thread")?;
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

impl Drop for AppAudioCapture {
    fn drop(&mut self) {
        // Cooperative shutdown without blocking the caller.
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn capture_thread(pid: u32, stop: Arc<AtomicBool>, mut on_frame: AudioFrameCallback) -> Result<()> {
    eprintln!("[probe] thread enter");
    // COM is required on this thread for the activation and WASAPI. MTA is the
    // natural choice for a worker thread; RPC_E_CHANGED_MODE means COM is
    // already initialized with a different model, which still works here.
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    eprintln!("[probe] CoInitializeEx -> {hr:?}");
    if hr.is_err() && hr != RPC_E_CHANGED_MODE_RAW {
        bail!("CoInitializeEx failed: {hr:?}");
    }

    let client = match activate_process_loopback_client(pid) {
        Ok(c) => {
            eprintln!("[probe] activated");
            c
        }
        Err(e) => {
            eprintln!("[probe] activation failed: {e:#}");
            return Err(e);
        }
    };

    unsafe {
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                    | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                0, // default buffer duration
                0,
                &loopback_wave_format(),
                None,
            )
            .context("IAudioClient::Initialize failed for the process loopback stream")?;

        let event = CreateEventW(None, false, false, PCWSTR::null())
            .context("CreateEventW failed for the capture event")?;
        client
            .SetEventHandle(event)
            .context("SetEventHandle failed")?;

        let capture: IAudioCaptureClient = client
            .GetService()
            .context("GetService(IAudioCaptureClient) failed")?;

        client.Start().context("IAudioClient::Start failed")?;

        let run_result = capture_loop(&capture, event, stop, &mut on_frame);
        let _ = client.Stop();
        let _ = CloseHandle(event);
        run_result
    }
}

/// Activate an `IAudioClient` bound to the audio of `pid`'s process tree via
/// the process-loopback virtual audio device.
///
/// The documented path is the asynchronous `ActivateAudioInterfaceAsync` with
/// the `AUDIOCLIENT_ACTIVATION_PARAMS` blob; the `VAD\Process_Loopback` device
/// is only resolvable through it (a plain `IMMDeviceEnumerator::GetDevice`
/// rejects the path with `E_INVALIDARG`). Two lifetime rules make this safe:
///
/// 1. The params blob must stay valid until the activation **completes** —
///    the audio service reads it from its own thread after the call returns.
///    Hence the `Box` is dropped only after `GetActivateResult`.
/// 2. The `PROPVARIANT` must never run its `Drop` (which would call
///    `PropVariantClear` and `CoTaskMemFree` the blob pointer that Rust
///    frees itself — a double free). It is wrapped in `ManuallyDrop`.
fn activate_process_loopback_client(pid: u32) -> Result<IAudioClient> {
    let activation = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    let mut activation = Box::new(activation);

    let mut propvariant = std::mem::ManuallyDrop::new(PROPVARIANT::default());
    use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT_0_0, PROPVARIANT_0_0_0};
    propvariant.Anonymous.Anonymous = std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
        vt: VT_BLOB,
        wReserved1: 0,
        wReserved2: 0,
        wReserved3: 0,
        Anonymous: PROPVARIANT_0_0_0 {
            blob: BLOB {
                cbSize: std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
                pBlobData: activation.as_mut() as *mut AUDIOCLIENT_ACTIVATION_PARAMS as *mut u8,
            },
        },
    });

    let event = unsafe { CreateEventW(None, false, false, PCWSTR::null()) }
        .context("CreateEventW failed for the activation event")?;
    let handler: IActivateAudioInterfaceCompletionHandler = ActivationHandler {
        event: event.0 as isize,
    }
    .into();
    eprintln!("[probe] handler created");

    let operation = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&*propvariant),
            &handler,
        )
    }
    .context("ActivateAudioInterfaceAsync failed")?;
    eprintln!("[probe] activate call returned, waiting");

    let wait = unsafe { WaitForSingleObject(event, ACTIVATION_TIMEOUT_MS) };
    let _ = unsafe { CloseHandle(event) };
    if wait != WAIT_OBJECT_0 {
        bail!("process loopback activation timed out after {ACTIVATION_TIMEOUT_MS} ms");
    }

    // The result is read from the operation after completion; afterwards the
    // params blob may be released (rule 1).
    let mut result_hr = HRESULT(0);
    let mut activated: Option<windows::core::IUnknown> = None;
    unsafe {
        operation
            .GetActivateResult(&mut result_hr, &mut activated)
            .context("GetActivateResult failed")?;
    }
    // Both the `ManuallyDrop` wrapper and the params `Box` fall out of
    // scope here. The wrapper's scope-end drop is a no-op by design —
    // `PropVariantClear` must never run on a variant whose blob pointer
    // points into memory owned by the `Box` (it would `CoTaskMemFree` a
    // pointer that Rust frees itself — heap corruption; this exact bug was
    // caught by the round-trip test below). The union content is simply
    // leaked along with the `Box` at scope end, matching wasapi-rs and the
    // Microsoft ApplicationLoopback sample.
    if result_hr.is_err() {
        bail!("process loopback activation failed: {result_hr:?}");
    }
    let unknown = activated.context("activation completed without an interface")?;
    unknown
        .cast::<IAudioClient>()
        .context("activated interface is not an IAudioClient")
}

fn capture_loop(
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
            // Timeout: the target is silent. Loop back and re-check stop.
            continue;
        }
        // Drain every packet that became available.
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

/// List running processes (`pid` + executable name) for the Application-source
/// picker. Uses the ToolHelp snapshot — no audio subsystem required, so it is
/// also safe to call while no session is active.
pub fn list_audio_processes() -> Vec<AppAudioProcess> {
    let mut processes = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return processes;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let name_len = entry
                    .szExeFile
                    .iter()
                    .position(|c| *c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..name_len]);
                if entry.th32ProcessID != 0 && !name.is_empty() {
                    processes.push(AppAudioProcess {
                        pid: entry.th32ProcessID,
                        name,
                    });
                }
                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    processes.sort_by_key(|p| p.name.to_lowercase());
    processes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_format_matches_engine_caps() {
        // The engine's routed appsrcs declare
        // `audio/x-raw,format=F32LE,layout=interleaved,channels=2,rate=48000`.
        // WAVEFORMATEX is `packed(1)`, so fields are destructured (copied)
        // instead of referenced in place.
        let windows::Win32::Media::Audio::WAVEFORMATEX {
            wFormatTag,
            nChannels,
            nSamplesPerSec,
            nAvgBytesPerSec,
            nBlockAlign,
            wBitsPerSample,
            cbSize,
        } = loopback_wave_format();
        assert_eq!(wFormatTag, WAVE_FORMAT_IEEE_FLOAT as u16);
        assert_eq!(nChannels, 2);
        assert_eq!(nSamplesPerSec, 48_000);
        assert_eq!(wBitsPerSample, 32);
        assert_eq!(nBlockAlign, 8);
        assert_eq!(nAvgBytesPerSec, 48_000 * 8);
        assert_eq!(cbSize, 0);
    }

    #[test]
    fn process_list_contains_own_process() {
        // The ToolHelp snapshot always contains this test process.
        let processes = list_audio_processes();
        assert!(
            !processes.is_empty(),
            "a running Windows system has at least one process"
        );
        // PIDs are unique in the snapshot.
        let mut pids: Vec<u32> = processes.iter().map(|p| p.pid).collect();
        pids.sort_unstable();
        pids.dedup();
        assert_eq!(pids.len(), processes.len(), "snapshot pids must be unique");
        // Names are non-empty.
        assert!(processes.iter().all(|p| !p.name.is_empty()));
    }

    #[test]
    fn app_capture_rejects_pid_zero() {
        let result = AppAudioCapture::start(0, Box::new(|_| {}));
        assert!(
            result.is_err(),
            "pid 0 is the idle process and uncapturable"
        );
    }

    #[test]
    fn app_capture_start_stop_round_trip_on_unknown_pid() {
        // An unknown-but-plausible pid must not panic: the thread starts,
        // fails to activate (or captures nothing), and stop() joins cleanly.
        // Use our own pid so the activation itself succeeds on systems with
        // process loopback support; a failure is logged, not panicked.
        let pid = std::process::id();
        let frames = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = frames.clone();
        let capture = AppAudioCapture::start(
            pid,
            Box::new(move |_| {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }),
        )
        .expect("capture thread must spawn for a valid pid");
        std::thread::sleep(std::time::Duration::from_millis(120));
        capture.stop();
        // Deterministic assertion: the thread exited without hanging. Frame
        // counts depend on whether this process emits audio, so only the
        // clean-join contract is verified here.
        let _ = frames.load(std::sync::atomic::Ordering::Relaxed);
    }
}
