#!/usr/bin/env bash
# Generates the app thumbnail and the app icons from the Rivulet logo.
#
# Requirements: ImageMagick 6.9+ / 7 (`magick`, or ImageMagick's `convert`)
# and Python 3 (to pack the macOS .icns container).
#
# Outputs (relative to the repository root):
#   docs/thumbnail.png        1280x720 brand thumbnail (README hero image)
#   docs/social-preview.png   1280x640 GitHub social preview (2:1)
#   docs/opengraph.png        1200x630 OpenGraph fallback (X/Facebook/LinkedIn)
#   packaging/rivulet.png     512x512 transparent Linux AppImage icon
#   packaging/rivulet.icns    macOS app icon (16..1024 px, PNG-based)
#
# The output is deterministic: re-running the script reproduces the committed
# assets byte-for-byte. Because text rasterization depends on the FreeType
# version, the canonical output is produced with ImageMagick 7.1.2-12
# (dpokidov/imagemagick:7.1.2-12, pinned by digest in the wrapper/CI); see the
# README "Assets" section. Override
# the fonts via RIVULET_FONT_BOLD / RIVULET_FONT_REGULAR (font file paths).
#
# Usage: scripts/generate-assets.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

LOGO="$REPO_ROOT/rivulet-gui/assets/rivulet_logo.jpg"
THUMBNAIL="$REPO_ROOT/docs/thumbnail.png"
SOCIAL="$REPO_ROOT/docs/social-preview.png"
OPENGRAPH="$REPO_ROOT/docs/opengraph.png"
ICON="$REPO_ROOT/packaging/rivulet.png"
ICNS="$REPO_ROOT/packaging/rivulet.icns"

# Brand palette, darkened for the hero thumbnail so the light-blue logo and
# white wordmark read clearly. LIGHT_BLUE is the brand accent (from the logo).
NAVY="#081C34"
BLUE="#16457A"
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

# Pick a working Python 3 interpreter. On Windows, `python3` can be the
# Microsoft Store alias, which prints an error but still exits 0, so probe
# with a real import instead of trusting `command -v`.
PYTHON=""
for candidate in python3 python; do
  if "$candidate" -c 'import sys; sys.stdout.write("ok")' 2>/dev/null | grep -q '^ok$'; then
    PYTHON="$candidate"
    break
  fi
done
if [[ -z "$PYTHON" ]]; then
  echo "error: a working Python 3 interpreter is required to generate the macOS icon." >&2
  exit 1
fi

# --- Fonts ----------------------------------------------------------------
# Text uses committed DejaVu Sans TTFs so the wordmark/tagline render
# byte-for-byte identically on every platform (system fonts differ: Arial on
# Windows, DejaVu on Linux, Helvetica on macOS). Override with font file paths
# via RIVULET_FONT_BOLD / RIVULET_FONT_REGULAR if desired.
FONT_BOLD="${RIVULET_FONT_BOLD:-$REPO_ROOT/rivulet-gui/assets/fonts/DejaVuSans-Bold.ttf}"
FONT_REGULAR="${RIVULET_FONT_REGULAR:-$REPO_ROOT/rivulet-gui/assets/fonts/DejaVuSans.ttf}"
BOLD_ARGS=(-font "$FONT_BOLD")
REGULAR_ARGS=(-font "$FONT_REGULAR")

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

# Recolor the symbol for the thumbnail only: the original medium blue sits too
# close to the background gradient, so the hero uses the light brand accent.
# The app icons keep the original blue (better on light/transparent surfaces).
SYMBOL_LIGHT="$WORK/rivulet_symbol_light.png"
"$MAGICK" "$SYMBOL" -fill "$LIGHT_BLUE" -colorize 100 "$SYMBOL_LIGHT"

