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

The multistreaming slice now builds a GStreamer fan-out with one named tee branch and RTMP sink per target. Target configuration and lifecycle state remain independent; the primary target is retained as a compatibility fallback. The reusable `StreamingReconnectSupervisor` now provides per-target failure isolation, bounded exponential backoff, retry limits, and reset-on-success semantics. Wiring bus messages and timers into the running GStreamer pipeline remains follow-up work.


## SRT/RIST contribution contract

The core now validates SRT/RIST contribution settings, bounds latency, validates optional SRT passphrases, redacts secrets from application-facing diagnostics, and emits protocol-specific sink fragments. This is the configuration slice; live GStreamer transport and interoperability evidence remain open.

## WHIP strategy spike

The WHIP/WebRTC strategy is documented in [`whip-strategy-spike.md`](whip-strategy-spike.md), and its roadmap rationale is recorded in [`obs-vision-roadmap.md`](obs-vision-roadmap.md).
The current core contract exposes `WhipSettings` with endpoint validation,
bounded signaling timeout, explicit trickle-ICE configuration, and token-safe
endpoint reporting. This is a strategy/configuration slice, not a live WebRTC
transport; the implementation-phase Definition of Done in that document remains
open.

## Reconnect and live bitrate integration

The reconnect supervisor is now connected to the engine's target status API. Callers handling GStreamer bus `Error`/`Eos` messages should call `handle_stream_target_failure`; after a successful sink restart they should call `handle_stream_target_live`. `SinkBusEvent` provides the tested mapping for fatal (`Error`/`Eos`), non-fatal (`Warning`), and successful state-change events. Retry state, limits, and exponential backoff are deterministic and fully tested. The actual asynchronous timer/branch rebuild remains platform-runtime work.

Adaptive bitrate remains bounded by the existing policy and updates the active `video_enc` property when present. Health polling, cooldown/hysteresis, and guaranteed execution on the GStreamer context remain follow-up work.

## Runtime supervision

`StreamingReconnectSupervisor` now exposes non-blocking `due_retries()` polling, so a runtime worker can schedule branch-local reconnects without blocking the capture or UI thread. `SinkBusEvent` maps fatal sink events to the target-local retry state. `AdaptiveBitrateController` requires stable health samples and enforces a cooldown before changing the encoder bitrate. `DelaySupervisor` bounds and retains delayed media across a temporary disconnect.

SRT/RIST settings now generate protocol-specific sink fragments including latency and, for SRT, the validated passphrase. `ContributionSettings::plugin_available()` provides a safe runtime check for `srtsink`/`ristsink` before a branch is started; missing plugins can therefore be reported per target instead of failing the complete fan-out. Receiver interoperability must still be verified on each supported platform.

## Scope boundary

The adaptive-bitrate implementation provides a bounded policy and applies changed values to the active `video_enc` bitrate property. Health polling/network measurement and cooldown policy remain follow-up work.

Stream delay is applied per fan-out branch as a bounded outgoing stage and exposes reconnect delay semantics; reconnect-aware buffering and retry backoff remain follow-up work. Multitrack Video currently validates and carries the representation count,
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
- [ ] Verify one target failure cannot stop healthy targets; the supervisor contract
      already isolates retry state, while live bus-message wiring remains an
      integration check.
- [ ] Configure SRT and RIST endpoints and verify the protocol scheme, latency bounds, and passphrase validation.
- [ ] Validate the WHIP endpoint and verify that non-loopback HTTP is rejected;
      confirm bearer tokens never appear in status, logs, or diagnostics.
- [ ] Test the view at the M3 viewport profiles from the common quality-gate
      matrix, including narrow windows and long custom endpoint names.

## Decision

This increment passes its feature gate. M3 as a whole remains in progress until
live encoder reconfiguration, reconnect/delay, and the remaining protocol and
multistreaming work are complete.
