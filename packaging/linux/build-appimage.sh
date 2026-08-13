#!/usr/bin/env bash
# Baut ein Linux-AppImage aus dem Staging-Verzeichnis.
#
# Verwendung: packaging/linux/build-appimage.sh <version> <staging-dir> <out-file>
set -euo pipefail

VERSION="${1:?Version fehlt}"
STAGING="${2:?Staging-Verzeichnis fehlt}"
OUT="${3:?Ausgabedatei fehlt}"
ARCH="$(uname -m)"

if [[ "$ARCH" == "x86_64" ]]; then
  APPIMAGE_ARCH="x86_64"
elif [[ "$ARCH" == "aarch64" ]]; then
  APPIMAGE_ARCH="aarch64"
else
  echo "Nicht unterstützte Architektur: $ARCH" >&2
  exit 1
fi

APPDIR="$STAGING/AppDir"
mkdir -p "$APPDIR/usr/bin"

# Binary und Desktop-Integration einspielen.
cp "$STAGING/rivulet-gui" "$APPDIR/usr/bin/rivulet-gui"
cat > "$APPDIR/rivulet.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Rivulet
Comment=Modern Screen Recording & Streaming Software
Exec=rivulet-gui
Icon=rivulet
Terminal=false
Categories=AudioVideo;Recorder;Streaming;
EOF

if [[ -f "packaging/rivulet.png" ]]; then
  cp packaging/rivulet.png "$APPDIR/rivulet.png"
else
  # Fallback-Icon: 1x1-PNG erzeugen, falls keins vorhanden ist.
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

# appimagetool herunterladen (deterministischer Release).
APPIMAGETOOL="$STAGING/appimagetool"
if [[ ! -x "$APPIMAGETOOL" ]]; then
  URL="https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-$APPIMAGE_ARCH.AppImage"
  curl -fsSL -o "$APPIMAGETOOL" "$URL"
  chmod +x "$APPIMAGETOOL"
  # AppImages benötigen FUSE; im Container ggf. nicht verfügbar -> Extraktion.
  "$APPIMAGETOOL" --appimage-extract-and-run --version >/dev/null 2>&1 || \
    ARCH="$APPIMAGE_ARCH" "$APPIMAGETOOL" --version >/dev/null 2>&1 || true
fi

# Desktop-Datei-Kopie nach usr/share/applications für die Integration.
mkdir -p "$APPDIR/usr/share/applications"
cp "$APPDIR/rivulet.desktop" "$APPDIR/usr/share/applications/rivulet.desktop"

export ARCH="$APPIMAGE_ARCH"
if "$APPIMAGETOOL" --appimage-extract-and-run "$APPDIR" "$OUT" 2>/dev/null; then
  echo "AppImage erstellt: $OUT"
else
  "$APPIMAGETOOL" "$APPDIR" "$OUT"
  echo "AppImage erstellt (ohne Extract-Run): $OUT"
fi
