#!/usr/bin/env bash
set -euo pipefail

for script in scripts/srt-receiver-smoke.sh scripts/rist-receiver-smoke.sh; do
  test -f "$script"
  grep -q 'SMOKE_IMAGE:-' "$script"
  grep -q 'trap cleanup EXIT' "$script"
  grep -Eq 'timeout|kill-after' "$script"
done

echo "transport smoke scripts satisfy bounded cleanup contract"
