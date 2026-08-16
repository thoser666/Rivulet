#!/usr/bin/env bash
# Codesigns the Rivulet.app bundle, builds the DMG and notarizes + staples
# it for macOS Gatekeeper.
#
# Usage: packaging/macos/sign-notarize.sh <version> <staging-dir> <out-dmg>
#
# Requires the following environment variables:
#   MACOS_CERT_BASE64   Base64-encoded .p12 Developer ID Application cert
#   MACOS_CERT_PASSWORD Password of the .p12 archive
#   APPLE_ID            Apple ID used for notarization
#   APPLE_APP_PASSWORD  App-specific password for the Apple ID
#   APPLE_TEAM_ID       Apple Developer Team ID
set -euo pipefail

VERSION="${1:?Missing version}"
STAGING="${2:?Missing staging directory}"
OUT="${3:?Missing output DMG path}"

: "${MACOS_CERT_BASE64:?MACOS_CERT_BASE64 not set}"
: "${MACOS_CERT_PASSWORD:?MACOS_CERT_PASSWORD not set}"
: "${APPLE_ID:?APPLE_ID not set}"
: "${APPLE_APP_PASSWORD:?APPLE_APP_PASSWORD not set}"
: "${APPLE_TEAM_ID:?APPLE_TEAM_ID not set}"

APP="$STAGING/Rivulet.app"
if [[ ! -d "$APP" ]]; then
  echo "App bundle missing: $APP (run build-app.sh first)" >&2
  exit 1
fi

KEYCHAIN="rivulet-release.keychain"
CERT="/tmp/rivulet-cert.p12"

echo "$MACOS_CERT_BASE64" | base64 -d > "$CERT"
security create-keychain -p temp "$KEYCHAIN"
security default-keychain -s "$KEYCHAIN"
security unlock-keychain -p temp "$KEYCHAIN"
security import "$CERT" -k "$KEYCHAIN" -P "$MACOS_CERT_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple: -s -k temp "$KEYCHAIN"

# Sign the app bundle with the hardened runtime so it can be notarized.
codesign --force --options runtime --sign "Developer ID Application" --timestamp "$APP"
codesign --verify --verbose=2 "$APP"

# Package the signed app and notarize + staple the resulting DMG.
bash packaging/macos/build-dmg.sh "$VERSION" "$STAGING" "$OUT"

xcrun notarytool submit "$OUT" \
  --apple-id "$APPLE_ID" --password "$APPLE_APP_PASSWORD" --team-id "$APPLE_TEAM_ID" --wait
xcrun stapler staple "$OUT"

echo "Signed and notarized DMG created: $OUT"
