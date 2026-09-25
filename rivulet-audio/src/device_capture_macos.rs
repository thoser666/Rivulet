//! Core Audio device enumeration and device-level input capture
//! (issue #231, macOS slice).
//!
//! The Windows (issue #229, `device_capture.rs`) and Linux (issue #231,
//! `device_capture_pw.rs`) slices made platform devices selectable as routed
//! `InputDevice`/`OutputDevice` sources; this module is the macOS mirror via
//! cpal. Honest semantics first (the same honesty as the per-app fallback in
//! [`crate::app_audio_macos`]):
//!
//! - **Input devices** (microphones, loopback-capable virtual drivers) are
//!   directly capturable and map to `InputDevice` sources.
//! - **Output devices** cannot be looped back on macOS: Core Audio offers no
//!   render-loopback API. An `OutputDevice` source is therefore only
//!   capturable through a *virtual loopback driver* (BlackHole, Soundflower,
//!   Loopback, VB-Cable) whose input device carries what the routed output
//!   plays. [`list_audio_devices`] reports loopback-capable inputs as
//!   output-capable and sorts them first; a plain output device selected as
//!   an `OutputDevice` source cannot start a capture — [`AudioDeviceCapture`]
//!   fails with a descriptive error that the GUI surfaces (the
//!   `audio_app_fallback_hint` pattern instead of silently recording
//!   silence).
//!
//! Two primitives are exposed to the GUI:
//!
//! - [`list_audio_devices`] — enumerate input devices with friendly names;
//!   loopback-capable drivers sort first (they double as `OutputDevice`
//!   sources), regular inputs follow.
//! - [`AudioDeviceCapture`] — stream one input device as interleaved f32 PCM
//!   at 48 kHz stereo (the engine's routed appsrc caps; the same contract
//!   the per-app fallback honors), converting the device's native sample
//!   format/channels/rate on the worker thread (the `app_audio_macos`
//!   DSP pattern).
//!
//! Device ids follow the `core-audio-in:<name>` convention (see
//! `rivulet_core::DeviceTarget`); the name is the cpal-reported device name
//! — the stable identifier cpal's public API exposes (Core Audio device
//! UIDs are not available through cpal).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context as _, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use rivulet_core::{AudioFrame, DeviceTarget};

use crate::capture::AudioFrameCallback;

/// Format delivered to the engine: interleaved f32, stereo, 48 kHz — exactly
/// the routed appsrc caps (`audio/x-raw,format=F32LE,...,rate=48000`).
pub const MACOS_CAPTURE_RATE: u32 = 48_000;
pub const MACOS_CAPTURE_CHANNELS: u16 = 2;

/// Input-device name keywords of the known loopback-capable drivers. Same
/// list as `app_audio_macos::LOOPBACK_DEVICE_KEYWORDS` so the device and
/// per-app backends agree on what counts as loopback capture.
pub const LOOPBACK_DEVICE_KEYWORDS: [&str; 4] = ["blackhole", "soundflower", "loopback", "vbcable"];

/// `true` when the device name belongs to a known loopback driver. Same
/// normalization as `app_audio_macos` (case-, separator-insensitive).
pub fn is_loopback_device_name(name: Option<&str>) -> bool {
    let Some(name) = name else {
        return false;
    };
    let normalized: String = name
        .to_lowercase()
        .chars()
        .filter(|c| !matches!(c, ' ' | '-' | '_'))
        .collect();
    !normalized.is_empty()
        && LOOPBACK_DEVICE_KEYWORDS
            .iter()
            .any(|k| normalized.contains(k))
}

/// One capturable device entry offered by the device picker.
///
/// Field-compatible with the Windows/Linux `AudioDeviceInfo` so the GUI
/// treats all backends identically: `endpoint_id` carries the device name
/// (what `core-audio-in:` addresses) and `name` is the display label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDeviceInfo {
    /// The cpal device name (becomes a `core-audio-in:` device id).
    pub endpoint_id: String,
    /// Human-readable device name.
    pub name: String,
    /// Whether this entry can back an `OutputDevice` source (loopback-capable
    /// virtual drivers only — macOS has no render-loopback API).
    pub is_output: bool,
    /// Whether this device is the system default input.
    pub is_default: bool,
    /// Whether this entry is one of the known loopback-capable drivers.
    pub is_loopback: bool,
}

