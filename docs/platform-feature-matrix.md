# Platform feature matrix (M5 exit evidence)

The M5 gate (`docs/milestone-quality-gates.md`) requires that Windows, macOS
and Linux expose **equivalent core workflows or clearly label platform-specific
limitations**, with a platform feature matrix as exit evidence. This document is
that matrix. It is a point-in-time snapshot of the documented feature state,
reviewed on **2026-09-05**.

Legend:

- ✅ — supported / fully provided (or the documented equivalent)
- ⚠️ — partial: works with a documented limitation or follow-up
- ✖️ — not available on this platform

| Feature / core workflow | Windows | Linux | macOS | Evidence & limitations |
| --- | :-: | :-: | :-: | --- |
| Monitor capture (source picker + preview) | ✅ | ✅ | ✅ | xcap backend on all platforms; Windows also uses `windows-capture` in the GUI. macOS requires the Screen Recording permission, Linux the display-server access (see Permissions). |
| Window capture | ✅ | ✅ | ✅ | Windows: Windows Graphics Capture / `windows-capture`; Linux X11: xcap (`xdotool` for enumeration, CI-verified); macOS: xcap. The new monitor-scoped window list and "keep the monitor on window pick" behaviour apply on all three. |
| Region capture | ✅ | ✅ | ⚠️ | Source picker exists for Windows & Linux (drag select + X/Y/W/H + full-monitor reset, `README.md`). macOS offers only **Monitor/Window** sources; the engine keeps monitor-only crop. |
| Game Capture | ✅ | ⚠️ | ✖️ | Windows: G2 DXGI Desktop Duplication, G3 Vulkan implicit layer, G4 OpenGL hook (all done). Linux: G6 PipeWire/xdg-desktop-portal (Wayland) and `xcap` composite (X11). macOS: no hook layer; windowed xcap capture is the fallback. `docs/game-capture-strategy.md`. |
| Screen recording permission flow | ✅ | ✅ | ✅ | Windows: system capture permission; Linux: Wayland portal/PipeWire, X11 running X server; macOS: System Settings → Privacy & Security → Screen Recording (hint shown in UI). |
| Recording pipeline (codecs, containers, presets, rate control) | ✅ | ✅ | ✅ | H.264/H.265/VP9, MP4/MKV/MOV/TS, presets, VBR/CQ/CQ-VBR, overlay, replay buffer, NDI, auto-remux, cloud upload, background finalization — shared engine/GUI path. `docs/macos-recording.md` lists them as common. |
| Hardware encoding detection + fallback | ✅ | ✅ | ⚠️ | GStreamer encoder detection (NVENC/QuickSync/AMF) with automatic software-x264 fallback on all platforms; actual device availability depends on the vendor plugin and driver. macOS verification not separately documented (relies on GStreamer plugins). |
| Microphone + system audio capture | ✅ | ✅ | ✅ | Linux: GStreamer; macOS: cpal mic + loopback driver (**BlackHole/Soundflower/VB-Cable**) for system audio; Windows: mixer UI with system/mic. |
| Separate system/microphone audio tracks | ✅ | ✅ | ✅ | Linux GUI option and macOS `push_audio_track` documented; Windows via the Mixer/`push_audio_track` path (`docs/user-guide.md`). |
| Audio filters (noise suppression, gate, compressor, limiter, expander, gain, 10-band EQ) | ✅ | ✅ | ✖️ | GStreamer elements (`webrtcdsp`, `audiodynamic`, `audioamplify`, `equalizer-10bands`) used by the stream pipeline; missing elements are skipped with a status + log. **macOS: explicitly not implemented yet** (documented follow-up, `docs/macos-recording.md`). |
| Per-source volume sliders + live monitoring | ✅ | ✅ | ✖️ | Mixer UI on Windows/Linux; **macOS: not implemented yet** (follow-up). |
| Recording live preview + performance metrics | ✅ | ✅ | ✅ | Throttled preview thumbnail; live FPS, encoder load, file size via GStreamer pad probes. |
| Streaming (RTMP/RTMPS, WHIP/SRT/RIST) + health stats | ✅ | ✅ | ✅ | M3 flow: FLV/RTMP(S) publish, stream health (`Connecting/Good/Warning/Poor`) with drop/sent counters. Multitrack protocol list per `docs/m3-streaming-completion-report.md`. |
| Hotkeys (global, remappable) + source delete | ✅ | ✅ | ✅ | F9/F10/F11/F12 defaults + `delete_source` on `Delete` (OBS 32.2 parity); global bindings + conflict-free remapping; delete stays in-app only (destructive guard) everywhere (`docs/hotkeys.md`). |
| Auto-update + installers | ✅ | ✅ | ✅ | MSI (+ portable ZIP) / AppImage / DMG built by CI; updater downloads the matching platform package; Windows uses the detached `rivulet-updater` watchdog. `docs/release-platforms.md`. |
| Internationalized UI | ✅ | ✅ | ✅ | Locale files (EN default, DE included), EN/DE parity check on every platform. |
| Activity status + Discord Rich Presence | ✅ | ✅ | ✅ | Privacy-safe status model; opt-out, non-blocking Discord adapter; no stream keys/URLs/paths/window titles. `docs/activity-status.md`. |
| OBS WebSocket remote control (Stream Deck / TouchPortal) | ✅ | ✅ | ✅ | Protocol-compatible v5 server bound to `127.0.0.1`. **Compatibility/risk boundary, not OBS equivalence** — `docs/obs-websocket.md`. |
| Scene composition (M2) + sources | ✅ | ✅ | ✅ | Scene manager, sources (image/text/webcam/browser/media/color/audio/capture), transitions, Studio Mode. |
| Accessibility + diagnostics | ⚠️ | ⚠️ | ⚠️ | CI-gated accessibility/screenshot contracts and locale checks exist; per-package accessibility and diagnostics review is part of the open M5 gate follow-ups (`docs/ui-audit.md`). |
| On-device verification | ✅ | ⚠️ | ⚠️ | Windows is the primary development target. Linux: CI windowed smoke on Xvfb. macOS: **compile-checked only** — live on-device recording needs a real Mac (documented scope note). |

## Explicit platform limitations (summary)

- **macOS** — no Game Capture hooks; no region source picker; audio **filters**,
  per-source volume sliders and live monitoring not implemented yet; system
  audio needs a loopback driver (BlackHole/Soundflower/VB-Cable); screen capture
  requires the Screen Recording privacy permission; live verification still
  needs a real Mac (CI compile-checks the paths and runs the audio-DSP unit
  tests).
- **Linux** — Wayland requires portal/PipeWire access and X11 needs a running X
  server; desktop audio capture needs a running PipeWire (else video-only);
  the hook-based Game Capture backends are Windows-only — the Linux fullscreen
  path is the compositor (G6).
- **Windows** — none of the core workflows are platform-limited; capture
  permission and anti-cheat-blocked paths surface an explicit "unsupported /
  blocked" status rather than silently failing.

Cross-reference: M5 gate `docs/milestone-quality-gates.md` ("Review trust,
permissions, installation, and cross-platform consistency"), release channels
`docs/release-platforms.md`, macOS recording `docs/macos-recording.md`, Linux
build requirements `docs/LINUX_BUILD.md`, game capture `docs/game-capture-strategy.md`.