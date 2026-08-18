#!/usr/bin/env bash
# End-to-end smoke test for the macOS codesign step using a self-signed
# certificate. Notarization is intentionally out of scope: it requires a real
# Apple Developer ID and Apple credentials.
#
# Usage: packaging/macos/test-signing.sh
set -euo pipefail

CERT_DIR="$(mktemp -d)"
APP_ROOT="$(mktemp -d)"
APP="$APP_ROOT/RivuletTest.app"

cleanup() {
  security delete-keychain "$HOME/Library/Keychains/rivulet-release.keychain-db" >/dev/null 2>&1 || true
  rm -rf "$CERT_DIR" "$APP_ROOT"
}
trap cleanup EXIT

# 1. Self-signed code signing certificate wrapped in a p12.
#
# The certificate needs the Code Signing extended key usage: without it
# `security find-identity -p codesigning` reports "0 valid identities"
# and codesign refuses to use it.
openssl req -x509 -newkey rsa:2048 -keyout "$CERT_DIR/key.pem" \
  -out "$CERT_DIR/cert.pem" -days 1 -nodes -subj "/CN=Rivulet CI Test" \
  -addext "extendedKeyUsage=codeSigning" -addext "keyUsage=digitalSignature"
# Export with -legacy so the p12 uses RC2-40-CBC + SHA-1 MAC — the exact
# format Keychain Access produces. macOS 26 `security import` rejects
# OpenSSL 3's modern defaults ("MAC verification failed") and does not
# make the 3DES-shrouded key usable ("0 valid identities").
openssl pkcs12 -export -legacy -out "$CERT_DIR/cert.p12" \
  -inkey "$CERT_DIR/key.pem" -in "$CERT_DIR/cert.pem" -passout pass:rivulet-test

# 2. Minimal app bundle with a real Mach-O executable.
mkdir -p "$APP/Contents/MacOS"
cp /usr/bin/true "$APP/Contents/MacOS/rivulet-gui"
cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>com.rivulet.test</string>
  <key>CFBundleExecutable</key><string>rivulet-gui</string>
  <key>CFBundlePackageType</key><string>APPL</string>
</dict>
</plist>
PLIST

# 3. Sign through the production codesign path.
MACOS_CERT_BASE64="$(base64 < "$CERT_DIR/cert.p12")"
MACOS_CERT_PASSWORD="rivulet-test"
MACOS_SIGN_IDENTITY="Rivulet CI Test"
export MACOS_CERT_BASE64 MACOS_CERT_PASSWORD MACOS_SIGN_IDENTITY

bash packaging/macos/codesign-app.sh "$APP"

# 4. Prove that a real (non-adhoc) signature was applied.
codesign --verify --verbose=2 "$APP"
codesign -dvv "$APP" 2>&1 | grep -q "Authority=Rivulet CI Test"

echo "macOS codesign smoke test passed."
