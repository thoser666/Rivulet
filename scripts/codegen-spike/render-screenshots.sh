#!/usr/bin/env bash
# render-screenshots.sh — Screenshot every generated overlay artifact with
# headless Edge (WebView2 engine, same as Rivulet's browser source on Windows).
#
# Produces results/<model>/<prompt-name>.png next to each artifact. The PNGs
# are the "visual pass" input for the spike score; they are gitignored.
#
# Usage: scripts/codegen-spike/render-screenshots.sh [results-dir]

set -uo pipefail
cd "$(dirname "$0")"
RESULTS="${1:-results}"

EDGE_CANDIDATES=(
  "/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe"
  "/c/Program Files/Microsoft/Edge/Application/msedge.exe"
)
EDGE=""
for c in "${EDGE_CANDIDATES[@]}"; do
  [[ -f "$c" ]] && EDGE="$c" && break
done
if [[ -z "$EDGE" ]]; then
  echo "ERROR: msedge.exe not found (WebView2 host required for the visual pass)" >&2
  exit 2
fi

shot=0
failed=0
for html in "$RESULTS"/*/*.html; do
  [[ -e "$html" ]] || continue
  png="${html%.html}.png"
  win_path=$(cygpath -w "$(realpath "$html")")
  png_abs=$(cygpath -w "$(realpath "$png")")
  if "$EDGE" --headless=new --disable-gpu --hide-scrollbars \
       --window-size=1280,720 --virtual-time-budget=3500 \
       --screenshot="$png_abs" \
       "file:///${win_path//\\//}" >/dev/null 2>&1 && [[ -s "$png" ]]; then
    echo "  shot: $png ($(stat -c%s "$png" 2>/dev/null || wc -c < "$png") bytes)"
    shot=$((shot + 1))
  else
    echo "  FAILED screenshot: $html" >&2
    failed=$((failed + 1))
  fi
done
echo "screenshots: $shot ok, $failed failed"
exit $failed
