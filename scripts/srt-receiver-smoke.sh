#!/usr/bin/env bash
set -euo pipefail

IMAGE="${SRT_SMOKE_IMAGE:-rivulet-srt-smoke:ci}"
NETWORK="rivulet-srt-smoke-$$"
RECEIVER="rivulet-srt-receiver-$$"
SENDER="rivulet-srt-sender-$$"
PORT="${SRT_SMOKE_PORT:-10080}"
cleanup() {
  docker rm -f "$RECEIVER" "$SENDER" >/dev/null 2>&1 || true
  docker network rm "$NETWORK" >/dev/null 2>&1 || true
}
trap cleanup EXIT

docker network create "$NETWORK" >/dev/null
docker run -d --name "$RECEIVER" --network "$NETWORK" "$IMAGE" \
  sh -ceu 'gst-launch-1.0 -q -e srtsrc uri="srt://:10080?mode=listener" ! fakesink sync=false' >/dev/null

for _ in $(seq 1 30); do
  if docker exec "$RECEIVER" sh -c 'pgrep -f gst-launch-1.0 >/dev/null' 2>/dev/null; then break; fi
  sleep 0.5
done

docker run --rm --name "$SENDER" --network "$NETWORK" "$IMAGE" \
  gst-launch-1.0 -e videotestsrc num-buffers=30 ! videoconvert ! x264enc tune=zerolatency ! mpegtsmux ! \
  srtsink uri="srt://$RECEIVER:$PORT?mode=caller" wait-for-connection=false

echo "SRT receiver smoke test passed (image: $IMAGE)"
