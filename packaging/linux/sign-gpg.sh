#!/usr/bin/env bash
# Signs one or more files with the Rivulet Linux GPG signing key, producing
# detached ASCII-armored signatures (`<file>.asc`).
#
# Usage: packaging/linux/sign-gpg.sh <file> [<file> ...]
#
# Requires the following environment variables:
#   LINUX_GPG_PRIVATE_KEY   ASCII-armored OR base64-encoded private key
#   LINUX_GPG_PASSPHRASE    (optional) passphrase of the private key
#
# The key is imported into a throwaway GNUPGHOME under $HOME/.gnupg-rivulet-
# <pid> that is deleted on exit, so the runner's default keyring is never
# touched. Signing is always detached (`.asc` next to each input file), as
# distributions and package managers expect detached signatures for AppImages.
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "Usage: $0 <file> [<file> ...]" >&2
  exit 1
fi

: "${LINUX_GPG_PRIVATE_KEY:?LINUX_GPG_PRIVATE_KEY not set}"

GNUPGHOME="$HOME/.gnupg-rivulet-$$"
mkdir -p "$GNUPGHOME"
chmod 700 "$GNUPGHOME"
export GNUPGHOME
trap 'rm -rf "$GNUPGHOME"' EXIT

# The key secret is stored either ASCII-armored (starts with the PGP header)
# or base64-encoded (portable, no multi-line YAML headaches). Decode both
# into the temp keyring.
KEY_FILE="$GNUPGHOME/rivulet-key.asc"
if [[ "$LINUX_GPG_PRIVATE_KEY" == "-----BEGIN PGP PRIVATE KEY BLOCK-----"* ]]; then
  printf '%s\n' "$LINUX_GPG_PRIVATE_KEY" > "$KEY_FILE"
else
  # base64: accept single-line or newline-wrapped input.
  printf '%s' "$LINUX_GPG_PRIVATE_KEY" | tr -d ' \n' | base64 -d > "$KEY_FILE"
fi
gpg --batch --import "$KEY_FILE"

# Extract the signing key fingerprint (first secret key in the keyring).
FPR="$(gpg --batch --with-colons --list-secret-keys | awk -F: '/^fpr:/{print $10; exit}')"
if [[ -z "$FPR" ]]; then
  echo "No secret key found after import" >&2
  exit 1
fi

# Batch signing: loopback pinentry reads the passphrase from the env var.
PASSPHRASE_ARGS=()
if [[ -n "${LINUX_GPG_PASSPHRASE:-}" ]]; then
  PASSPHRASE_ARGS=(--pinentry-mode loopback --passphrase "$LINUX_GPG_PASSPHRASE")
else
  PASSPHRASE_ARGS=(--pinentry-mode loopback --passphrase "")
fi

for file in "$@"; do
  [[ -f "$file" ]] || { echo "Not a file: $file" >&2; exit 1; }
  echo "Signing: $file"
  gpg --batch --yes "${PASSPHRASE_ARGS[@]}" \
    --local-user "$FPR" --detach-sign --armor \
    --output "$file.asc" "$file"
done

echo "Signed $# file(s) with key $FPR"