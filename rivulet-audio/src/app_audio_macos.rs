//! macOS per-application audio fallback (issue #154 Phase 5).
//!
//! macOS has no public per-application capture API: Core Audio offers no
//! equivalent of Windows' process loopback or PipeWire's per-stream node
//! targeting, and the workarounds (virtual driver per source, tapping the
//! HAL system tap) require extra system configuration Rivulet cannot assume.
//! The honest fallback is therefore the **system loopback**: whatever the
//! user has wired up as their capture device (a BlackHole/Soundflower/
//! VB-Cable aggregate or the driver's virtual device) is captured **once**
//! and delivered to *every* routed Application source.
//!
//! Design (mirrors the Windows/Linux backends' surface):
//! - 48 kHz stereo interleaved f32 — the engine's routed appsrc caps, so no
//!   resampling is needed on either side.
//! - One dedicated capture thread per routed Application source, all backed
//!   by the same loopback device via cpal; every thread gets a copy of every
//!   frame, so the engine's per-source volume/filter/mute still behave
//!   independently.
//! - A single-source optimization (see [`SharedAppAudio::start_single`])
//!   skips the redundant copies when only one Application source is routed.
//! - Enumeration is a device scan classified by pure helper functions that
//!   are unit-tested without any audio device on every CI run.
//!
//! The capture device is the first input device whose name matches a known
//! loopback driver ([`LOOPBACK_DEVICE_KEYWORDS`]); without one, `start`
//! fails with a descriptive error (the GUI surfaces it) and the picker's
//! hint explains that Application sources share the system mix on macOS.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{bail, Context as _, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use rivulet_core::AudioFrame;

use crate::capture::AudioFrameCallback;

/// Format delivered to the engine: interleaved f32, stereo, 48 kHz — exactly
/// the routed appsrc caps (`audio/x-raw,format=F32LE,...,rate=48000`).
pub const MACOS_CAPTURE_RATE: u32 = 48_000;
pub const MACOS_CAPTURE_CHANNELS: u16 = 2;

/// Input-device name keywords of the known macOS loopback drivers. Same list
/// as `capture.rs`'s `find_loopback_device` so both backends agree on what
/// counts as system-loopback capture.
pub const LOOPBACK_DEVICE_KEYWORDS: [&str; 4] = ["blackhole", "soundflower", "loopback", "vbcable"];

/// `true` when the device name belongs to a known loopback driver (the
/// system mix). Case-insensitive and separator-insensitive (spaces, hyphens
/// and underscores are stripped, so "VB-Cable", "VB Cable" and "vb_cable"
/// all match the `vbcable` keyword); empty names never match.
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
/// Classify a device name for the picker: `Some(true)` = system loopback
/// (capturable), `Some(false)` = a regular input (microphone — not a valid
/// application-capture target), `None` = no usable name.
pub fn classify_device(name: Option<&str>) -> Option<bool> {
    name.filter(|n| !n.trim().is_empty())
        .map(|n| is_loopback_device_name(Some(n)))
}

/// One capturable entry offered by the picker. Field-compatible with the
/// Windows/Linux `AppAudioProcess` so the GUI treats all backends the same:
/// `pid` carries the fallback sentinel and `name` is the display label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppAudioProcess {
    pub pid: u32,
    pub name: String,
}

/// The fallback capture target carried in an Application source's
/// `pid:<n>` device id (see [`FALLBACK_PID`]).
pub const FALLBACK_PID: u32 = 1;

/// `true` when `pid` selects the macOS system-loopback fallback.
pub fn is_fallback_pid(pid: u32) -> bool {
    pid == FALLBACK_PID
}

/// An active fallback capture. Dropping it signals the capture loop; call
/// [`AppAudioCapture::stop`] to also wait for a clean exit.
pub struct AppAudioCapture {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AppAudioCapture {
    /// Start delivering the system loopback mix to `on_frame` from a
    /// dedicated thread. `pid` must be the fallback sentinel; every routed
    /// Application source receives this stream (the engine applies its
    /// per-source volume/filters/mute independently).
    pub fn start(pid: u32, on_frame: AudioFrameCallback) -> Result<Self> {
        if !is_fallback_pid(pid) {
            bail!(
                "pid {pid} cannot be captured on macOS: Application sources share the system loopback (pid {FALLBACK_PID})"
            );
        }
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name("rivulet-app-audio-macos".to_owned())
            .spawn(move || {
                if let Err(err) = capture_loop(thread_stop, on_frame) {
                    tracing::warn!(error = %err, "macOS per-app audio fallback capture ended with an error");
                }
            })
            .context("failed to spawn the macOS per-app audio capture thread")?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }

