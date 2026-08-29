#!/usr/bin/env bash
set -euo pipefail
IMAGE="${RIST_SMOKE_IMAGE:-rivulet-rist-smoke:ci}"
NETWORK="rivulet-rist-smoke-$$"
RECEIVER="rivulet-rist-receiver-$$"
SENDER="rivulet-rist-sender-$$"
cleanup() { docker rm -f "$RECEIVER" "$SENDER" >/dev/null 2>&1 || true; docker network rm "$NETWORK" >/dev/null 2>&1 || true; }
trap cleanup EXIT
docker network create "$NETWORK" >/dev/null
docker run -d --name "$RECEIVER" --network "$NETWORK" "$IMAGE" sh -ceu 'timeout --signal=TERM --kill-after=5s 30s gst-launch-1.0 -e ristsrc address=0.0.0.0 port=10080 ! identity silent=false dump=true name=receive_probe ! fakesink sync=false 2>&1' >/dev/null
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
sender_status=0
set +e
docker run --rm --name "$SENDER" --network "$NETWORK" "$IMAGE" \
  timeout --signal=TERM --kill-after=5s 10s \
  gst-launch-1.0 -e videotestsrc num-buffers=30 ! videoconvert ! \
  x264enc tune=zerolatency ! h264parse ! mpegtsmux alignment=7 ! \
  rtpmp2tpay ! \
  ristsink address="$RECEIVER" port=10080
sender_status=$?
set -e
if [ "$sender_status" -ne 0 ] && [ "$sender_status" -ne 124 ]; then
  echo "RIST sender failed with exit code $sender_status" >&2
  docker logs "$RECEIVER" >&2 || true
  exit "$sender_status"
fi
# Confirm that the receiver actually processed at least one buffer. A running
# receiver alone only proves that GStreamer started, not interoperability.
# `identity dump=true` emits a stable hexadecimal buffer dump, unlike the
# human-readable `last-message` field which is not printed by all GStreamer
# versions.
received_buffers="$(docker logs "$RECEIVER" 2>&1 | grep -Ec '^[0-9a-fA-F]{2}([[:space:]][0-9a-fA-F]{2})+' || true)"
if [ "$received_buffers" -eq 0 ]; then
  echo "RIST receiver received no buffers" >&2
  docker logs "$RECEIVER" >&2 || true
  exit 1
fi
echo "RIST receiver smoke test passed (image: $IMAGE, buffers: $received_buffers)"
