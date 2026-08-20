# Building Rivulet on Linux

This guide covers building Rivulet from source on various Linux distributions.

## System Requirements

- **Rust**: 1.70 or newer
- **GStreamer**: For video/audio encoding and muxing
- **PipeWire**: For desktop audio capture
- **X11/Wayland**: Display server libraries
- **xdotool** (runtime): Used by game capture to enumerate windows (via `list_game_windows()`); without it, game capture shows an empty window list

## Linux game-window verification

The CI installs `xdotool`, `xvfb`, `x11-apps` and `x11-utils`. It starts a
virtual X11 display plus an 800×600 test window, then runs the opt-in
`list_game_windows_finds_the_ci_window` test. The test requires
`RIVULET_TEST_XDOTOOL=1` and asserts both that the result is non-empty and that
the CI window is present, so a missing `xdotool`, `DISPLAY` or X11 geometry
query fails explicitly instead of silently skipping the check.

To reproduce the check locally on a Debian/Ubuntu host with Xvfb:

```bash
sudo apt-get install -y xdotool xvfb x11-apps x11-utils
export DISPLAY=:99
Xvfb "$DISPLAY" -screen 0 1280x800x24 -nolisten tcp >/tmp/rivulet-xvfb.log 2>&1 &
xvfb_pid=$!
trap 'kill "$xvfb_pid" 2>/dev/null || true' EXIT
sleep 1
xmessage -display "$DISPLAY" -geometry 800x600+0+0 \
  -name RivuletGameCaptureTest \
  "Rivulet game capture test window" &
window_pid=$!
trap 'kill "$window_pid" "$xvfb_pid" 2>/dev/null || true' EXIT
sleep 1
RIVULET_TEST_XDOTOOL=1 cargo test -p rivulet-core \
  list_game_windows_finds_the_ci_window -- --nocapture
```

The regular local test suite leaves this environment-dependent test inactive;
CI enables it explicitly. The CI build cache intentionally stores only Cargo's
registry/git downloads, not `target/`, because compiler- and dependency-specific
artifacts can otherwise produce stale `E0460` errors after toolchain updates.

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
