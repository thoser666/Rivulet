#!/usr/bin/env bash
# End-to-end smoke test for packaging/linux/sign-gpg.sh using a throwaway
# GPG key generated on the fly. Verifies the full loop: key import (armored
# AND base64 variants), detached signing, and `gpg --verify` acceptance.
#
# Usage: packaging/linux/test-gpg-signing.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SIGN_SCRIPT="$SCRIPT_DIR/sign-gpg.sh"

TMP="$(mktemp -d)"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT

# 1. Generate a throwaway signing key (no passphrase, EdDSA).
export GNUPGHOME="$TMP/gnupg"
mkdir -p "$GNUPGHOME"
chmod 700 "$GNUPGHOME"
cat > "$TMP/key-params" <<'EOF'
%no-protection
Key-Type: eddsa
Key-Curve: ed25519
Key-Usage: sign
Name-Real: Rivulet CI Test
Name-Email: ci@rivulet.invalid
Expire-Date: 1d
%commit
EOF
gpg --batch --gen-key "$TMP/key-params"

# 2. Export the key in both supported encodings.
ARMORED="$(gpg --armor --export-secret-keys ci@rivulet.invalid)"
BASE64="$(printf '%s' "$ARMORED" | base64 -w 0)"

# 3. A payload to sign.
echo "rivulet smoke payload" > "$TMP/payload.bin"

run_case() {
  local label="$1" key_value="$2"
  local out
  out="$(LINUX_GPG_PRIVATE_KEY="$key_value" bash "$SIGN_SCRIPT" "$TMP/payload.bin" 2>&1)" || {
    echo "FAIL [$label]: sign-gpg.sh exited non-zero:" >&2
    echo "$out" >&2
    exit 1
  }
  [[ -f "$TMP/payload.bin.asc" ]] || {
    echo "FAIL [$label]: no signature file produced" >&2
    exit 1
  }
  gpg --verify "$TMP/payload.bin.asc" "$TMP/payload.bin" || {
    echo "FAIL [$label]: signature does not verify" >&2
    exit 1
  }
  rm -f "$TMP/payload.bin.asc"
  echo "PASS [$label]"
}

run_case "armored key" "$ARMORED"
run_case "base64 key" "$BASE64"

# 4. Failure path: missing env var must abort with a clear error.
if LINUX_GPG_PRIVATE_KEY= bash "$SIGN_SCRIPT" "$TMP/payload.bin" >/dev/null 2>&1; then
  echo "FAIL [missing key]: script succeeded without LINUX_GPG_PRIVATE_KEY" >&2
  exit 1
fi
echo "PASS [missing key aborts]"

echo "All GPG signing smoke tests passed."