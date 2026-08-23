#!/usr/bin/env bash
# mirror-gstreamer-msi.sh — Mirror GStreamer MSIs to GitHub Releases.
#
# Usage:
#   scripts/mirror-gstreamer-msi.sh                  # mirror default version
#   scripts/mirror-gstreamer-msi.sh 1.24.13          # mirror specific version
#
# Requires: gh CLI authenticated, curl
#
# The MSIs are uploaded to a release tagged "gstreamer-msi-<version>" in the
# Rivulet repo. CI downloads from this release first (faster, no 503s), with
# freedesktop.org as fallback.

set -euo pipefail

VERSION="${1:-1.24.13}"
REPO="thoser666/Rivulet"
TAG="gstreamer-msi-${VERSION}"
BASE_URL="https://gstreamer.freedesktop.org/data/pkg/windows/${VERSION}/msvc"

RUNTIME_MSI="gstreamer-1.0-msvc-x86_64-${VERSION}.msi"
DEVEL_MSI="gstreamer-1.0-devel-msvc-x86_64-${VERSION}.msi"

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

echo "=== Mirroring GStreamer ${VERSION} MSIs to GitHub Releases ==="

# Check if release already exists
if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
    echo "Release $TAG already exists. Checking assets..."
    EXISTING=$(gh release view "$TAG" --repo "$REPO" --json assets --jq '.assets[].name' 2>/dev/null || true)
    if echo "$EXISTING" | grep -q "$RUNTIME_MSI" && echo "$EXISTING" | grep -q "$DEVEL_MSI"; then
        echo "Both MSIs already uploaded. Nothing to do."
        exit 0
    fi
    echo "Some assets missing. Uploading missing ones..."
fi

# Download MSIs from freedesktop.org
echo "Downloading runtime MSI..."
curl -fSL --retry 3 --retry-delay 10 \
    -o "$TMPDIR/$RUNTIME_MSI" \
    "$BASE_URL/$RUNTIME_MSI" || {
    echo "ERROR: Failed to download runtime MSI from freedesktop.org"
    echo "The server may be down. Try again later."
    exit 1
}

echo "Downloading devel MSI..."
curl -fSL --retry 3 --retry-delay 10 \
    -o "$TMPDIR/$DEVEL_MSI" \
    "$BASE_URL/$DEVEL_MSI" || {
    echo "ERROR: Failed to download devel MSI from freedesktop.org"
    echo "The server may be down. Try again later."
    exit 1
}

echo "Downloads complete. File sizes:"
ls -lh "$TMPDIR"/*.msi

# Create or update the release
if ! gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
    echo "Creating release $TAG..."
    gh release create "$TAG" \
        --repo "$REPO" \
        --title "GStreamer ${VERSION} MSIs" \
        --notes "Mirrored GStreamer ${VERSION} MSIs for CI. Source: freedesktop.org" \
        --prerelease \
        "$TMPDIR/$RUNTIME_MSI" \
        "$TMPDIR/$DEVEL_MSI"
else
    echo "Uploading assets to existing release $TAG..."
    gh release upload "$TAG" \
        --repo "$REPO" \
        --clobber \
        "$TMPDIR/$RUNTIME_MSI" \
        "$TMPDIR/$DEVEL_MSI"
fi

echo "=== Done ==="
echo "CI can now download from: https://github.com/$REPO/releases/tag/$TAG"
