<div align="center">

# 🌊 Rivulet

**Modern Screen Recording & Streaming Software**

[![CI](https://github.com/thoser666/rivulet/workflows/CI/badge.svg)](https://github.com/thoser666/rivulet/actions)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org)
[![Status](https://img.shields.io/badge/status-alpha%20v0.1-yellow.svg)](https://github.com/thoser666/rivulet)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/thoser666/rivulet)

*A complete Rust reimplementation of OBS Studio - built for performance, safety, and reliability*

[Features](#-features) • [Installation](#-installation) • [Roadmap](#-roadmap) • [Contributing](#-contributing)

![Rivulet Screenshot](docs/screenshot.png)
<!-- Add screenshot later -->

</div>

---

## 🎯 Vision

Rivulet aims to be a **complete reimplementation of OBS Studio in Rust**, providing all the features streamers and content creators need while leveraging Rust's safety and performance guarantees.

### Why Rust?
- 🔒 **Memory Safety** - No segfaults, no data races
- ⚡ **Performance** - Zero-cost abstractions
- 🛡️ **Reliability** - Catch bugs at compile time
- 🌍 **Cross-Platform** - Write once, run everywhere

### Long-term Goals (v1.0+)
- **Feature Parity** with OBS Studio
- **Plugin Compatibility** with existing OBS plugins
- **Modern Architecture** - Clean, maintainable codebase
- **Active Community** - Open development, regular updates

---

## ✨ Features

### ✅ Currently Available (v0.1)

- **Screen Capture** - Capture your primary monitor in real-time
- **Video Encoding** - H.264 encoding via FFmpeg
- **Audio Capture (engine, Linux)** - Desktop sound and microphone, mixed in real time via GStreamer (48 kHz stereo, per-source volume); usable via the `rivulet-audio` crate and the `record_screen_audio` example
- **Live Preview** - See what you're recording as you record
- **Tab-Based Interface** - Clean, DaVinci Resolve-style UI
  - Record Tab - Main recording controls and preview
  - Settings Tab - All configuration options
  - About Tab - Project information
- **Customizable Settings**
  - Adjustable FPS (15-60)
  - Bitrate control (1-50 Mbps)
  - Custom output path with file picker
  - Auto-timestamped filenames
- **Cross-Platform** - Windows, macOS, and Linux support (via xcap)
- **Modern UI** - Clean interface built with egui

### 🚧 In Development (v0.2 - December 2025)

- **Audio Mixer UI**
  - Mixer UI with volume sliders for system/mic
  - Separate audio tracks (system/mic)
- **Hardware Encoding**
  - NVIDIA NVENC
  - Intel QuickSync
  - AMD AMF
  - Auto-detection of best encoder

### 📅 Planned Features

See [Roadmap](#-roadmap) for detailed timeline.

---

## 🚀 Roadmap

> **Current Version:** v0.1 (October 2025)  
> **Next Release:** v0.2 (December 31, 2025)

### Q4 2025 - v0.2: Audio & Performance 🔊

**Target: December 31, 2025** | **Status: In Progress**

**Audio Capture**
- [x] System audio capture (desktop/game sound)
- [x] Microphone audio capture
- [x] Audio/video synchronization
- [ ] Audio mixer UI with volume sliders
- [ ] Separate audio tracks (system/mic)

**Performance**
- [ ] Hardware encoding support
  - [ ] NVIDIA NVENC (H264/HEVC)
  - [ ] Intel QuickSync
  - [ ] AMD AMF
- [ ] Auto-detect best available encoder
- [ ] Fallback to software encoding
- [ ] Performance metrics display

**Quality of Life**
- [ ] Better error messages
- [ ] Recording time display
- [ ] FPS counter during recording
- [ ] Estimated file size calculator

**Goal:** Enable high-quality recording with audio for YouTube/Twitch content creators.

---

### Q1 2026 - v0.3: UX & Features 🎨

**Target: March 31, 2026**

**User Experience**
- [ ] Settings persistence (save/load config)
- [ ] Keyboard shortcuts
  - [ ] F9: Start/Stop recording
  - [ ] F8: Pause/Resume
  - [ ] F7: Mute microphone
- [ ] System tray integration
- [ ] Recording timer overlay
- [ ] Recent recordings list

**Features**
- [ ] Monitor selection (multi-monitor support)
- [ ] Region capture (select specific area)
- [ ] Codec selection UI (H264/H265/VP9)
- [ ] Audio level visualizer
- [ ] Preset management (1080p60, 720p30, etc.)
- [ ] Export settings presets

**Developer Experience**
- [ ] Update check system
- [ ] Crash reporting (opt-in)
- [ ] Beta testing program
- [ ] Documentation website

**Goal:** Feature-complete recording software with polished, professional UX.

---

### Q2 2026 - v0.5: Streaming 📡

**Target: June 30, 2026**

**Streaming Core**
- [ ] RTMP client implementation
- [ ] Stream health monitoring
- [ ] Adaptive bitrate
- [ ] Stream presets (Twitch, YouTube, Facebook)
- [ ] Network statistics display

**Advanced Features**
- [ ] Dual output (record + stream simultaneously)
- [ ] Basic overlays (text, images, webcam)
- [ ] Scene management (basic)
- [ ] Transition effects
- [ ] Stream delay/buffer

**Integrations**
- [ ] Twitch integration (OAuth, chat)
- [ ] YouTube Live integration
- [ ] Custom RTMP server support
- [ ] Stream key management

**Goal:** Enable live streaming to major platforms.

---

### Q3 2026 - v1.0: Production Release 🚀

**Target: September 30, 2026**

**Advanced Features**
- [ ] Multi-scene management
- [ ] Advanced overlays & scenes
- [ ] Source composition (layers)
- [ ] Audio filters (noise suppression, compressor)
- [ ] Multi-track audio export
- [ ] Replay buffer
- [ ] Instant replay

**Production Ready**
- [ ] Auto-update system
- [ ] Professional QA testing
- [ ] Complete documentation
- [ ] Video tutorials
- [ ] Community Discord/Forum
- [ ] Support ticketing system

**Polish**
- [ ] Installer (Windows MSI, macOS DMG, Linux AppImage)
- [ ] Code signing (Windows/macOS)
- [ ] Telemetry (opt-in, privacy-first)
- [ ] Marketing website
- [ ] Press kit

**Goal:** Public launch - Production-ready OBS alternative for content creators.

---

### Future (v2.0+) 🔮

**Advanced Features**
- [ ] Virtual camera output
- [ ] Browser sources (CEF integration)
- [ ] Plugin system (native Rust plugins)
- [ ] OBS plugin compatibility layer
- [ ] Cloud integration (cloud recordings)
- [ ] Mobile companion app (remote control)
- [ ] AI features (auto-framing, noise removal)
- [ ] Multi-language support
- [ ] Chroma key (green screen)
- [ ] Video filters & effects

---

## 📦 Installation

### Prerequisites

**FFmpeg** must be installed and available in PATH:

#### Windows
```powershell
# Using Chocolatey
choco install ffmpeg

# Or download from: https://ffmpeg.org/download.html