# 2. 1280x720 brand thumbnail: gradient + symbol + wordmark + tagline.
"$MAGICK" -size 1280x720 "gradient:${NAVY}-${BLUE}" \
  \( "$SYMBOL_LIGHT" -filter Mitchell -resize 'x260' \) -gravity north -geometry +0+35 -composite \
  -gravity north "${BOLD_ARGS[@]}" -fill white -pointsize 150 -annotate +0+320 'Rivulet' \
  -gravity north "${REGULAR_ARGS[@]}" -fill "$LIGHT_BLUE" -pointsize 44 -annotate +0+545 'Modern Screen Recording & Streaming' \
  "${STRIP[@]}" "$THUMBNAIL"

# 3. 512x512 transparent icon for the Linux AppImage. Resize up to fill the
#    448px safe area (no `>`: a same-size no-op resize resamples in a
#    filter-dependent way, so always do a real resize).
"$MAGICK" "$SYMBOL" -filter Mitchell -resize '448x448' -gravity center -background none -extent 512x512 \
  "${STRIP[@]}" "$ICON"

# 4. macOS app icon (.icns). The ICNS container stores one PNG per supported
#    size; the type codes mirror `iconutil`'s output (ic11..ic14 are the @2x
#    "retina" variants of the base sizes). Upscale the symbol to fill 1024px
#    first, so every smaller slice fills its canvas instead of floating small.
SQUARE="$WORK/rivulet_square.png"
"$MAGICK" "$SYMBOL" -filter Mitchell -resize '1024x1024' -gravity center -background none \
  -extent 1024x1024 "${STRIP[@]}" "$SQUARE"

ICONSET="$WORK/iconset"
mkdir -p "$ICONSET"
for size in 16 32 64 128 256 512 1024; do
  "$MAGICK" "$SQUARE" -filter Mitchell -resize "${size}x${size}" "${STRIP[@]}" "$ICONSET/icon_${size}.png"
done

"$PYTHON" "$SCRIPT_DIR/png2icns.py" "$ICNS" \
  "icp4=$ICONSET/icon_16.png" \
  "icp5=$ICONSET/icon_32.png" \
  "ic07=$ICONSET/icon_128.png" \
  "ic08=$ICONSET/icon_256.png" \
  "ic09=$ICONSET/icon_512.png" \
  "ic10=$ICONSET/icon_1024.png" \
  "ic11=$ICONSET/icon_32.png" \
  "ic12=$ICONSET/icon_64.png" \
  "ic13=$ICONSET/icon_256.png" \
  "ic14=$ICONSET/icon_512.png"

# 5. 1280x640 GitHub social preview (OpenGraph): same brand, 2:1 ratio, used
#    when release/repository links are shared. Reuses the light symbol.
"$MAGICK" -size 1280x640 "gradient:${NAVY}-${BLUE}" \
  \( "$SYMBOL_LIGHT" -filter Mitchell -resize 'x200' \) -gravity north -geometry +0+30 -composite \
  -gravity north "${BOLD_ARGS[@]}" -fill white -pointsize 130 -annotate +0+260 'Rivulet' \
  -gravity north "${REGULAR_ARGS[@]}" -fill "$LIGHT_BLUE" -pointsize 38 -annotate +0+455 'Modern Screen Recording & Streaming' \
  "${STRIP[@]}" "$SOCIAL"

# 6. 1200x630 OpenGraph fallback for X/Facebook/LinkedIn, same composition as
#    the GitHub card but at the more widely supported 1.91:1 ratio.
"$MAGICK" -size 1200x630 "gradient:${NAVY}-${BLUE}" \
  \( "$SYMBOL_LIGHT" -filter Mitchell -resize 'x200' \) -gravity north -geometry +0+30 -composite \
  -gravity north "${BOLD_ARGS[@]}" -fill white -pointsize 130 -annotate +0+255 'Rivulet' \
  -gravity north "${REGULAR_ARGS[@]}" -fill "$LIGHT_BLUE" -pointsize 38 -annotate +0+445 'Modern Screen Recording & Streaming' \
  "${STRIP[@]}" "$OPENGRAPH"

echo "Generated:"
echo "  $THUMBNAIL"
echo "  $SOCIAL"
echo "  $OPENGRAPH"
echo "  $ICON"
echo "  $ICNS"
