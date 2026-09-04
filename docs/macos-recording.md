# macOS Recording

macOS screen/window recording is implemented as part of the M5
Windows/macOS feature-parity work. The Record view on macOS captures a
monitor or window with **xcap** and pushes the frames into the same
GStreamer recording pipeline as Windows/Linux (H.264/H.265/VP9, container
selection, presets, region crop, overlay, replay buffer, NDI, auto-remux,
cloud upload, background finalization). Audio is captured with **cpal** and
recorded on **separate system/microphone tracks** — the same
`AudioTrack::System`/`AudioTrack::Microphone` path the Linux backend uses.

> **Scope note:** live on-device verification still needs a real Mac (the
> CI compile-checks the macOS code paths and runs the audio-DSP unit tests).
> Audio **filters** (noise suppression, compressor, EQ), per-source volume
> sliders and live monitoring are not implemented on macOS yet — they remain
> a documented follow-up.

## Getting started

1. Grant the **Screen Recording** permission:
   System Settings → Privacy & Security → Screen Recording → enable
   **Rivulet**, then quit and reopen Rivulet. Without the permission the
   monitor/window lists stay empty and the view shows a hint.
2. Open the **Record** view.
3. Pick a source: **Monitor** or **Window** (dropdowns list the detected
   sources with resolution/title).
4. Optionally choose a **Preset** and toggle the **Overlay (timer + FPS)**.
5. Press **⏺ Start recording**, choose the target file, and Rivulet records.
   The recording timer runs, the live preview thumbnail updates, the overlay
   (when enabled) is baked into the video, and audio is mixed in on separate
   system/mic tracks.
6. Press **⏹ Stop recording**. The UI switches to **“Finalizing recording…”**
   immediately (the GStreamer teardown runs on a background thread, so the
   interface never freezes) and flips to **“Recording saved.”** once the
   file is finalized (including auto-remux and cloud upload, when enabled).

The **Record hotkey** (default F9) toggles the same flow. **Pause** (F10)
skips video frames while capture keeps running (audio frames are discarded
while paused); **Mute** (F11) drops the audio frames without stopping the
capture.

## Audio capture

- **Microphone** — captured from the default input device (cpal). Works out
  of the box.
- **System audio (“what you hear”)** — macOS offers no public API to capture
  the system output directly, so Rivulet looks for an installed **loopback
  driver** among the input devices: **BlackHole**, **Soundflower** or
  **VB-Cable**. When one is found its stream is captured; when none is
  installed the Record view shows a warning and the microphone keeps working
  (system audio is silently skipped instead of failing the recording).
  Install BlackHole (`brew install blackhole-2ch`) and, if you want to hear
  your desktop while recording, route the system output to it via
  Audio MIDI Setup (Multi-Output Device).
- **Tracks** — system and microphone are delivered on separate channels and
  recorded as distinct audio tracks in the file (`push_audio_track`), the
  same as the Linux backend with separate tracks enabled.
- **Sample handling** — the device-native format (f32/i16/u16) is converted
  to interleaved `f32` in `[-1.0, 1.0]`, mapped to stereo (mono is
  duplicated, >2 channels keep the first pair) and resampled to the engine
  rate (48 kHz) with linear interpolation. These DSP steps live in
  `rivulet-audio::capture::dsp` and are unit-tested on every platform.

## What is shared with the other platforms

Everything that is not capture itself is the common engine/GUI path:

- Codecs H.264/H.265/VP9, containers MP4/MKV/MOV/TS, presets and rate
  control (Settings → Recording).
- Region crop (monitor capture only), video effects, recording-file
  management (split by time/size, filename patterns, auto-record), replay
  buffer, NDI output.
- Background stop finalization with the visible status, error surfacing
  (e.g. “No frames received — recording stopped.”), and the per-frame
  capture/audio drain that keeps the UI responsive.

## Implementation notes

- `RivuletApp` gains macOS-only state: `is_recording`, the xcap
  `monitors`/`windows` lists with their selected indices, the frame channel
  `raw_rx`, and the audio handle (`audio`, `audio_system_rx`,
  `audio_mic_rx`, `audio_preview`, `audio_warning`).
- `rivulet-audio` gains a macOS backend behind the same `AudioCapture` API:
  cpal streams for mic + loopback system source, per-source volumes applied
  on the samples, and a `system_source_note()` the GUI surfaces (found
  device or the “install BlackHole…” hint). The Linux GStreamer backend is
  unchanged; other platforms keep the stub error.
- `refresh_macos_sources()` re-enumerates monitors/windows (resetting stale
  selections) and is called on every Record-view draw.
- `start_macos_recording()` validates the source selection *before* opening
  the file dialog, enables the engine's separate audio tracks, starts the
  cpal capture, then spawns the xcap capture thread and starts the engine
  pipeline.
- `drain_macos_frames()` / `drain_macos_audio()` run in the per-frame
  `ui()` tick (not in a blocking loop), so a slow capture can never freeze
  the UI.
- `stop_macos_recording()` signals the capture thread, stops the audio
  capture and uses the shared `begin_background_stop()`/
  `poll_stop_finalization()` machinery, giving the same
  “Finalizing…” → “Recording saved.” feedback as Windows/Linux.

## Testing

- macOS-gated GUI behavior tests cover the stop lifecycle (idle stop is a
  no-op; an active stop arms the background finalization and flips the
  status), stale-selection resets, and the no-source start guard.
- A platform-independent source-contract test pins that the Record view
  dispatches to the macOS drawer, refreshes the source lists, drains frames
  and audio from the per-frame update, and that the macOS audio path goes
  through `push_audio_track` for both tracks.
- `rivulet-audio` unit tests (run on every platform) cover the sample-format
  conversion (f32/i16/u16), mono→stereo duplication, >2-channel
  downmixing, and linear resampling (identity, up, down, stereo
  interleaving); a macOS-gated test asserts `AudioCapture` constructs and
  reports a system-source note.
- The i18n keys (`mac_permission_hint`, `mac_audio_hint`,
  `mac_source_monitor`, `mac_source_window`, `mac_none`) are part of the
  EN/DE parity check.