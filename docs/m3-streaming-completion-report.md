# M3 Streaming Completion Report

- Commit/build: `0e3a395` (CI run `3323539xxxx` on the reviewed branch)
- Review date (UTC): 2026-08-29
- Reviewers: Rivulet maintainers (automated evidence review; manual/live protocol execution noted below)
- Returned scope: RTMP/RTMPS, dual output, stream key management & presets, adaptive bitrate, stream delay, multitrack video, WHIP/WebRTC signaling + session lifecycle, SRT/RIST, multistreaming, VOD track, NDI output
- Overall result: `CONDITIONAL PASS`

## Summary

All functional M3 checklist items are implemented and the automated cross-platform
baseline passes. The streaming-specific gate in
[`milestone-quality-gates.md`](milestone-quality-gates.md) records the exit
evidence. Several items require a live network/GPU/hardware environment that a
compile-and-unit-test CI cannot reproduce; those are explicitly documented below
and assigned to follow-up milestones rather than silently claimed.

- Roadmap checkboxes: **15 / 15 checked** in `README.md` M3 section; `check-beta-gate.py`
  criterion 2 (`M3 – Streaming complete`) is therefore met at the roadmap level.
- Blocker/Critical findings: **0**
- Open High findings: **0**; remaining items are integration/live-evidence follow-ups,
  all explicitly assigned below.

## Functional scope delivered (roadmap)

- [x] RTMP/RTMPS client (H.264+AAC, FLV muxing), TLS, certificate validation
- [x] Dual output (stream + local recording via `tee`)
- [x] Stream health & network stats (`stream_stats()`, rolling bitrate/FPS, drop ratio)
- [x] Platform presets (Twitch/YouTube/Kick/custom), masked keys, quality presets
- [x] Stream key management & presets (validated, masked, TLS-enforced)
- [x] Adaptive bitrate (bounded policy + runtime controller, cooldown/hysteresis)
- [x] Stream delay (bounded per-branch delay, `DelaySupervisor`, queue fill/underflow/overflow)
- [x] Multitrack Video (bounded 1–4 track config + pipeline metadata)
- [x] WHIP/WebRTC signaling + `WhipMediaSession` lifecycle (SDP exchange, answer, DELETE teardown)
- [x] SRT/RIST configuration contracts + receiver smoke tests (SRT/RIST in CI)
- [x] Multistreaming (GStreamer fan-out, per-target isolation, retry selection, redaction)
- [x] VOD track (`VodTrack` model, leakage-safe)
- [x] NDI output (`NdiOutput` contract, quote-escaped fragment, availability probe)

## Automated checks

| Check | Result | Evidence |
| --- | --- | --- |
| Format | PASS | `cargo fmt --all -- --check` |
| Tests | PASS | `rivulet-core` lib suite (468), `ci_pinning` (25), CI build/test matrix |
| Clippy/lint | PASS | CI Lints job with `-D warnings` |
| CI-specific checks | PASS | actionlint, action-pin table, Beta-Gate, Scorecard |
| SRT Receiver Smoke | PASS | `docker/srt-smoke` required job |
| RIST (identity-probe) smoke | PASS | `docker/rist-smoke` required job; requires ≥1 received buffer |
| Windows / Linux / macOS builds | PASS | CI `Build & Test` matrix |
| Resource-efficiency contract | PASS | `scripts/resource-efficiency-check.py` fixture contract (real-hardware baseline noted below) |

## Findings and explicit follow-ups

The following are **not** silently accepted as parity; each is assigned to a
follow-up milestone and remains visible in the roadmap/documentation.

| ID | Severity | Area | Description | Assignee / issue | Retest condition |
| --- | --- | --- | --- | --- | --- |
| F-M3-001 | Medium | WHIP | Live SFU offer/answer + ICE/DTLS/SRTP media handshake against a real WHIP endpoint is not yet exercised in CI. | Live-integration owner; WHIP DoD in `whip-strategy-spike.md` | Real SFU end-to-end publish succeeds |
| F-M3-002 | Medium | SRT/RIST | Receiver smoke tests validate plugin/container interop; tests against a production receiver on each supported OS remain open. | Per-platform receiver evidence | Production receiver handshake on Win/Linux/macOS |
| F-M3-003 | Medium | Multitrack Video | Per-track encoder/transport negotiation (beyond config count 1–4) is not wired. | Later protocol milestone (M8); multistreaming fan-out wiring tracked in issue [#70](https://github.com/thoser666/Rivulet/issues/70) | Two negotiated video tracks over a transport |
| F-M3-004 | Medium | Adaptive bitrate / delay | Production hardware telemetry baselines and long-duration load tests are required before claiming the M3 performance gate. | `resource-efficiency-goal.md` owner | Real-hardware baseline report recorded |
| F-M3-005 | Medium | VOD track | Per-track GStreamer routing into the muxed output (and routing UI) is not yet wired; configuration contract is complete. | M8 / streaming-UI follow-up (issue [#78](https://github.com/thoser666/Rivulet/issues/78)) | VOD audio lands in a separate track in the recording |
| F-M3-006 | Medium | NDI | NewTek NDI runtime and LAN interoperability are not yet verified; `NdiOutput` contract is complete. | M5 platform / NDI integration follow-up (issue [#77](https://github.com/thoser666/Rivulet/issues/77)) | Real NDI consumer receives a named feed |
| F-M3-007 | Medium | Credential storage | Stream keys are masked in the app layer; an OS-protected credential store should replace ordinary storage before stable. | Security follow-up | Keys stored in an OS-backed store |

There are no Blocker, Critical, or open High findings.

## Decision

- [ ] M3 gate passed without conditions.
- [x] Conditional pass; all remaining findings are Medium/live-evidence and explicitly assigned to follow-up milestones.
- [ ] Failed; release-blocking streaming work remains.

M3 is complete for its functional scope. The milestone must not be used to claim
beta feature parity for live SFU WHIP interop, multitrack transport, VOD routing,
or NDI LAN interoperability; each remains an explicit post-M3 item. The report is
linked from the M3 roadmap gate in `README.md`; release notes should reference it.