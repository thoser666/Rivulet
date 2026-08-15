# Building Rivulet on Linux

This guide covers building Rivulet from source on various Linux distributions.

## System Requirements

- **Rust**: 1.70 or newer
- **FFmpeg**: For video encoding
- **X11/Wayland**: Display server libraries

## Installation by Distribution

### Ubuntu / Debian

```bash
# Install build dependencies
sudo apt-get update
sudo apt-get install -y \
  curl \
  build-essential \
  pkg-config \
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
  ffmpeg

# Install Rust (if not already installed)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Clone and build
git clone https://github.com/thoser666/rivulet.git
cd rivulet
cargo build --release

# Run
./target/release/rivulet

