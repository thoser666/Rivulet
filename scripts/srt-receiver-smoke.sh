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
  sh -ceu 'timeout --signal=TERM --kill-after=5s 30s gst-launch-1.0 -q -e srtsrc uri="srt://:10080?mode=listener" ! fakesink sync=false' >/dev/null

receiver_ready=false
for _ in $(seq 1 30); do
  if docker inspect -f '{{.State.Running}}' "$RECEIVER" 2>/dev/null | grep -q true; then
    receiver_ready=true
    break
  fi
  sleep 0.5
done
if [ "$receiver_ready" != true ]; then
  echo "SRT receiver did not become ready" >&2
  docker logs "$RECEIVER" >&2 || true
  exit 1
fi

docker run --rm --name "$SENDER" --network "$NETWORK" "$IMAGE" \
  timeout --signal=TERM --kill-after=5s 10s \
  gst-launch-1.0 -e videotestsrc num-buffers=30 ! videoconvert ! x264enc tune=zerolatency ! \
  mpegtsmux ! srtsink uri="srt://$RECEIVER:$PORT?mode=caller" wait-for-connection=false

echo "SRT receiver smoke test passed (image: $IMAGE)"
