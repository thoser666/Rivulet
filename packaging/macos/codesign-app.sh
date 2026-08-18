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
#   MACOS_SIGN_TIMESTAMP (optional) "0" skips the RFC3161 timestamp request
set -euo pipefail

APP="${1:?Missing app bundle path}"

: "${MACOS_CERT_BASE64:?MACOS_CERT_BASE64 not set}"
: "${MACOS_CERT_PASSWORD:?MACOS_CERT_PASSWORD not set}"
IDENTITY="${MACOS_SIGN_IDENTITY:-Developer ID Application}"

if [[ ! -d "$APP" ]]; then
  echo "App bundle missing: $APP" >&2
  exit 1
fi

KEYCHAIN_PATH="$HOME/Library/Keychains/rivulet-release.keychain-db"
CERT="/tmp/rivulet-cert.p12"

echo "$MACOS_CERT_BASE64" | base64 -D > "$CERT"
security create-keychain -p temp "$KEYCHAIN_PATH"
security set-keychain-settings -lut 21600 "$KEYCHAIN_PATH"
security default-keychain -s "$KEYCHAIN_PATH"
security unlock-keychain -p temp "$KEYCHAIN_PATH"
security import "$CERT" -k "$KEYCHAIN_PATH" -P "$MACOS_CERT_PASSWORD" -A -t cert -f pkcs12
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k temp "$KEYCHAIN_PATH"

# Resolve the signing identity. `codesign --sign <name>` matches the
# identity's common name, but a freshly imported self-signed certificate
# is not reliably matched by name on macOS 26 ("<name>: no identity found").
# Prefer the exact name match, then fall back to the first codesigning
# identity's keychain hash, which is always stable.
#
# `find-identity -v` only lists *trusted* identities; a self-signed test
# certificate is never trusted, so the smoke test would see "0 valid
# identities found" even though the identity signs fine. Drop `-v` to list
# every identity that matches the codesigning policy.
echo "Available signing identities:"
security find-identity -p codesigning "$KEYCHAIN_PATH" 2>/dev/null || true
SIGN_IDENTITY="$(security find-identity -p codesigning "$KEYCHAIN_PATH" 2>/dev/null | grep -F "\"$IDENTITY\"" | awk '{print $2}' | head -1 || true)"
if [[ -z "$SIGN_IDENTITY" ]]; then
  SIGN_IDENTITY="$(security find-identity -p codesigning "$KEYCHAIN_PATH" 2>/dev/null | awk '/^[[:space:]]*[0-9]+\)/{print $2; exit}')"
fi
if [[ -z "$SIGN_IDENTITY" ]]; then
  echo "No codesigning identity found in $KEYCHAIN_PATH" >&2
  exit 1
fi

# MACOS_SIGN_TIMESTAMP=0 skips the RFC3161 timestamp request (the smoke
# test sets this to stay offline and deterministic).
if [[ "${MACOS_SIGN_TIMESTAMP:-1}" == "0" ]]; then
  codesign --keychain "$KEYCHAIN_PATH" --force --options runtime --sign "$SIGN_IDENTITY" "$APP"
else
  codesign --keychain "$KEYCHAIN_PATH" --force --options runtime --sign "$SIGN_IDENTITY" --timestamp "$APP"
fi
codesign --verify --verbose=2 "$APP"

echo "Codesigned: $APP"
