#!/usr/bin/env bash
# mirror-gstreamer-msi.sh — Mirror GStreamer Windows installers to GitHub Releases.
#
# Usage:
#   scripts/mirror-gstreamer-msi.sh                  # mirror default version
#   scripts/mirror-gstreamer-msi.sh 1.26.11          # classic MSI pair
#   scripts/mirror-gstreamer-msi.sh 1.28.6           # unified .exe generation
#
# Requires: gh CLI authenticated, curl
#
# The installers are uploaded to a release tagged "gstreamer-msi-<version>" in the
# Rivulet repo. CI downloads from this release first (faster, no 503s), with
# freedesktop.org as fallback.
#
# Format handling (auto-detected from what upstream ships):
#   * <= 1.26.x: two classic MSIs (runtime + devel)
#   * >= 1.28.x: ONE unified Inno Setup .exe installer (the per-component
#     MSIs no longer exist for these versions). CI installs it silently
#     via packaging/windows/install-gstreamer.ps1 (/VERYSILENT /TYPE=devel
#     /DIR=...), so nothing else changes on the CI side.
#
# The release tag name "gstreamer-msi-<version>" is historic; it hosts the
# .exe assets too, and CI's mirror URL builder uses it for both formats.

set -euo pipefail

VERSION="${1:-1.26.11}"
REPO="thoser666/Rivulet"
TAG="gstreamer-msi-${VERSION}"
BASE_URL="https://gstreamer.freedesktop.org/data/pkg/windows/${VERSION}/msvc"

EXE_INSTALLER="gstreamer-1.0-msvc-x86_64-${VERSION}.exe"
RUNTIME_MSI="gstreamer-1.0-msvc-x86_64-${VERSION}.msi"
DEVEL_MSI="gstreamer-1.0-devel-msvc-x86_64-${VERSION}.msi"

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

# Detect which generation upstream ships for this version: the unified
# .exe exists only for the Inno-era releases (>= 1.28).
if curl -sfIL -o /dev/null "$BASE_URL/$EXE_INSTALLER"; then
    FORMAT="exe"
    ARTIFACTS=("$EXE_INSTALLER")
else
    FORMAT="msi"
    ARTIFACTS=("$RUNTIME_MSI" "$DEVEL_MSI")
fi

echo "=== Mirroring GStreamer ${VERSION} (${FORMAT} format) to GitHub Releases ==="

# Check if release already exists
if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
    echo "Release $TAG already exists. Checking assets..."
    EXISTING=$(gh release view "$TAG" --repo "$REPO" --json assets --jq '.assets[].name' 2>/dev/null || true)
    ALL_PRESENT=true
    for artifact in "${ARTIFACTS[@]}"; do
        if ! echo "$EXISTING" | grep -q "$artifact"; then
            ALL_PRESENT=false
        fi
    done
    if [[ "$ALL_PRESENT" == "true" ]]; then
        echo "All needed assets already uploaded. Nothing to do."
        exit 0
    fi
    echo "Some assets missing. Uploading missing ones..."
fi

# Download the artifacts from freedesktop.org
for artifact in "${ARTIFACTS[@]}"; do
    echo "Downloading $artifact..."
    curl -fSL --retry 3 --retry-delay 10 \
        -o "$TMPDIR/$artifact" \
        "$BASE_URL/$artifact" || {
        echo "ERROR: Failed to download $artifact from freedesktop.org"
        echo "The server may be down. Try again later."
        exit 1
    }
done

echo "Downloads complete. File sizes:"
ls -lh "$TMPDIR"

# Create or update the release
if ! gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
    echo "Creating release $TAG..."
    args=("$TAG"
        --repo "$REPO"
        --title "GStreamer ${VERSION} Windows installers"
        --notes "Mirrored GStreamer ${VERSION} MSVC x86_64 installers (${FORMAT} format) for CI. Source: freedesktop.org"
        --prerelease)
    for artifact in "${ARTIFACTS[@]}"; do
        args+=("$TMPDIR/$artifact")
    done
    gh release create "${args[@]}"
else
    echo "Uploading assets to existing release $TAG..."
    upload_args=("$TAG" --repo "$REPO" --clobber)
    for artifact in "${ARTIFACTS[@]}"; do
        upload_args+=("$TMPDIR/$artifact")
    done
    gh release upload "${upload_args[@]}"
fi

echo "=== Done ==="
echo "CI can now download from: https://github.com/$REPO/releases/tag/$TAG"