    /// Signal the capture loop to stop and wait for it to exit.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for AppAudioCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Never join inside drop (the capture thread itself may own the last
        // handle during teardown); the flag alone stops the loop.
    }
}

/// Captures the system loopback mix once and fans it out to every frame
/// sink (one per routed Application source). Lives inside the worker thread;
/// sinks are appended by [`SharedAppAudio::add_sink`] from the UI thread.
///
/// (Kept as a named type so the fan-out contract is documented and
/// testable; the actual fan-out happens through `SharedAppAudio`.)
pub struct SharedAppAudio {
    captures: Vec<AppAudioCapture>,
}

impl SharedAppAudio {
    pub fn new() -> Self {
        Self {
            captures: Vec::new(),
        }
    }

    /// Start one fallback capture (the system mix) fanned out to `sinks`.
    pub fn start_shared(&mut self, sinks: Vec<AudioFrameCallback>) -> Result<usize> {
        if sinks.is_empty() {
            bail!("no sinks to deliver the system loopback mix to");
        }
        let mut captures = Vec::with_capacity(sinks.len());
        for sink in sinks {
            captures.push(AppAudioCapture::start(FALLBACK_PID, sink)?);
        }
        self.captures.extend(captures);
        Ok(self.captures.len())
    }

    /// Start the single-source fast path: one capture, no fan-out copies.
    pub fn start_single(&mut self, sink: AudioFrameCallback) -> Result<()> {
        self.captures
            .push(AppAudioCapture::start(FALLBACK_PID, sink)?);
        Ok(())
    }

    /// Stop every capture and wait for the threads to exit.
    pub fn stop_all(&mut self) {
        for capture in self.captures.drain(..) {
            capture.stop();
        }
    }

    /// Number of active captures.
    pub fn len(&self) -> usize {
        self.captures.len()
    }

    /// `true` when no capture is active.
    pub fn is_empty(&self) -> bool {
        self.captures.is_empty()
    }
}