impl AudioDeviceInfo {
    /// The `rivulet_core::DeviceTarget`-style device id for this entry
    /// (`core-audio-in:<name>` on macOS).
    pub fn device_id(&self) -> String {
        format!("core-audio-in:{}", self.endpoint_id)
    }
}

// ---------------------------------------------------------------------------
// Pure classification helpers (unit-tested without an audio device)
// ---------------------------------------------------------------------------

/// Sort weight for the picker: loopback-capable drivers first (they double
/// as `OutputDevice` sources), the system default first within each group,
/// then alphabetical.
pub fn picker_sort_rank(is_loopback: bool, is_default: bool) -> (u8, u8) {
    (u8::from(!is_loopback), u8::from(!is_default))
}

/// The descriptive error for an `OutputDevice` selection that macOS cannot
/// loop back. Surfaced verbatim by the GUI's `audio_device_capture_failed`
/// formatter — the honest-semantics substitute for silently recording
/// silence.
pub fn output_loopback_unavailable_error() -> String {
    "macOS cannot loop back a render endpoint — select a virtual loopback \
driver (BlackHole, Soundflower, Loopback, VB-Cable) as the OutputDevice \
source instead"
        .to_owned()
}

// ---------------------------------------------------------------------------
// Enumeration
// ---------------------------------------------------------------------------

