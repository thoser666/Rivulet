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
- **Screen + Audio Recording (Linux GUI)** - Full recording flow in the GUI: monitor selection, start/stop, recording timer; system audio + microphone are captured and mixed into the MP4
- **Video Encoding** - H.264 encoding via FFmpeg
- **Audio Capture (engine, Linux)** - Desktop sound and microphone, mixed in real time via GStreamer (48 kHz stereo, per-source volume); usable via the `rivulet-audio` crate and the `record_screen_audio` example
- **Audio Mixer UI (Linux GUI)** - Start/stop audio capture with live level meter (dB), per-source volume sliders for system/mic
- **Separate Audio Tracks** - System and microphone output as separate tracks in the MP4 (via the "Getrennte Tracks" option, Linux GUI); engine API `push_audio_track(frame, AudioTrack)`
- **RTMPS Streaming** - Live to Twitch, Kick, YouTube or any custom RTMP/RTMPS ingest (H.264+AAC over FLV); `StreamSettings` presets + `set_stream_settings` engine API; example `stream_rtmps`
- **Dual Output** - Record locally and stream simultaneously; once encoded, split via `tee` into the MP4 file and the FLV/RTMPS sink (enabled by configuring both a local recording and stream settings)
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

### 🚧 In Development (v0.2)

- **Hardware Encoding**
  - NVIDIA NVENC
  - Intel QuickSync
  - AMD AMF
  - Auto-detection of best encoder

### 📅 Planned Features

