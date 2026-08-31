<div align="center">

# 🌊 Rivulet

**Modern Screen Recording & Streaming Software**

[![CI](https://github.com/thoser666/rivulet/workflows/CI/badge.svg)](https://github.com/thoser666/rivulet/actions)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org)
[![Status](https://img.shields.io/badge/status-alpha%20v0.9-yellow.svg)](https://github.com/thoser666/rivulet/releases)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/thoser666/rivulet)

*A Rust recording and streaming engine — built for performance, safety, reliability, and embeddability*

[Features](#-features) • [Installation](#-installation) • [Bedienungsanleitung](https://github.com/thoser666/Rivulet/blob/develop/docs/user-guide.md) • [Hilfe](https://github.com/thoser666/Rivulet/blob/develop/docs/user-guide.md#hilfe-menü) • [Erster Stream](docs/first-stream-checklist.md) • [Roadmap](#-roadmap) • [Contributing](#-contributing)

![Rivulet](docs/thumbnail.png)

</div>

---

## 🎯 Vision

Rivulet is **not an OBS clone**. It is an **embeddable, deterministic recording & streaming engine written in Rust**, with a modern GUI on top. It provides the OBS core feature set (capture, encoding, audio, streaming, dual output) while turning OBS's architectural gaps into its own strengths: automation instead of interactivity, a library instead of a monolith, a modern render path instead of legacy OpenGL/D3D, and a stable plugin ABI instead of version-sensitive C DLLs.

### Why Rust?
- 🔒 **Memory Safety** - No segfaults, no data races
- ⚡ **Performance** - Zero-cost abstractions
- 🛡️ **Reliability** - Catch bugs at compile time
- 🌍 **Cross-Platform** - Write once, run everywhere

### 🧭 Positioning: OBS weaknesses as Rivulet strengths

| OBS strength | OBS weakness | Rivulet's answer |
| --- | --- | --- |
| Powerful, mature feature set | Interactive-first: no headless/CI/render-farm use, hard to automate | Deterministic pipeline, headless CLI, testable engine (M6) |
| Monolithic app + libobs | `libobs` is not a clean library API; embedding it into products is painful | `rivulet-core` as a normal, semver-stable crate (M7) |
| Plugin ecosystem | C/C++ plugins against the libobs ABI, version-sensitive, can crash the app | WASM plugin runtime + temporary OBS compat mode (M5) |
| Powerful renderer | OpenGL/D3D legacy, hard to modernize | WebGPU/Zero-copy from the ground up (M8) |
| Streaming foundation | Core is RTMP/FLV, low-latency protocols are fiddly | WebRTC/WHIP & SRT/RIST as first-class citizens (M3) |
| Windows-first | Uneven platform parity (macOS/Linux weaker) | Parity as a release blocker, not an afterthought (M5) |

### Long-term Goals (v1.0+)
- **Feature Parity** with OBS Studio (core feature set)
- **Temporary OBS plugin compatibility** as a bridge, long-term **WASM plugin ecosystem**
- **Modern Architecture** - Clean, maintainable codebase
- **Deterministic & Embeddable** - Engine as a library, headless-capable, CI-friendly
- **Active Community** - Open development, regular updates
- **Internationalization** - UI and docs are language-neutral; locale files drive all visible strings

---

## ✨ Features

### ✅ Currently Available

- **Screen Capture** - Capture your primary monitor in real-time
- **Window Capture** - Capture a single application window (games, etc.) in addition to full monitors, with a window picker in the GUI (Linux & Windows); the Record view provides a live thumbnail so the target can be verified before starting
- **Camera Capture** - Capture from USB/webcam devices via GStreamer's device provider API; dropdown selection of available cameras with configurable resolution and FPS; engine API `list_cameras()` / `start_camera_capture()`
- **Game Capture** - Capture a specific game window using Windows Graphics Capture (Windows) or xcap window capture (Linux), with a checkbox toggle and window picker in the GUI
- **Scene Organisation** - Folders (parent scenes), color coding, and search/filter for managing multiple scenes; engine API supports `parent` UUID and `color` u32 ARGB on `Scene` struct; engine API `list_game_windows()` / `start_game_capture()` (window listing uses `xdotool` on Linux)
- **Multi-Monitor Selection** - Pick any connected monitor from a dropdown instead of being limited to the primary display (Linux & Windows)
- **Region Capture** - Record a rectangular area of a monitor with an interactive drag selector over a live preview, plus precise X/Y/W/H inputs and a one-click "full monitor" reset (Linux & Windows)
- **Screen + Audio Recording (Linux GUI)** - Full recording flow in the GUI: monitor selection, start/stop, recording timer; system audio + microphone are captured and mixed into the MP4
- **Video Encoding** - H.264/H.265/VP9 encoding via GStreamer with codec selection UI; hardware-accelerated encoders (NVENC, QuickSync, AMF) with automatic detection and software fallback
- **Hardware Encoding** - NVIDIA NVENC (`nvh264enc`), Intel QuickSync (`qsvh264enc`) and AMD AMF (`amfh264enc`) with automatic detection of the best available encoder and fallback to software x264; engine API `set_video_encoder(VideoEncoder)` / `set_video_bitrate(kbps)`
- **Audio Capture (engine, Linux)** - Desktop sound and microphone, mixed in real time via GStreamer (48 kHz stereo, per-source volume); usable via the `rivulet-audio` crate and the `record_screen_audio` example
- **Audio Mixer UI (Linux GUI)** - Start/stop audio capture with live level meter (dB), per-source volume sliders for system/mic
- **Separate Audio Tracks** - System and microphone output as separate tracks in the MP4 (via the "Separate Tracks" option, Linux GUI); engine API `push_audio_track(frame, AudioTrack)`
- **RTMPS Streaming** - Live to Twitch, Kick, YouTube or any custom RTMP/RTMPS ingest (H.264+AAC over FLV); `StreamSettings` presets, engine APIs `set_stream_settings` + `start_streaming` (pure stream) or `start_local_recording` (dual output); example `stream_rtmps`
- **Dual Output** - Record locally and stream simultaneously; once encoded, split via `tee` into the MP4 file and the FLV/RTMPS sink (enabled by configuring both a local recording and stream settings)
- **Stream Health & Network Stats** - Live status (`Connecting`/`Good`/`Warning`/`Poor`) with sent/dropped frame counters, bitrate (kbps) and FPS over a sliding window; derived from drop ratio (>5% Poor, >1% Warning), throughput collapse and stalls; engine API `stream_stats()`, polled in `stream_rtmps`
- **Recording Performance Metrics** - Live FPS, encoder load (%) and output file size during a recording, measured via GStreamer pad probes (per-frame encode duration paired by PTS, filesink byte counter); engine API `recording_stats()`, shown in the GUI next to the recording timer
- **Hotkeys** - F9 toggles recording, F10 pauses/resumes, F11 mutes/unmutes audio; pause skips video frames while capture keeps running, mute drops audio frames; hotkey hints shown in the UI and on the Start Recording button
- **Recording Presets** - Resolution and FPS presets (Original, 1080p60, 1080p30, 720p60, 720p30, 480p30) with automatic videoscale/videorate insertion, per-preset bitrate defaults, and encoder-aware caps; engine API `set_preset(RecordingPreset)`, selectable in the GUI
- **Recording Overlay** - Burn-in timer (HH:MM:SS) and live FPS counter onto the recorded video via GStreamer `textoverlay`; toggle in the GUI, engine API `set_overlay_enabled(bool)` + `update_overlay_text(&str)`
- **Replay Buffer / Instant Replay** - Ring buffer of the last N seconds (H.264+AAC, keyframe-aligned) kept while recording; save the clip as a standalone MP4 via the F12 hotkey or the GUI button; engine API `set_replay_duration()` / `save_replay()`
- **Auto-Update** - Checks the GitHub Releases API on startup and manually for newer versions, downloads the matching platform package (MSI / AppImage / DMG) with a live progress bar and launches the installer; Windows upgrades retain one Control Panel entry and update the launcher target; the alpha channel keeps up with every feature push. On Windows a detached `rivulet-updater` watchdog takes over the install: it waits for the running GUI (and launcher) to fully terminate — so the executing files are no longer locked — and only then runs `msiexec` to completion. This prevents the classic "only the registry is updated but the old files stay" failure.
- **Recording Live Preview** - The Record view shows a throttled thumbnail of the selected monitor or window before recording and continues with the actual encoder-bound frames while recording; the panel reports whether it is ready, waiting for the first frame, or active
- **Sidebar Navigation** - Clean, DaVinci Resolve-style UI with a collapsible left sidebar (see [docs/ui-design.md](docs/ui-design.md))
  - Record - Main recording controls and preview
  - Mixer - Audio mixer with live level meter and volume sliders
  - Scenes - Scene composition (M2, planned)
  - Stream - Live streaming (M3, planned)
  - Assistant - AI chat assistant (M9, planned)
  - Settings - All configuration options (recording, codec, presets, hotkeys, replay buffer, language)
- **Customizable Settings**
  - Recording presets (1080p60, 720p30, ...) or original resolution
  - Codec selection (H.264, H.265, VP9)
  - Custom output path with file picker
  - Auto-timestamped filenames
- **Cross-Platform** - Windows, macOS, and Linux support (via xcap)
- **Modern UI** - Clean interface built with egui
- **Internationalized UI** - All visible strings are driven by locale files (English by default, German included)
- **Activity status** - The UI exposes a privacy-safe “Rivulet nimmt auf / streamt” activity model, ready for an optional Discord Rich Presence adapter; stream keys, URLs, paths, and window titles are never included

---

## 📖 Bedienungsanleitung und Wiki

Der zentrale [Rivulet User Guide](docs/user-guide.md) erklärt Installation, Navigation,
Aufnahme, Quellen, Live-Vorschau, Szenen, Streaming, Updates und Fehlerdiagnose.

Für den praktischen Einstieg gibt es die [Checkliste für den ersten Twitch-, YouTube-
oder Kick-Stream](docs/first-stream-checklist.md) sowie die technische [Stream-Setup-
Dokumentation](docs/stream-setup.md
- [UI-Smoke-Tests und Secret-Redaction](docs/ui-smoke-testing.md)). Dort ist auch der lokale RTMPS-Smoke-Test
mit seinen Grenzen beschrieben.

Für Einsteiger-How-tos, Plattformtipps, FAQ und Community-Workarounds ist das
[GitHub Wiki](https://github.com/thoser666/Rivulet/wiki) vorgesehen. Das Wiki ist
bilingual (English/Deutsch) und bietet auf jeder Kernseite einen Sprachumschalter.
Fehlende Sprachpaare werden automatisch durch den Workflow geprüft; der Ablauf ist in
[docs/wiki-translation-workflow.md](docs/wiki-translation-workflow.md) beschrieben. Die Grenzen
zwischen versionierter Repository-Doku und Wiki sind in
[`docs/wiki-content-policy.md`](docs/wiki-content-policy.md) beschrieben. Das
Wiki muss in den GitHub-Repository-Settings durch einen Administrator aktiviert
werden.

## 🚀 Roadmap

> **Goal:** OBS core parity *and* the architectural strengths OBS cannot offer (determinism, embeddability, modernity).
> **Strategy:** First a stable, embeddable engine, then scenes & streaming, then the differentiation features (M6–M9) as product identity.
> **Priority:** Gaming streamers are a primary audience — Game Capture (M2) and Replay Buffer (M4) are prioritized accordingly.
> 
> **#1 Priority**: Game capture (D3D/Vulkan/OpenGL hook, zero-overhead) — without this, Rivulet cannot replace OBS for gaming. Current: Window capture via WGC (Windows) / xcap (Linux) for gaming window selection. Roadmap: D3D11/D3D12 interposition (M3), Vulkan interposition (M3), OpenGL hook (M3) as architecture steps after M2.

> **UI development:** new features should follow the navigation structure and
> egui conventions in [`docs/ui-design.md`](docs/ui-design.md). Every milestone
> also uses the reusable [`docs/milestone-quality-gates.md`](docs/milestone-quality-gates.md)
> for UI, usability, accessibility, reliability, platform, and resource-efficiency review.
> The ongoing UI audit, its CI-gated accessibility/screenshot contracts, and the
> accessibility-scanning backlog are tracked in [`docs/ui-audit.md`](docs/ui-audit.md).
> The cross-cutting resource goal and measurement method are documented in
> [`docs/resource-efficiency-goal.md`](docs/resource-efficiency-goal.md).

### Milestone overview

| Milestone | Focus | Status | Target | Open issues |
| --- | --- | --- | --- | --- |
| M0 – Recording Foundation | Capture, Encoding, Audio, GUI | ✅ Done | — | [![M0](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fapi.github.com%2Frepos%2Fthoser666%2FRivulet%2Fmilestones%2F3&query=open_issues&label=M0&color=blue)](https://api.github.com/repos/thoser666/Rivulet/milestones/3) |
| M1 – Solid Recording | Audio tracks, Hardware encoding, QoL, Overlay | ✅ Done (milestone closed) | — | [![M1](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fapi.github.com%2Frepos%2Fthoser666%2FRivulet%2Fmilestones%2F4&query=open_issues&label=M1&color=blue)](https://api.github.com/repos/thoser666/Rivulet/milestones/4) |
| M2 – Scenes & Composition | Scenes, Sources, Game Capture, Scene Organisation, Transitions, Studio Mode | ✅ Complete (conditional UI/UX gate; milestone closed) | — | [![M2](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fapi.github.com%2Frepos%2Fthoser666%2FRivulet%2Fmilestones%2F1&query=open_issues&label=M2&color=blue)](https://api.github.com/repos/thoser666/Rivulet/milestones/1) |
| M3 – Streaming | RTMP/RTMPS, WebRTC/WHIP, SRT/RIST, Multitrack Video | ✅ Complete (conditional: live interop follow-ups) | — | [![M3](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fapi.github.com%2Frepos%2Fthoser666%2FRivulet%2Fmilestones%2F5&query=open_issues&label=M3&color=blue)](https://api.github.com/repos/thoser666/Rivulet/milestones/5) |
| M4 – Advanced Output | Virtual Camera, Replay Buffer, Filters, Formats | ✅ Complete (conditional: integration follow-ups; milestone closed) | — | [![M4](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fapi.github.com%2Frepos%2Fthoser666%2FRivulet%2Fmilestones%2F2&query=open_issues&label=M4&color=blue)](https://api.github.com/repos/thoser666/Rivulet/milestones/2) |
| M5 – Ecosystem & Parity | WASM Plugins, OBS Compat, Platform Parity | 🚧 In progress (installers, signing, i18n done) | — | [![M5](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fapi.github.com%2Frepos%2Fthoser666%2FRivulet%2Fmilestones%2F6&query=open_issues&label=M5&color=blue)](https://api.github.com/repos/thoser666/Rivulet/milestones/6) |
| M10 – Extensible UI & Plugins | Persisted layouts, view registry, declarative/WASM plugins, permissions | 📅 Planned | — | [![M10](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fapi.github.com%2Frepos%2Fthoser666%2FRivulet%2Fmilestones%2F11&query=open_issues&label=M10&color=blue)](https://api.github.com/repos/thoser666/Rivulet/milestones/11) |
| M6 – Automation & Determinism | Headless CLI, CI Rendering, Reproducible Pipelines | 📅 Planned | — | [![M6](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fapi.github.com%2Frepos%2Fthoser666%2FRivulet%2Fmilestones%2F7&query=open_issues&label=M6&color=blue)](https://api.github.com/repos/thoser666/Rivulet/milestones/7) |
| M7 – Embeddable Engine | Stable `rivulet-core` API, Docs, Tooling | 📅 Planned | — | [![M7](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fapi.github.com%2Frepos%2Fthoser666%2FRivulet%2Fmilestones%2F8&query=open_issues&label=M7&color=blue)](https://api.github.com/repos/thoser666/Rivulet/milestones/8) |
| M8 – Modern Architecture | WebGPU, Zero-copy, Compute Filters | 📅 Planned | — | [![M8](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fapi.github.com%2Frepos%2Fthoser666%2FRivulet%2Fmilestones%2F9&query=open_issues&label=M8&color=blue)](https://api.github.com/repos/thoser666/Rivulet/milestones/9) |
| M9 – AI Chat Assistant | Local-first LLM Chat Bot for Streamers | 📅 Planned | — | [![M9](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fapi.github.com%2Frepos%2Fthoser666%2FRivulet%2Fmilestones%2F10&query=open_issues&label=M9&color=blue)](https://api.github.com/repos/thoser666/Rivulet/milestones/10) |

---

### Beta-Gate

Beta is **not date-driven** — it is a manual, criteria-based decision. The
project leaves alpha and enters beta (tag `vX.Y.Z-beta.1`, see
[Releases](#releases)) only when **all** of the following verifiable criteria
are met:

| # | Criterion | Verifiable via |
| --- | --- | --- |
| 1 | **M1 – Solid Recording complete** | ✅ All M1 items checked (audio filters/monitoring, hardware encoding, hotkeys, timer overlay, region capture, codec UI, presets) |
| 2 | **M3 – Streaming complete** | All M3 checklist items checked (RTMP/RTMPS, dual output, adaptive bitrate, stream-key management, WebRTC/WHIP, SRT/RIST) |
| 3 | **Platform parity (M5 release blocker)** | Recording + streaming verified on **Windows and Linux**; macOS capture implemented and verified (currently open) |
| 4 | **Code-signing secrets configured** | `WINDOWS_CERT_*` and `MACOS_CERT_*`/`APPLE_*` secrets set in the repo so a beta build produces **signed** packages (the automation itself is already smoke-tested without secrets by `signing-e2e.yml`) |
| 5 | **CI fully green on `develop`** | All checks pass: build, full test suite, actionlint, ShellCheck, action-pin checks, asset-drift check |
| 6 | **No known release-blocking bugs** | No open issue marked as release blocker (the recording frame-loss and no-frame-timeout regressions are fixed) |

**Gatekeeper:** The beta tag is set manually (see the Beta/RC/stable channel
below). The roadmap checklist above is the source of truth for *when*; the
release channel only describes *how* the tag is built and published.

---

### ✅ M0 – Recording Foundation

**Status: Done**

- [x] Screen capture (monitor, real-time)
- [x] H.264 video encoding (FFmpeg/GStreamer)
- [x] Audio capture (system + microphone, mixed, 48 kHz stereo)
- [x] Audio/video synchronization
- [x] Audio mixer UI with live level meter and volume sliders
- [x] GUI recording with monitor selection and recording timer (Linux)
- [x] Skeleton: engine, recorder, scene/source models, plugin system (stubs)

---

### ✅ M1 – Solid Recording

**Status: Done**

**Audio**
- [x] Separate audio tracks (system/mic)
- [x] Audio filters (noise suppression, compressor, limiter)
- [x] Audio monitoring (source preview)

**Performance**
- [x] Hardware encoding (NVIDIA NVENC, Intel QuickSync, AMD AMF)
- [x] Automatic detection of the best encoder with fallback
- [x] Performance metrics (FPS, encode load, file size)

**Updates**
- **Auto-update (update check, download & install via GitHub Releases)** — Windows installs run through a detached `rivulet-updater` watchdog that waits for the app to exit before invoking `msiexec`, so updated files replace the locked executables instead of leaving the old build behind.
- [x] Code signing (signing automation present, secrets needed)

**Quality of Life**
- [x] Hotkeys (record, pause, mute)
- [x] Recording timer overlay and FPS counter
- [x] Region capture and multi-monitor selection
- [x] Codec selection UI (H264/H265/VP9)
- [x] Preset management (1080p60, 720p30, ...)

**Goal:** High-quality recording with audio as a solid foundation for scenes & streaming.

---

### 🎨 M2 – Scenes & Composition

**Status: Complete (conditional UI/UX gate)**

The core concept of OBS: scenes, sources, and transitions. Core scene
organisation, collections/profiles, duplication, and source transforms are now
implemented. Remaining M2 work is focused on composition UX, transitions,
and multi-view/projectors. Studio Mode is now implemented with separate Preview/Program roles and a transition-aware Take action.

**Priority for gaming streamers:**

*Game capture (D3D/Vulkan/OpenGL hook, zero-overhead) — #1 priority: without this, Rivulet cannot replace OBS for gaming. Split into work packages G1–G6:*
- [x] **G1 – Capture strategy spike** — evaluate the capture approach per graphics API (DXGI Desktop Duplication, Vulkan layer, wgl hook; OBS backends as reference) and define the overhead budget (<1% frame time) and abort criteria. *DoD: decision document in docs/ with per-API recommendation and feasibility assessment — [docs/game-capture-strategy.md](docs/game-capture-strategy.md).*
- [x] **G2 – Windows DXGI backend** — Desktop Duplication for fullscreen/exclusive fullscreen with Graphics Capture fallback; GPU-texture path instead of CPU copy; usable as a scene source. *Backend done: `rivulet-capture` gains a DXGI Desktop Duplication source (`DxgiDesktopDuplication`) with a zero-copy GPU-texture path (shared-handle hand-off, no CPU readback in the hot path), BGRA→RGBA readback for the compatibility path, output enumeration, and the strategy's abort/fallback rules (access-lost → re-create, protected content → explicit "not capturable", timeout → retry). Unit tests run on every platform; a live-DXGI smoke test runs locally (`cargo test -p rivulet-capture -- --ignored`). The Windows GUI probes the backend at recording start and shows the active backend (Desktop Duplication vs. Graphics Capture fallback, with reason) during the recording. Scene-source wiring follows with S1 (Source abstraction). DoD: DX9/11/12 fullscreen captured via zero-copy path, tests + docs.*
- [x] **G3 – Vulkan hook** — VK_LAYER-based capture (OBS vulkan-capture approach) for fullscreen Vulkan games. *Layer done: `rivulet-vulkan-layer` — cdylib that the Vulkan loader discovers via `VkLayer_rivulet_capture.json`. Intercepts `vkQueuePresentKHR` → staging buffer readback (image transition, `cmd_copy_image_to_buffer`, HOST_VISIBLE map). Ash 0.38 for raw Vulkan API access. Layer negotiates via `vkNegotiateLoaderLayerInterfaceVersion`, forwards all calls to next layer/ICD. Capture pipeline initialized on `vkCreateSwapchainKHR`, frames captured on each present. Layer activation via `VulkanLayerConfig` — sets `VK_LAYER_PATH` + `VK_INSTANCE_LAYERS` for child process or current process. Frames transferred via shared memory IPC (`capture_channel` — `FrameHeader` protocol, `ShmReader`/`ShmWriter`). `start_vulkan_layer_capture()` in `game_capture.rs` provides channel-based frame reading. The Windows GUI now prefers the layer for fullscreen game capture: when the layer's shared-memory channel is available the frames flow straight into the recording pipeline, otherwise it falls back to Windows Graphics Capture on the window and shows the active backend + fallback reason in the recording view. DoD: Vulkan fullscreen captured within the overhead budget, tests + docs — implementation complete (layer, GUI wiring, fallback status, tests); the *within-the-budget* number itself is measured by G5 and still open.*
- [x] **G4 – OpenGL hook** — wglSwapBuffers interception for OpenGL games. *Backend done: `rivulet-opengl-hook-dll` (cdylib) hooks `wglSwapBuffers` via IAT patching, reads the back buffer via GDI `GetDIBits`, and writes BGRA frames to shared memory. Host-side `rivulet-core/src/opengl_hook.rs` provides DLL injection config, SHM reader, and `start_opengl_hook_capture()` (same poll-loop pattern as the Vulkan layer). `BackendKind::OpenGLHook` in `rivulet-capture/src/backend.rs` with i18n keys (EN/DE) and GUI status display (green for active, orange for fallback). The Windows GUI tries Vulkan → DXGI → OpenGL hook → WGC in order. Unit tests for frame header, SHM name, backend status, i18n keys, and error display. DoD: the within-the-overhead number is measured by G5.*
- [x] **G5 – Performance verification** — FPS/frame-time comparison with vs. without capture, benchmarked against OBS, with a CI regression guard. *DoD: benchmark script in scripts/, overhead budget verified for all three backends — done: `rivulet-core/src/benchmark.rs` (percentile calculation, budget checker, CI report) + `scripts/g5-benchmark.py` (CI gate, smoke test, JSON report validation). CI regression guard wired into Lints job. All 12 unit tests pass. Actual GPU measurements require running on real hardware; the framework and CI gate are in place.* Resource-efficiency methodology and game-first targets are documented in [`docs/resource-efficiency-goal.md`](docs/resource-efficiency-goal.md).
- [x] **G6 – Linux fullscreen path** (optional) — PipeWire/portal capture on Wayland with X11 fallback. *DoD: fullscreen capture on Wayland and X11, tests + docs — done: `rivulet-capture/src/pipewire_portal.rs` (ashpd portal session + PipeWire frame capture), `BackendKind::PipeWirePortal` with i18n keys (EN/DE), GUI wires PipeWire portal as primary Linux capture with xcap fallback. 3 unit tests. All 11 capture tests + 16 i18n tests pass.*
- [x] Game capture (window-based) — Windows Graphics Capture (Windows) / xcap (Linux), with checkbox toggle and window picker in the GUI
- [x] **Game capture live preview** — show a live thumbnail of the selected game window in the GUI before recording starts, so the user can verify the correct window is targeted. *Done: `GamePreview` grabs a frame via xcap on window selection and refreshes every 500ms, displayed as an `egui::Texture` below the game-window picker (Linux + Windows). Also fixed the Source dropdown leaking the selected window title. The window list refreshes live while the picker is open (selection preserved by id) and a manual 🔄 button forces an immediate re-scan.*

*Sources (image, text, webcam, browser/embedded Chromium, media, color, per-app audio). Split into work packages S1–S8:*
- [x] **S1 – Source abstraction** — complete the Source trait, properties UI, and per-source transforms. *Done: `rivulet-core/src/source.rs` — `SourceKind` enum (Image, Text, Webcam, Browser, Media, Color, GameCapture, ScreenCapture, Audio), `Transform` struct (x/y/width/height/rotation/opacity), `Source` struct with kind/transform/visibility/locked/z_order, `SceneSource` for per-scene transform overrides, `SourceManager` for source CRUD + scene bindings + z-order reordering. 36 unit tests + 18 i18n tests pass. DoD met: any source type can be added to a scene with a configurable transform.*
- [x] **S2 – Image source** — PNG/JPEG/GIF, folder loop/slideshow. *DoD: static and slideshow image sources usable in scenes, tests + docs.* Done: `rivulet-core/src/image_source.rs` (10 unit tests, i18n keys EN/DE).
- [x] **S3 – Text source** — rich text, scrolling, outline/background. *DoD: styled text source usable in scenes, tests + docs.* Done: `rivulet-core/src/text_source.rs` (Rgba, FontWeight, TextAlign, ScrollDirection, TextSource with scrolling animation, 30 unit tests + i18n keys EN/DE).
- [x] **S4 – Webcam as scene source** — expose the existing camera capture as a scene source with a properties panel. *DoD: webcam selectable per scene with resolution/framerate settings, tests + docs.* Done: `rivulet-core/src/webcam_source.rs` (PixelFormat, Resolution, WebcamSource with resolution/framerate/mirror/timeout, 19 unit tests + i18n keys EN/DE).
- [x] **S5a – Browser spike** — evaluate platform webviews (WebView2 / WebKitGTK / WKWebView) and render into a GPU texture. *DoD: spike document with per-platform recommendation and a rendered-frame proof.* Done: `docs/browser-source-spike.md` (wry recommendation, GPU texture integration, risk analysis).
- [x] **S5b – Browser source** — URL, interaction, transparency, config UI. *DoD: portable browser-source contract, bounded input queue, RGBA frame hand-off, configuration panel, tests + docs.* Done: `rivulet-core/src/browser_source.rs` validates `http(s)`/`file` URLs, manages viewport/FPS/zoom/transparency/CSS, queues interaction events, validates rendered RGBA frames, and exposes a platform-neutral adapter trait for WebView2/WebKitGTK/WKWebView. The Scenes view provides the configuration panel; native webview adapter wiring remains an explicit follow-up.
- [x] **S6 – Media source** — video/audio file playback in scenes (loop, restart on scene entry, speed). *DoD: media files play in scenes with controls, tests + docs. Tracked in [#66](https://github.com/thoser666/Rivulet/issues/66).* Done: `rivulet-core/src/media_source.rs` (MediaType, PlaybackMode, MediaSource with speed/volume/loop/restart, 26 unit tests + i18n EN/DE).
- [x] **S7 – Color source** — solid-color background/banner. *DoD: color source usable in scenes, tests + docs. Tracked in [#67](https://github.com/thoser666/Rivulet/issues/67).* Done: `rivulet-core/src/color_source.rs` (ColorSource with hex parsing, opacity, scene-fill, 15 unit tests + i18n EN/DE).
- [x] **S8 – Audio sources** — per-app audio capture (Windows WASAPI loopback) and audio input/output device capture as scene sources. *DoD: per-app and device audio selectable per scene, tests + docs. Tracked in [#68](https://github.com/thoser666/Rivulet/issues/68).* Done: `rivulet-core/src/audio_source.rs` (AudioSourceKind: Application/Input/Output/Mixed, AudioSource with volume/mute, 16 unit tests + i18n EN/DE).
- [x] Sources — camera capture (webcam via GStreamer)

**GUI polish:**
- [x] **Glassmorphism panels** — semi-transparent top menu bar and left sidebar (`theme::glass_frame()`) with rounded corners, desktop shows through with frosted-glass effect
- [x] **Hover, focus & active accent strokes** — shared `theme::accent_button()` / `theme::paint_interaction_stroke()` helpers apply the palette accent (1.5px at 60% alpha on hover, 2px opaque on focus/press) without overriding egui's disabled visuals
- [x] **Preview fade-in animation** — `theme::preview_fade_alpha()` wrapping `ctx.animate_bool` for smooth game-preview tint fade-in

**Scene system:**
- [x] **Scene management** (multiple scenes, switching, add/rename/remove, switch-back history) — `SceneManager` in `rivulet-core/src/scene.rs`, live Scenes view in the sidebar
- [x] **Scene organisation** (folders, search/filter, color coding) — *top community request, active PRs in OBS; `Scene` model with parent UUIDs and ARGB color coding (folders via hierarchy), tested in `rivulet-core/src/scene.rs`*
- [x] **Undo/Redo** (Ctrl+Z / Ctrl+Y) — scene add/remove/rename/switch operations are undoable; toolbar buttons are disabled when no history is available
- [x] **Scene collections & profiles** — named collection/profile context plus JSON import/export are available in the Scenes view; selected recording presets/codecs are applied when recording starts, while profile-specific preset storage remains follow-up work. *Tracked in [#69](https://github.com/thoser666/Rivulet/issues/69).*
- [x] **Duplicate scene / source** — one-click scene duplication with fresh identity and copied properties, plus source duplication with no accidental bindings; scene-local binding duplication is available from the composition editor
- [x] **Scene hotkeys & auto-switching** — assign per-scene F1–F8 hotkeys and define window-title auto-switch rules; focus integration beyond title matching remains a follow-up
- [x] Sources — window capture (monitor + individual windows, Linux & Windows)
- [x] **Source composition** (layers, position, scaling, cropping — each scene stores its own layout per source; switching scenes auto-moves sources to their saved positions) — the Scenes view now provides a source/layer editor for per-scene transform, crop, visibility, lock, z-order, and duplication; native media rendering and transform copy/paste remain follow-up work
- [x] **Transitions** — Cut and configurable Fade transitions are selectable in the Scenes view, with a non-blocking progress indicator; stinger transitions remain open
- [x] **Scene overlays** — optional text overlay with localized controls in the Scenes view; image/PiP overlays remain follow-up work
- [x] **Chroma key / green screen** — per-source enablement and similarity/smoothness controls; renderer-specific activation remains documented as a follow-up
- [x] **Studio mode (preview/program)** — separate Preview and Program scene roles with a Take action using Cut/Fade transitions; composition edits target Preview while Program remains live
- [x] **Multi-view & projectors** — optional scene-grid mode and projector-preview lifecycle controls; native fullscreen second-display routing remains a platform follow-up
- [x] **Scene snapshot** — one-click export of the active scene or Studio Mode Preview as a PNG layout snapshot. Visible layers, scene-local transforms, opacity, visibility, z-order, collection, and profile are captured deterministically; native source-pixel rendering remains a follow-up when platform source renderers are connected. *Done: `rivulet-core/src/scene_snapshot.rs` + localized Scenes-view export button; unit tests cover ordering, visibility, dimensions, and alpha blending.*

**Goal:** The composable workspace OBS users expect.

**M2 completion gate:** ✅ Complete. The cross-platform UI/UX review followed [`docs/m2-ui-ux-review.md`](docs/m2-ui-ux-review.md) and the common/M2-specific criteria in [`docs/milestone-quality-gates.md`](docs/milestone-quality-gates.md). The recorded result is a conditional pass in [`docs/m2-ui-ux-review-report.md`](docs/m2-ui-ux-review-report.md): zero Blocker/Critical findings and zero open High findings. Remaining Medium follow-ups (native projector routing, native source rendering, and native browser adapters) are explicitly assigned to M5/M8 and are not hidden. Issue #75 was moved to M5 and closed as the native projector follow-up.

---

### 📡 M3 – Streaming

**Status: Complete (conditional)**

M3 is functionally complete: the roadmap checklist is 15/15, and the completion
gate is recorded in [`docs/m3-streaming-completion-report.md`](docs/m3-streaming-completion-report.md).
Remaining live-integration evidence (real SFU WHIP handshake, SRT/RIST receiver
interop, multitrack transport, VOD routing, NDI LAN, production resource baselines,
and OS-backed credential storage) is explicitly assigned to follow-up milestones and
must not be implied by beta parity.

- [x] RTMP/RTMPS client implementation (H.264+AAC, FLV muxing)
- [x] TLS-encrypted streaming (RTMPS) with certificate validation
- [x] Dual output (stream + recording in parallel; encoded once, split via `tee`)
- [x] Stream health and network stats (status via drop ratio/throughput, `stream_stats()` API)
- [x] Platform integrations (Twitch, Kick, YouTube via RTMPS; custom)
- [x] Custom RTMP server support (arbitrary `rtmp://`/`rtmps://` URLs)
- [x] **Stream key management and stream presets** — validated Twitch/YouTube/Kick/custom endpoints, masked key display, TLS enforcement for platform presets, and Low/Standard/High/Custom quality presets; keys are never exposed by the UI masking API
- [x] **Adaptive bitrate** — bounded policy applies changed values to the active GStreamer encoder, reducing bitrate on Poor health and recovering on Good health; network monitoring and cooldown remain follow-up
- [x] **Stream delay** — configurable, bounded per-target packet delay integrated into fan-out branches; reconnect-aware buffering and backoff remain follow-up
- [x] **Multitrack Video** — bounded multi-representation configuration (1–4 tracks) and pipeline metadata; per-track encoder/transport negotiation remains a follow-up
- [x] **WebRTC/WHIP** as a first-class protocol (ultra-low-latency, SFU-compatible) — SDP offer/answer signaling with secure endpoint validation; `WhipMediaSession` now owns the deterministic session lifecycle (`SdpOffer` H.264/Opus offer generation, `post_offer` exchange, answer application, `Idle→Negotiating→Live/Failed` state tracking, and HTTP `DELETE` teardown on the resource URL), fully covered by offline unit and local-HTTP-mock tests. The live SFU/ICE/DTLS/SRTP media handshake remains the final integration evidence. See [`docs/whip-strategy-spike.md`](docs/whip-strategy-spike.md).; vision-approved in [`docs/obs-vision-roadmap.md`](docs/obs-vision-roadmap.md)
- [x] **SRT/RIST configuration contract** — validated protocol-specific endpoints, bounded latency, passphrase validation/redaction, and sink-fragment generation; live GStreamer transport/interoperability remains open
- [x] **Multistreaming** — GStreamer fan-out with one named branch/sink per validated RTMP/RTMPS target, up to four named destinations, independent target states, retry-target selection, duplicate-name protection, and secret masking; production bus monitoring/retry scheduling remains follow-up work. *Tracked in [#70](https://github.com/thoser666/Rivulet/issues/70).*
- [x] **NDI output** — LAN contribution/monitoring configuration contract (`NdiOutput` on `rivulet-core`): validated source name/group, quote-escaping so a hostile name cannot break the pipeline string, an `ndisink` availability probe, and a deterministic GStreamer fragment. Off by default so a feed is never published unintentionally. Real NewTek NDI runtime/LAN interoperability remains an integration follow-up.
- [x] **VOD track** — separate copyright-safe audio track (`VodTrack` on `StreamSettings`) for the Twitch VOD workflow while streaming: deterministic `enabled`/`recorded` model that is only ever active when both are set, so it can never silently leak into the live ingest; unit tests confirm the `ivod` flag signal and off-by-default behavior. Actual per-track GStreamer routing into the muxed output remains an integration follow-up.

**Goal:** Live streaming to the common platforms *and* low-latency protocols as native citizens instead of RTMP legacy.

---

### 🎥 M4 – Advanced Output & Capture

**Status: 🚧 Done — milestone closed** (all M4 roadmap bullets implemented, tested and documented). Integration follow-ups remain open and are tracked openly: VST3 hosting (loading a plugin into the audio graph), actual S3 upload after stop, NDI/VOD-track GStreamer routing, and Windows/macOS platform parity (a release blocker, not an afterthought).

**M4 completion gate:** ✅ Complete (conditional). The output quality gate was recorded in [`docs/m4-output-quality-gate.md`](docs/m4-output-quality-gate.md) using the criteria in [`docs/milestone-quality-gates.md`](docs/milestone-quality-gates.md): zero Blocker/Critical findings; hardware-dependent resource-efficiency measurements are explicitly marked `BLOCKED`/`N/A` rather than hidden. The milestone [M4 on GitHub](https://github.com/thoser666/Rivulet/milestone/2) is `closed`.

- [x] **Replay buffer / instant replay** — *essential for gaming streamers (clip moments without recording the whole session); ring buffer (H.264+AAC) with instant replay save via F12 hotkey or GUI button*
- [x] **Virtual camera output — platform-neutral contract** — validated format/resolution/FPS configuration and explicit Starting/Running/Stopping/Unavailable/Error lifecycle; platform driver integration and consumer smoke tests remain open
- [x] **Video filters & effects** (color correction, blur, sharpen) — *`VideoEffects` (brightness/contrast/saturation/hue via `videobalance`, `gaussianblur`, `cas` sharpen) inserted between `videoconvert` and the encoder caps; availability-gated like the audio filters. LUT colour-grading and chroma-key refinement remain open (no portable `.cube`/`lut3d` path in core plugins; chroma key is available per source). Engine `set_video_effects()` + GUI section in the recording settings*
- [x] **Audio filters** (noise gate, compressor, limiter, expander, gain, 10-band EQ; noise suppression via webrtcdsp) — *`AudioFilters` extended with noise gate + expander (`audiodynamic mode=expander`), makeup gain (`audioamplify`, dB→linear) and a 10-band EQ (`equalizer-10bands`), on top of the existing `webrtcdsp` noise suppression, compressor and limiter; all availability-gated so a missing optional element is skipped instead of failing capture; per-source GUI toggles + sliders in the Mixer view*
- [x] **Audio ducking** (sidechain compression — lower music/crowd while the mic speaks) — *sidechain policy model with threshold/attenuation/attack/release + hysteresis (issue #79)*
- [x] **VST 3.x support** — *`VstPlugin`/`VstChain` config contract plus deterministic bundle discovery from the platform-standard VST3 search directories (Windows Program Files/AppData, macOS /Library + ~/Library, Linux /usr/lib + ~/.vst3): validated `.vst3` bundle paths, ordered per-track chain, availability probe — all testable without a plugin binary. Hosting (loading a VST3 module into the audio graph) is a documented integration follow-up*
- [x] **Master audio mix** (output VU meter, master volume control, monitoring) — *master volume applied to the whole mix after the sources are summed (a single output level on top of the per-source volumes), a labelled master-output VU meter showing the mixed level in dB, and per-source monitoring with its own volume; `AudioConfig::master_volume` + `set_master_volume()` on `rivulet-audio`, i18n EN/DE keys, GUI wiring on the Mixer view (Linux)*
- [x] **Additional recording formats (MKV, MOV, TS — crash-safe alternatives to MP4)** — *container selection on the recording pipeline (MP4 default preserves the codec-native muxer; MKV/MOV/TS opt into a crash-safe intermediate), GUI container picker, `RecordingContainer`/`RemuxPlan` on `rivulet-core`*
- [x] **Remux recordings** (MKV/MOV → MP4 remux after stop, automatic or manual). *Tracked in [#71](https://github.com/thoser666/Rivulet/issues/71).* — *validated `RemuxPlan`, `RemuxSettings` with `auto_remux_after_stop`, and GStreamer remux execution via `remux_to_mp4` after stop (element-availability gating); GUI toggle next to the container picker. See [docs/recording-formats.md](docs/recording-formats.md).*
- [x] **Recording file management** (split by time/size, filename patterns, auto-record alongside stream) — *`FileNamePattern` with validated `{name}/{date}/{time}/{seq}/{stream}` tokens, `SplitBy` time/size rules with `RecordingSession` part sequencing and `auto_record_with_stream`; GUI split/auto-record toggles and pattern-driven default filenames. Live splitting maps `SplitBy` onto `splitmuxsink` (`max-size-time`/`max-size-bytes`, `%02d` part numbering, crash-safe containers only). See [docs/recording-files.md](docs/recording-files.md).*
- [x] **Advanced rate control** (VBR, CQ, CQVBR, custom encoder options) — *`RateControl`/`RateControlMode` (CBR default, VBR, constant-quality CQ, CQ-VBR with a bitrate cap) plus free-form extra encoder options folded into the encoder `parse_launch` fragment; full property mapping for software x264 (`pass`/`quantizer`/`vbv-buf-capacity`) and NVENC (`rc-mode`/`max-bitrate`/`qp-const`/`const-quality`), graceful fallback to average bitrate for QSV/AMF/VP9/software x265; `engine.set_rate_control()` and a GUI section in the recording settings*
- [x] **Multi-track audio export** (system + microphone stored on separate tracks) — *`engine.set_separate_audio_tracks`/`set_audio_track_enabled` build one `avenc_aac` branch per enabled source into the shared muxer; per-track export toggles in the Mixer view that are decoupled from the live mix and gated on capture; `AudioTrack::label`/`i18n_key` for display*
- [x] **Cloud integration (cloud recordings)** — *S3-compatible `CloudRecording` contract (endpoint/bucket/region/prefix + credentials) with validation, deterministic object-key builder, secret masking (custom `Debug` never prints the access key) and off-by-default upload. Actual S3 `PUT` upload is a documented integration follow-up*

**Goal:** The advanced output and production features of OBS.

---

### 🔌 M5 – Ecosystem & Platform Parity

**Status: In progress**

- [ ] Plugin system (native Rust plugins)
- [ ] **WASM plugin runtime** (stable ABI, sandboxed: plugins can never crash the app; long-term target plugin model)
- [ ] **OBS plugin compatibility layer** — *temporary bridge*: opt-in "Compatibility Mode" (explicitly marked as unsafe), loads native libobs plugins (encoders/filters/sources without UI); UI plugins (Qt) are out of scope; the goal is migration to the WASM plugin system
- [ ] Mobile companion app (remote control)
- [ ] **obs-websocket compatibility** — Streamdeck/TouchPortal ecosystem remote control (bridge to the mobile companion API). *Tracked in [#72](https://github.com/thoser666/Rivulet/issues/72).*
- [ ] **Windows/macOS feature parity** (currently Linux-first; window capture on Windows exists, macOS still open) — as a release blocker, not an afterthought
- [x] Installers (Windows MSI, macOS DMG, Linux AppImage) — automated in CI
- [x] Code signing (signing automation present, secrets needed)
- [ ] Telemetry (opt-in, privacy-friendly)
- [ ] **Optional Discord Rich Presence adapter** — non-blocking activity updates using the Rivulet status model; explicit opt-out, no stream keys/URLs/paths/window titles, and graceful operation when Discord is unavailable. Planned after M5 ecosystem foundations.
- [ ] Multi-language support (locale files fully wired)
- [ ] **MIDI device support** (map controllers like Korg NanoKontrol to scene switches, volume faders, filter toggles) — *frequently requested for live production and music streams*
- [ ] **Global hotkeys & remapping UI** — OBS-style hotkey settings: per-action rebinding, global hotkeys that work while the app is unfocused
- [ ] **Multi-channel distribution rollout** — tracked in [`docs/release-platforms.md`](docs/release-platforms.md):
  - [x] **Stage 1 – GitHub Releases:** canonical artifacts, checksums, changelog, and updater source (already active; signing is enabled when release secrets are configured).
  - [ ] **Stage 2 – WinGet + Flathub:** first external channels after stable package identity, signing, Flatpak metadata, and review are complete.
  - [ ] **Stage 3 – Homebrew Cask + Steam:** macOS cask after notarization; Steam for Windows/macOS after beta stability, App/Depot setup, and SteamPipe verification.
  - [ ] **Stage 4 – Microsoft Store:** optional MSIX/Partner Center channel after the MSIX packaging decision.

**Goal:** OBS core parity across all platforms, with a plugin model structurally superior to OBS (WASM instead of a C ABI), plus OBS compat as a transition bridge.

---

### 🧩 M10 – Extensible UI & Plugin Platform

Details, guardrails, and the completion gate: [`docs/extensible-ui-roadmap.md`](docs/extensible-ui-roadmap.md) and [`docs/milestone-quality-gates.md`](docs/milestone-quality-gates.md).

The UI extensibility work is intentionally isolated from the streaming and capture
milestones. It starts with a versioned, persistable layout and view registry,
then adds declarative plugins before considering sandboxed WASM execution.

- [ ] **P1 – Persistable UI layout** — versioned workspace/layout state, safe defaults, migrations, and no secrets or runtime handles.
- [ ] **P2 – View registry** — stable view IDs for built-in and optional views; navigation, Help, and accessibility derive from the registry.
- [ ] **P3 – Declarative UI plugins** — manifest-defined panels, menu items, help pages, enable/disable state, and API compatibility.
- [ ] **P4 – Permission model** — explicit UI/network/filesystem/capture/audio/secrets capabilities; sensitive permissions denied by default.
- [ ] **P5 – Isolated plugin execution** — preferably WASM, with timeouts, resource limits, and crash isolation.
- [ ] **P6 – Plugin quality gate** — compatibility, migration, accessibility, UX, performance, security, and example-plugin checks.

**Definition of Done:** The versioned layout survives restart, built-in and plugin views use one stable registry, plugins cannot access secrets without explicit capability approval, and a failing plugin cannot terminate the GUI.

### 🤖 M6 – Automation & Determinism ("Render-First")

**Status: Planned**

*Differentiation pillar #1: OBS is interactive-first, Rivulet is deterministic.*

- [ ] Deterministic pipeline (controllable engine clock, reproducible output from the same inputs)
- [ ] Headless CLI: capture/rendering without a GUI (`rivulet record ...`), usable as binary and library
- [ ] CI-friendly rendering: generate video from code (Remotion approach, native in Rust) — e.g. batch creation, tests, per-frame screenshots
- [ ] Reproducible distribution inputs: deterministic packages, SHA-256 manifests, and post-publish verification for the M5 channel rollout (see [`docs/release-platforms.md`](docs/release-platforms.md))
- [ ] Pipeline inspector/diagnostics tooling (analogous to `gst-inspect`, `gst-launch`), embedded in the engine
- [ ] Deterministic tests as first-class citizens (golden-frame tests, exact PTS/DTS verification)

**Goal:** "Video from code" and reproducible capture pipelines — the reason a developer/team *cannot* use OBS but can use Rivulet.

---

### 📦 M7 – Embeddable Engine & API

**Status: Planned**

*Differentiation pillar #2: `rivulet-core` is a normal library, not a monolith with retrofitted API.*

- [ ] Stabilized public API for `rivulet-core` (semver 1.0, `#![warn(missing_docs)]`, crate-style types)
- [ ] Comprehensive API docs + examples (recording, streaming, dual output, encoder selection, frame streaming)
- [ ] In-process capture API: embed a recording feature in any Rust app (audio/video capture, encoding, file/stream)
- [ ] Feature detection and runtime diagnostics as an API (`detect_available_encoders()`, encoder fallback)
- [ ] Abstraction of capture backends (xcap, PipeWire, Metal/WGC) behind stable traits

**Goal:** "Recording you can embed into your product" — the use case OBS is architecturally not built for.

---

### ⚡ M8 – Modern Architecture

**Status: Planned**

*Differentiation pillar #3: a modern render path from the ground up instead of OpenGL/D3D legacy.*

- [ ] WebGPU renderer (wgpu) for preview, compositing, and effects
- [ ] Compute-based video/audio filters (chroma key, scaling, filters) instead of CPU/shader legacy
- [ ] Zero-copy/GPU-direct capture paths (DMA-BUF, WGC/NVENC-direct, Metal IO)
- [ ] Performance and footprint metrics (idle CPU/RAM, laptop battery) as CI checks
- [ ] Modern UI foundation (egui) consistent across all platforms

**Goal:** Lightweight and future-proof — technically ahead where OBS has to retrofit.

**Cross-cutting resource-efficiency goal:** Rivulet follows a game-first performance policy: capture and streaming should preserve CPU/GPU/memory headroom for the game, with p99 frame-time, idle CPU, memory stability, and fallback evidence measured rather than inferred. See [`docs/resource-efficiency-goal.md`](docs/resource-efficiency-goal.md).

---

### 🤖 M9 – AI Chat Assistant

**Status: Planned**

*Differentiation pillar #4: OBS has no native chat; streamers use cloud SaaS. Rivulet ships a local-first AI chat assistant.*

- [ ] Chat adapters: Twitch (IRC/WebSocket), Kick (WebSocket), YouTube (Live API)
- [ ] LLM provider abstraction — **local-first**:
  - Ollama (`http://localhost:11434`) as default
  - llama.cpp server / in-process (`llama_cpp2`)
  - OpenAI-compatible endpoints (cloud, optional)
- [ ] Personas: configurable system prompts per "personality" (TOML)
- [ ] Custom commands (`!so`, `!song`, `!setup`, ...) — deterministic, no LLM needed
- [ ] Context/memory: short chat history + streamer info
- [ ] Moderation: toxicity filter as a pre-check before responding
- [ ] Bot coexistence: detect other active bots in the same channel (e.g. the
      [Vivid](https://github.com/thoser666/Vivid) Android bot) and suppress
      double replies — shared reply cooldown, command-claiming, optional
      silent/observer mode
- [ ] GUI panel (chat + bot settings) wired through the i18n layer

**Goal:** A private, subscription-free, API-free AI chat assistant that runs fully locally — the counter-position to cloud chat bots like StreamChatAI.

> ⚠️ **Coexistence with Vivid:** Rivulet (desktop) and Vivid (Android) can be used at the same time — e.g. Vivid streaming from the phone while Rivulet runs on the PC. The Rivulet bot must therefore never fight with the Vivid bot for the same chat: no duplicate replies, coordinated `!`-commands, and a per-channel configurable bot identity.

---

### 🎯 Feature-Parity Checklist (vs. OBS)

| OBS category | Status |
| --- | --- |
| Capture sources (display, window, webcam) | Partial (display + window + region, multi-monitor) |
| Game capture (D3D/Vulkan/OpenGL hooks) | In progress (G2 DXGI done; G3 Vulkan layer + capture pipeline; G4 OpenGL hook; G5 perf verification done; G6 Linux PipeWire portal done) |
| Media & color sources | Done (S6 + S7 done; S5b browser source open) |
| Image & text sources | Done (S2 + S3 + S4 done) |
| Browser sources | S5b contract/configuration done; native WebView adapter follow-up |
| Audio sources (microphone, per-app, devices) | Done (S8 done) |
| Scenes & transitions | In progress (scene management, collections/profiles, duplicate scene, Cut/Fade transitions, Studio Mode; stinger/multi-view open, M2) |
| Scene collections & profiles | Partial (named collection/profile context; import/export and per-profile settings open, M2) |
| Studio mode & multi-view | Partial (Studio Mode done; multi-view/projectors open, M2) |
| Source transforms & composition | ✅ Done (M2 S1) |
| Hotkeys (incl. remapping, global) | Partial (record/pause/mute/save-replay; remapping planned M5) |
| Undo/Redo | Done (M2) |
| Audio mixer (sources, tracks, filters) | Partial (mixer, separate tracks, filters) |
| Video filters (color correction, LUT, blur, sharpen, chroma key) | Partial (M4: color correction/blur/sharpen done; LUT grading + chroma-key refinement open) |
| Audio filters (noise gate, compressor, limiter, expander, gain, EQ) | Done (M4: gate, compressor, limiter, expander, gain, 10-band EQ, NS) |
| Audio ducking (sidechain) | Done (M4 sidechain policy with hysteresis) |
| VST 3.x support | Partial (M4 config contract + bundle discovery; hosting open) |
| Recording & encoding | Partial (H.264/H.265/VP9, HW + SW) |
| Replay buffer | Done (ring buffer, F12 hotkey, save-to-MP4) |
| Remux & file management | Done (M4: remux after stop, split, patterns, auto-record) |
| Virtual camera | Partial (M4 platform-neutral contract + lifecycle; driver integration open) |
| Streaming (RTMP/RTMPS, platforms) | Partial (RTMPS Twitch/Kick/YouTube; validated presets and key handling) |
| Multistreaming | Partial (multi-target fan-out wired; per-target reconnect supervisor and UI remain open) |
| Adaptive bitrate | Partial (bounded policy implemented; live encoder reconfiguration remains open) |
| WebRTC/WHIP, SRT/RIST, NDI | WHIP session/lifecycle+DELETE; SRT/RIST + NDI config contracts done; live SFU/ICE/DTLS handshake + real NDI LAN interop open (M3) |
| VOD track | Done (config model clean; per-track mux routing follow-up, M3) |
| Multi-track audio | Partial (2 local tracks; VOD-track model in M3) |
| Plugin ecosystem & OBS compatibility | Open |
| obs-websocket / Streamdeck | Open (M5) |
| Mobile remote & MIDI | Open (M5) |
| Cloud & telemetry | Partial (M4 S3 contract + secret masking; live upload open) |
| Discord Rich Presence | Planned (optional, privacy-safe activity status; M5) |
| Multi-language support | Partial (DE/EN wired) |
| Platform parity (Windows/macOS) | Open |
| AI chat assistant | Open (M9) |

> The checklist is verified against the machine-readable OBS catalog
> [`scripts/obs-features.json`](scripts/obs-features.json) by
> [`scripts/check-parity-checklist.py`](scripts/check-parity-checklist.py) in CI —
> a new OBS capability without a matching checklist row fails the check.

---

## 🔄 OBS upstream feature monitoring

A weekly GitHub Actions workflow checks the latest OBS release notes for new feature candidates, evaluates their fit against Rivulet's machine-readable vision pillars, and publishes an advisory report plus a review queue in [`docs/obs-vision-candidates.md`](docs/obs-vision-candidates.md). Maintainer-approved candidates are recorded in the curated [`docs/obs-vision-roadmap.md`](docs/obs-vision-roadmap.md) before being promoted into the milestone roadmap. Run it locally with `python3 scripts/check-obs-upstream.py --self-test` or inspect [`docs/obs-upstream-check.md`](docs/obs-upstream-check.md). Candidates require maintainer review before changing the parity checklist; features that do not fit the vision are not added merely for parity.

## 🧪 UI smoke tests

The cross-platform headless UI smoke contract runs on Linux, Windows, and macOS and checks navigation, keyboard guards, GUI-visible diagnostics, screenshot invariants, secret redaction, focus/hover semantics, and theme-contrast hooks:

```bash
cargo test -p rivulet-gui --test ui_smoke
```

See [`docs/ui-smoke-testing.md`](docs/ui-smoke-testing.md) for the evidence workflow and the limitations of deterministic headless checks versus native visual/accessibility review. The optional activity-status contract is documented in [`docs/activity-status.md`](docs/activity-status.md).

## 📺 Streaming Example

The `stream_rtmps` example (`rivulet-audio`) captures the screen together with
mixed system/microphone audio and streams live via RTMPS to Twitch, Kick,
YouTube, or any RTMP server:

```bash
cd rivulet-audio

# Twitch (default) or YouTube
RIVULET_STREAM_KEY="your-stream-key" \
RIVULET_STREAM_SECS=60 \
cargo run --example stream_rtmps

# Kick requires the IVS ingest endpoint
RIVULET_STREAM_URL="rtmps://<id>.global-contribute.live-video.net/app" \
RIVULET_STREAM_PLATFORM=kick \
RIVULET_STREAM_KEY="your-stream-key" \
cargo run --example stream_rtmps

# Any RTMP/RTMPS server (e.g. local)
RIVULET_STREAM_URL="rtmp://127.0.0.1:1935/live" \
RIVULET_STREAM_PLATFORM=custom \
RIVULET_STREAM_KEY="test" \
cargo run --example stream_rtmps
```

**Environment variables:**

| Variable | Meaning | Required |
| --- | --- | --- |
| `RIVULET_STREAM_KEY` | Stream key from the platform dashboard | Yes |
| `RIVULET_STREAM_PLATFORM` | `twitch` (default) · `kick` · `youtube` · `custom` | No |
| `RIVULET_STREAM_URL` | Ingest URL (for `custom` and `kick`) | depends on platform |
| `RIVULET_STREAM_SECS` | Stream duration in seconds (default: 10) | No |
| `RIVULET_STREAM_RECORD_PATH` | Optional MP4 path for parallel recording (dual output) | No |

Every two seconds the example prints the live health status, e.g.
`[health] Good | 2500 kbps | 30.0 fps | 120 sent / 0 dropped`.

---

## 📦 Release platforms

GitHub Releases is Rivulet's canonical, signed distribution channel. The
recommended expansion path is **WinGet** for Windows and **Flathub** for Linux,
followed by **Homebrew Cask** for macOS and **Steam** for the gaming audience;
Microsoft Store/MSIX is a later option. All external channels must publish the
same version and checksums as GitHub Releases and should initially be limited
to beta/stable builds.

The manual, dry-run-first workflow
[Distribution Readiness](.github/workflows/distribution-readiness.yml)
checks release assets for each planned channel without submitting anything
externally. See [docs/release-platforms.md](docs/release-platforms.md) for
prerequisites and activation checklists.

---

## 🛠 Development

### Repository hygiene

Which file types and directories are handled exclusively via `.gitattributes`
and `.gitignore` (line endings, binary markers, tool caches, nested repos) is
normative in [`docs/repo-hygiene.md`](docs/repo-hygiene.md). Keep the working
tree clean before committing: never `git add -A`, stage only the files that
belong to a work package, and mark new binary assets in `.gitattributes`.

### Diagnostics and crash logs

Rivulet writes structured, ANSI-free logs to the per-user data directory under
`Rivulet/logs/`, using one file per local calendar day:

- Windows: `%LOCALAPPDATA%\\Rivulet\\logs\\rivulet-YYYY-MM-DD.log`
- Linux: `$XDG_DATA_HOME/Rivulet/logs/rivulet-YYYY-MM-DD.log` (or the platform data directory)
- macOS: the platform data directory under `Rivulet/logs/`

The default retention is 14 days. Set `RIVULET_LOG_RETENTION_DAYS` before
starting the app to change it; values below one are clamped to one day. Crash
reports are delimited by `===== RIVULET CRASH =====`, making them easy to find
and attach to bug reports. When reporting a crash, include the relevant daily
log and the app version, but remove personal paths or stream keys first. If the
app starts with an empty log, verify that you are inspecting today's file and
run once with `RUST_LOG=info`; see [docs/logging.md](docs/logging.md) for
startup troubleshooting and the logging fallback behavior. Windows packages
also include a dependency-free launcher that records pre-Rust failures and can
opt in to WER crash dumps with `RIVULET_ENABLE_CRASH_DUMPS=1`.



### Tests

Run the full test suite:

```bash
# Build tests and run them
cargo test --workspace

# Run tests for a single crate
cargo test -p rivulet-core
```

On Linux, the game-window enumerator has an additional opt-in integration test.
The CI provisions `xdotool` and an Xvfb test display and runs:

```bash
RIVULET_TEST_XDOTOOL=1 cargo test -p rivulet-core \
  list_game_windows_finds_the_ci_window -- --nocapture
```

It asserts that `list_game_windows()` returns a non-empty list containing the
known CI window. See [`docs/LINUX_BUILD.md`](docs/LINUX_BUILD.md) for a local
Xvfb setup. The CI Cargo cache stores registry data only; compiler artifacts in
`target/` are deliberately not cached to prevent stale `E0460` failures after
Rust or dependency updates.

The tests cover the pure data structures (`AudioFrame`, `OutputSettings`, `RecordingSettings`, ...), the engine's recording pipeline (synthetic video + audio to MP4, including separate audio tracks verified via the GStreamer Discoverer), the streaming pipelines (RTMPS/dual output parse and route audio correctly), stream health tracking (status derivation from drop ratio, bitrate/FPS collapse and stalls), the encoder/recorder lifecycle, the video encoder abstraction (element names, detection order, fallback, pipeline fragments, 4:2:0 input caps), the recording performance metrics (FPS estimation, encoder load from a sliding window, file byte accounting, reset semantics), the recording presets (resolution/FPS presets with caps and transform fragments), the recording overlay (textoverlay pipeline integration, enable/disable toggle, duration formatting), the scene system (Scene model with parent UUIDs and ARGB color coding; SceneManager with add/remove/rename, active-scene switching with undo history, and depth-first ordered listing), the updater (GitHub release parsing, version comparison, platform asset selection, HTTP fetch against a local test server) and the i18n layer (locale resolution and translation lookups), plus the release code-signing automation (`rivulet-core/tests/ci_signing.rs` — verifies the signing scripts exist, that signing is gated on secret-presence outputs rather than raw secrets in `if:` conditions, and that the Windows MSI is signed after it is built). Building and running the tests requires GStreamer (dev packages + plugins) and, on Linux, the `LIBCLANG_PATH` environment variable. The hardware-encoding end-to-end test only runs when the matching GPU plugin is present.

### Linting & Formatting

```bash
# Check formatting
cargo fmt --all -- --check

# Lint the whole workspace (including tests and examples)
cargo clippy --workspace --all-targets -- -D warnings
```

The CI also runs [actionlint](https://github.com/rhysd/actionlint) to lint the
GitHub Actions workflow files and [ShellCheck](https://www.shellcheck.net/) on
the macOS packaging scripts. All third-party GitHub Actions are pinned to a
full commit SHA (with the upstream version in a trailing comment) so a
repointed tag or branch cannot swap in a malicious commit;
`rivulet-core/tests/ci_pinning.rs` enforces this and
[`docs/ci-action-pins.md`](docs/ci-action-pins.md) maps each pin to its
upstream version for reviewing Dependabot updates (its table is generated
from the workflows by `scripts/generate-action-pins.py`). Dependabot PRs are
approved and auto-merged via `.github/workflows/dependabot-auto-merge.yml`

Dependency update ownership is split intentionally: Renovate handles grouped
Cargo updates and the dependency dashboard (`renovate.json`), while Dependabot
handles GitHub Actions SHA pins. This avoids duplicate update PRs; all updates
remain subject to the required CI, security, and pinning checks.
once the required checks (including the pinning tests) pass; this needs
"Allow auto-merge" and branch protection enabled in the repo settings. A
daily nightly job (`scripts/check-action-pins.py`) also compares every pinned
SHA against upstream — failing on missed same-major updates and reporting
newer majors (`--fail-on-major` makes those fatal). `--json` gives a
machine-readable result and `--comment` a compact Markdown notification,
published to the nightly run's step summary.

### GitHub Security

The repository security baseline is enabled and enforced in two layers:

- **Secret Scanning + Push Protection:** enabled in the GitHub repository settings.
  Pushes containing detected credentials are blocked before they reach the
  repository. Signing secrets and stream keys must still remain in GitHub
  Actions Secrets and must never be printed to logs.
- **CodeQL:** `.github/workflows/security.yml` scans Rust on pushes to `develop`,
  pull requests, and the weekly scheduled run. Results are uploaded to GitHub
  Code Scanning.
- **Dependency Review:** the same workflow reviews pull-request dependency
  changes and fails on high-severity advisories; it posts a summary to the PR.
- **Cargo Audit + Cargo Deny:** the Security workflow checks RustSec advisories,
  yanked crates, licenses, dependency sources, and the checked-in `deny.toml`
  policy.
- **OpenSSF Scorecard:** `.github/workflows/scorecard.yml` evaluates supply-chain
  practices weekly and on repository changes, publishes the signed result, and
  uploads a separate SARIF category to Code Scanning.

The workflow actions are pinned to commit SHAs and covered by
`rivulet-core/tests/ci_pinning.rs`. Verify the repository-level settings with:

```bash
gh api repos/thoser666/rivulet --jq '.security_and_analysis'
```

The expected state is `secret_scanning.status=enabled` and
`secret_scanning_push_protection.status=enabled`. These settings are managed by
GitHub and cannot be represented fully in a repository file. See
[`docs/security.md`](docs/security.md) for incident handling and plan limitations.

Security vulnerabilities are handled via responsible disclosure, outlined in
[`SECURITY.md`](SECURITY.md): report privately through a security advisory,
never in a public issue.

The `develop` branch ruleset requires `CI`, `Security`, `OpenSSF Scorecard`,
`CodeQL (rust)`, `Dependency Review`, and `Pinning-Tests` before merge and blocks
direct updates for ordinary contributors. GitHub does not allow an Actions-only
bypass on this personal repository, so the active exception is the broader
administrator bypass needed by the release automation; see
[`docs/security.md`](docs/security.md) for details.

Beta-readiness is evaluated on every push by `scripts/check-beta-gate.py`
(CI job `Beta-Gate readiness`): it parses the roadmap checkboxes for M1/M3
and the 3-OS CI build matrix, and checks the signing secrets, the latest CI
run and open `release-blocker` issues via the GitHub API when a token is
available. The verdict is published to the run's step summary. It is
informational by default (exit 0 — the project is expected to be NOT READY
while it is still in alpha); `--fail` turns any unmet or unverified criterion
into exit 1 (e.g. as a gate for a beta tag), and `--json`/`--comment` give
machine-readable or Markdown output. Note that a workflow `GITHUB_TOKEN` can
never read Actions secrets, so the signing criterion stays `unverified` in
CI by design and is confirmed locally with a PAT that has secret read access.

### GUI

Run the desktop GUI with `cargo run -p rivulet-gui` (or the packaged
binary). If a recording receives no frames within a timeout (default
**5 seconds**) it is aborted automatically with an error in the UI — the
recording pipeline is only started by the first captured frame, so a
capture that silently delivers nothing would otherwise look like a
running recording while writing no file. On Windows the timeout only
applies before the first frame (a static screen can legitimately pause);
on Linux it also fires mid-recording, because xcap captures actively on
every loop iteration and a frame gap therefore means the capture thread
died or its source became unavailable. The timeout is configurable at
startup:

```bash
# wait 30 seconds for the first frame before aborting
cargo run -p rivulet-gui -- --no-frame-timeout 30
```

### Assets

Scene snapshots are exported from the Scenes view with **Save scene snapshot**. The export is a deterministic 1920×1080 RGBA8 PNG of the current scene layout: visible source layers are ordered by z-order and rendered with their transform and opacity. Since native source-pixel rendering is platform-specific and not yet connected to the scene compositor, source kinds use stable layout colors; this keeps exports reproducible and makes the remaining renderer integration explicit.

The README thumbnail (`docs/thumbnail.png`), the GitHub social preview
(`docs/social-preview.png`), the OpenGraph fallback (`docs/opengraph.png`), the
Linux AppImage icon (`packaging/rivulet.png`) and the macOS app icon
(`packaging/rivulet.icns`) are generated from the logo by
`scripts/generate-assets.sh` (requires ImageMagick and Python 3). Text uses the
committed DejaVu Sans fonts and the resize filter is pinned; because text
rasterization depends on the FreeType version, the canonical output is produced
with a pinned ImageMagick (`dpokidov/imagemagick:7.1.2-12`, pinned by content
digest `sha256:87998ec1…53cee1`). ImageMagick publishes no official container
image, so the image is multi-arch (amd64/arm64) and the digest pin guards
against tag mutation. The `scripts/generate-assets-docker.sh` wrapper runs that
pinned image for you via Docker or Podman, so no local ImageMagick
installation is needed:

```bash
scripts/generate-assets-docker.sh            # regenerate all assets
scripts/generate-assets-docker.sh --check    # regenerate + verify they match the committed files
```

CI runs the same pinned image and fails if the committed assets drift
(`scripts/check-assets.py`), so logo or generator changes must be committed
together with the regenerated output. There is no separate Python image:
`png2icns.py` and `check-assets.py` use only the Python standard library and
run under the `python3` installed inside the pinned ImageMagick container.

To activate the social preview, upload `docs/social-preview.png` as the
repository's *Social preview* image under **Settings → General → Social
preview** on GitHub; it is used when repository and release links are shared
(OpenGraph/`og:image`). `docs/opengraph.png` (1200×630) is the matching
fallback for X, Facebook and LinkedIn cards.

GitHub exposes no public API for that Settings upload, so the release workflow
attaches both images to every release instead: they are then available at a
stable URL (`releases/latest/download/opengraph.png`) that a website can
reference from its `og:image` meta tag.

> **Note:** On Linux, set `LIBCLANG_PATH` to your LLVM `lib` directory (e.g. `export LIBCLANG_PATH=/usr/lib/llvm-14/lib`) if the `clang-sys` build fails.

---

## 📦 Installation

### Prerequisites

**GStreamer** (Core + `gst-plugins-good`/`gst-plugins-bad`/`gst-plugins-ugly` + `gst-libav`) is required by the engine for encoding, audio mixing, and streaming (H.264 + AAC).

**FFmpeg** is only used by the legacy encoder in `rivulet-streaming` and must be available when that path is used:

#### Windows
```powershell
# Using Chocolatey
choco install ffmpeg

# Or download from: https://ffmpeg.org/download.html
```

---

## 📦 Releases

Releases are fully automated via GitHub Actions:

### Alpha channel (`release.yml`)

Every push with a `feat:` or `fix:` commit (Conventional Commits) to `develop`
produces a new alpha release:

| Commit type | SemVer bump | Example |
| --- | --- | --- |
| `fix(...)` | Patch | `0.20.0` → `0.20.1-alpha.1` |
| `feat(...)` | Minor | `0.20.0` → `0.21.0-alpha.1` |
| `feat!(...)` | Major | `0.x.y` → `1.0.0-alpha.1` |

1. **Versioning**: The next version is computed via SemVer from the `feat:`/`fix:`
   commits since the last tag (`scripts/release-version.sh`), plus `-alpha.N`
   (N = next free number). `Cargo.toml` and `CHANGELOG.md` are bumped and
   committed as `chore(release): prepare vX.Y.Z-alpha.N`.
2. **Tag**: `vX.Y.Z-alpha.N` is set and pushed.
3. **Binaries**: Three platforms (Linux x86_64, Windows x86_64, macOS
   aarch64) build release binaries from the tag.
4. **Packages**:
   - Linux: AppImage (`packaging/linux/build-appimage.sh`)
   - Windows: portable ZIP + MSI (`packaging/windows/`) — the GStreamer runtime
     is bundled, so the bundle runs without a GStreamer installation
   - macOS: DMG (`packaging/macos/build-dmg.sh`)
5. **Release**: A formal GitHub release (pre-release) with all artifacts
   is created.

Dependabot pushes are skipped (no release build for merged bot PRs).

> **Orphaned tags:** if a platform build fails, the tag is still pushed but no
> GitHub Release is created. `scripts/backfill-releases.sh --check` detects
> such orphaned tags and backfills them from the original workflow artifacts;
> see [docs/release-backfill.md](docs/release-backfill.md).

### Beta/RC/stable channel (`ci.yml`)

A manually set tag `vX.Y.Z` (without `-alpha.*`) builds the same packages and
publishes a GitHub release. Tags with `-beta.*`/`-rc.*` are marked as
pre-releases; stable tags as full releases.

Entering beta is gated on the [Beta-Gate](#beta-gate) criteria defined in the
[Roadmap](#roadmap): the tag may only be set once all six criteria are met.

### Code signing

Release packages are signed automatically when the matching secrets are
configured; without secrets, unsigned packages are built (default). Signing
runs only when *all* required secrets for a platform are present, so CI keeps
working for forks and for the unsigned development builds.

- **Windows** (`packaging/windows/sign.ps1`): the staged `rivulet-gui.exe` is
  signed *before* the portable ZIP and MSI are packaged (so the installer
  embeds the signed binary), and the resulting MSI is signed as well. Secrets:
  - `WINDOWS_CERT_BASE64` — base64-encoded `.pfx` code signing certificate
    (`certutil -encode cert.pfx cert.txt`, then paste the body without the
    `-----BEGIN/END CERTIFICATE-----` lines).
  - `WINDOWS_CERT_PASSWORD` — password of the `.pfx`.
- **macOS** (`packaging/macos/sign-notarize.sh`): the app bundle is codesigned
  with the hardened runtime, packaged into a DMG, then notarized and stapled
  via `notarytool`. Secrets:
  - `MACOS_CERT_BASE64` — base64-encoded `.p12` "Developer ID Application"
    certificate (`base64 -i cert.p12`).
  - `MACOS_CERT_PASSWORD` — password of the `.p12`.
  - `APPLE_ID` — Apple ID used for notarization.
  - `APPLE_APP_PASSWORD` — app-specific password for the Apple ID.
  - `APPLE_TEAM_ID` — Apple Developer Team ID.

Add the secrets under **Settings → Secrets and variables → Actions**.

### Setting up the signing secrets

All seven secrets are a [Beta-Gate](#beta-gate) criterion (criterion 4): CI
keeps working without them (unsigned packages are built), but a beta release
must be signed. Configure them in the repository settings
(**Settings → Secrets and variables → Actions**). The automation itself is
smoke-tested on every push without secrets
(`.github/workflows/signing-e2e.yml`), so the scripts are verified even before
real certificates exist.

#### Windows (`WINDOWS_CERT_*`)

1. **Obtain a certificate.** Buy a code-signing certificate from an issuer
   (DigiCert, Sectigo, GlobalSign, …). For a first smoke test you can create a
   self-signed one with PowerShell — note that Windows will not trust it
   (warning “unknown publisher”), so a purchased certificate is required for
   real users:
   ```powershell
   New-SelfSignedCertificate -Type CodeSigningCert -Subject "CN=Rivulet" `
     -CertStoreLocation Cert:\CurrentUser\My
   ```
2. **Export it to a `.pfx` file** (the `.pfx` password is stored as
   `WINDOWS_CERT_PASSWORD`):
   ```powershell
   $thumb = (Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert).Thumbprint
   $pwd = Read-Host "PFX password" -AsSecureString
   Export-PfxCertificate -Cert "Cert:\CurrentUser\My\$thumb" `
     -FilePath cert.pfx -Password $pwd
   ```
3. **Base64-encode the `.pfx`** into a single line:
   ```powershell
   [Convert]::ToBase64String([IO.File]::ReadAllBytes("cert.pfx"))
   ```
   (Alternatively `certutil -encode cert.pfx cert.txt` and paste the body
   without the `-----BEGIN/END CERTIFICATE-----` lines.)
4. **Create the secrets:** paste the base64 blob into `WINDOWS_CERT_BASE64`
   and the password into `WINDOWS_CERT_PASSWORD`. `packaging/windows/sign.ps1`
   decodes the blob, signs the staged `rivulet-gui.exe` before packaging, and
   signs the resulting MSI.

#### macOS (`MACOS_CERT_*` + `APPLE_*`)

1. **Obtain a certificate.** In the Apple Developer portal
   (developer.apple.com → Certificates) create a *Developer ID Application*
   certificate, download it and install it into the macOS Keychain.
2. **Export it to a `.p12` file**: Keychain Access → right-click the
   certificate → *Export…* → format *Personal Information Exchange (.p12)*.
   The export password is stored as `MACOS_CERT_PASSWORD`.
3. **Base64-encode the `.p12`** into a single line:
   ```bash
   base64 -i cert.p12
   ```
4. **Create the secrets:** `MACOS_CERT_BASE64` (the base64 blob) and
   `MACOS_CERT_PASSWORD` (the export password). `codesign-app.sh` imports the
   `.p12` into a temporary keychain and signs the app bundle with the hardened
   runtime.
5. **Notarization credentials** — create an *app-specific password* for your
   Apple ID (appleid.apple.com → Sign-In & Security → App-Specific Passwords):
   - `APPLE_ID` — your Apple ID (the sign-in email).
   - `APPLE_APP_PASSWORD` — the app-specific password (never your Apple ID
     password).
   - `APPLE_TEAM_ID` — your Team ID (developer.apple.com → Membership).
6. *(Optional)* if your certificate identity is not `Developer ID
   Application`, set `MACOS_SIGN_IDENTITY` accordingly (defaults to
   `Developer ID Application`).

#### Verifying

- `gh secret list` shows which names are configured.
- `scripts/check-beta-gate.py` (run with a token that can read secrets)
  reports exactly which of the seven secrets are still missing — criterion 4
  of the Beta-Gate.
- With all secrets present, the next release build signs the packages; without
  them, unsigned packages are produced (the default).


## Updater troubleshooting

See [docs/updater-troubleshooting.md](docs/updater-troubleshooting.md) for Windows installer exit codes, update crash diagnostics, and recovery steps.

During a Windows update, the app spawns the detached `rivulet-updater` watchdog (`rivulet-updater/src/bin/rivulet-updater.rs`), passing the running process IDs with `--wait-pid` and the downloaded MSI with `--install`. The watchdog copies itself to the temp directory (so the launcher's file lock never blocks it), waits up to 60s for each PID to exit, then runs `msiexec /i <msi>` and waits for it to finish. Because the old executables have fully exited before the WiX `MajorUpgrade` runs `RemoveExistingProducts`, the new files replace the old ones instead of the installer bailing on locked in-use files.
