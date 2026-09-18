# M6 Creator Toolkit Completion Report

- Commit/PRs: see the per-feature table below; the audio-routing phases landed
  in `f539328` (#155), `8e6f663` (#156), `a9b7731` (#157), `97162da` (#158),
  `fd63781` (#160), `e84d12c` (#161)
- Review date (UTC): 2026-09-15
- Reviewers: Rivulet maintainers (automated evidence review; manual/live
  platform execution noted below)
- Returned scope: chat-driven auto-clips, multi-platform restream, mobile/HTTP
  remote companion, multi-track audio routing
- Overall result: `CONDITIONAL PASS`

## Summary

All four M6 features are implemented, tested, and merged; their issues are
closed (#97, #98, #99, #154). The automated cross-platform baseline passes,
the audio-routing quality-gate checklist in
[`docs/m6-audio-routing.md`](m6-audio-routing.md) is fully checked, and the
resource report
([`docs/m6-audio-resource-report.md`](m6-audio-resource-report.md)) evidences
the budget criterion. Two items require a live hardware/network environment
that a compile-and-unit-test CI cannot reproduce and that the project has not
yet measured on reference hardware; those are explicitly documented below as
honest follow-ups, matching how M3 and M5 handled live-evidence gaps.

- Roadmap checkboxes: **4 / 4 checked** in `README.md` M6 section;
  the milestone is complete and closed at the roadmap level.
- Blocker/Critical findings: **0**
- Open High findings: **0**; the two remaining items are live-evidence
  follow-ups, explicitly assigned below.

## Functional scope delivered (roadmap)

- [x] Chat-driven auto-clips (issue #97) — sliding-window message-rate
  detector plus `!clip` command, threshold/window/cooldown configurable in the
  Stream view, replay-buffer availability checked before saving, i18n EN/DE.
- [x] Multi-platform restream (issue #98) — one pipeline to Twitch, YouTube,
  and Kick; per-target name/platform/ingest/key persisted, independent health
  per target, max 4 targets with duplicate-name protection.
- [x] Mobile & HTTP remote companion (issue #99) — self-contained mobile page,
  explicit LAN bind, LAN-requires-password policy, permission gate for remote
  stream start/stop.
- [x] Multi-track audio routing (issue #154) — Phases 1–5: engine routing
  matrix + versioned `audio_routing_v1` persistence, per-source engine API with
  live volume/mute/filters, routing-aware pipelines (record per-source tracks,
  stream mixed FLV track, dual output), the Mixer-view routing matrix with the
  shared inline mixers in Record/Stream views, and the three capture backends:
  WASAPI process loopback (Windows, Phase 3), PipeWire sink-input capture
  (Linux, Phase 4), and the macOS system-loopback fallback (Phase 5).

## Per-feature delivery

| Feature | Issue | Key PRs | Evidence |
| --- | --- | --- | --- |
| Chat-driven auto-clips | #97 | (merged before this report) | `autoclip_chat_driven_replay_save_is_wired` ci_pinning guard; Stream-view config; i18n EN/DE |
| Multi-platform restream | #98 | (merged before this report) | `restream_multitarget_fanout_is_wired_and_documented` ci_pinning guard |
| Mobile/HTTP remote companion | #99 | (merged before this report) | `m6_remote_companion_is_wired_up_and_pinned` ci_pinning guard |
| Multi-track audio routing | #154 | #155, #156, #157, #158, #160, #161 | `m6_audio_routing_*` ci_pinning guards (phase 1–5 + resource report harness); quality-gate checklist in the spec; [`docs/m6-audio-resource-report.md`](m6-audio-resource-report.md) |

## Automated checks

| Check | Result | Evidence |
| --- | --- | --- |
| Format | PASS | `cargo fmt --all -- --check` |
| Tests | PASS | `rivulet-core` lib suite (872), `ci_pinning` (108), CI build/test matrix (Windows/Linux/macOS) |
| Clippy/lint | PASS | CI Lints job with `-D warnings` |
| CI-specific checks | PASS | actionlint, action-pin table, Beta-Gate, Scorecard |
| Resource-efficiency contract | PASS | `scripts/resource-efficiency-check.py` validates `target/m6-audio-resource-report.json` (gate: PASS; 6 sources p99 ≤ 60 µs, CPU delta ≈ 0, memory ≤ 3.2 MiB) |
| Pre-push hook | PASS | fmt + workspace clippy `-D warnings` + `ci_pinning` + action-pin/parity/release-notes/contrast checks |

## Quality-gate criteria (M6 audio routing)

All criteria in the M6 quality gate (`docs/m6-audio-routing.md`) are checked:

| Criterion | Test / evidence |
| --- | --- |
| Per-source volume/mute/filter round-trip | `routing_config_round_trip_preserves_everything` (`rivulet-core/src/audio_source.rs`) |
| Routing matrix isolates record vs stream | `routed_recording_pipeline_has_one_branch_per_record_routed_source`, `routed_streaming_pipeline_mixes_stream_routed_sources_into_single_flv_track` (`rivulet-core/src/lib.rs`) |
| ≥1 record source → recording starts; 0 → warning, no tracks | `audio_routing_warning_zero_record_routed_sources`; `audio_routing_warning()` engine API pinned |
| Stream mix contains exactly `stream = true` sources | `routed_streaming_pipeline_single_source_skips_mixer` |
| macOS loopback fallback + hint | Phase 5; `m6_audio_routing_phase5_macos_fallback_is_pinned`; `audio_app_fallback_hint` i18n EN/DE |
| Resource budget (5+ sources, full chains) | [`docs/m6-audio-resource-report.md`](m6-audio-resource-report.md) (6 sources, PASS) |
| i18n parity EN/DE | `m6_audio_routing_phase2_gui_surface_is_pinned` parity assertions |

## Findings and explicit follow-ups

The following are **not** silently accepted as parity; each is assigned to a
follow-up and remains visible in the roadmap/documentation.

| ID | Severity | Area | Description | Tracking issue | Retest condition |
| --- | --- | --- | --- | --- | --- |
| F-M6-001 | Medium | macOS audio capture | The system-loopback fallback (Phase 5) is implemented and pinned in source, but no real Mac hardware has verified a live loopback capture end-to-end (the M5 platform parity note records macOS live verification as hardware-blocked). | [#182](https://github.com/thoser666/Rivulet/issues/182) | Loopback capture and per-source routing verified on reference macOS hardware |
| F-M6-002 | Medium | Windows/Linux audio capture | WASAPI process-loopback and PipeWire sink-input capture are implemented and pinned, but long-duration multi-app sessions on reference hardware have not been measured for the M6 resource gate (the harness feeds synthetic frames). | [#183](https://github.com/thoser666/Rivulet/issues/183) | Real-hardware per-app capture baseline recorded against the M6 budget |

There are no Blocker, Critical, or open High findings.

## Decision

- [ ] M6 gate passed without conditions.
- [x] Conditional pass; the remaining findings are Medium/live-evidence and
  explicitly assigned to follow-ups.
- [ ] Failed; release-blocking M6 work remains.

M6 is complete for its functional scope. The milestone must not be used to
claim beta feature parity for real-hardware macOS loopback capture or
long-duration per-app capture baselines; each remains an explicit follow-up,
tracked as [#182](https://github.com/thoser666/Rivulet/issues/182)
(F-M6-001) and [#183](https://github.com/thoser666/Rivulet/issues/183)
(F-M6-002).
The report is linked from the M6 roadmap gate in `README.md` and the M6
quality gate in `docs/milestone-quality-gates.md`; release notes should
reference it.