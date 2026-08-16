#!/usr/bin/env bash
# Builds a macOS DMG from an existing Rivulet.app bundle in the staging
# directory. Run packaging/macos/build-app.sh first to create the bundle.
#
# Usage: packaging/macos/build-dmg.sh <version> <staging-dir> <out-file>
set -euo pipefail

VERSION="${1:?Missing version}"
STAGING="${2:?Missing staging directory}"
OUT="${3:?Missing output file}"

APP="$STAGING/Rivulet.app"
if [[ ! -d "$APP" ]]; then
  echo "App bundle missing: $APP (run build-app.sh first)" >&2
  exit 1
fi

DMG="$STAGING/rivulet.dmg"
rm -f "$DMG"
hdiutil create -volname "Rivulet $VERSION" -srcfolder "$APP" -ov -format UDZO "$DMG" >/dev/null
cp "$DMG" "$OUT"
echo "DMG created: $OUT"