See [Roadmap](#-roadmap) for detailed timeline.

---

## 🚀 Roadmap

> **Ziel:** Feature-Parität mit der aktuellen OBS-Version.
> **Strategie:** Von der stabilen Aufnahme-Basis schrittweise zu Szenen, Streaming und schließlich dem vollständigen OBS-Feature-Set.

### Meilenstein-Übersicht

| Meilenstein | Fokus | Status |
| --- | --- | --- |
| M0 – Recording Foundation | Aufnahme, Encoding, Audio, GUI | ✅ Erreicht |
| M1 – Solid Recording | Audio-Tracks, Hardware-Encoding, QoL | 🚧 In Arbeit |
| M2 – Scenes & Composition | OBS-Kern: Szenen, Quellen, Übergänge | 📅 Geplant |
| M3 – Streaming | RTMP, Dual Output, Plattformen | 🚧 In Arbeit |
| M4 – Advanced Output | Virtual Camera, Replay Buffer, Filter | 📅 Geplant |
| M5 – Ecosystem & Parität | Plugins, Kompatibilität, Plattform-Parität | 📅 Geplant |

---

### ✅ M0 – Recording Foundation

**Status: Erreicht**

- [x] Screen-Capture (Monitor, Echtzeit)
- [x] H.264-Video-Encoding (FFmpeg/GStreamer)
- [x] Audio-Capture (System + Mikrofon, gemischt, 48 kHz Stereo)
- [x] Audio/Video-Synchronisation
- [x] Audio-Mixer-UI mit Live-Pegel-Meter und Volume-Slidern
- [x] GUI-Aufnahme mit Monitor-Auswahl und Aufnahme-Timer (Linux)
- [x] Grundgerüst: Engine, Recorder, Scene/Source-Modelle, Plugin-System (Stubs)

---

### 🚧 M1 – Solid Recording

**Status: In Arbeit**

**Audio**
- [x] Separate Audio-Tracks (System/Mikrofon)
- [ ] Audio-Filter (Noise Suppression, Kompressor, Limiter)
- [ ] Audio-Monitoring (Quellen-Vorschau)

**Performance**
- [ ] Hardware-Encoding (NVIDIA NVENC, Intel QuickSync, AMD AMF)
- [ ] Auto-Detection des besten Encoders mit Fallback
- [ ] Performance-Metriken (FPS, Encode-Last, Dateigröße)

**Quality of Life**
- [ ] Hotkeys (Aufnahme, Pause, Stumm)
- [ ] Aufnahme-Timer-Overlay und FPS-Counter
- [ ] Region-Capture und Mehrfach-Monitor-Auswahl
- [ ] Codec-Auswahl-UI (H264/H265/VP9)
- [ ] Preset-Management (1080p60, 720p30, ...)

**Ziel:** Hochwertiges Recording mit Audio als solide Basis für Scenes & Streaming.

---

### 🎨 M2 – Scenes & Composition

**Status: Geplant**

Das Kernkonzept von OBS: Szenen, Quellen und Übergänge.

- [ ] Szenen-Verwaltung (mehrere Szenen, Umschalten)
- [ ] Quellen (Display, Fenster, Bild, Text, Webcam, Stummschaltung)
- [ ] Quellen-Komposition (Ebenen, Position, Skalierung, Zuschneiden)
- [ ] Übergänge (Fade, Cut, Stinger)
- [ ] Overlays (Bild-in-Bild, Banner)
- [ ] Chroma Key / Green Screen
- [ ] Studio-Modus (Vorschau/Programm)

**Ziel:** Der komponierbare Arbeitsbereich, den OBS-Nutzer erwarten.

---

### 📡 M3 – Streaming

**Status: In Arbeit**

- [x] RTMP/RTMPS-Client-Implementierung (H.264+AAC, FLV-Muxing)
- [x] TLS-verschlüsseltes Streaming (RTMPS) mit Zertifikats-Validierung
- [x] Dual Output (Stream + Aufnahme parallel; einmal codiert, per `tee` aufgeteilt)
- [ ] Stream-Health und Netzwerk-Statistiken
- [ ] Adaptive Bitrate
- [x] Plattform-Integrationen (Twitch, Kick, YouTube via RTMPS; Custom)
- [ ] Stream-Key-Management und Stream-Presets
- [x] Custom-RTMP-Server-Support (beliebige `rtmp://`/`rtmps://`-URL)

**Ziel:** Live-Streaming zu den gängigen Plattformen.

---

### 🎥 M4 – Advanced Output & Capture

**Status: Geplant**

- [ ] Virtual Camera Output
- [ ] Replay Buffer / Instant Replay
- [ ] Browser Sources (CEF-Integration)
- [ ] Multi-Track-Audio-Export
- [ ] Video-Filter & Effekte
- [ ] Cloud-Integration (Cloud-Aufnahmen)

**Ziel:** Die erweiterten Ausgabe- und Produktions-Features von OBS.

---

### 🔌 M5 – Ecosystem & Plattform-Parität

**Status: Geplant**

- [ ] Plugin-System (native Rust-Plugins)
- [ ] OBS-Plugin-Kompatibilitätsschicht
- [ ] Mobile Companion App (Remote Control)
- [ ] Windows/macOS-Feature-Parität (aktuell vollständig nur auf Linux)
- [ ] Installer (Windows MSI, macOS DMG, Linux AppImage)
- [ ] Code-Signing und Auto-Update
- [ ] Telemetrie (opt-in, datenschutzfreundlich)
- [ ] Multi-Language-Support

**Ziel:** Vollständige OBS-Feature-Parität über alle Plattformen.

---

### 🎯 Feature-Paritäts-Checkliste (gegenüber OBS)

| OBS-Kategorie | Status |
| --- | --- |
| Capture-Quellen (Display, Fenster, Webcam) | Teilweise (Display/Fenster) |
| Szenen & Übergänge | Offen |
| Audio-Mixer (Quellen, Tracks, Filter) | Teilweise (Mixer, Separate Tracks) |
| Recording & Encoding | Teilweise (H.264-Software) |
| Streaming (RTMP, Plattformen) | Teilweise (RTMPS Twitch/Kick/YouTube) |
| Virtual Camera | Offen |
| Replay Buffer | Offen |
| Browser Sources | Offen |
| Studio-Modus & Hotkeys | Offen |
| Plugin-Ökosystem & OBS-Kompatibilität | Offen |
| Multi-Track-Audio | Teilweise (2 Tracks) |
| Plattform-Parität (Windows/macOS) | Offen |

---

## 🛠 Development

### Tests

Run the full test suite:

```bash
# Build tests and run them
cargo test --workspace

# Run tests for a single crate
cargo test -p rivulet-core
```

The tests cover the pure data structures (`AudioFrame`, `OutputSettings`, `RecordingSettings`, ...), the engine's recording pipeline (synthetic video + audio to MP4, including separate audio tracks verified via the GStreamer Discoverer), the encoder/recorder lifecycle, and audio configuration validation. Building and running the tests requires GStreamer (dev packages + plugins) and, on Linux, the `LIBCLANG_PATH` environment variable.

### Linting & Formatting

```bash
# Check formatting
cargo fmt --all -- --check

# Lint the whole workspace (including tests and examples)
cargo clippy --workspace --all-targets -- -D warnings
```

> **Note:** On Linux, set `LIBCLANG_PATH` to your LLVM `lib` directory (e.g. `export LIBCLANG_PATH=/usr/lib/llvm-14/lib`) if the `clang-sys` build fails.

---

## 📦 Installation

### Prerequisites

**FFmpeg** must be installed and available in PATH:

#### Windows
```powershell
# Using Chocolatey
choco install ffmpeg

# Or download from: https://ffmpeg.org/download.html