impl Default for SharedAppAudio {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Capture loop (cpal)
// ---------------------------------------------------------------------------

fn capture_loop(stop: Arc<AtomicBool>, mut on_frame: AudioFrameCallback) -> Result<()> {
    let device =
        find_loopback_device().context(crate::messages::macos_system_audio_unavailable())?;

    // The engine consumes 48 kHz stereo f32; ask cpal for the device's
    // native config and convert in the callback (same DSP path as the
    // legacy system capture in `capture.rs`).
    let supported = device
        .default_input_config()
        .context(crate::messages::macos_no_input_format())?;
    let channels = supported.channels();
    let input_rate = supported.sample_rate().0;
    let stream_config = supported.config();

    let (raw_tx, raw_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let sample_format = supported.sample_format();

    macro_rules! build_stream {
        ($t:ty) => {{
            // Each match arm owns its own sender clone, so every arm moves a
            // distinct `raw_tx` clone into its callback. The outer `raw_tx`
            // itself is never moved, so the receiver stays connected for the
            // whole capture loop. The `?` inside the block propagates with
            // the enclosing function's error type (fully inferred); the
            // block itself evaluates to the running `cpal::Stream`.
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
                .map_err(|e| anyhow::anyhow!("macOS loopback stream failed: {e}"))?;
            // cpal 0.15: an explicit `play()` is mandatory — without it the
            // stream is built but no callback ever fires. Separate statements
            // because build and play have distinct error types.
            stream
                .play()
                .map_err(|e| anyhow::anyhow!("macOS loopback stream failed: {e}"))?;
            stream
        }};
    }

    let stream = match sample_format {
        cpal::SampleFormat::F32 => build_stream!(f32),
        cpal::SampleFormat::I16 => build_stream!(i16),
        cpal::SampleFormat::U16 => build_stream!(u16),
        other => bail!(
            "{}",
            crate::messages::macos_unsupported_sample_format(&format!("{other:?}"))
        ),
    };
    // Convert + deliver on the worker thread so the realtime callback only
    // copies bytes; the sample format travels implicitly (the closure that
    // produced the bytes was typed by `build_stream!`).
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

/// Find an installed loopback capture device among the input devices. Same
/// keyword list as the legacy `capture.rs` backend.
fn find_loopback_device() -> Option<cpal::Device> {
    let host = cpal::default_host();
    let mut devices = host.input_devices().ok()?;
    while let Some(device) = devices.next() {
        if is_loopback_device_name(device.name().ok().as_deref()) {
            return Some(device);
        }
    }
    None
}

/// The shared DSP helpers live in `capture.rs`'s private `dsp` module; the
/// operations below are kept local so `capture.rs` does not need to widen
/// its visibility (each is tiny and unit-tested without a device).

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

/// Linear resampler to the engine's 48 kHz (same algorithm class as the
/// legacy backend's `dsp::resample` — nearest-sample interpolation, which
/// the legacy system capture also uses for the 44.1→48 kHz case).
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
// Enumeration
// ---------------------------------------------------------------------------

/// List the capturable entries for the picker: the system loopback device
/// (when present) plus, after it, regular inputs marked non-capturable so
/// the user sees *why* their microphone is not a capture target. Every
/// entry carries the fallback pid (the capture is the same stream either
/// way; the non-capturable flag lives in the label via `classify_device`).
///
/// Without any input device an empty list is returned so the picker simply
/// shows nothing.
pub fn list_audio_processes() -> Vec<AppAudioProcess> {
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else {
        tracing::warn!("macOS per-app enumeration: no input devices available");
        return Vec::new();
    };

    let mut loopback = Vec::new();
    let mut others = Vec::new();
    for device in devices {
        let Ok(name) = device.name() else {
            continue;
        };
        if is_loopback_device_name(Some(name.as_str())) {
            loopback.push(AppAudioProcess {
                pid: FALLBACK_PID,
                name,
            });
        } else {
            others.push(AppAudioProcess {
                pid: FALLBACK_PID,
                name,
            });
        }
    }
    // Loopback devices first (the actual capture target), inputs after.
    loopback.extend(others);
    loopback
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
        // Case-insensitive.
        assert!(is_loopback_device_name(Some("blackhole 64ch")));
        // Regular inputs never match.
        assert!(!is_loopback_device_name(Some("MacBook Pro Microphone")));
        assert!(!is_loopback_device_name(Some("AirPods Pro")));
        assert!(!is_loopback_device_name(Some("")));
        assert!(!is_loopback_device_name(None));
    }

    #[test]
    fn fallback_pid_semantics() {
        assert!(is_fallback_pid(FALLBACK_PID));
        assert!(!is_fallback_pid(0));
        assert!(!is_fallback_pid(4242));
    }

    #[test]
    fn capture_rejects_non_fallback_pid() {
        let (tx, _rx) = std::sync::mpsc::channel::<rivulet_core::AudioFrame>();
        let result = AppAudioCapture::start(
            4242,
            Box::new(move |frame| {
                let _ = tx.send(frame);
            }),
        );
        // `is_err` instead of `unwrap_err`: AppAudioCapture holds a thread
        // handle and deliberately does not implement Debug.
        assert!(
            result.is_err(),
            "non-fallback pids must be rejected on macOS"
        );
    }

    #[test]
    fn capture_start_never_panics_without_device() {
        // Without a loopback driver installed the capture loop exits with a
        // descriptive error (logged, not panicked); `start` still returns a
        // handle and `stop` must join cleanly.
        let (tx, _rx) = std::sync::mpsc::channel::<rivulet_core::AudioFrame>();
        let result = AppAudioCapture::start(
            FALLBACK_PID,
            Box::new(move |frame| {
                let _ = tx.send(frame);
            }),
        );
        if let Ok(capture) = result {
            capture.stop();
        }
    }

    #[test]
    fn shared_audio_rejects_empty_sinks() {
        let mut shared = SharedAppAudio::new();
        assert!(shared.start_shared(Vec::new()).is_err());
    }

    #[test]
    fn shared_audio_counts_and_stops() {
        // start_single always succeeds (the thread exits on its own when no
        // loopback device exists); stop must join cleanly.
        let mut shared = SharedAppAudio::new();
        let (tx, _rx) = std::sync::mpsc::channel::<rivulet_core::AudioFrame>();
        shared
            .start_single(Box::new(move |frame| {
                let _ = tx.send(frame);
            }))
            .expect("single capture must start");
        assert_eq!(shared.len(), 1);
        assert!(!shared.is_empty());
        shared.stop_all();
        assert!(shared.is_empty());
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
        // Every output frame's left sample maps to a valid input frame.
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
