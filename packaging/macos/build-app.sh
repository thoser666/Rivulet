#!/usr/bin/env bash
# Creates the Rivulet.app bundle in the staging directory from the staged
# release binary. The bundle is later packaged into a DMG by build-dmg.sh
# (unsigned) or sign-notarize.sh (signed + notarized).
#
# Usage: packaging/macos/build-app.sh <version> <staging-dir>
set -euo pipefail

VERSION="${1:?Missing version}"
STAGING="${2:?Missing staging directory}"

APP="$STAGING/Rivulet.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$STAGING/rivulet-gui" "$APP/Contents/MacOS/rivulet-gui"
chmod +x "$APP/Contents/MacOS/rivulet-gui"

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

chmod -R u+rwX "$APP"
echo "App bundle created: $APP"
