# Building Rivulet on Linux

This guide covers building Rivulet from source on various Linux distributions.

## System Requirements

- **Rust**: 1.70 or newer
- **GStreamer**: For video/audio encoding and muxing
- **PipeWire**: For desktop audio capture
- **X11/Wayland**: Display server libraries
- **xdotool** (runtime): Used by game capture to enumerate windows (via `list_game_windows()`); without it, game capture shows an empty window list

## Installation by Distribution

### Ubuntu / Debian

```bash
# Install build dependencies
sudo apt-get update
sudo apt-get install -y \
  curl \
  build-essential \
  pkg-config \
  libgstreamer1.0-dev \
  libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad \
  gstreamer1.0-plugins-ugly \
  gstreamer1.0-libav \
  libpipewire-0.3-dev \
  libxcb1-dev \
  libxcb-render0-dev \
  libxcb-shape0-dev \
  libxcb-xfixes0-dev \
  libxkbcommon-dev \
  libssl-dev \
  libdbus-1-dev \
  libx11-dev \
  libxrandr-dev \
  libxi-dev \
  libgl1-mesa-dev \
  libasound2-dev \
  xdotool

# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Clone and build
git clone https://github.com/thoser666/rivulet.git
cd rivulet
cargo build --release

# Run
./target/release/rivulet
```

### Fedora

```bash
sudo dnf install -y \
  gcc \
  pkg-config \
  gstreamer1-devel \
  gstreamer1-plugins-base-devel \
  gstreamer1-plugins-good \
  gstreamer1-plugins-bad-free \
  gstreamer1-plugins-ugly-free \
  pipewire-devel \
  libxcb-devel \
  libXrandr-devel \
  libXi-devel \
  mesa-libGL-devel \
  alsa-lib-devel \
  openssl-devel \
  dbus-devel \
  xdotool

cargo build --release
```

### Arch Linux

```bash
sudo pacman -S --needed \
  gcc \
  pkg-config \
  gstreamer \
  gst-plugins-base \
  gst-plugins-good \
  gst-plugins-bad \
  gst-plugins-ugly \
  gst-libav \
  pipewire \
  libxcb \
  libxrandr \
  libxi \
  mesa \
  alsa-lib \
  openssl \
  dbus \
  xdotool

cargo build --release
```

## Troubleshooting

### GStreamer plugins not found

Ensure `gstreamer1.0-plugins-good` (or equivalent) is installed. The `x264enc` encoder is in the `bad` plugins package on some distributions.

### PipeWire errors

PipeWire is used for desktop audio capture. If it's not running (e.g., in a headless environment), audio capture will be unavailable but video recording still works.
