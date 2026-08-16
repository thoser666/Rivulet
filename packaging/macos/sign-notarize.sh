#!/usr/bin/env bash
# Codesigns the Rivulet.app bundle, builds the DMG and notarizes + staples
# it for macOS Gatekeeper.
#
# Usage: packaging/macos/sign-notarize.sh <version> <staging-dir> <out-dmg>
#
# The certificate environment variables are documented in codesign-app.sh.
# Additionally requires:
#   APPLE_ID            Apple ID used for notarization
#   APPLE_APP_PASSWORD  App-specific password for the Apple ID
#   APPLE_TEAM_ID       Apple Developer Team ID
set -euo pipefail

VERSION="${1:?Missing version}"
STAGING="${2:?Missing staging directory}"
OUT="${3:?Missing output DMG path}"

: "${APPLE_ID:?APPLE_ID not set}"
: "${APPLE_APP_PASSWORD:?APPLE_APP_PASSWORD not set}"
: "${APPLE_TEAM_ID:?APPLE_TEAM_ID not set}"

APP="$STAGING/Rivulet.app"

# Import the certificate and codesign the app bundle.
bash packaging/macos/codesign-app.sh "$APP"

# Package the signed app and notarize + staple the resulting DMG.
bash packaging/macos/build-dmg.sh "$VERSION" "$STAGING" "$OUT"

xcrun notarytool submit "$OUT" \
  --apple-id "$APPLE_ID" --password "$APPLE_APP_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait
xcrun stapler staple "$OUT"

echo "Signed and notarized DMG created: $OUT"
