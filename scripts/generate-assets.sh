#!/usr/bin/env bash
# Generates the app thumbnail and the Linux AppImage icon from the Rivulet logo.
#
# Requirements: ImageMagick 6.9+ / 7 (`magick`, or ImageMagick's `convert`).
#
# Outputs (relative to the repository root):
#   docs/thumbnail.png      1280x720 brand thumbnail (README hero image)
#   packaging/rivulet.png   512x512 transparent app icon
#
# The output is deterministic: re-running the script reproduces the committed
# assets byte-for-byte. Override the fonts via RIVULET_FONT_BOLD and
# RIVULET_FONT_REGULAR (ImageMagick font names).
#
# Usage: scripts/generate-assets.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

LOGO="$REPO_ROOT/rivulet-gui/assets/rivulet_logo.jpg"
THUMBNAIL="$REPO_ROOT/docs/thumbnail.png"
ICON="$REPO_ROOT/packaging/rivulet.png"

# Rivulet brand colors (from the logo).
NAVY="#0B2545"
BLUE="#357AC6"
LIGHT_BLUE="#A0DAED"

# --- ImageMagick detection ------------------------------------------------
if command -v magick >/dev/null 2>&1; then
  MAGICK=magick
elif command -v convert >/dev/null 2>&1 && convert -version 2>/dev/null | grep -qi imagemagick; then
  MAGICK=convert
else
  echo "error: ImageMagick (magick) is required to generate the assets." >&2
  exit 1
fi

# --- Font selection -------------------------------------------------------
# Pick the first available sans-serif font so the script also works on hosts
# without Arial (e.g. Linux CI runners with DejaVu/Liberation).
pick_font() {
  local name
  for name in "$@"; do
    if "$MAGICK" -list font 2>/dev/null | grep -qiE "^[[:space:]]*Font:[[:space:]]*${name}[[:space:]]*$"; then
      printf '%s\n' "$name"
      return 0
    fi
  done
  return 1
}

FONT_BOLD="${RIVULET_FONT_BOLD:-$(pick_font "Arial-Bold" "DejaVu-Sans-Bold" "Liberation-Sans-Bold" "Helvetica-Bold" || true)}"
FONT_REGULAR="${RIVULET_FONT_REGULAR:-$(pick_font "Arial" "DejaVu-Sans" "Liberation-Sans" "Helvetica" || true)}"

BOLD_ARGS=()
[[ -n "$FONT_BOLD" ]] && BOLD_ARGS=(-font "$FONT_BOLD")
REGULAR_ARGS=()
[[ -n "$FONT_REGULAR" ]] && REGULAR_ARGS=(-font "$FONT_REGULAR")

# Strip all metadata and force 8-bit depth so the PNGs are byte-for-byte
# reproducible across ImageMagick versions.
STRIP=(-strip -depth 8 +set date:create +set date:modify)

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

SYMBOL="$WORK/rivulet_symbol.png"

# 1. Extract the wave symbol and drop the white background. The logo also
#    carries a "Rivulet" wordmark below the wave; trimming to the top 83%
#    keeps only the wave, so the thumbnail gets a single wordmark (the one
#    annotated below) and the app icon stays text-free.
"$MAGICK" "$LOGO" -fuzz 8% -transparent white -trim +repage \
  -gravity north -crop '100%x83%+0+0' +repage "$SYMBOL"

# 2. 1280x720 brand thumbnail: gradient + symbol + wordmark + tagline.
"$MAGICK" -size 1280x720 "gradient:${NAVY}-${BLUE}" \
  \( "$SYMBOL" -resize 'x260' \) -gravity north -geometry +0+35 -composite \
  -gravity north "${BOLD_ARGS[@]}" -fill white -pointsize 150 -annotate +0+320 'Rivulet' \
  -gravity north "${REGULAR_ARGS[@]}" -fill "$LIGHT_BLUE" -pointsize 44 -annotate +0+545 'Modern Screen Recording & Streaming' \
  "${STRIP[@]}" "$THUMBNAIL"

# 3. 512x512 transparent icon for the Linux AppImage.
"$MAGICK" "$SYMBOL" -resize '448x448>' -gravity center -background none -extent 512x512 \
  "${STRIP[@]}" "$ICON"

echo "Generated:"
echo "  $THUMBNAIL"
echo "  $ICON"
