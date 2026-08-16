#!/usr/bin/env bash
# Builds a Linux AppImage from the staging directory.
#
# Usage: packaging/linux/build-appimage.sh <version> <staging-dir> <out-file>
set -euo pipefail

VERSION="${1:?Missing version}"
STAGING="${2:?Missing staging directory}"
OUT="${3:?Missing output file}"
ARCH="$(uname -m)"

if [[ "$ARCH" == "x86_64" ]]; then
  APPIMAGE_ARCH="x86_64"
elif [[ "$ARCH" == "aarch64" ]]; then
  APPIMAGE_ARCH="aarch64"
else
  echo "Unsupported architecture: $ARCH" >&2
  exit 1
fi

APPDIR="$STAGING/AppDir"
mkdir -p "$APPDIR/usr/bin"

# Install the binary and desktop integration.
cp "$STAGING/rivulet-gui" "$APPDIR/usr/bin/rivulet-gui"
cat > "$APPDIR/rivulet.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Rivulet
Comment=Modern Screen Recording & Streaming Software
Exec=rivulet-gui
Icon=rivulet
Terminal=false
Categories=AudioVideo;Recorder;
EOF

if [[ -f "packaging/rivulet.png" ]]; then
  cp packaging/rivulet.png "$APPDIR/rivulet.png"
else
  # Fallback icon: generate a 1x1 PNG if none is present.
  python3 - <<'PY'
import struct, zlib
def png():
    raw = b''.join(b'\x00' + b'\x00\x00\x00\x00' * 1 for _ in range(1))
    return b'\x89PNG\r\n\x1a\n' + struct.pack('>II', 13, 0x49484452) + struct.pack('>IIBBBBB', 1, 1, 8, 2, 0, 0, 0) + zlib.compress(raw) + struct.pack('>II', 0, 0x49454E44)
open('/tmp/rivulet_fallback.png','wb').write(png())
PY
  cp /tmp/rivulet_fallback.png "$APPDIR/rivulet.png"
fi

cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
SELF="$(readlink -f "$0")"
HERE="${SELF%/*}"
export PATH="$HERE/usr/bin:$PATH"
exec "$HERE/usr/bin/rivulet-gui" "$@"
EOF
chmod +x "$APPDIR/AppRun" "$APPDIR/usr/bin/rivulet-gui"

# Download appimagetool (deterministic release).
APPIMAGETOOL="$STAGING/appimagetool"
if [[ ! -x "$APPIMAGETOOL" ]]; then
  URL="https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-$APPIMAGE_ARCH.AppImage"
  curl -fsSL -o "$APPIMAGETOOL" "$URL"
  chmod +x "$APPIMAGETOOL"
  # AppImages need FUSE; it may be unavailable in containers -> extract.
  "$APPIMAGETOOL" --appimage-extract-and-run --version >/dev/null 2>&1 || \
    ARCH="$APPIMAGE_ARCH" "$APPIMAGETOOL" --version >/dev/null 2>&1 || true
fi

# Copy the desktop file to usr/share/applications for integration.
mkdir -p "$APPDIR/usr/share/applications"
cp "$APPDIR/rivulet.desktop" "$APPDIR/usr/share/applications/rivulet.desktop"

export ARCH="$APPIMAGE_ARCH"
# appimagetool itself is an AppImage and needs FUSE; in CI/containers
# libfuse2 is usually not installed -> always use --appimage-extract-and-run.
if "$APPIMAGETOOL" --appimage-extract-and-run "$APPDIR" "$OUT"; then
  echo "AppImage created: $OUT"
else
  echo "appimagetool (extract-and-run) failed." >&2
  exit 1
fi
