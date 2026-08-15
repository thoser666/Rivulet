<div align="center">

# 🌊 Rivulet

**Modern Screen Recording & Streaming Software**

[![CI](https://github.com/thoser666/rivulet/workflows/CI/badge.svg)](https://github.com/thoser666/rivulet/actions)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org)
[![Status](https://img.shields.io/badge/status-alpha%20v0.1-yellow.svg)](https://github.com/thoser666/rivulet)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/thoser666/rivulet)

*A complete Rust reimplementation of OBS Studio - built for performance, safety, and reliability*

[Features](#-features) • [Installation](#-installation) • [Roadmap](#-roadmap) • [Releases](#-releases) • [Contributing](#-contributing)

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
- **Window Capture** - Capture a single application window (games, etc.) in addition to full monitors, with a window picker in the GUI (Linux & Windows)
- **Screen + Audio Recording (Linux GUI)** - Full recording flow in the GUI: monitor selection, start/stop, recording timer; system audio + microphone are captured and mixed into the MP4
- **Video Encoding** - H.264 encoding via GStreamer (`x264enc`, low-latency tuning)
- **Audio Capture (engine, Linux)** - Desktop sound and microphone, mixed in real time via GStreamer (48 kHz stereo, per-source volume); usable via the `rivulet-audio` crate and the `record_screen_audio` example
- **Audio Mixer UI (Linux GUI)** - Start/stop audio capture with live level meter (dB), per-source volume sliders for system/mic
- **Separate Audio Tracks** - System and microphone output as separate tracks in the MP4 (via the "Getrennte Tracks" option, Linux GUI); engine API `push_audio_track(frame, AudioTrack)`
- **RTMPS Streaming** - Live to Twitch, Kick, YouTube or any custom RTMP/RTMPS ingest (H.264+AAC over FLV); `StreamSettings` presets, engine APIs `set_stream_settings` + `start_streaming` (pure stream) or `start_local_recording` (dual output); example `stream_rtmps`
- **Dual Output** - Record locally and stream simultaneously; once encoded, split via `tee` into the MP4 file and the FLV/RTMPS sink (enabled by configuring both a local recording and stream settings)
- **Stream Health & Network Stats** - Live status (`Connecting`/`Good`/`Warning`/`Poor`) with sent/dropped frame counters, bitrate (kbps) and FPS over a sliding window; derived from drop ratio (>5% Poor, >1% Warning), throughput collapse and stalls; engine API `stream_stats()`, polled in `stream_rtmps`
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
- [x] Quellen — Fenster-Capture (Monitor + einzelne Fenster, Linux & Windows)
- [ ] Quellen (Bild, Text, Webcam, Stummschaltung)
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
- [x] Stream-Health und Netzwerk-Statistiken (Status via Drop-Ratio/Throughput, `stream_stats()`-API)
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
- [x] Installer (Windows MSI, macOS DMG, Linux AppImage) — automatisiert in CI
- [ ] Code-Signing und Auto-Update (Signing-Automatik vorhanden, Secrets nötig)
- [ ] Telemetrie (opt-in, datenschutzfreundlich)
- [ ] Multi-Language-Support

**Ziel:** Vollständige OBS-Feature-Parität über alle Plattformen.

---

### 🎯 Feature-Paritäts-Checkliste (gegenüber OBS)

| OBS-Kategorie | Status |
| --- | --- |
| Capture-Quellen (Display, Fenster, Webcam) | Teilweise (Display + Fenster) |
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

## 📺 Streaming-Beispiel

Das Beispiel `stream_rtmps` (`rivulet-audio`) erfasst den Bildschirm samt
gemischtem System-/Mikrofon-Audio und streamt live per RTMPS zu Twitch, Kick,
YouTube oder einem beliebigen RTMP-Server:

```bash
cd rivulet-audio

# Twitch (default) oder YouTube
RIVULET_STREAM_KEY="dein-stream-key" \
RIVULET_STREAM_SECS=60 \
cargo run --example stream_rtmps

# Kick benötigt den IVS-Ingest-Endpoint
RIVULET_STREAM_URL="rtmps://<id>.global-contribute.live-video.net/app" \
RIVULET_STREAM_PLATFORM=kick \
RIVULET_STREAM_KEY="dein-stream-key" \
cargo run --example stream_rtmps

# Beliebiger RTMP/RTMPS-Server (z.B. lokal)
RIVULET_STREAM_URL="rtmp://127.0.0.1:1935/live" \
RIVULET_STREAM_PLATFORM=custom \
RIVULET_STREAM_KEY="test" \
cargo run --example stream_rtmps
```

**Umgebungsvariablen:**

| Variable | Bedeutung | Pflicht |
| --- | --- | --- |
| `RIVULET_STREAM_KEY` | Stream-Key aus dem Plattform-Dashboard | Ja |
| `RIVULET_STREAM_PLATFORM` | `twitch` (Default) · `kick` · `youtube` · `custom` | Nein |
| `RIVULET_STREAM_URL` | Ingest-URL (für `custom` und `kick`) | je nach Plattform |
| `RIVULET_STREAM_SECS` | Stream-Dauer in Sekunden (Default: 10) | Nein |
| `RIVULET_STREAM_RECORD_PATH` | Optionaler MP4-Pfad für parallele Aufnahme (Dual Output) | Nein |

Alle zwei Sekunden gibt das Beispiel den Live-Health-Status aus, z. B.
`[health] Good | 2500 kbps | 30.0 fps | 120 sent / 0 dropped`.

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

The tests cover the pure data structures (`AudioFrame`, `OutputSettings`, `RecordingSettings`, ...), the engine's recording pipeline (synthetic video + audio to MP4, including separate audio tracks verified via the GStreamer Discoverer), the streaming pipelines (RTMPS/dual output parse and route audio correctly), stream health tracking (status derivation from drop ratio, bitrate/FPS collapse and stalls) and the encoder/recorder lifecycle. Building and running the tests requires GStreamer (dev packages + plugins) and, on Linux, the `LIBCLANG_PATH` environment variable.

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

**GStreamer** (Core + `gst-plugins-good`/`gst-plugins-bad`/`gst-plugins-ugly` + `gst-libav`) wird von der Engine für Encoding, Audio-Mixing und Streaming (H.264 + AAC) benötigt.

**FFmpeg** wird nur noch vom Legacy-Encoder in `rivulet-streaming` verwendet und muss verfügbar sein, wenn dieser Pfad genutzt wird:

#### Windows
```powershell
# Using Chocolatey
choco install ffmpeg

# Or download from: https://ffmpeg.org/download.html
```

---

## 📦 Releases

Releases werden vollständig automatisiert über GitHub Actions erzeugt:

### Alpha-Kanal (`release.yml`)

Jeder Push mit einem `feat:`-Commit (Conventional Commits) auf `develop` erzeugt
ein neues Alpha-Release:

1. **Versionierung**: Die nächste Version wird per SemVer aus den Commits seit
   dem letzten Tag berechnet (`scripts/release-version.sh`), plus `-alpha.N`
   (N = nächste freie Nummer). `Cargo.toml` und `CHANGELOG.md` werden gebumpt
   und als `chore(release): prepare vX.Y.Z-alpha.N` committet.
2. **Tag**: `vX.Y.Z-alpha.N` wird gesetzt und gepusht.
3. **Binaries**: Drei Plattformen (Linux x86_64, Windows x86_64, macOS
   aarch64) bauen Release-Binaries aus dem Tag.
4. **Pakete**:
   - Linux: AppImage (`packaging/linux/build-appimage.sh`)
   - Windows: Portable-ZIP + MSI (`packaging/windows/`) — GStreamer-Runtime wird
     mitgeliefert, sodass das Bundle ohne GStreamer-Installation läuft
   - macOS: DMG (`packaging/macos/build-dmg.sh`)
5. **Release**: Ein formelles GitHub-Release (Pre-Release) mit allen Artefakten
   wird erstellt.

Dependabot-Pushes werden übersprungen (kein Release-Build für gemergte
Bot-PRs).

### Beta/RC/Stable-Kanal (`ci.yml`)

Ein manuell gesetzter Tag `vX.Y.Z` (ohne `-alpha.*`) baut dieselben Pakete und
veröffentlicht ein GitHub-Release. Tags mit `-beta.*`/`-rc.*` werden als
Pre-Release markiert, stabile Tags als vollständiges Release.

### Code-Signing

- **Windows**: `signtool` signiert die Binary (nur wenn `WINDOWS_CERT_BASE64`
  und `WINDOWS_CERT_PASSWORD` als Secrets hinterlegt sind).
- **macOS**: `codesign` + Notarisierung via `notarytool` (nur wenn die Secrets
  `MACOS_CERT_BASE64`, `MACOS_CERT_PASSWORD`, `APPLE_ID`, `APPLE_APP_PASSWORD`
  und `APPLE_TEAM_ID` hinterlegt sind).
- Ohne Secrets werden unsignierte Pakete gebaut (Standard).
