#!/usr/bin/env bash
set -euo pipefail

# This validates the local GStreamer RTMP(S) pipeline contract only. The local
# listener is intentionally not an RTMP server; the expected sink connection
# failure proves endpoint handling but cannot validate RTMP handshaking. Set
# RTMPS_SMOKE_URL to a real test ingest to run the optional network check.
# It never requires a real stream key by default.
IMAGE="${RTMPS_SMOKE_IMAGE:-}"
GST_LAUNCH=(gst-launch-1.0)
if [[ -n "$IMAGE" ]]; then
  GST_LAUNCH=(docker run --rm "$IMAGE" gst-launch-1.0)
fi

if ! command -v "${GST_LAUNCH[0]}" >/dev/null 2>&1 && [[ -z "$IMAGE" ]]; then
  echo "gst-launch-1.0 is required (or set RTMPS_SMOKE_IMAGE)" >&2
  exit 2
fi

port="${RTMPS_SMOKE_PORT:-19355}"
target="${RTMPS_SMOKE_URL:-rtmp://127.0.0.1:${port}/smoke/key}"
log="$(mktemp)"
listener_pid=""
cleanup() {
  if [[ -n "$listener_pid" ]]; then kill "$listener_pid" 2>/dev/null || true; fi
  rm -f "$log"
}
trap cleanup EXIT

python3 - "$port" <<'PY' &
import socket, sys
port = int(sys.argv[1])
server = socket.socket()
server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
server.bind(("127.0.0.1", port))
server.listen(1)
conn, _ = server.accept()
conn.settimeout(3)
try:
    while conn.recv(4096):
        pass
except (TimeoutError, ConnectionError, OSError):
    pass
finally:
    conn.close()
    server.close()
PY
listener_pid=$!
sleep 0.2

set +e
"${GST_LAUNCH[@]}" -q \
  videotestsrc num-buffers=30 is-live=true ! videoconvert ! \
  x264enc tune=zerolatency bitrate=500 key-int-max=30 ! h264parse ! flvmux streamable=true ! \
  rtmpsink location="$target" \
  >"$log" 2>&1
status=$?
set -e

if [[ -z "${RTMPS_SMOKE_URL:-}" ]]; then
  # A raw TCP listener cannot complete RTMP handshaking. A non-zero status is
  # therefore expected and confirms that GStreamer reached the local endpoint.
  if grep -q 'Could not connect to RTMP stream' "$log"; then
    echo "RTMPS local endpoint smoke passed (expected handshake boundary)."
    exit 0
  fi
  cat "$log" >&2
  echo "RTMPS local endpoint smoke failed before reaching the listener" >&2
  exit "$status"
fi

if [[ "$status" -ne 0 ]]; then
  cat "$log" >&2
  echo "RTMPS test-ingest smoke failed" >&2
  exit "$status"
fi

echo "RTMPS test-ingest smoke passed: $target"
