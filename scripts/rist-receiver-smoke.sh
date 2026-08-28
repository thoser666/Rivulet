#!/usr/bin/env bash
set -euo pipefail
IMAGE="${RIST_SMOKE_IMAGE:-rivulet-rist-smoke:ci}"
NETWORK="rivulet-rist-smoke-$$"
RECEIVER="rivulet-rist-receiver-$$"
SENDER="rivulet-rist-sender-$$"
cleanup() { docker rm -f "$RECEIVER" "$SENDER" >/dev/null 2>&1 || true; docker network rm "$NETWORK" >/dev/null 2>&1 || true; }
trap cleanup EXIT
docker network create "$NETWORK" >/dev/null
docker run -d --name "$RECEIVER" --network "$NETWORK" "$IMAGE" sh -ceu 'timeout --signal=TERM --kill-after=5s 30s gst-launch-1.0.0 -q -e ristsrc address=0.0.0.0 port=10080 ! fakesink sync=false' >/dev/null
receiver_ready=false
for _ in $(seq 1 20); do
  if docker inspect -f '{{.State.Running}}' "$RECEIVER" 2>/dev/null | grep -q true; then
    receiver_ready=true
    break
  fi
  if ! docker inspect -f '{{.State.Running}}' "$RECEIVER" 2>/dev/null | grep -q true; then
    echo "RIST receiver exited before becoming ready" >&2
    docker logs "$RECEIVER" >&2 || true
    exit 1
  fi
  sleep 0.5
done
if [ "$receiver_ready" != true ]; then
  echo "RIST receiver did not remain running within 10 seconds" >&2
  docker logs "$RECEIVER" >&2 || true
  exit 1
fi
docker run --rm --name "$SENDER" --network "$NETWORK" "$IMAGE" \
  timeout --signal=TERM --kill-after=5s 30s \
  gst-launch-1.0 -e videotestsrc num-buffers=30 ! videoconvert ! \
  x264enc tune=zerolatency ! h264parse ! mpegtsmux alignment=7 ! \
  rtpmp2tpay ! \
  ristsink address="$RECEIVER" port=10080 >/dev/null
# A finite sender must terminate; the receiver is deliberately bounded so a
# failed handshake cannot leave the CI job waiting indefinitely.
echo "RIST receiver smoke test passed (image: $IMAGE)"
