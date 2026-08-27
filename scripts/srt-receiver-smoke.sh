#!/usr/bin/env bash
set -euo pipefail

IMAGE="${SRT_SMOKE_IMAGE:-ossrs/srs:5.0.184}"
NETWORK="rivulet-srt-smoke-$$"
PORT="${SRT_SMOKE_PORT:-10080}"
cleanup() {
  docker rm -f rivulet-srt-receiver-$$ rivulet-srt-sender-$$ >/dev/null 2>&1 || true
  docker network rm "$NETWORK" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker network create "$NETWORK" >/dev/null
docker run -d --name "rivulet-srt-receiver-$$" --network "$NETWORK" "$IMAGE" >/dev/null
sleep 2

docker run --rm --name "rivulet-srt-sender-$$" --network "$NETWORK" \
  -e GST_DEBUG=2 \
  "$IMAGE" sh -ceu '
    command -v gst-launch-1.0 >/dev/null
    gst-launch-1.0 -e \
      videotestsrc num-buffers=30 ! videoconvert ! x264enc tune=zerolatency ! mpegtsmux ! \
      srtsink uri="srt://rivulet-srt-receiver:'"$PORT"'" wait-for-connection=false
  '

echo "SRT receiver smoke test passed (image: $IMAGE)"
