#!/usr/bin/env bash
# Regenerate the branding assets inside the pinned ImageMagick container, so
# developers get byte-identical output without installing ImageMagick locally.
#
# The canonical ImageMagick version is pinned because text rasterization
# depends on the FreeType version; see the README "Assets" section.
#
# Usage:
#   scripts/generate-assets-docker.sh            # regenerate assets
#   scripts/generate-assets-docker.sh --check    # regenerate + verify vs HEAD
#
# Environment:
#   RIVULET_IMAGEMAGICK_IMAGE  container image (default: dpokidov/imagemagick:7.1.2-12, pinned by digest)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Pinned by content digest (manifest-list digest, so it stays multi-arch).
# ImageMagick publishes no official container, so the tag is also pinned to a
# specific digest to guard against tag mutation; see the README "Assets" section.
IMAGE="${RIVULET_IMAGEMAGICK_IMAGE:-dpokidov/imagemagick:7.1.2-12@sha256:87998ec1b8127b2f73f626f74f7b05e8827f9d7605fa52da5370588f7e53cee1}"

CHECK=0
case "${1:-}" in
  "") ;;
  "--check" | "-c") CHECK=1 ;;
  "--help" | "-h")
    sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  *) echo "error: unknown argument: $1 (see --help)" >&2; exit 2 ;;
esac

# Prefer a working Docker daemon, then Podman (works daemonless).
RUNTIME=""
if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  RUNTIME=docker
elif command -v podman >/dev/null 2>&1; then
  RUNTIME=podman
else
  echo "error: no usable container runtime found (need a Docker daemon or Podman)." >&2
  exit 1
fi

# Git Bash on Windows rewrites `/work` into a Windows path; stop that so the
# container-side paths stay intact, and pass the host path in Windows form.
if command -v cygpath >/dev/null 2>&1; then
  HOST_ROOT="$(cygpath -w "$REPO_ROOT")"
else
  HOST_ROOT="$REPO_ROOT"
fi
export MSYS_NO_PATHCONV=1

INNER='export DEBIAN_FRONTEND=noninteractive; apt-get update -qq >/dev/null 2>&1; '
if [[ "$CHECK" == 1 ]]; then
  INNER+='apt-get install -y -qq python3 git >/dev/null 2>&1; git config --global --add safe.directory /work; '
else
  INNER+='apt-get install -y -qq python3 >/dev/null 2>&1; '
fi
INNER+='bash scripts/generate-assets.sh'
if [[ "$CHECK" == 1 ]]; then
  INNER+=' && python3 scripts/check-assets.py'
fi

if [[ "$CHECK" == 1 ]]; then
  echo "==> $RUNTIME: $IMAGE (regenerate + check)"
else
  echo "==> $RUNTIME: $IMAGE (regenerate)"
fi
"$RUNTIME" run --rm -v "$HOST_ROOT:/work" -w /work --entrypoint bash "$IMAGE" -c "set -e; $INNER"