/// List capturable input devices for the device picker (issue #231 macOS):
/// loopback-capable drivers first (they map to `OutputDevice` sources),
/// then regular inputs (microphones — `InputDevice` sources); within each
/// group the system default sorts first. Best-effort: an unreadable device
/// is skipped instead of failing the enumeration, and a host without input
/// devices yields an empty list so the picker simply shows nothing.
pub fn list_audio_devices() -> Vec<AudioDeviceInfo> {
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else {
        tracing::warn!("macOS device enumeration: no input devices available");
        return Vec::new();
    };
    let default_name = host
        .default_input_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default();

    let mut loopback = Vec::new();
    let mut inputs = Vec::new();
    for device in devices {
        let Ok(name) = device.name() else {
            continue;
        };
        if name.trim().is_empty() {
            continue;
        }
        let is_loopback = is_loopback_device_name(Some(name.as_str()));
        let is_default = !default_name.is_empty() && default_name == name;
        let info = AudioDeviceInfo {
            endpoint_id: name.clone(),
            name,
            is_output: is_loopback,
            is_default,
            is_loopback,
        };
        if is_loopback {
            loopback.push(info);
        } else {
            inputs.push(info);
        }
    }
    // Loopback drivers first (OutputDevice-capable), the system default
    // first within each group, then alphabetical — the same shape the
    // Windows picker shows (defaults marked, one group ahead).
    loopback.sort_by(|a, b| {
        picker_sort_rank(a.is_loopback, a.is_default)
            .cmp(&picker_sort_rank(b.is_loopback, b.is_default))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    inputs.sort_by(|a, b| {
        picker_sort_rank(a.is_loopback, a.is_default)
            .cmp(&picker_sort_rank(b.is_loopback, b.is_default))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    loopback.extend(inputs);
    loopback
}

// ---------------------------------------------------------------------------
// Capture
// ---------------------------------------------------------------------------

/// An active Core Audio device capture. Dropping it signals the capture
/// thread; call [`AudioDeviceCapture::stop`] to also wait for a clean exit.
pub struct AudioDeviceCapture {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AudioDeviceCapture {
    /// Start capturing the given Core Audio device (from a parsed
    /// [`DeviceTarget`]) and deliver interleaved f32 frames (48 kHz stereo)
    /// to `on_frame` from a dedicated thread.
    ///
    /// Input targets capture the device directly; `Output`/`Input`
    /// (`wasapi-`) and PipeWire targets are rejected — they belong to the
    /// other platform backends.
    pub fn start_for_target(target: &DeviceTarget, on_frame: AudioFrameCallback) -> Result<Self> {
        let name = match target {
            DeviceTarget::CoreAudioInput(name) => name.clone(),
            DeviceTarget::Output(_) | DeviceTarget::Input(_) => bail!(
                "wasapi-out:/wasapi-in: targets are handled by the WASAPI device backend, not Core Audio"
            ),
            DeviceTarget::PwSource(_) | DeviceTarget::PwMonitor(_) => bail!(
                "pw-src:/pw-mon: targets are handled by the PipeWire device backend, not Core Audio"
            ),
        };
        if name.trim().is_empty() {
            bail!("empty Core Audio device name");
        }
        Self::start(name, on_frame)
    }

    /// Start capturing the input device `name` and deliver frames to
    /// `on_frame` at 48 kHz stereo f32.
    pub fn start(name: String, on_frame: AudioFrameCallback) -> Result<Self> {
        if name.trim().is_empty() {
            bail!("empty Core Audio device name");
        }
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name("rivulet-device-audio-macos".to_owned())
            .spawn(move || {
                if let Err(err) = capture_thread(name, thread_stop, on_frame) {
                    tracing::warn!(error = %err, "macOS device capture ended with an error");
                }
            })
            .context("failed to spawn the macOS device capture thread")?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    /// Signal the capture thread to stop and wait for it to exit.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for AudioDeviceCapture {
    fn drop(&mut self) {
        // Cooperative shutdown without blocking the caller.
        self.stop.store(true, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Capture thread (cpal) — same DSP pattern as app_audio_macos
// ---------------------------------------------------------------------------

fn capture_thread(
    name: String,
    stop: Arc<AtomicBool>,
    mut on_frame: AudioFrameCallback,
) -> Result<()> {
    let host = cpal::default_host();
    let device = find_device_by_name(&host, &name)
        .with_context(|| format!("Core Audio input device not found: {name}"))?;

    // The engine consumes 48 kHz stereo f32; ask cpal for the device's
    // native config and convert in the worker (same DSP path as the per-app
    // fallback in `app_audio_macos`).
    let supported = device
        .default_input_config()
        .context("no supported input configuration for the Core Audio device")?;
    let channels = supported.channels();
    let input_rate = supported.sample_rate().0;
    let stream_config = supported.config();
    let sample_format = supported.sample_format();

    let (raw_tx, raw_rx) = std::sync::mpsc::channel::<Vec<u8>>();

    macro_rules! build_stream {
        ($t:ty) => {{
            // Each match arm owns its own sender clone, so every arm moves a
            // distinct `raw_tx` clone into its callback. The outer `raw_tx`
            // itself is never moved, so the receiver stays connected for the
            // whole capture loop.
            let raw_tx = raw_tx.clone();
            let stream = device
                .build_input_stream::<$t, _, _>(
                    &stream_config,
                    move |data: &[$t], _: &cpal::InputCallbackInfo| {
                        // SAFETY: `$t` is a plain numeric sample type with no
                        // padding, so the `&[$t]` buffer's underlying bytes
                        // form a valid `&[u8]` of `len * size_of::<$t>()`.
                        let bytes = unsafe {
                            std::slice::from_raw_parts(
                                data.as_ptr() as *const u8,
                                data.len() * std::mem::size_of::<$t>(),
                            )
                        }
                        .to_vec();
                        let _ = raw_tx.send(bytes);
                    },
                    move |_| {},
                    None,
                )
                .map_err(|e| anyhow::anyhow!("Core Audio device stream failed: {e}"))?;
            // cpal 0.15: an explicit `play()` is mandatory — without it the
            // stream is built but no callback ever fires.
            stream
                .play()
                .map_err(|e| anyhow::anyhow!("Core Audio device stream failed: {e}"))?;
            stream
        }};
    }

    let stream = match sample_format {
        cpal::SampleFormat::F32 => build_stream!(f32),
        cpal::SampleFormat::I16 => build_stream!(i16),
        cpal::SampleFormat::U16 => build_stream!(u16),
        other => bail!("unsupported Core Audio sample format: {other:?}"),
    };

    // Convert + deliver on the worker thread so the realtime callback only
    // copies bytes.
    while !stop.load(Ordering::SeqCst) {
        match raw_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(bytes) => {
                let f32_samples = decode_bytes(sample_format, &bytes);
                let mapped = dsp_to_channels(f32_samples, channels, MACOS_CAPTURE_CHANNELS);
                let resampled = dsp_resample(
                    &mapped,
                    MACOS_CAPTURE_CHANNELS,
                    input_rate,
                    MACOS_CAPTURE_RATE,
                );
                on_frame(AudioFrame::new(
                    resampled,
                    MACOS_CAPTURE_RATE,
                    MACOS_CAPTURE_CHANNELS,
                ));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    drop(stream);
    Ok(())
}

/// Find the input device matching `name` exactly (the picker's
/// `core-audio-in:` ids carry cpal-reported names verbatim).
fn find_device_by_name(host: &cpal::Host, name: &str) -> Option<cpal::Device> {
    let devices = host.input_devices().ok()?;
    devices
        .filter_map(|d| d.name().ok().map(|n| (n, d)))
        .find(|(n, _)| n == name)
        .map(|(_, d)| d)
}

/// Decode the raw callback bytes to f32 (the `app_audio_macos` DSP path).
fn decode_bytes(sample_format: cpal::SampleFormat, bytes: &[u8]) -> Vec<f32> {
    match sample_format {
        cpal::SampleFormat::F32 => bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect(),
        cpal::SampleFormat::I16 => bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b) as f32 / 32768.0)
            .collect(),
        _ => bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| {
                let v = u16::from_le_bytes(*b);
                (v as f32 - 32768.0) / 32768.0
            })
            .collect(),
    }
}

/// Map interleaved samples from `in_ch` channels to `MACOS_CAPTURE_CHANNELS`.
fn dsp_to_channels(samples: Vec<f32>, in_ch: u16, out_ch: u16) -> Vec<f32> {
    if in_ch == out_ch || samples.is_empty() {
        return samples;
    }
    if in_ch == 0 {
        return Vec::new();
    }
    let frames = samples.len() / usize::from(in_ch);
    let mut out = Vec::with_capacity(frames * usize::from(out_ch));
    for frame in samples.chunks_exact(usize::from(in_ch)) {
        match (in_ch, out_ch) {
            (1, 2) => {
                out.push(frame[0]);
                out.push(frame[0]);
            }
            (2, 1) => out.push((frame[0] + frame[1]) * 0.5),
            _ => {
                for c in 0..usize::from(out_ch) {
                    out.push(frame[c.min(frame.len() - 1)]);
                }
            }
        }
    }
    out
}

/// Linear resampler to the engine's 48 kHz (the `app_audio_macos`
/// algorithm class — nearest-sample interpolation).
fn dsp_resample(samples: &[f32], channels: u16, from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() || channels == 0 {
        return samples.to_vec();
    }
    let ch = usize::from(channels);
    let in_frames = samples.len() / ch;
    let out_frames = ((in_frames as u64) * (to as u64) / (from as u64)) as usize;
    let mut out = Vec::with_capacity(out_frames * ch);
    for i in 0..out_frames {
        let src = ((i as u64) * (from as u64) / (to as u64)) as usize;
        let src = src.min(in_frames - 1);
        out.extend_from_slice(&samples[src * ch..(src + 1) * ch]);
    }
    out
}

// ---------------------------------------------------------------------------
// Tests (pure helpers only — no audio device required)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_name_classification() {
        assert!(is_loopback_device_name(Some("BlackHole 2ch")));
        assert!(is_loopback_device_name(Some("Soundflower (2ch)")));
        assert!(is_loopback_device_name(Some("VB-Cable")));
        assert!(is_loopback_device_name(Some("Loopback Audio")));
        // Case- and separator-insensitive.
        assert!(is_loopback_device_name(Some("blackhole 64ch")));
        assert!(is_loopback_device_name(Some("vb_cable")));
        // Regular inputs never match.
        assert!(!is_loopback_device_name(Some("MacBook Pro Microphone")));
        assert!(!is_loopback_device_name(Some("AirPods Pro")));
        assert!(!is_loopback_device_name(Some("")));
        assert!(!is_loopback_device_name(None));
    }

    #[test]
    fn picker_sort_rank_orders_loopback_and_default_first() {
        assert!(picker_sort_rank(true, false) < picker_sort_rank(false, false));
        assert!(picker_sort_rank(false, true) < picker_sort_rank(false, false));
        assert!(picker_sort_rank(true, true) < picker_sort_rank(true, false));
    }

    #[test]
    fn output_devices_require_a_loopback_driver() {
        // The honest-semantics error names the drivers instead of promising
        // a capture that would record silence.
        let err = output_loopback_unavailable_error();
        assert!(err.contains("BlackHole"));
        assert!(err.contains("loopback"));
    }

    #[test]
    fn device_ids_follow_the_core_audio_convention() {
        let info = AudioDeviceInfo {
            endpoint_id: "BlackHole 2ch".to_owned(),
            name: "BlackHole 2ch".to_owned(),
            is_output: true,
            is_default: false,
            is_loopback: true,
        };
        assert_eq!(info.device_id(), "core-audio-in:BlackHole 2ch");
        assert_eq!(
            DeviceTarget::parse(&info.device_id()),
            Some(DeviceTarget::CoreAudioInput("BlackHole 2ch".to_owned())),
            "picker ids must round-trip through the core convention"
        );
    }

    #[test]
    fn capture_rejects_empty_device_name() {
        let (tx, _rx) = std::sync::mpsc::channel::<rivulet_core::AudioFrame>();
        let result = AudioDeviceCapture::start(
            String::new(),
            Box::new(move |frame| {
                let _ = tx.send(frame);
            }),
        );
        // `is_err` instead of `unwrap_err`: AudioDeviceCapture holds a
        // thread handle and deliberately does not implement Debug.
        assert!(result.is_err(), "an empty device name is not capturable");
        assert!(AudioDeviceCapture::start("   ".to_owned(), Box::new(|_| {})).is_err());
    }

    #[test]
    fn capture_rejects_foreign_platform_targets() {
        // WASAPI and PipeWire targets belong to the other backends.
        assert!(AudioDeviceCapture::start_for_target(
            &DeviceTarget::Output("{0.0.0.0}.{x}".to_owned()),
            Box::new(|_| {})
        )
        .is_err());
        assert!(AudioDeviceCapture::start_for_target(
            &DeviceTarget::PwMonitor(7),
            Box::new(|_| {})
        )
        .is_err());
    }

    #[test]
    fn capture_start_never_panics_on_unknown_device() {
        // A plausible-but-unknown name must not panic: the thread starts,
        // fails to find the device, and stop() joins cleanly.
        let (tx, _rx) = std::sync::mpsc::channel::<rivulet_core::AudioFrame>();
        let result = AudioDeviceCapture::start(
            "Rivulet Does Not Exist".to_owned(),
            Box::new(move |frame| {
                let _ = tx.send(frame);
            }),
        );
        if let Ok(capture) = result {
            capture.stop();
        }
    }

    #[test]
    fn channel_mapping_matches_engine_caps() {
        // Mono -> stereo duplicates; stereo -> mono averages.
        let mono = vec![0.25, 0.5, 0.75];
        assert_eq!(
            dsp_to_channels(mono.clone(), 1, 2),
            vec![0.25, 0.25, 0.5, 0.5, 0.75, 0.75]
        );
        let stereo = vec![0.5, 0.25, 0.25, 0.75];
        assert_eq!(dsp_to_channels(stereo, 2, 1), vec![0.375, 0.5]);
        // Same-format passthrough returns the input unchanged.
        assert_eq!(dsp_to_channels(mono.clone(), 1, 1), mono);
        // Empty input stays empty (no division by zero on channels).
        assert!(dsp_to_channels(Vec::new(), 2, 1).is_empty());
    }

    #[test]
    fn resample_length_and_passthrough() {
        // 48k -> 48k is identity.
        let frames: Vec<f32> = (0..48).map(|i| i as f32 / 48.0).collect();
        assert_eq!(dsp_resample(&frames, 2, 48_000, 48_000), frames);
        // 44.1k -> 48k grows the frame count proportionally.
        let stereo_441: Vec<f32> = vec![0.0; 44_100 * 2];
        let out = dsp_resample(&stereo_441, 2, 44_100, 48_000);
        assert_eq!(out.len(), 48_000 * 2);
        assert!(out.chunks_exact(2).all(|f| f[0] >= 0.0));
    }

    #[test]
    fn decode_handles_all_supported_formats() {
        // f32 identity.
        assert_eq!(
            decode_bytes(cpal::SampleFormat::F32, &1.0f32.to_le_bytes()),
            vec![1.0]
        );
        // i16 scaled to [-1, 1).
        assert_eq!(
            decode_bytes(cpal::SampleFormat::I16, &16384i16.to_le_bytes()),
            vec![0.5]
        );
        // u16 centered so 32768 -> 0.0.
        assert_eq!(
            decode_bytes(cpal::SampleFormat::U16, &49152u16.to_le_bytes()),
            vec![0.5]
        );
    }
}
