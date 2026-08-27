#!/usr/bin/env bash
set -euo pipefail
IMAGE="${RIST_SMOKE_IMAGE:-rivulet-rist-smoke:ci}"
NETWORK="rivulet-rist-smoke-$$"
RECEIVER="rivulet-rist-receiver-$$"
SENDER="rivulet-rist-sender-$$"
cleanup() { docker rm -f "$RECEIVER" "$SENDER" >/dev/null 2>&1 || true; docker network rm "$NETWORK" >/dev/null 2>&1 || true; }
trap cleanup EXIT
docker network create "$NETWORK" >/dev/null
docker run -d --name "$RECEIVER" --network "$NETWORK" "$IMAGE" sh -ceu 'gst-launch-1.0 -q -e ristsrc address=0.0.0.0 port=10080 ! fakesink sync=false' >/dev/null
sleep 2
docker run --rm --name "$SENDER" --network "$NETWORK" "$IMAGE" \
  gst-launch-1.0 -e videotestsrc num-buffers=30 ! videoconvert ! \
  x264enc tune=zerolatency ! h264parse ! mpegtsmux alignment=7 ! \
  rtpmp2tpay ! ristsink address="$RECEIVER" port=10080 >/dev/null
echo "RIST receiver smoke test passed (image: $IMAGE)"
