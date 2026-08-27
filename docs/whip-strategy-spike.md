# WHIP/WebRTC strategy spike

## Decision

Rivulet should implement WHIP as a first-class low-latency streaming protocol,
but not by extending the RTMP URL builder. WHIP requires an HTTP offer/answer
exchange followed by a WebRTC media session:

1. Build a GStreamer `webrtcbin` pipeline for H.264 and Opus/Audio.
2. Create an SDP offer and wait for ICE gathering.
3. POST the offer to the configured WHIP endpoint with
   `Content-Type: application/sdp`.
4. Apply the SDP answer returned by the SFU/WHIP server.
5. Keep the returned resource URL and use HTTP DELETE during a clean stop.
6. Use trickle ICE only when the endpoint advertises support; otherwise send a
   complete gathered offer.

The initial implementation should use GStreamer's `webrtcbin` and a small Rust
HTTP/SDP session adapter. This keeps encoding and media timestamps in the
existing pipeline while keeping HTTP signaling testable and replaceable.

## Configuration contract

A future `WhipSettings` API should contain:

- `endpoint: https://...` — HTTPS is required except for an explicit localhost
  development endpoint;
- optional bearer token, held in memory and redacted from logs;
- STUN server list and optional TURN server credentials;
- connect timeout and bounded reconnect policy;
- requested video/audio codecs and bitrate policy;
- whether trickle ICE is allowed.

The stream UI must display the endpoint host, connection state, ICE state, and
whether the session is local or remote, but never display bearer tokens.

## Overhead and compatibility budget

- Signaling timeout: 10 seconds by default, configurable within 2–60 seconds.
- No busy-waiting on the UI thread; signaling runs asynchronously.
- Target one encoded video path and one audio path initially.
- Initial compatibility target: H.264 video and Opus audio, with SFUs that accept
  standard WHIP SDP.
- A failed WHIP session must not stop an unrelated local recording or RTMP
  target when dual-output orchestration is added.

## Security and failure behavior

- Reject non-HTTPS endpoints except loopback development URLs.
- Do not put bearer tokens in URLs, SDP logs, reports, crash dumps, or errors.
- Validate HTTP status and `application/sdp` response before applying an answer.
- Treat 401/403 as authentication errors, 409/4xx as negotiation errors, and
  5xx/network failures as retryable according to bounded backoff.
- DELETE the WHIP resource on clean shutdown when the server supplied a resource
  URL. A failed DELETE is logged as cleanup-only and must not mask the recording
  result.

## Explicit non-goals of this spike

The signaling slice now provides `WhipSettings::post_offer`, which sends an SDP
offer with the required `application/sdp` headers, parses the SDP answer and
resource `Location`, and maps authentication, server, HTTP, content-type, and
empty-answer failures to deterministic errors. It is deliberately synchronous
so callers can place it on a worker thread; it does not claim to be the media
transport. The implementation now exposes a deterministic `webrtcbin` pipeline contract
and runtime availability probe. The remaining evidence is the real SFU
offer/answer, ICE/DTLS media handshake, cleanup, and platform smoke run.

## Definition of done for the implementation phase

- `WhipSettings` validates endpoint, timeout, token handling, and ICE options.
- SDP offer/answer exchange is covered by deterministic HTTP mock tests.
- GStreamer `webrtcbin` pipeline starts and reports state transitions (the element contract and availability probe are implemented; full SFU media negotiation remains the final integration item).
- At least one local SFU/WHIP smoke test publishes H.264 + Opus successfully.
- Reconnect and cleanup behavior are tested.
- GUI exposes status and actionable errors without secrets.
- M3 quality gate records platform/protocol evidence and redacted diagnostics.
