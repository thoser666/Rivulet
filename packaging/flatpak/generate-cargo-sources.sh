#!/usr/bin/env bash
# Regenerates packaging/flatpak/cargo/cargo-sources.json from the repository's
# Cargo.lock using the official flatpak-cargo-generator with a pinned commit.
#
# The flatpak build is fully offline: every crate is vendored via this file,
# so it must never drift from Cargo.lock. CI re-runs this script and fails if
# the committed file changed (see .github/workflows/flatpak-build.yml).
#
# Usage: packaging/flatpak/generate-cargo-sources.sh [--verify]
set -euo pipefail

GENERATOR_COMMIT="f03a673abe6ce189cea1c2857e2b44af2dd79d1f"
GENERATOR_URL="https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/${GENERATOR_COMMIT}/cargo/flatpak-cargo-generator.py"

HERE="$(cd "$(dirname "$0")" && pwd)"
LOCK="$HERE/../../Cargo.lock"
OUT="$HERE/cargo/cargo-sources.json"
TMP="$(mktemp -d)"

trap 'rm -rf "$TMP"' EXIT

curl -fsSL "$GENERATOR_URL" -o "$TMP/flatpak-cargo-generator.py"

python3 "$TMP/flatpak-cargo-generator.py" "$LOCK" -o "$TMP/cargo-sources.json"

if [[ "${1:-}" == "--verify" ]]; then
  if ! diff -u "$OUT" "$TMP/cargo-sources.json"; then
    echo "error: cargo-sources.json is stale. Run packaging/flatpak/generate-cargo-sources.sh and commit the change." >&2
    exit 1
  fi
  echo "cargo-sources.json is up to date."
else
  cp "$TMP/cargo-sources.json" "$OUT"
  echo "Regenerated $OUT"
fi