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

## RIST interoperability

The RIST sink uses the plugin's supported `address`, `port`, and `latency`
properties rather than the unsupported `uri` property. CI now includes a dedicated RIST receiver smoke job using
`docker/rist-smoke/Dockerfile` and `scripts/rist-receiver-smoke.sh`. The test
uses a finite MPEG-TS test source, a listener receiver, and a caller sender;
plugin availability and real receiver compatibility remain explicit evidence.

## Resource-efficiency evidence

The cross-cutting resource gate is executable via
`scripts/resource-efficiency-check.py`. It validates CPU delta, memory growth,
p99 frame time, and frame-time regression using a shared JSON report shape.
The CI fixture is intentionally synthetic; real GPU/CPU measurements remain
required as documented hardware evidence before claiming the M3 budget.

## Automated evidence

- Stream model tests cover endpoint defaults, validation, masking, preset values,
  adaptive-bitrate bounds, disabled behavior, and compatibility with existing
  pipeline location construction.
- Existing workspace tests continue to cover RTMP/RTMPS pipeline construction,
  health classification, and dual-output routing.
- Multistream model tests cover target limits, duplicate-name rejection, invalid
  target immutability, removal, and secret masking.

## Multistreaming scope

The multistreaming slice now builds a GStreamer fan-out with one dynamically addressable branch per target. Each branch owns a bounded input queue, a branch tee, an output queue, the configured delay stage, and a target-specific sink; the complete branch can therefore be torn down and recreated without affecting peer targets. RTMP/RTMPS targets use `rtmp2sink`; SRT/RIST targets use their validated `srtsink`/`ristsink` fragment and retain the same target-local lifecycle. Target configuration and lifecycle state remain independent; the primary target is retained as a compatibility fallback. The reusable `StreamingReconnectSupervisor` provides per-target failure isolation, bounded exponential backoff, retry limits, and reset-on-success semantics. `RivuletEngine::poll_stream_bus()` drains pending GStreamer bus messages without blocking, maps only `stream_sink_<index>` sources to target-local events, and starts retries for fatal sink events. `RivuletEngine::poll_stream_reconnects()` exposes non-blocking due-retry claims. On a target failure, `ReconnectWorker` now starts a cancellable target-local timer, and `RivuletEngine::drain_reconnect_commands()` lets the pipeline owner consume the resulting `RebuildBranch` command on its GStreamer context. `resolve_reconnect_command()` validates the command against the current target list before any mutation. Sink names are parsed strictly with `target_index_from_sink_name()`. The worker deliberately does not mutate GStreamer from a background thread; `rebuild_stream_target()` performs target-local queue/tee/delay/sink branch teardown and recreation on the pipeline-owner context, then synchronizes the replacement sink with the running pipeline. A missing sink or target returns an error without stopping peer targets.


## SRT/RIST contribution contract

The core now validates SRT/RIST contribution settings, bounds latency, validates optional SRT passphrases, redacts secrets from application-facing diagnostics, and emits protocol-specific sink fragments. The fan-out builder selects the matching sink element and refreshes plugin availability before runtime use; unavailable plugins are represented as a target-local `Unavailable` status and must not be reported as a healthy stream. Receiver interoperability is covered by `scripts/srt-receiver-smoke.sh`, which runs a finite GStreamer test stream against a receiver in a reproducible container image. CI builds `docker/srt-smoke/Dockerfile` from the supported Ubuntu 24.04 base and refreshes it explicitly with `--pull=true`; the package installation is non-interactive and the smoke test runs as a required job; real-world endpoint testing remains an explicit runtime evidence requirement.

## WHIP strategy spike

The WHIP/WebRTC strategy is documented in [`whip-strategy-spike.md`](whip-strategy-spike.md), and its roadmap rationale is recorded in [`obs-vision-roadmap.md`](obs-vision-roadmap.md).
The current core contract exposes `WhipSettings` with endpoint validation,
bounded signaling timeout, explicit trickle-ICE configuration, token-safe
endpoint reporting, and a deterministic `webrtcbin` media-branch contract.
The real SFU/ICE/DTLS media handshake remains the implementation-phase evidence;
`webrtcbin` availability and the deterministic branch contract are tested locally.

