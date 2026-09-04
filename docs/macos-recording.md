# macOS Recording

macOS screen/window recording is implemented as part of the M5
Windows/macOS feature-parity work. The Record view on macOS captures a
monitor or window with **xcap** and pushes the frames into the same
GStreamer recording pipeline as Windows/Linux (H.264/H.265/VP9, container
selection, presets, region crop, overlay, replay buffer, NDI, auto-remux,
cloud upload, background finalization).

> **Scope note:** macOS recording is **video-only**. System-audio and
> microphone capture on macOS are not implemented yet — the Record view
> shows a hint to that effect. Live on-device verification also still needs
> a real Mac (the CI only compile-checks the macOS code paths).

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
   The recording timer runs, the live preview thumbnail updates, and the
   overlay (when enabled) is baked into the video.
6. Press **⏹ Stop recording**. The UI switches to **“Finalizing recording…”**
   immediately (the GStreamer teardown runs on a background thread, so the
   interface never freezes) and flips to **“Recording saved.”** once the
   file is finalized (including auto-remux and cloud upload, when enabled).

The **Record hotkey** (default F9) toggles the same flow. **Pause** (F10)
skips frames while the capture keeps running; **Mute** (F11) is a no-op on
macOS (no audio track yet).

## What is shared with the other platforms

Everything that is not capture itself is the common engine/GUI path:

- Codecs H.264/H.265/VP9, containers MP4/MKV/MOV/TS, presets and rate
  control (Settings → Recording).
- Region crop (monitor capture only), video effects, recording-file
  management (split by time/size, filename patterns, auto-record), replay
  buffer, NDI output.
- Background stop finalization with the visible status, error surfacing
  (e.g. “No frames received — recording stopped.”), and the per-frame
  capture drain that keeps the UI responsive.

## Implementation notes

- `RivuletApp` gains macOS-only state: `is_recording`, the xcap
  `monitors`/`windows` lists with their selected indices, and the frame
  channel `raw_rx`.
- `refresh_macos_sources()` re-enumerates monitors/windows (resetting stale
  selections) and is called on every Record-view draw.
- `start_macos_recording()` validates the source selection *before* opening
  the file dialog, applies the current recording settings to the engine,
  then spawns the xcap capture thread and starts the engine pipeline.
- `drain_macos_frames()` runs in the per-frame `ui()` tick (not in a
  blocking loop), so a slow capture can never freeze the UI.
- `stop_macos_recording()` signals the capture thread and uses the shared
  `begin_background_stop()`/`poll_stop_finalization()` machinery, giving the
  same “Finalizing…” → “Recording saved.” feedback as Windows/Linux.

## Testing

- macOS-gated GUI behavior tests cover the stop lifecycle (idle stop is a
  no-op; an active stop arms the background finalization and flips the
  status), stale-selection resets, and the no-source start guard.
- A platform-independent source-contract test pins that the Record view
  dispatches to the macOS drawer, refreshes the source lists, and drains
  frames from the per-frame update.
- The i18n keys (`mac_permission_hint`, `mac_video_only_hint`,
  `mac_source_monitor`, `mac_source_window`, `mac_none`) are part of the
  EN/DE parity check.