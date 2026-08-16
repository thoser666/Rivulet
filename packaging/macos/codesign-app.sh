#!/usr/bin/env bash
# Imports the macOS signing certificate into a temporary keychain and
# codesigns the given app bundle with the hardened runtime.
#
# Usage: packaging/macos/codesign-app.sh <app-bundle>
#
# Requires the following environment variables:
#   MACOS_CERT_BASE64    Base64-encoded .p12 signing certificate
#   MACOS_CERT_PASSWORD  Password of the .p12 archive
#   MACOS_SIGN_IDENTITY  (optional) codesign identity; defaults to
#                        "Developer ID Application"
set -euo pipefail

APP="${1:?Missing app bundle path}"

: "${MACOS_CERT_BASE64:?MACOS_CERT_BASE64 not set}"
: "${MACOS_CERT_PASSWORD:?MACOS_CERT_PASSWORD not set}"
IDENTITY="${MACOS_SIGN_IDENTITY:-Developer ID Application}"

if [[ ! -d "$APP" ]]; then
  echo "App bundle missing: $APP" >&2
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

codesign --force --options runtime --sign "$IDENTITY" --timestamp "$APP"
codesign --verify --verbose=2 "$APP"

echo "Codesigned: $APP"
