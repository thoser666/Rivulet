#!/usr/bin/env bash
# Baut ein macOS-DMG aus dem Staging-Verzeichnis.
#
# Verwendung: packaging/macos/build-dmg.sh <version> <staging-dir> <out-file>
set -euo pipefail

VERSION="${1:?Version fehlt}"
STAGING="${2:?Staging-Verzeichnis fehlt}"
OUT="${3:?Ausgabedatei fehlt}"

APP="$STAGING/Rivulet.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# Binary einspielen.
cp "$STAGING/rivulet-gui" "$APP/Contents/MacOS/rivulet-gui"
chmod +x "$APP/Contents/MacOS/rivulet-gui"

# Info.plist erzeugen.
cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Rivulet</string>
  <key>CFBundleDisplayName</key><string>Rivulet</string>
  <key>CFBundleIdentifier</key><string>com.rivulet.app</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleExecutable</key><string>rivulet-gui</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleIconFile</key><string>AppIcon</string>
</dict>
</plist>
EOF

if [[ -f "packaging/rivulet.icns" ]]; then
  cp packaging/rivulet.icns "$APP/Contents/Resources/AppIcon.icns"
fi

# Ausführbarkeit für die App festlegen.
chmod -R u+rwX "$APP"

# DMG im Staging bauen.
DMG="$STAGING/rivulet.dmg"
rm -f "$DMG"
hdiutil create -volname "Rivulet $VERSION" -srcfolder "$APP" -ov -format UDZO "$DMG" >/dev/null
cp "$DMG" "$OUT"
echo "DMG erstellt: $OUT"