## Reconnect and live bitrate integration

The reconnect supervisor is now connected to the engine's target status API. Callers handling GStreamer bus `Error`/`Eos` messages should call `handle_stream_target_failure`; after a successful sink restart they should call `handle_stream_target_live`. `SinkBusEvent` provides the tested mapping for fatal (`Error`/`Eos`), non-fatal (`Warning`), and successful state-change events. Retry state, limits, and exponential backoff are deterministic and fully tested. The pipeline-owner `service_streaming()` tick now polls bus messages, drains due reconnect commands, and invokes target-local branch rebuilds without moving GStreamer mutations to the worker thread. Rebuilds preserve the configured transport by reconstructing the validated RTMP/RTMPS, SRT, or RIST sink fragment rather than hard-coding RTMP. Duplicate failures are ignored while a retry is already pending, preventing retry storms.

Adaptive bitrate remains bounded by the existing policy and updates the active `video_enc` property when present. `service_streaming()` now samples `StreamHealthMonitor` and invokes the runtime controller in the pipeline-owner tick; cooldown and hysteresis prevent flapping. Production hardware measurement remains gate evidence.

## Runtime supervision

`StreamingReconnectSupervisor` now exposes non-blocking `due_retries()` polling, so a runtime worker can schedule branch-local reconnects without blocking the capture or UI thread. `SinkBusEvent` maps fatal sink events to the target-local retry state. `AdaptiveBitrateController` requires stable health samples and enforces a cooldown before changing the encoder bitrate. `DelaySupervisor` bounds and retains delayed media across a temporary disconnect.

SRT/RIST settings now generate protocol-specific sink fragments including latency and, for SRT, the validated passphrase. `ContributionSettings::plugin_available()` provides a safe runtime check for `srtsink`/`ristsink` before a branch is started; missing plugins can therefore be reported per target instead of failing the complete fan-out. Receiver interoperability must still be verified on each supported platform.

## Scope boundary

The adaptive-bitrate implementation provides a bounded policy and applies changed values to the active `video_enc` bitrate property. `AdaptiveBitrateController` adds stable-sample and cooldown protection; wiring real network telemetry into that controller remains follow-up work.

Stream delay is applied per fan-out branch as a bounded outgoing stage. `DelaySupervisor` retains bounded media across reconnects and counts overflow drops; each live branch now owns queue and delay stages, so queue overflow/underflow behavior is observable and testable. Multitrack Video currently validates and carries the representation count,
but the RTMP/FLV transport still emits one negotiated track. Per-track encoder
and transport negotiation requires a later protocol implementation.

Likewise, stream keys are masked in the application-facing representation, but a
future OS-backed credential store should replace ordinary in-memory/config-file
storage before stable releases. Until then, keys must be supplied through the
existing protected runtime configuration and must never be committed, logged, or
included in screenshots.

## Release verification

The alpha release workflow is intentionally two-phase: it first prepares a
release branch, then builds all platform packages, and only after the matrix
succeeds creates or reuses the tag and publishes the GitHub Release. A tag
without a release is considered a failed run and must be repaired with
`scripts/backfill-releases.sh --check` followed by the targeted backfill.
Retries for the same alpha version reuse the existing `release/<tag>` branch.
The workflow resets the generated version/changelog changes before switching to
that branch and skips an empty version-bump commit, preventing checkout and
non-fast-forward failures.
Manual verification should inspect the workflow summary and confirm the tag,
release URL, prerelease flag, and uploaded assets.

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
- [ ] Configure SRT and RIST endpoints and verify the protocol scheme, latency bounds, passphrase validation, and target-local missing-plugin status.
- [ ] Validate the WHIP endpoint and verify that non-loopback HTTP is rejected;
      confirm bearer tokens never appear in status, logs, or diagnostics.
- [ ] Test the view at the M3 viewport profiles from the common quality-gate
      matrix, including narrow windows and long custom endpoint names.

## Decision

This increment passes its feature gate. M3 as a whole remains in progress until
live encoder reconfiguration, reconnect/delay, and the remaining protocol and
multistreaming work are complete.
