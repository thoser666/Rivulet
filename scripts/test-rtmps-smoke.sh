#!/usr/bin/env bash
set -euo pipefail

script="$(dirname "$0")/rtmps-smoke.sh"
grep -q '127.0.0.1' "$script"
grep -q 'RTMPS_SMOKE_IMAGE' "$script"
grep -q 'videotestsrc' "$script"
grep -q 'rtmpsink' "$script"
! grep -q 'RIVULET_STREAM_KEY' "$script"
echo "RTMPS smoke script contract passed"
