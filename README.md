<div align="center">

# 🌊 Rivulet

**Modern Screen Recording & Streaming Software**

[![CI](https://github.com/thoser666/rivulet/workflows/CI/badge.svg)](https://github.com/thoser666/rivulet/actions)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)](https://www.rust-lang.org)
[![Status](https://img.shields.io/badge/status-alpha%20v0.2-yellow.svg)](https://github.com/thoser666/rivulet)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey.svg)](https://github.com/thoser666/rivulet)

*A complete Rust reimplementation of OBS Studio - built for performance, safety, and reliability*

[Features](#-features) • [Installation](#-installation) • [Roadmap](#-roadmap) • [Releases](#-releases) • [Contributing](#-contributing)

![Rivulet Screenshot](docs/screenshot.png)
<!-- Add screenshot later -->

</div>

---

## 🎯 Vision

Rivulet ist **kein OBS-Klon, sondern eine embeddbare, deterministische Recording- & Streaming-Engine in Rust mit moderner GUI.** Sie liefert OBS-Kernfeatures (Capture, Encoding, Audio, Streaming, Dual Output) und nutzt gleichzeitig die architektonischen Lücken von OBS als eigene Stärken: Automation statt Interaktion, Bibliothek statt Monolith, moderner Render-Pfad statt Legacy, stabiles Plugin-ABI statt C-DLLs.

### Why Rust?
- 🔒 **Memory Safety** - No segfaults, no data races
- ⚡ **Performance** - Zero-cost abstractions
- 🛡️ **Reliability** - Catch bugs at compile time
- 🌍 **Cross-Platform** - Write once, run everywhere

### 🧭 Positionierung: OBS-Schwächen als Rivulet-Stärken

| OBS-Stärke | OBS-Schwäche | Rivulet-Antwort |
| --- | --- | --- |
| Mächtiges, ausgereiftes Feature-Set | Interaktiv-first: keine Headless-/CI-/Render-Farm-Nutzung, schlecht automatisierbar | Deterministische Pipeline, Headless-CLI, testbare Engine (M6) |
| Monolithische App + libobs | `libobs` ist keine saubere Library-API; Einbettung in Produkte ist Krampf | `rivulet-core` als normale, semver-stabile Crate (M7) |
| Plugin-Ökosystem | C/C++-Plugins gegen libobs-ABI, versionssensitiv, können App crashen | WASM-Plugin-Runtime + temporärer OBS-Compat-Mode (M5) |
| Leistungsfähiger Renderer | OpenGL/D3D-Legacy, schwer modernisierbar | WebGPU/Zero-Copy von Grund auf (M8) |
| Streaming-Basis | Kern RTMP/FLV, Low-Latency-Protokolle frickelig | WebRTC/WHIP & SRT/RIST als First-Class (M3) |
| Windows-first | Plattform-Parität ungleich (macOS/Linux schwächer) | Parität als Release-Blocker, nicht Afterthought (M5) |

### Long-term Goals (v1.0+)
- **Feature Parity** mit OBS Studio (Kern-Features)
- **Temporäre OBS-Plugin-Kompatibilität** als Brücke, langfristig **WASM-Plugin-Ökosystem**
- **Modern Architecture** - Clean, maintainable codebase
- **Deterministic & Embeddable** - Engine als Bibliothek, Headless nutzbar, CI-tauglich
- **Active Community** - Open development, regular updates

---

## ✨ Features

### ✅ Currently Available (v0.2)

- **Screen Capture** - Capture your primary monitor in real-time
- **Window Capture** - Capture a single application window (games, etc.) in addition to full monitors, with a window picker in the GUI (Linux & Windows)
- **Screen + Audio Recording (Linux GUI)** - Full recording flow in the GUI: monitor selection, start/stop, recording timer; system audio + microphone are captured and mixed into the MP4
- **Video Encoding** - H.264 encoding via GStreamer (`x264enc`, low-latency tuning) or hardware-accelerated encoders
- **Hardware Encoding** - NVIDIA NVENC (`nvh264enc`), Intel QuickSync (`qsvh264enc`) and AMD AMF (`amfh264enc`) with automatic detection of the best available encoder and fallback to software x264; engine API `set_video_encoder(VideoEncoder)` / `set_video_bitrate(kbps)`
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

### 🚧 In Development (v0.3)

See [Roadmap](#-roadmap) for detailed timeline.

---

## 🚀 Roadmap

> **Ziel:** OBS-Kern-Parität *und* die architektonischen Stärken, die OBS nicht bieten kann (Determinismus, Einbettbarkeit, Modernität).
> **Strategie:** Erst die stabile, embeddbare Engine, dann Szenen & Streaming, dann die Differenzierungs-Features (M6–M8) als Produkt-Identität.

### Meilenstein-Übersicht

| Meilenstein | Fokus | Status |
| --- | --- | --- |
| M0 – Recording Foundation | Aufnahme, Encoding, Audio, GUI | ✅ Erreicht |
| M1 – Solid Recording | Audio-Tracks, Hardware-Encoding, QoL | 🚧 In Arbeit |
| M2 – Scenes & Composition | OBS-Kern: Szenen, Quellen, Übergänge | 📅 Geplant |
| M3 – Streaming | RTMP/RTMPS, Dual Output, WebRTC/SRT | 🚧 In Arbeit |
| M4 – Advanced Output | Virtual Camera, Replay Buffer, Filter | 📅 Geplant |
| M5 – Ecosystem & Parität | WASM-Plugins, OBS-Compat, Plattform-Parität | 📅 Geplant |
| M6 – Automation & Determinism | Headless, CI-Rendering, reproduzierbare Pipeline | 📅 Geplant |
| M7 – Embeddable Engine | Stabile `rivulet-core`-API, Doku, Tooling | 📅 Geplant |
| M8 – Modern Architecture | WebGPU-Renderer, Zero-Copy, Leichtgewicht | 📅 Geplant |

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
- [x] Hardware-Encoding (NVIDIA NVENC, Intel QuickSync, AMD AMF)
- [x] Auto-Detection des besten Encoders mit Fallback
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
- [ ] **WebRTC/WHIP** als First-Class-Protokoll (ultra-low-latency, SFU-kompatibel)
- [ ] **SRT/RIST** für professionelle Contribution/Relay

**Ziel:** Live-Streaming zu den gängigen Plattformen *und* Low-Latency-Protokolle als native Bürger statt RTMP-Legacy.

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
- [ ] **WASM-Plugin-Runtime** (stabiles ABI, sandboxed: Plugins können die App nie crashen; langfristiges Ziel-Plugin-Modell)
- [ ] **OBS-Plugin-Kompatibilitätsschicht** — *temporäre Brücke*: opt-in "Compatibility Mode" (explizit als unsafe markiert), lädt native libobs-Plugins (Encoder/Filter/Quellen ohne UI); UI-Plugins (Qt) out of scope; Ziel ist der Übergang zum WASM-Plugin-System
- [ ] Mobile Companion App (Remote Control)
- [ ] **Windows/macOS-Feature-Parität** (aktuell Linux-first; Fenster-Capture Windows vorhanden, macOS noch offen) — als Release-Blocker, nicht Afterthought
- [x] Installer (Windows MSI, macOS DMG, Linux AppImage) — automatisiert in CI
- [ ] Code-Signing und Auto-Update (Signing-Automatik vorhanden, Secrets nötig)
- [ ] Telemetrie (opt-in, datenschutzfreundlich)
- [ ] Multi-Language-Support

**Ziel:** OBS-Kern-Parität über alle Plattformen, mit einem Plugin-Modell, das OBS strukturell überlegen ist (WASM statt C-ABI), plus OBS-Compat als Übergangs-Brücke.

---

### 🤖 M6 – Automation & Determinism ("Render-First")

**Status: Geplant**

*Die Differenzierungs-Säule Nr. 1: OBS ist interaktiv-first, Rivulet ist deterministisch.*

- [ ] Deterministische Pipeline (steuerbare Engine-Clock, reproduzierbare Ausgabe aus denselben Eingaben)
- [ ] Headless-CLI: Aufnahme/Rendering ohne GUI (`rivulet record ...`), als Binary und Bibliothek nutzbar
- [ ] CI-taugliches Rendering: Video aus Code generieren (Remotion-Ansatz, nativ in Rust) — z. B. Batch-Erstellung, Tests, Screenshots pro Frame
- [ ] Pipeline-Inspektor/Diagnose-Tooling (analog `gst-inspect`, `gst-launch`), eingebettet in die Engine
- [ ] Deterministische Tests als First-Class-Bürger (Golden-Frame-Tests, exakte PTS/DTS-Verifikation)

**Ziel:** "Video aus Code" und reproduzierbare Aufnahme-Pipelines — der Grund, warum ein Entwickler/Team OBS nicht verwenden *kann*, Rivulet aber schon.

---

### 📦 M7 – Embeddable Engine & API

**Status: Geplant**

*Differenzierungs-Säule Nr. 2: `rivulet-core` ist eine normale Bibliothek, nicht ein Monolith mit API-Nachrüstung.*

- [ ] Stabilisierte Public API von `rivulet-core` (semver 1.0, `#![warn(missing_docs)]`, Kabelbaum-Typen)
- [ ] Umfassende API-Doku + Beispiele (Aufnahme, Streaming, Dual Output, Encoder-Auswahl, Frame-Stream)
- [ ] In-Prozess-Aufnahme-API: Recording-Feature in jede Rust-App einbetten (Audio-/Video-Capture, Encoding, Datei/Stream)
- [ ] Feature-Detection und Runtime-Diagnose als API (`detect_available_encoders()`, Encoder-Fallback)
- [ ] Abstraktion von Capture-Backends (xcap, PipeWire, Metal/WGC) hinter stabilen Traits

**Ziel:** "Recording, das man in sein Produkt einbettet" — der Anwendungsfall, für den OBS architektonisch nicht gebaut ist.

---

### ⚡ M8 – Modern Architecture

**Status: Geplant**

*Differenzierungs-Säule Nr. 3: von Grund auf moderner Render-Pfad statt OpenGL/D3D-Legacy.*

- [ ] WebGPU-Renderer (wgpu) für Preview, Composite und Effekte
- [ ] Compute-basierte Video-/Audio-Filter (Chroma Key, Skalierung, Filter) statt CPU/Shader-Legacy
- [ ] Zero-Copy-/GPUDirect-Capture-Pfade (DMA-BUF, WGC/NVENC-Direct, Metal IO)
- [ ] Leistungs- und Footprint-Metriken (Idle-CPU/RAM, Laptop-Battery) als CI-Checks
- [ ] Modernes UI-Fundament (egui) konsistent über alle Plattformen

**Ziel:** Leichtgewichtig und zukunftsfähig — dort technisch vorne, wo OBS nachrüsten muss.

---

### 🎯 Feature-Paritäts-Checkliste (gegenüber OBS)

| OBS-Kategorie | Status |
| --- | --- |
| Capture-Quellen (Display, Fenster, Webcam) | Teilweise (Display + Fenster) |
| Szenen & Übergänge | Offen |
| Audio-Mixer (Quellen, Tracks, Filter) | Teilweise (Mixer, Separate Tracks) |
| Recording & Encoding | Teilweise (H.264 Hardware + Software) |
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

The tests cover the pure data structures (`AudioFrame`, `OutputSettings`, `RecordingSettings`, ...), the engine's recording pipeline (synthetic video + audio to MP4, including separate audio tracks verified via the GStreamer Discoverer), the streaming pipelines (RTMPS/dual output parse and route audio correctly), stream health tracking (status derivation from drop ratio, bitrate/FPS collapse and stalls), the encoder/recorder lifecycle, and the video encoder abstraction (element names, detection order, fallback, pipeline fragments, 4:2:0 input caps). Building and running the tests requires GStreamer (dev packages + plugins) and, on Linux, the `LIBCLANG_PATH` environment variable. The hardware-encoding end-to-end test only runs when the matching GPU plugin is present.

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
