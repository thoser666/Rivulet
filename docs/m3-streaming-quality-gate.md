# M3 Streaming Quality Gate

This document records the feature-level gate for stream-key management, stream
presets, adaptive bitrate, and the reviewed WHIP strategy. It supplements the full M3 gate in
[`milestone-quality-gates.md`](milestone-quality-gates.md).

## Completed in this increment

- Platform presets for Twitch, YouTube, Kick, and custom ingest URLs.
- Validation before connection:
  - stream key must be non-empty;
  - ingest URL must use `rtmp://` or `rtmps://`;
  - built-in platform presets require `rtmps://`.
- Masked stream-key representation for UI and diagnostics.
- Low, Standard, High, and Custom quality presets.
- Bounded adaptive-bitrate policy with minimum, maximum, and step values.
- Policy behavior:
  - `Poor` health lowers bitrate by one step, never below the minimum;
  - `Good` health raises bitrate by one step, never above the maximum;
  - `Warning`/`Connecting` hold the current bounded value;
  - disabled adaptive bitrate never changes the configured bitrate.
- Configurable stream delay is bounded to five minutes and represented in the
  pipeline as a non-blocking delay element.
- Multitrack Video configuration accepts one to four representations and keeps
  the existing single-track transport behavior explicit until negotiation lands.
- Multistream target configuration supports up to four independently named
  targets, validates each endpoint/key, rejects duplicate names, masks keys, and
  removes targets without mutating unrelated entries.

## Automated evidence

- Stream model tests cover endpoint defaults, validation, masking, preset values,
  adaptive-bitrate bounds, disabled behavior, and compatibility with existing
  pipeline location construction.
- Existing workspace tests continue to cover RTMP/RTMPS pipeline construction,
  health classification, and dual-output routing.
- Multistream model tests cover target limits, duplicate-name rejection, invalid
  target immutability, removal, and secret masking.

## Multistreaming scope

The first multistreaming slice is the shared, validated target model. Each target
has an independent identity and configuration, which is the basis for later
transport fan-out and per-target health/retry isolation. The current GStreamer
pipeline still has one RTMP sink; it must not be presented as simultaneous live
fan-out until that transport integration is implemented.

## WHIP strategy spike

The WHIP/WebRTC strategy is documented in [`whip-strategy-spike.md`](whip-strategy-spike.md), and its roadmap rationale is recorded in [`obs-vision-roadmap.md`](obs-vision-roadmap.md).
The current core contract exposes `WhipSettings` with endpoint validation,
bounded signaling timeout, explicit trickle-ICE configuration, and token-safe
endpoint reporting. This is a strategy/configuration slice, not a live WebRTC
transport; the implementation-phase Definition of Done in that document remains
open.

## Scope boundary

The adaptive-bitrate implementation currently provides a deterministic policy
layer. It does not yet mutate a running GStreamer encoder or reconnect a live
stream. That live reconfiguration is a separate M3 follow-up and must not be
claimed as complete by the UI or release notes.

Stream delay is currently applied to the outgoing pipeline as a bounded delay
stage; reconnect-aware buffering and user-visible live controls remain follow-up
work. Multitrack Video currently validates and carries the representation count,
but the RTMP/FLV transport still emits one negotiated track. Per-track encoder
and transport negotiation requires a later protocol implementation.

Likewise, stream keys are masked in the application-facing representation, but a
future OS-backed credential store should replace ordinary in-memory/config-file
storage before stable releases. Until then, keys must be supplied through the
existing protected runtime configuration and must never be committed, logged, or
included in screenshots.

## Manual review checklist

- [ ] Enter an empty key and confirm the stream action remains blocked with an
      actionable validation message.
- [ ] Select Twitch, YouTube, Kick, and Custom and verify the endpoint label and
      TLS status are visible.
- [ ] Confirm the key field is masked and copying does not expose it in logs,
      diagnostics, or screenshots.
- [ ] Select each quality preset and verify the effective bitrate is visible.
- [ ] Enable adaptive bitrate and simulate Poor/Good health; verify changes remain
      inside the configured bounds.
- [ ] Enable a typical stream delay and verify the status explains the added
      latency; test the maximum bound and disabled state.
- [ ] Configure two to four video representations and verify the selected count;
      confirm the UI does not claim that RTMP/FLV is carrying multiple tracks yet.
- [ ] Add multiple stream targets, verify duplicate names and invalid endpoints are
      rejected, and confirm each target's key remains masked.
- [ ] Verify one target failure cannot stop healthy targets once transport fan-out
      is implemented; until then, record this as an open integration check.
- [ ] Validate the WHIP endpoint and verify that non-loopback HTTP is rejected;
      confirm bearer tokens never appear in status, logs, or diagnostics.
- [ ] Test the view at the M3 viewport profiles from the common quality-gate
      matrix, including narrow windows and long custom endpoint names.

## Decision

This increment passes its feature gate. M3 as a whole remains in progress until
live encoder reconfiguration, reconnect/delay, and the remaining protocol and
multistreaming work are complete.
